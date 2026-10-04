//! "October, under 300 euro, by train" — the multi-constraint search, minus the
//! network.
//!
//! Everything here is pure: request validation, month expansion, candidate
//! merge, the shortlist, the weighted factors, ranking, the `why` sentences and
//! the adopted payload. The HTTP calls live in [`crate::upstream`] behind the
//! [`Sources`] trait, which is the split `windows.rs` already states and which
//! is what lets every rule below be tested without a network.
//!
//! ## Two contracts this file keeps deliberately
//!
//! **A guess that looks like a measurement is worse than a blank**
//! (`Packs/travel/ISA.md:57`). A factor that could not be measured is absent
//! from `factors[]` and the remaining weights are re-normalised to sum to 1;
//! `degraded[]` names every input that was missing. A neutral 0.5 for absent
//! climate is a number a reader cannot tell apart from a measured one — the
//! same argument the repo already makes for transit's `unscored_legs`.
//!
//! **No calendar entry title reaches this code path.** The feasibility input is
//! calendar `GET /api/windows`, whose `FeasibleWindow` carries dates and a
//! verdict and nothing else. `crate::windows::day_loads`, `rank` and their
//! `collisions` vector collect entry *titles* and are out of bounds here:
//! `projection.rs` writes an adopted payload verbatim into the operator's
//! cloud-synced vault, so a title that reached this struct would leave the host
//! by a mechanism nobody chose. [`FeasibleWindow`] below has no field to hold
//! one, which is the structural half of the rule; the payload test is the other.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::{PlaceRef, TransportMode};

/// Bumped when a weight, a factor or the scoring arithmetic changes, so two
/// responses are never silently compared across a change. Same mechanism, and
/// the same reason, as `EVALUATOR_REVISION` in `capabilities/comms/src/evaluation.rs`.
///
/// v3 (2026-09-24): weights are clamped to zero or above, and a measured weight
/// total of zero falls back to the default weights. The default weights are v2's.
pub const PLAN_SEARCH_REVISION: &str = "plan-search-v3";

/// The default weights of `plan-search-v2`. All five factors sum to 1.
pub const WEIGHT_BUDGET_FIT: f64 = 0.30;
pub const WEIGHT_FEASIBILITY: f64 = 0.25;
pub const WEIGHT_SEASON: f64 = 0.15;
pub const WEIGHT_EVENTS: f64 = 0.20;
pub const WEIGHT_RETROSPECTIVE: f64 = 0.10;

/// The fifth factor slot, computed in v2.
///
/// travel-season-cost publishes `GET /api/retrospectives/summary` — how a past
/// trip to this destination actually went. In v2, this takes this key and this label,
/// weights are read from the traveler profile (or default SoftWeights), and
/// `PLAN_SEARCH_REVISION` is `plan-search-v2`.
pub const FACTOR_RETROSPECTIVE: (&str, &str) = ("retrospective", "How the last trip here went");

/// The soft ranking weights, matching `capabilities/traveler` profile's `soft` section.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoftWeights {
    pub budget_fit: f64,
    pub feasibility: f64,
    pub season: f64,
    pub events: f64,
    pub retrospective: f64,
}

impl SoftWeights {
    /// Every weight at `>= 0`, with a NaN read as 0. The traveler profile refuses
    /// a negative weight, but this file also takes weights from a request.
    pub fn clamped(self) -> Self {
        let floor = |w: f64| if w > 0.0 { w } else { 0.0 };
        Self {
            budget_fit: floor(self.budget_fit),
            feasibility: floor(self.feasibility),
            season: floor(self.season),
            events: floor(self.events),
            retrospective: floor(self.retrospective),
        }
    }
}

impl Default for SoftWeights {
    fn default() -> Self {
        Self {
            budget_fit: WEIGHT_BUDGET_FIT,
            feasibility: WEIGHT_FEASIBILITY,
            season: WEIGHT_SEASON,
            events: WEIGHT_EVENTS,
            retrospective: WEIGHT_RETROSPECTIVE,
        }
    }
}

/// The longest span a search may cover — the same bound `flight_when` already
/// enforces, so a month and a hand-typed window cannot mean different things.
pub const MAX_WINDOW_DAYS: i64 = 42;

/// How far from a destination an opportunity still counts as "at" it.
///
/// A third home for great-circle distance in this repo, and deliberately not a
/// dependency: `capabilities/places/src/backfill.rs` (`haversine_km`) and
/// `dashboard/src/lib/travel/travel-candidates.ts` hold the other two, and a
/// capability does not link another capability's crate
/// (`capabilities/places/Cargo.toml` states that rule for the fingerprint lib).
pub const EVENT_RADIUS_KM: f64 = 75.0;

/// Default and bound for how many destinations get a fare probe. Pricing is the
/// only expensive step, so this is the number that decides the wall time.
pub const DEFAULT_MAX_CANDIDATES: usize = 8;
pub const MAX_MAX_CANDIDATES: usize = 20;

/// What a caller sends. `request_schema` publishes this shape on `/routes`.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PlanSearchRequest {
    pub origin: PlaceRef,
    /// `YYYY-MM`. Exactly one of `month` and `date_window`.
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub date_window: Option<DateWindow>,
    #[serde(default)]
    pub min_days: Option<u32>,
    #[serde(default)]
    pub budget_cents: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub modes: Vec<TransportMode>,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub max_candidates: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct DateWindow {
    pub from: String,
    /// Inclusive, like `flight_when`'s `date_to` and unlike calendar's
    /// exclusive `ends_before`. The conversion happens once, on the way out.
    pub to: String,
}

/// Where the searched window came from. `Month` is the shape that cannot
/// proceed without the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Month,
    Caller,
}

#[derive(Debug, Clone)]
pub struct ValidRequest {
    pub origin: PlaceRef,
    pub from: String,
    /// Inclusive.
    pub to: String,
    pub kind: WindowKind,
    pub min_days: u32,
    pub budget_cents: Option<i64>,
    pub currency: String,
    pub modes: Vec<TransportMode>,
    pub interests: String,
    pub max_candidates: usize,
}

impl ValidRequest {
    /// The primary transport mode a fare is priced for. Train unless the caller
    /// named something else first, because transit is the only fare source
    /// wired into this search today.
    pub fn mode(&self) -> TransportMode {
        self.modes.first().cloned().unwrap_or(TransportMode::Train)
    }

    /// The calendar month a climate normal is asked for.
    pub fn month_number(&self) -> u32 {
        self.from
            .get(5..7)
            .and_then(|m| m.parse().ok())
            .unwrap_or(1)
    }
}

/// Validate, naming the field that is wrong.
///
/// A 400 that says "bad request" sends the caller to the source; a 400 that
/// names the field sends it back to its own body. Same rule as
/// `trips::store::validate_payload`.
pub fn validate(request: PlanSearchRequest) -> Result<ValidRequest, String> {
    let (from, to, kind) = match (&request.month, &request.date_window) {
        (Some(_), Some(_)) => {
            return Err("send month or date_window, not both".into());
        }
        (None, None) => {
            return Err("one of month (YYYY-MM) or date_window {from,to} is required".into());
        }
        (Some(month), None) => {
            let (from, to) =
                expand_month(month).ok_or_else(|| format!("month is not YYYY-MM: {month}"))?;
            (from, to, WindowKind::Month)
        }
        (None, Some(window)) => (window.from.clone(), window.to.clone(), WindowKind::Caller),
    };

    let Some(from_day) = crate::windows::day_number(&from) else {
        return Err(format!("date_window.from is not ISO: {from}"));
    };
    let Some(to_day) = crate::windows::day_number(&to) else {
        return Err(format!("date_window.to is not ISO: {to}"));
    };
    if to_day < from_day {
        return Err("date_window.from must be on or before date_window.to".into());
    }
    if to_day - from_day > MAX_WINDOW_DAYS {
        return Err(format!(
            "the window spans {} days; the bound is {MAX_WINDOW_DAYS}",
            to_day - from_day
        ));
    }
    if let Some(budget) = request.budget_cents {
        if budget <= 0 {
            return Err("budget_cents must be above zero".into());
        }
    }
    if request.origin.id.trim().is_empty() && request.origin.name.trim().is_empty() {
        return Err("origin needs an id or a name".into());
    }

    // Re-rendered from the day numbers, not kept as the caller wrote them.
    // `day_number` checks the shape and stops at the day field, so
    // `{"from": "2026-01-01&min_days=0"}` passes both checks above and then goes
    // verbatim into `upstream::calendar_windows`' query string and into
    // `starts_on`, which `projection.rs` writes into the operator's vault. The
    // same clip the flight-when handler now does, at the one place this route
    // parses a date. `iso_of_day_number` is `day_number`'s documented inverse, so
    // a well-formed date round-trips to itself.
    let from = crate::windows::iso_of_day_number(from_day);
    let to = crate::windows::iso_of_day_number(to_day);

    let span_days = (to_day - from_day + 1) as u32;
    let min_days = request.min_days.unwrap_or(3).clamp(1, span_days);
    let max_candidates = request
        .max_candidates
        .unwrap_or(DEFAULT_MAX_CANDIDATES)
        .clamp(1, MAX_MAX_CANDIDATES);

    Ok(ValidRequest {
        origin: request.origin,
        from,
        to,
        kind,
        min_days,
        budget_cents: request.budget_cents,
        currency: request.currency.unwrap_or_else(|| "EUR".into()),
        modes: request.modes,
        interests: request.interests,
        max_candidates,
    })
}

/// `YYYY-MM` to its first and last day, inclusive.
pub fn expand_month(month: &str) -> Option<(String, String)> {
    let (year, number) = month.split_once('-')?;
    let year: i32 = year.parse().ok()?;
    let number: u32 = number.parse().ok()?;
    if !(1..=12).contains(&number) || !(1970..=9999).contains(&year) {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = match number {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        _ => 28,
    };
    Some((
        format!("{year:04}-{number:02}-01"),
        format!("{year:04}-{number:02}-{length:02}"),
    ))
}

/// A run of days travel is possible in, as calendar serves it.
///
/// A deliberate subset of `capabilities/calendar/src/correlate/events.rs`'s
/// `FeasibleWindow`, and deliberately NOT a shared type: there is no field here
/// that could hold a calendar entry title, so no future edit to the producer can
/// carry one across this boundary by accident.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FeasibleWindow {
    pub starts_on: String,
    /// Exclusive, as calendar serves it.
    pub ends_before: String,
    #[serde(default)]
    pub days: Vec<String>,
    /// `free`, `needs-travel-day` or `conflicts` (calendar's kebab-case
    /// `Feasibility`). Empty when the window is the caller's own, unchecked.
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub days_needing_travel_day: Vec<String>,
}

/// How each upstream answered. `"ok"`, `"absent"`, or `"error: <reason>"`.
///
/// The response says what it reached rather than what it assumed, which is the
/// half `flight_when` leaves out: it swallows a calendar failure with `.ok()`
/// and every day comes back `Free`, so a fully committed week ranks
/// cheapest-first with nothing in the body saying the calendar was never read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reach {
    pub calendar: String,
    pub places: String,
    pub transit: String,
    pub scouting: String,
    pub climate: String,
}

impl Default for Reach {
    fn default() -> Self {
        Self {
            calendar: "absent".into(),
            places: "absent".into(),
            transit: "absent".into(),
            scouting: "absent".into(),
            climate: "absent".into(),
        }
    }
}

pub fn reached(outcome: &Result<impl Sized, String>) -> String {
    match outcome {
        Ok(_) => "ok".into(),
        Err(reason) => format!("error: {reason}"),
    }
}

/// One place a search may propose, before anything expensive happens to it.
#[derive(Debug, Clone, PartialEq)]
pub struct DestinationCandidate {
    pub place_id: String,
    pub destination: PlaceRef,
    /// `places`, `plan` or `scouting` — every source that offered this place.
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateEvent {
    pub id: String,
    pub title: String,
    pub starts_at: Option<String>,
    pub url: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    #[serde(default)]
    pub distance_km: Option<f64>,
}

/// A climate normal, reduced to what a rank needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SeasonScore {
    pub month: u32,
    /// 0..1.
    pub score: f64,
    #[serde(default)]
    pub best_month: Option<u32>,
}

/// One visible reason a candidate scored what it scored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreFactor {
    pub key: String,
    pub label: String,
    pub score: f64,
    pub weight: f64,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedCandidate {
    pub destination: PlaceRef,
    pub place_id: String,
    pub window: FeasibleWindow,
    pub mode: TransportMode,
    pub estimated_cost_cents: Option<i64>,
    pub currency: String,
    /// `transit-search` when a fare was found, `unpriced` when none was.
    pub cost_basis: String,
    pub priced_at: Option<String>,
    pub events: Vec<CandidateEvent>,
    pub season: Option<SeasonScore>,
    /// `None` when NO factor could be measured. A `0.0` there would read as a
    /// measured worst candidate, which is the same defect as a neutral 0.5 for
    /// an absent factor (`Packs/travel/ISA.md`: a guess that looks like a
    /// measurement is worse than a blank).
    pub score: Option<f64>,
    pub factors: Vec<ScoreFactor>,
    /// Name-free by construction. C2 never leaves places as a row: this is
    /// derived from a count and an overlap, it reaches the browser, and it is
    /// never written into the adopted payload.
    pub companion_hint: Option<String>,
    pub why: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryEcho {
    pub from: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub time: String,
    pub budget_cents: Option<i64>,
    pub currency: String,
    pub modes: Vec<TransportMode>,
    pub interests: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanSearchResult {
    pub revision: String,
    /// `calendar` when the feasible windows were measured, `caller` when the
    /// caller's own window is being used because calendar was unreachable.
    pub window_source: String,
    pub windows: Vec<FeasibleWindow>,
    pub reach: Reach,
    pub degraded: Vec<String>,
    pub considered: usize,
    pub priced: usize,
    pub unpriced: usize,
    pub candidates: Vec<RankedCandidate>,
    pub query: QueryEcho,
    pub observed_at: String,
}

/// What a month search is told when calendar is down.
pub const NO_CALENDAR_FOR_A_MONTH: &str =
    "calendar unavailable — a month search needs feasible windows";

/// What a search says when it stopped pricing because its budget ran out.
pub const PRICING_BUDGET_SPENT: &str = "pricing budget exhausted";

/// What a search says when the budget ran out before the companion hints.
pub const COMPANION_HINTS_CUT: &str = "companion hints skipped: the search spent its budget first";

/// The transport modes the fare source behind this search can price.
///
/// `HttpSources::price` asks transit's `/api/suggest` and `/api/search`, which
/// are rail. Flights are priced by `crate::kiwi` on a different route and no
/// coach, ferry or car fare exists in this repository at all, so anything else
/// would be a rail fare wearing another mode's name.
pub fn transit_prices(mode: &TransportMode) -> bool {
    matches!(mode, TransportMode::Train)
}

/// What `degraded[]` and the `why` lines say about a mode nothing can price.
pub fn no_fare_source_for(mode: &TransportMode) -> String {
    format!("no fare source covers {} in this search", mode_slug(mode))
}

/// Every upstream the composition reads, behind one trait.
///
/// A trait rather than five function arguments so that the composition below is
/// one testable unit: the deadline, the shortlist cap and the degradation paths
/// are all properties of the ORDER of these calls, and a test that cannot drive
/// the order cannot check them.
pub trait Sources {
    fn calendar_windows(
        &self,
        from: &str,
        to: &str,
        min_days: u32,
    ) -> Result<Vec<FeasibleWindow>, String>;
    fn cities(&self) -> Result<Vec<DestinationCandidate>, String>;
    fn opportunities(&self) -> Result<Vec<CandidateEvent>, String>;
    /// The batch form, called once per search. Any non-200, including a 404
    /// from a deployment where the climate route does not exist yet, is an
    /// `Err` and sets `reach.climate = "absent"`.
    fn climate(
        &self,
        place_ids: &[String],
        month: u32,
    ) -> Result<HashMap<String, SeasonScore>, String>;
    /// `Ok(None)` means the fare source answered and had no price for this
    /// destination — a measurement, not a failure. `Err` means it could not be
    /// asked.
    fn price(
        &self,
        origin: &PlaceRef,
        destination: &DestinationCandidate,
        depart_iso: &str,
    ) -> Result<Option<i64>, String>;
    /// `(known_companions, overlap_days)`. Never a row, never a name.
    fn presence(
        &self,
        latitude: f64,
        longitude: f64,
        from: &str,
        to: &str,
    ) -> Result<(u32, u32), String>;
    /// True once the job has spent [`crate::jobs::JOB_DEADLINE_S`].
    fn deadline_spent(&self) -> bool;
    /// Now, as an ISO instant, for `priced_at` and `observed_at`.
    fn observed_at(&self) -> String;
    /// The soft ranking weights from traveler profile, if available.
    fn soft_weights(&self) -> Option<SoftWeights> {
        None
    }
    /// Past trip retrospectives summarized by destination key.
    fn retrospectives(&self) -> HashMap<String, crate::retrospective::DestinationFactor> {
        HashMap::new()
    }
}

/// The whole search, with every network call behind `sources`.
pub fn compose(
    request: &ValidRequest,
    plan_destinations: Vec<DestinationCandidate>,
    sources: &dyn Sources,
) -> Result<PlanSearchResult, String> {
    let mut reach = Reach::default();
    let mut degraded: Vec<String> = Vec::new();

    // 1. Calendar first, because calendar -> transit is the load-bearing order:
    //    a fare for a day the operator cannot travel is not an answer.
    let windows_result = sources.calendar_windows(&request.from, &request.to, request.min_days);
    reach.calendar = reached(&windows_result);
    let (windows, window_source, calendar_reached) = match windows_result {
        Ok(windows) => (windows, "calendar", true),
        Err(_) if request.kind == WindowKind::Month => {
            // NOT all-free. `flight_when` degrades that way by a stated
            // contract; this route follows the ISA instead.
            return Err(NO_CALENDAR_FOR_A_MONTH.into());
        }
        Err(_) => {
            degraded.push("calendar".into());
            (
                vec![FeasibleWindow {
                    starts_on: request.from.clone(),
                    ends_before: exclusive_end(&request.to),
                    days: Vec::new(),
                    verdict: String::new(),
                    days_needing_travel_day: Vec::new(),
                }],
                "caller",
                false,
            )
        }
    };
    let Some(window) = best_window(&windows).cloned() else {
        return Err(format!(
            "no feasible window of at least {} day(s) between {} and {}",
            request.min_days, request.from, request.to
        ));
    };

    // 2. Candidates. Every source that answers contributes; one that does not
    //    is named rather than assumed empty.
    let cities_result = sources.cities();
    reach.places = reached(&cities_result);
    if cities_result.is_err() {
        degraded.push("places".into());
    }
    let opportunities_result = sources.opportunities();
    reach.scouting = reached(&opportunities_result);
    if opportunities_result.is_err() {
        degraded.push("scouting".into());
    }
    let opportunities = opportunities_result.as_ref().ok();

    let mut candidates = merge_candidates(cities_result.unwrap_or_default(), plan_destinations);
    // The origin is not a destination. Left in, a Berlin -> Berlin candidate is
    // ranked, takes one of the few fare probes and comes back unpriced anyway,
    // because `price` answers `Ok(None)` when both ends resolve to one station.
    candidates.retain(|candidate| !is_origin(&request.origin, candidate));
    if candidates.is_empty() {
        return Err(
            "no destination candidates — places, the plan list and scouting all came back empty"
                .into(),
        );
    }
    let considered = candidates.len();

    // 3. Events per candidate, then the cheap shortlist. Only the shortlist is
    //    priced, which is the number that decides the wall time.
    let events_by_place: HashMap<String, Vec<CandidateEvent>> = match opportunities {
        Some(list) => events_near(&candidates, list, &window),
        None => HashMap::new(),
    };
    candidates.sort_by(|a, b| {
        cheap_rank(b, &events_by_place)
            .total_cmp(&cheap_rank(a, &events_by_place))
            .then(a.destination.name.cmp(&b.destination.name))
    });
    candidates.truncate(request.max_candidates);

    // 4. Climate: the batch form, once per search, over the shortlist only.
    let place_ids: Vec<String> = candidates.iter().map(|c| c.place_id.clone()).collect();
    let climate_result = sources.climate(&place_ids, request.month_number());
    reach.climate = reached(&climate_result);
    if climate_result.is_err() {
        degraded.push("climate".into());
    }
    let climate = climate_result.unwrap_or_default();

    // 5. Fares, one per shortlisted candidate, until the deadline is spent.
    //    Only for a mode the fare source actually covers: a rail fare labelled
    //    `mode: "flight"` measures the wrong thing under the right name.
    let mode = request.mode();
    let priceable = transit_prices(&mode);
    let mut priced_cents: HashMap<String, Option<i64>> = HashMap::new();
    let mut price_errors: HashMap<String, String> = HashMap::new();
    let mut transit_asked = false;
    let mut transit_failed: Option<String> = None;
    let mut budget_spent = false;
    if priceable {
        for candidate in &candidates {
            if sources.deadline_spent() {
                budget_spent = true;
                break;
            }
            transit_asked = true;
            match sources.price(&request.origin, candidate, &window.starts_on) {
                Ok(cents) => {
                    priced_cents.insert(candidate.place_id.clone(), cents);
                }
                Err(reason) => {
                    transit_failed.get_or_insert_with(|| reason.clone());
                    price_errors.insert(candidate.place_id.clone(), reason);
                }
            }
        }
    } else {
        degraded.push(no_fare_source_for(&mode));
    }
    reach.transit = match (&transit_failed, transit_asked) {
        (Some(reason), _) => format!("error: {reason}"),
        (None, true) => "ok".into(),
        (None, false) => "absent".into(),
    };
    if transit_failed.is_some() {
        degraded.push("transit".into());
    }
    if budget_spent {
        degraded.push(PRICING_BUDGET_SPENT.into());
    }

    // 6. The companion hint. A count and an overlap, never a row.
    //    `to` is INCLUSIVE on the places route (its `from`/`to` are both, and
    //    its span arithmetic adds one), while `ends_before` is exclusive as
    //    calendar serves it, so the window's last day is what goes on the wire.
    //    Passing `ends_before` asks about one day too many and the hint
    //    over-reports a seven-day window as eight.
    let presence_to = inclusive_end(&window.ends_before);
    let mut hints: HashMap<String, String> = HashMap::new();
    let mut presence_failed = false;
    let mut hints_cut = false;
    for candidate in &candidates {
        // The same budget the pricing loop keeps. Without it a search that has
        // already spent 180 s can add up to `max_candidates` more calls, and
        // the page gives up on a job that is still alive.
        if sources.deadline_spent() {
            hints_cut = true;
            break;
        }
        let (Some(latitude), Some(longitude)) = (
            candidate.destination.latitude,
            candidate.destination.longitude,
        ) else {
            continue;
        };
        match sources.presence(latitude, longitude, &window.starts_on, &presence_to) {
            Ok((0, _)) => {}
            Ok((people, overlap_days)) => {
                hints.insert(
                    candidate.place_id.clone(),
                    companion_hint(people, overlap_days),
                );
            }
            Err(_) => presence_failed = true,
        }
    }
    if presence_failed && !degraded.iter().any(|d| d == "places") {
        degraded.push("companion register".into());
    }
    if hints_cut {
        degraded.push(COMPANION_HINTS_CUT.into());
    }

    let observed_at = sources.observed_at();
    let weights = sources.soft_weights().unwrap_or_default();
    let retrospectives = sources.retrospectives();
    let mut ranked: Vec<RankedCandidate> = candidates
        .iter()
        .map(|candidate| {
            let cost = priced_cents.get(&candidate.place_id).copied().flatten();
            let events = events_by_place
                .get(&candidate.place_id)
                .cloned()
                .unwrap_or_default();
            let season = climate.get(&candidate.place_id).cloned();
            let normalized_name = crate::store::normalize_place_name(&candidate.destination.name);
            let retro = retrospectives.get(&normalized_name).cloned();
            let inputs = ScoringInputs {
                budget_cents: request.budget_cents,
                estimated_cost_cents: cost,
                window: calendar_reached.then(|| window.clone()),
                season: season.clone(),
                climate_reached: reach.climate == "ok",
                events: opportunities.map(|_| events.len()),
                retrospective: retro,
                weights,
            };
            let factors = factors(&inputs);
            RankedCandidate {
                destination: candidate.destination.clone(),
                place_id: candidate.place_id.clone(),
                window: window.clone(),
                mode: mode.clone(),
                estimated_cost_cents: cost,
                currency: request.currency.clone(),
                cost_basis: if cost.is_some() {
                    "transit-search".into()
                } else {
                    "unpriced".into()
                },
                priced_at: cost.is_some().then(|| observed_at.clone()),
                events,
                season,
                score: weighted_score(&factors),
                companion_hint: hints.get(&candidate.place_id).cloned(),
                why: why(
                    candidate,
                    cost,
                    request.budget_cents,
                    &request.currency,
                    &mode,
                    price_errors.get(&candidate.place_id).map(String::as_str),
                    budget_spent && !priced_cents.contains_key(&candidate.place_id),
                    &factors,
                ),
                factors,
            }
        })
        .collect();
    rank(&mut ranked);

    let priced = ranked
        .iter()
        .filter(|c| c.estimated_cost_cents.is_some())
        .count();
    Ok(PlanSearchResult {
        revision: PLAN_SEARCH_REVISION.into(),
        window_source: window_source.into(),
        windows,
        reach,
        degraded,
        considered,
        priced,
        unpriced: ranked.len() - priced,
        query: QueryEcho {
            from: request.origin.clone(),
            destinations: ranked.iter().map(|c| c.destination.clone()).collect(),
            time: format!("{}/{}", request.from, request.to),
            budget_cents: request.budget_cents,
            currency: request.currency.clone(),
            modes: request.modes.clone(),
            interests: request.interests.clone(),
        },
        candidates: ranked,
        observed_at,
    })
}

/// A count and an overlap, put into a sentence with no name in it.
///
/// The precedent is in `trips::store`'s own `booking` payload, which records
/// `traveler_name_present` as a boolean "because whose name is on a ticket is
/// personal data this repo has no reason to hold".
pub fn companion_hint(people: u32, overlap_days: u32) -> String {
    let who = if people == 1 {
        "a known companion is".to_string()
    } else {
        format!("{people} known companions are")
    };
    let how_long = if overlap_days == 1 {
        "1 day of this window".to_string()
    } else {
        format!("{overlap_days} days of this window")
    };
    format!("{who} near this destination for {how_long}")
}

/// One place per place id, remembering every source that offered it.
///
/// De-duplicated on the lowercased name as well as the id, because trips' own
/// plan destinations carry ids minted by whoever typed them while places' rows
/// carry registry ids: the same city reaches this function under two ids and
/// would otherwise take two of the eight fare probes.
///
/// Scouting does not add candidates. An opportunity carries a city string and
/// only rarely a coordinate, and a destination nobody can price or locate is
/// not a candidate — what scouting does is raise a candidate that already
/// exists, through [`events_near`] and [`cheap_rank`].
pub fn merge_candidates(
    cities: Vec<DestinationCandidate>,
    plan_destinations: Vec<DestinationCandidate>,
) -> Vec<DestinationCandidate> {
    // A BTreeMap so the merge is deterministic: two runs over the same data
    // must shortlist the same eight places.
    let mut merged: BTreeMap<String, DestinationCandidate> = BTreeMap::new();
    let mut key_of_name: HashMap<String, String> = HashMap::new();
    for candidate in cities.into_iter().chain(plan_destinations) {
        let name = candidate.destination.name.trim().to_lowercase();
        let key = key_of_name
            .entry(name)
            .or_insert_with(|| candidate.place_id.clone())
            .clone();
        match merged.get_mut(&key) {
            Some(existing) => {
                for source in candidate.sources {
                    if !existing.sources.contains(&source) {
                        existing.sources.push(source);
                    }
                }
                // A place with coordinates beats one without: distance is what
                // the events factor and the presence read both need.
                if existing.destination.latitude.is_none()
                    && candidate.destination.latitude.is_some()
                {
                    existing.destination = candidate.destination;
                    // The id and the destination are one fact. Keeping the
                    // first candidate's id here wrote `options[].id` into the
                    // adopted payload naming a place no `destination` in the
                    // same payload has, and that payload is projected into the
                    // operator's vault verbatim.
                    existing.place_id = existing.destination.id.clone();
                }
            }
            None => {
                merged.insert(key, candidate);
            }
        }
    }
    merged.into_values().collect()
}

/// The opportunities inside the window and within [`EVENT_RADIUS_KM`] of each
/// candidate, keyed by place id.
pub fn events_near(
    candidates: &[DestinationCandidate],
    opportunities: &[CandidateEvent],
    window: &FeasibleWindow,
) -> HashMap<String, Vec<CandidateEvent>> {
    let mut found: HashMap<String, Vec<CandidateEvent>> = HashMap::new();
    for candidate in candidates {
        let (Some(latitude), Some(longitude)) = (
            candidate.destination.latitude,
            candidate.destination.longitude,
        ) else {
            continue;
        };
        for event in opportunities {
            let Some(starts_at) = event.starts_at.as_deref() else {
                continue;
            };
            // `get`, never a byte slice. `starts_at` is external text from
            // whatever feed scouting read, and `&starts_at[..10]` panics inside
            // a multi-byte character -- which used to take the whole job with
            // it, and `jobs::open` never evicts a job that is still `Running`.
            let Some(day) = starts_at.get(..10).filter(|day| is_iso_day(day)) else {
                continue;
            };
            if day < window.starts_on.as_str() || day >= window.ends_before.as_str() {
                continue;
            }
            let (Some(event_latitude), Some(event_longitude)) = (event.latitude, event.longitude)
            else {
                continue;
            };
            let distance = haversine_km((latitude, longitude), (event_latitude, event_longitude));
            if distance <= EVENT_RADIUS_KM {
                let mut event = event.clone();
                event.distance_km = Some((distance * 10.0).round() / 10.0);
                found
                    .entry(candidate.place_id.clone())
                    .or_default()
                    .push(event);
            }
        }
    }
    found
}

/// What orders the shortlist before anything is priced: an event in the window
/// is the strongest signal a destination is worth a fare call, then a place the
/// operator already put on a plan.
pub fn cheap_rank(
    candidate: &DestinationCandidate,
    events: &HashMap<String, Vec<CandidateEvent>>,
) -> f64 {
    let event_count = events.get(&candidate.place_id).map_or(0, Vec::len) as f64;
    let on_a_plan = if candidate.sources.iter().any(|s| s == "plan") {
        1.0
    } else {
        0.0
    };
    let locatable = if candidate.destination.latitude.is_some() {
        0.5
    } else {
        0.0
    };
    event_count.min(3.0) * 2.0 + on_a_plan + locatable
}

/// Everything the score needs, with `None` meaning "not measured".
#[derive(Debug, Clone, Default)]
pub struct ScoringInputs {
    pub budget_cents: Option<i64>,
    pub estimated_cost_cents: Option<i64>,
    /// `None` when calendar was not reached, so feasibility is not scored.
    pub window: Option<FeasibleWindow>,
    pub season: Option<SeasonScore>,
    pub climate_reached: bool,
    /// `None` when scouting was not reached. `Some(0)` is a measured zero.
    pub events: Option<usize>,
    /// Past retrospective for this destination, if recorded.
    pub retrospective: Option<crate::retrospective::DestinationFactor>,
    /// The weights to apply to factors.
    pub weights: SoftWeights,
}

/// The factors that could be measured, with their weights re-normalised to 1.
///
/// A factor that could not be measured is ABSENT. It is never a neutral 0.5,
/// because a reader cannot tell that apart from a measured 0.5.
///
/// The caller's weights are clamped to `>= 0` first; a NaN counts as 0. When the
/// measured factors then carry no weight at all, the default weights stand in:
/// a total of 0 would score every candidate 0, which reads as "all equally bad"
/// rather than "the weights said nothing".
pub fn factors(inputs: &ScoringInputs) -> Vec<ScoreFactor> {
    let mut factors = unnormalised_factors(inputs, inputs.weights.clamped());
    let mut total: f64 = factors.iter().map(|f| f.weight).sum();
    if total.is_nan() || total <= 0.0 {
        factors = unnormalised_factors(inputs, SoftWeights::default());
        total = factors.iter().map(|f| f.weight).sum();
    }
    if total > 0.0 {
        for factor in &mut factors {
            factor.weight /= total;
        }
    }
    factors
}

fn unnormalised_factors(inputs: &ScoringInputs, weights: SoftWeights) -> Vec<ScoreFactor> {
    let mut factors: Vec<ScoreFactor> = Vec::new();

    if let (Some(budget), Some(cost)) = (inputs.budget_cents, inputs.estimated_cost_cents) {
        let ratio = cost as f64 / budget as f64;
        // Inside the budget scores 0.5..1.0 and cheaper is better; outside it
        // scores below 0.5 and falls off with the overrun. The break at 0.5 is
        // what makes "under budget" and "over budget" two visibly different
        // answers rather than one continuum.
        let score = if ratio <= 1.0 {
            1.0 - 0.5 * ratio
        } else {
            (0.5 / ratio).clamp(0.0, 0.5)
        };
        factors.push(ScoreFactor {
            key: "budget_fit".into(),
            label: "Fit to budget".into(),
            score,
            weight: weights.budget_fit,
            rationale: if ratio <= 1.0 {
                format!("{}% of the budget", (ratio * 100.0).round() as i64)
            } else {
                format!(
                    "{}% over the budget",
                    ((ratio - 1.0) * 100.0).round() as i64
                )
            },
        });
    }

    if let Some(window) = &inputs.window {
        let travel_days = window.days_needing_travel_day.len();
        let base = match window.verdict.as_str() {
            "free" => 1.0,
            "needs-travel-day" => 0.6,
            "conflicts" => 0.1,
            _ => 0.5,
        };
        let score = (base - 0.05 * travel_days as f64).clamp(0.0, 1.0);
        factors.push(ScoreFactor {
            key: "feasibility".into(),
            label: "Fits the calendar".into(),
            score,
            weight: weights.feasibility,
            rationale: if travel_days == 0 {
                format!("calendar verdict {}", window.verdict)
            } else {
                format!(
                    "calendar verdict {}, {travel_days} day(s) would cost a travel day",
                    window.verdict
                )
            },
        });
    }

    if inputs.climate_reached {
        if let Some(season) = &inputs.season {
            factors.push(ScoreFactor {
                key: "season".into(),
                label: "Right time of year".into(),
                score: season.score.clamp(0.0, 1.0),
                weight: weights.season,
                rationale: match season.best_month {
                    Some(best) if best == season.month => "the best month here".into(),
                    Some(best) => format!("month {}; month {best} is the best here", season.month),
                    None => format!("climate normal for month {}", season.month),
                },
            });
        }
    }

    if let Some(count) = inputs.events {
        factors.push(ScoreFactor {
            key: "events".into(),
            label: "Something is on".into(),
            score: (count as f64 / 3.0).clamp(0.0, 1.0),
            weight: weights.events,
            rationale: match count {
                0 => "nothing scouting knows about in the window".into(),
                1 => "1 event in the window".into(),
                n => format!("{n} events in the window"),
            },
        });
    }

    if let Some(dest_factor) = &inputs.retrospective {
        if dest_factor.n > 0 {
            let (min, max) = crate::retrospective::BOUNDS;
            let normalized_score = if max > min {
                ((dest_factor.factor - min) / (max - min)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let rating_desc = if dest_factor.mean_again > 0.25 {
                "favorably"
            } else if dest_factor.mean_again < -0.25 {
                "unfavorably"
            } else {
                "neutral"
            };
            factors.push(ScoreFactor {
                key: FACTOR_RETROSPECTIVE.0.into(),
                label: FACTOR_RETROSPECTIVE.1.into(),
                score: normalized_score,
                weight: weights.retrospective,
                rationale: format!(
                    "rated {} across {} past trip(s)",
                    rating_desc, dest_factor.n
                ),
            });
        }
    }

    factors
}

/// The weighted sum, or `None` when there was nothing to weigh.
///
/// A candidate whose every factor was dropped is unscored, not worst: a reader
/// cannot tell a computed `0.0` from a blank one, and the rest of this file
/// drops what it could not measure rather than filling it in.
pub fn weighted_score(factors: &[ScoreFactor]) -> Option<f64> {
    if factors.is_empty() {
        return None;
    }
    let score = factors
        .iter()
        .map(|f| f.score.clamp(0.0, 1.0) * f.weight)
        .sum::<f64>()
        .clamp(0.0, 1.0);
    // Negative zero is a real f64 that survives `clamp` (`-0.0 == 0.0`, so
    // clamp returns self) and reaches a response body as `-0.0`, which reads
    // as a bug.
    Some(if score == 0.0 { 0.0 } else { score })
}

/// Priced candidates first, then by score.
///
/// Unpriced last mirrors `windows::rank`, which orders by load band before
/// price: a candidate whose fare nobody could find is not a cheap candidate,
/// and sorting it among the priced ones would read as one.
pub fn rank(candidates: &mut [RankedCandidate]) {
    candidates.sort_by(|a, b| {
        a.estimated_cost_cents
            .is_none()
            .cmp(&b.estimated_cost_cents.is_none())
            // An unscored candidate sorts last inside its band, for the same
            // reason an unpriced one does: nobody measured it.
            .then(
                b.score
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&a.score.unwrap_or(f64::NEG_INFINITY)),
            )
            .then(
                a.estimated_cost_cents
                    .unwrap_or(i64::MAX)
                    .cmp(&b.estimated_cost_cents.unwrap_or(i64::MAX)),
            )
            .then(a.place_id.cmp(&b.place_id))
    });
}

#[allow(clippy::too_many_arguments)]
fn why(
    candidate: &DestinationCandidate,
    cost: Option<i64>,
    budget_cents: Option<i64>,
    currency: &str,
    mode: &TransportMode,
    price_error: Option<&str>,
    cut_by_deadline: bool,
    factors: &[ScoreFactor],
) -> Vec<String> {
    let mut why = Vec::new();
    match (cost, budget_cents) {
        (Some(cost), Some(budget)) if cost > budget => why.push(format!(
            "{} over the budget",
            money(cost - budget, currency)
        )),
        // The mode the fare was priced for, not a hardcoded one: the response
        // carries `mode` too, and the two must not contradict each other.
        (Some(cost), _) => why.push(format!(
            "{} one way by {}",
            money(cost, currency),
            mode_slug(mode)
        )),
        (None, _) if !transit_prices(mode) => {
            why.push(format!("not priced: {}", no_fare_source_for(mode)))
        }
        (None, _) if cut_by_deadline => {
            why.push("not priced: the search spent its pricing budget first".into())
        }
        (None, _) => why.push(match price_error {
            Some(reason) => format!("not priced: {reason}"),
            None => "not priced: no station suggestion or no fare for this destination".into(),
        }),
    }
    if let Some(events) = factors.iter().find(|f| f.key == "events") {
        why.push(events.rationale.clone());
    }
    if let Some(feasibility) = factors.iter().find(|f| f.key == "feasibility") {
        why.push(feasibility.rationale.clone());
    } else {
        why.push("the calendar was not reached, so feasibility was not scored".into());
    }
    if !factors.iter().any(|f| f.key == "season") {
        why.push("no climate normal for this place, so the season was not scored".into());
    }
    if let Some(retro) = factors.iter().find(|f| f.key == FACTOR_RETROSPECTIVE.0) {
        why.push(retro.rationale.clone());
    }
    if candidate.sources.iter().any(|s| s == "plan") {
        why.push("already a destination on one of your plans".into());
    }
    why
}

fn money(cents: i64, currency: &str) -> String {
    format!("{}.{:02} {currency}", cents / 100, (cents % 100).abs())
}

/// Calendar's window end is exclusive; a caller's `to` is inclusive.
fn exclusive_end(inclusive: &str) -> String {
    crate::windows::day_number(inclusive)
        .map(|day| crate::windows::iso_of_day_number(day + 1))
        .unwrap_or_else(|| inclusive.to_string())
}

/// [`exclusive_end`]'s inverse: the window's last day, for a route that counts
/// its `to` inclusively.
fn inclusive_end(exclusive: &str) -> String {
    crate::windows::day_number(exclusive)
        .map(|day| crate::windows::iso_of_day_number(day - 1))
        .unwrap_or_else(|| exclusive.to_string())
}

/// True for a `YYYY-MM-DD` this file can compare as text.
fn is_iso_day(day: &str) -> bool {
    crate::windows::day_number(day).is_some()
}

/// The window a search runs in: the best one calendar offered, not the first.
///
/// The ordering is `windows::rank`'s own rule, load band before anything else:
/// `free`, then a compromise, then a window with a conflict; inside a band the
/// one that costs fewer travel days, then the earlier start. Taking the first
/// window whose verdict was not `conflicts` let a `needs-travel-day` window
/// that happened to come first cost every candidate 0.75 of its feasibility
/// score while a `free` window sat in the same response.
pub fn best_window(windows: &[FeasibleWindow]) -> Option<&FeasibleWindow> {
    windows.iter().min_by(|a, b| {
        verdict_band(a)
            .cmp(&verdict_band(b))
            .then(
                a.days_needing_travel_day
                    .len()
                    .cmp(&b.days_needing_travel_day.len()),
            )
            .then(a.starts_on.cmp(&b.starts_on))
    })
}

/// Calendar's verdicts as an order. An unrecognised or empty verdict sits
/// between a measured compromise and a measured conflict: the caller's own
/// unchecked window carries an empty verdict, and it is the only window in that
/// case anyway.
fn verdict_band(window: &FeasibleWindow) -> u8 {
    match window.verdict.as_str() {
        "free" => 0,
        "needs-travel-day" => 1,
        "conflicts" => 3,
        _ => 2,
    }
}

/// Whether a candidate names the place the search starts from.
///
/// By id or by name, because the two sources that offer candidates mint their
/// own ids: the operator's own origin reaches this function under the id they
/// typed and under the registry's.
fn is_origin(origin: &PlaceRef, candidate: &DestinationCandidate) -> bool {
    let same_id = !origin.id.trim().is_empty()
        && (origin.id == candidate.place_id || origin.id == candidate.destination.id);
    let same_name = !origin.name.trim().is_empty()
        && origin.name.trim().to_lowercase() == candidate.destination.name.trim().to_lowercase();
    same_id || same_name
}

/// Great-circle distance in kilometres. See [`EVENT_RADIUS_KM`] for why this is
/// a copy rather than a dependency.
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let radius_km = 6371.0;
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * radius_km * h.sqrt().asin()
}

/// The one durable thing a search can produce: an `option_set` payload.
///
/// Money is integer minor units, which is this writer's whole vocabulary. It is
/// not the only writer of an `option_set` row: `tools/sparpreis-watch` posts
/// one every 12 hours (`capabilities/sparpreis-watch/service.toml`) carrying the
/// older float `total_price` and no `revision`, and rows in that shape already
/// exist. `schemas/trip-plan.schema.json` describes both.
///
/// Carries NO companion field and NO calendar title, because
/// `projection.rs` writes this payload verbatim into a fenced JSON block in the
/// operator's cloud-synced vault.
pub fn adopt_payload(result: &PlanSearchResult) -> Value {
    let options: Vec<Value> = result
        .candidates
        .iter()
        .map(|candidate| {
            json!({
                "id": format!("{}|{}", candidate.place_id, mode_slug(&candidate.mode)),
                "estimated_cost_cents": candidate.estimated_cost_cents,
                "currency": candidate.currency,
                "destination": candidate.destination,
                "window": {
                    "starts_on": candidate.window.starts_on,
                    "ends_before": candidate.window.ends_before,
                    "verdict": candidate.window.verdict,
                },
                "score": candidate.score,
                "factors": candidate.factors,
                "chosen": false,
            })
        })
        .collect();
    json!({
        "query": {
            // A string, because `$defs.optionSetPayload.properties.query.from`
            // declares one. The PlaceRef the operator chose rides in
            // `destinations[]`, which is the field that needs coordinates.
            "from": origin_label(&result.query.from),
            // A declared literal, described in schemas/trip-plan.schema.json,
            // rather than a magic value invented at the call site. A plan
            // search has no single destination by construction.
            "to": "(multiple)",
            "destinations": result.query.destinations,
            "time": result.query.time,
            "budget_cents": result.query.budget_cents,
            "currency": result.query.currency,
            "modes": result.query.modes,
        },
        "revision": result.revision,
        "degraded": result.degraded,
        "options": options,
        "observed_at": result.observed_at,
    })
}

/// The origin as one string, for the schema's `query.from`.
fn origin_label(origin: &PlaceRef) -> String {
    if origin.name.trim().is_empty() {
        origin.id.clone()
    } else {
        origin.name.clone()
    }
}

pub fn adopt_external_id(observed_at: &str, job: u64) -> String {
    format!("plan-search:{observed_at}:{job}")
}

fn mode_slug(mode: &TransportMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "train".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::PlaceKind;

    fn place(id: &str, name: &str, latitude: f64, longitude: f64) -> PlaceRef {
        PlaceRef {
            id: id.into(),
            name: name.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
        }
    }

    fn candidate(id: &str, name: &str) -> DestinationCandidate {
        DestinationCandidate {
            place_id: id.into(),
            destination: place(id, name, 50.0 + id.len() as f64 * 0.01, 8.0),
            sources: vec!["places".into()],
        }
    }

    fn free_window() -> FeasibleWindow {
        FeasibleWindow {
            starts_on: "2026-10-05".into(),
            ends_before: "2026-10-12".into(),
            days: vec!["2026-10-05".into()],
            verdict: "free".into(),
            days_needing_travel_day: Vec::new(),
        }
    }

    /// A stub whose every answer the test names. `calendar` is the one that
    /// makes a month search fail, so it is the first knob.
    struct Stub {
        calendar: Result<Vec<FeasibleWindow>, String>,
        cities: Result<Vec<DestinationCandidate>, String>,
        opportunities: Result<Vec<CandidateEvent>, String>,
        climate: Result<HashMap<String, SeasonScore>, String>,
        fare_cents: Option<i64>,
        deadline_after: usize,
        priced: std::cell::RefCell<Vec<String>>,
        /// The `(from, to)` every presence read was asked for, so a test can
        /// check the window that went on the wire rather than the one in the
        /// response.
        presence_asked: std::cell::RefCell<Vec<(String, String)>>,
        soft_weights: Option<SoftWeights>,
        retrospectives: HashMap<String, crate::retrospective::DestinationFactor>,
    }

    impl Default for Stub {
        fn default() -> Self {
            Self {
                calendar: Ok(vec![free_window()]),
                cities: Ok(vec![candidate("place:a", "Aachen")]),
                opportunities: Ok(Vec::new()),
                climate: Ok(HashMap::new()),
                fare_cents: Some(4200),
                deadline_after: usize::MAX,
                priced: std::cell::RefCell::new(Vec::new()),
                presence_asked: std::cell::RefCell::new(Vec::new()),
                soft_weights: None,
                retrospectives: HashMap::new(),
            }
        }
    }

    impl Sources for Stub {
        fn calendar_windows(
            &self,
            _: &str,
            _: &str,
            _: u32,
        ) -> Result<Vec<FeasibleWindow>, String> {
            self.calendar.clone()
        }
        fn cities(&self) -> Result<Vec<DestinationCandidate>, String> {
            self.cities.clone()
        }
        fn opportunities(&self) -> Result<Vec<CandidateEvent>, String> {
            self.opportunities.clone()
        }
        fn climate(&self, _: &[String], _: u32) -> Result<HashMap<String, SeasonScore>, String> {
            self.climate.clone()
        }
        fn price(
            &self,
            _: &PlaceRef,
            destination: &DestinationCandidate,
            _: &str,
        ) -> Result<Option<i64>, String> {
            self.priced.borrow_mut().push(destination.place_id.clone());
            Ok(self.fare_cents)
        }
        fn presence(&self, _: f64, _: f64, from: &str, to: &str) -> Result<(u32, u32), String> {
            self.presence_asked
                .borrow_mut()
                .push((from.to_string(), to.to_string()));
            Ok((0, 0))
        }
        fn deadline_spent(&self) -> bool {
            self.priced.borrow().len() >= self.deadline_after
        }
        fn observed_at(&self) -> String {
            "2026-10-01T09:00:00Z".into()
        }
        fn soft_weights(&self) -> Option<SoftWeights> {
            self.soft_weights
        }
        fn retrospectives(&self) -> HashMap<String, crate::retrospective::DestinationFactor> {
            self.retrospectives.clone()
        }
    }

    fn request(month: Option<&str>, window: Option<(&str, &str)>) -> ValidRequest {
        request_by(month, window, vec![TransportMode::Train])
    }

    fn request_by(
        month: Option<&str>,
        window: Option<(&str, &str)>,
        modes: Vec<TransportMode>,
    ) -> ValidRequest {
        validate(PlanSearchRequest {
            origin: place("place:home", "Home", 50.1, 8.7),
            month: month.map(str::to_string),
            date_window: window.map(|(from, to)| DateWindow {
                from: from.into(),
                to: to.into(),
            }),
            min_days: Some(3),
            budget_cents: Some(30_000),
            currency: None,
            modes,
            interests: String::new(),
            max_candidates: Some(8),
        })
        .expect("the fixture request is valid")
    }

    /// The defect `flight_when` ships by a stated contract, refused here: a
    /// month search with no calendar has no feasible windows and therefore no
    /// answer, so it fails by name rather than ranking every day as free.
    #[test]
    fn a_month_search_without_a_calendar_fails_instead_of_ranking_free_days() {
        let stub = Stub {
            calendar: Err("connection refused".into()),
            ..Default::default()
        };
        let error = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect_err("a month search must not proceed without windows");
        assert_eq!(error, NO_CALENDAR_FOR_A_MONTH);
    }

    /// An explicit window survives a missing calendar, and says so twice: in
    /// `degraded` and by the absence of a feasibility factor.
    #[test]
    fn an_explicit_window_without_a_calendar_degrades_and_names_it() {
        let stub = Stub {
            calendar: Err("connection refused".into()),
            ..Default::default()
        };
        let result = compose(
            &request(None, Some(("2026-10-05", "2026-10-11"))),
            Vec::new(),
            &stub,
        )
        .expect("an explicit window needs no calendar");
        assert_eq!(result.degraded, vec!["calendar".to_string()]);
        assert_eq!(result.window_source, "caller");
        assert!(result.reach.calendar.starts_with("error: "));
        for candidate in &result.candidates {
            assert!(
                !candidate.factors.iter().any(|f| f.key == "feasibility"),
                "an unmeasured factor must be absent, not neutral"
            );
        }
    }

    /// Across every degradation combination the surviving weights still sum to
    /// 1, so two candidates in one response stay comparable.
    #[test]
    fn a_dropped_factor_renormalises_the_rest_to_one() {
        for budget in [None, Some(30_000)] {
            for window in [None, Some(free_window())] {
                for climate in [false, true] {
                    for events in [None, Some(0usize), Some(4)] {
                        for retro in [
                            None,
                            Some(crate::retrospective::DestinationFactor {
                                key: "bonn".into(),
                                n: 2,
                                mean_again: 0.5,
                                factor: 1.1,
                                median_overrun_bp: None,
                                basis: vec!["plan-1".into()],
                            }),
                        ] {
                            let inputs = ScoringInputs {
                                budget_cents: budget,
                                estimated_cost_cents: Some(4200),
                                window: window.clone(),
                                season: climate.then_some(SeasonScore {
                                    month: 10,
                                    score: 0.8,
                                    best_month: Some(6),
                                }),
                                climate_reached: climate,
                                events,
                                retrospective: retro,
                                ..Default::default()
                            };
                            let factors = factors(&inputs);
                            if factors.is_empty() {
                                continue;
                            }
                            let total: f64 = factors.iter().map(|f| f.weight).sum();
                            assert!(
                                (total - 1.0).abs() < 1e-9,
                                "weights summed to {total} for {factors:?}"
                            );
                            let score = weighted_score(&factors).expect("a measured factor scores");
                            assert!((0.0..=1.0).contains(&score), "score out of range: {score}");
                        }
                    }
                }
            }
        }
    }

    /// A candidate nobody could price is not a cheap candidate.
    #[test]
    fn an_unpriced_candidate_ranks_below_every_priced_one() {
        let mut candidates = vec![
            RankedCandidate {
                destination: place("place:b", "Bonn", 50.7, 7.1),
                place_id: "place:b".into(),
                window: free_window(),
                mode: TransportMode::Train,
                estimated_cost_cents: None,
                currency: "EUR".into(),
                cost_basis: "unpriced".into(),
                priced_at: None,
                events: Vec::new(),
                season: None,
                score: Some(0.99),
                factors: Vec::new(),
                companion_hint: None,
                why: Vec::new(),
            },
            RankedCandidate {
                destination: place("place:a", "Aachen", 50.8, 6.1),
                place_id: "place:a".into(),
                window: free_window(),
                mode: TransportMode::Train,
                estimated_cost_cents: Some(9900),
                currency: "EUR".into(),
                cost_basis: "transit-search".into(),
                priced_at: None,
                events: Vec::new(),
                season: None,
                score: Some(0.10),
                factors: Vec::new(),
                companion_hint: None,
                why: Vec::new(),
            },
        ];
        rank(&mut candidates);
        assert_eq!(
            candidates[0].place_id, "place:a",
            "a priced candidate with a worse score still ranks above an unpriced one"
        );
    }

    /// 36 cities in, 8 fare probes out, and the response says 36 were looked at.
    #[test]
    fn the_shortlist_caps_priced_candidates_at_max_candidates() {
        let cities: Vec<DestinationCandidate> = (0..36)
            .map(|n| candidate(&format!("place:{n:02}"), &format!("City {n:02}")))
            .collect();
        let stub = Stub {
            cities: Ok(cities),
            ..Default::default()
        };
        let result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        assert_eq!(result.considered, 36);
        assert_eq!(
            stub.priced.borrow().len(),
            8,
            "only the shortlist is priced"
        );
        assert_eq!(result.candidates.len(), 8);
    }

    /// A spent deadline finishes the job and names the budget, rather than
    /// running past a bound nobody can see.
    #[test]
    fn a_spent_deadline_finishes_the_job_and_names_the_budget() {
        let cities: Vec<DestinationCandidate> = (0..8)
            .map(|n| candidate(&format!("place:{n:02}"), &format!("City {n:02}")))
            .collect();
        let stub = Stub {
            cities: Ok(cities),
            deadline_after: 3,
            ..Default::default()
        };
        let result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a job that spends its budget still finishes");
        assert!(result.priced < result.considered);
        assert_eq!(result.priced, 3);
        assert!(
            result.degraded.iter().any(|d| d == PRICING_BUDGET_SPENT),
            "the pricing budget must be named: {:?}",
            result.degraded
        );
        assert!(result
            .candidates
            .iter()
            .any(|c| c.why.iter().any(|w| w.contains("pricing budget"))));
    }

    /// The factor that gives the search its name moves the rank.
    #[test]
    fn over_budget_scores_below_under_budget_with_everything_else_equal() {
        let under = factors(&ScoringInputs {
            budget_cents: Some(30_000),
            estimated_cost_cents: Some(12_000),
            window: Some(free_window()),
            season: None,
            climate_reached: false,
            events: Some(0),
            ..Default::default()
        });
        let over = factors(&ScoringInputs {
            budget_cents: Some(30_000),
            estimated_cost_cents: Some(48_000),
            window: Some(free_window()),
            season: None,
            climate_reached: false,
            events: Some(0),
            ..Default::default()
        });
        assert!(
            weighted_score(&under) > weighted_score(&over),
            "under budget must outrank over budget"
        );
        assert!(over
            .iter()
            .find(|f| f.key == "budget_fit")
            .is_some_and(|f| f.score < 0.5));
    }

    /// `projection.rs` writes this payload verbatim into a cloud-synced vault,
    /// so the two things that must never be in it are checked as text.
    #[test]
    fn the_adopted_payload_carries_neither_a_companion_nor_a_calendar_title() {
        let stub = Stub::default();
        let mut result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        // A hint that DID reach the browser, and a window whose calendar entry
        // was titled — neither may survive the payload.
        result.candidates[0].companion_hint = Some(companion_hint(2, 3));
        result.candidates[0].window.verdict = "needs-travel-day".into();
        let payload = adopt_payload(&result).to_string();
        assert!(
            !payload.contains("companion"),
            "a companion-derived fact reached the vault payload"
        );
        // The structural half: the window type has no field for a title, so a
        // calendar title cannot be carried even by a future edit upstream.
        assert!(!payload.contains("collisions"));
        assert!(!payload.contains("title"));
    }

    /// The fields `schemas/trip-plan.schema.json` declares for
    /// `$defs.optionSetPayload`, checked here because `DECLARED_PAYLOADS` in
    /// the store validates top-level keys only.
    ///
    /// Presence of each declared field, not validation: nothing in this
    /// workspace validates a document against a JSON Schema, and a test named
    /// for a guarantee it does not give is worse than no test.
    #[test]
    fn the_adopted_payload_carries_every_declared_field() {
        let stub = Stub::default();
        let result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        let payload = adopt_payload(&result);
        let schema: Value =
            serde_json::from_str(include_str!("../../../schemas/trip-plan.schema.json"))
                .expect("the declared schema parses");
        let declared = &schema["$defs"]["optionSetPayload"];

        for required in declared["required"]
            .as_array()
            .expect("optionSetPayload declares required fields")
        {
            let key = required.as_str().unwrap();
            assert!(
                payload.get(key).is_some(),
                "the payload is missing the declared field {key}"
            );
        }
        for key in ["from", "to", "destinations", "time"] {
            assert!(
                payload["query"].get(key).is_some(),
                "query.{key} is missing"
            );
        }
        assert_eq!(payload["query"]["to"], "(multiple)");
        // Declared but not required, because the rows tools/sparpreis-watch
        // has been writing since before this route carry none. This writer
        // always sets it.
        assert_eq!(payload["revision"], PLAN_SEARCH_REVISION);
        let option = &payload["options"][0];
        assert!(option.get("estimated_cost_cents").is_some());
        assert!(
            option.get("total_price").is_none(),
            "money is carried as integer minor units, never as the legacy float"
        );
        assert!(
            declared["properties"]["options"]["items"]["properties"]
                .get("estimated_cost_cents")
                .is_some(),
            "the schema must declare the money field this writer produces"
        );
    }

    #[test]
    fn a_month_expands_to_its_first_and_last_day() {
        assert_eq!(
            expand_month("2026-10"),
            Some(("2026-10-01".into(), "2026-10-31".into()))
        );
        assert_eq!(
            expand_month("2028-02"),
            Some(("2028-02-01".into(), "2028-02-29".into()))
        );
        assert_eq!(expand_month("2026-13"), None);
        assert_eq!(expand_month("October"), None);
    }

    #[test]
    fn validation_names_the_field_that_is_wrong() {
        let base = || PlanSearchRequest {
            origin: place("place:home", "Home", 50.1, 8.7),
            month: None,
            date_window: None,
            min_days: None,
            budget_cents: None,
            currency: None,
            modes: Vec::new(),
            interests: String::new(),
            max_candidates: None,
        };
        assert!(validate(base()).unwrap_err().contains("month"));
        let both = PlanSearchRequest {
            month: Some("2026-10".into()),
            date_window: Some(DateWindow {
                from: "2026-10-01".into(),
                to: "2026-10-03".into(),
            }),
            ..base()
        };
        assert!(validate(both).unwrap_err().contains("not both"));
        let too_long = PlanSearchRequest {
            date_window: Some(DateWindow {
                from: "2026-10-01".into(),
                to: "2026-12-31".into(),
            }),
            ..base()
        };
        assert!(validate(too_long).unwrap_err().contains("42"));
        let broke = PlanSearchRequest {
            date_window: Some(DateWindow {
                from: "2026-10-01".into(),
                to: "2026-10-08".into(),
            }),
            budget_cents: Some(-1),
            ..base()
        };
        assert!(validate(broke).unwrap_err().contains("budget_cents"));
        let clamped = validate(PlanSearchRequest {
            month: Some("2026-10".into()),
            max_candidates: Some(500),
            ..base()
        })
        .expect("a clamped max_candidates is not an error");
        assert_eq!(clamped.max_candidates, MAX_MAX_CANDIDATES);
    }

    /// The dates a validated request carries are re-rendered from their day
    /// numbers, so the caller's own bytes never reach `upstream`'s query strings
    /// or `projection.rs`' vault note.
    ///
    /// `day_number` is a shape check that stops at the day field, so it accepts
    /// the first string below; that is the premise, asserted here rather than
    /// assumed. Same class as CodeQL alert 67 on `flight_when`, one route over.
    #[test]
    fn a_window_is_clipped_to_the_date_it_parsed_as() {
        assert!(
            crate::windows::day_number("2026-10-01&min_days=0").is_some(),
            "the premise: the shape check accepts a date with a query string on it"
        );
        let clipped = request(None, Some(("2026-10-01&min_days=0", "2026-10-08#x")));
        assert_eq!(clipped.from, "2026-10-01");
        assert_eq!(clipped.to, "2026-10-08");

        // A single-digit month is normalised the same way, which is what makes
        // this a canonicalisation and not a string trim. (A single-digit DAY is
        // refused outright: `unix_day_of_iso` reads that field two characters
        // wide, so `2026-1-1` never parses at all.)
        let loose = request(None, Some(("2026-1-01", "2026-1-08")));
        assert_eq!(loose.from, "2026-01-01");
        assert_eq!(loose.to, "2026-01-08");
    }

    /// The best window, not the first one calendar happened to list. A
    /// `needs-travel-day` window ahead of a `free` one used to cost every
    /// candidate three quarters of its feasibility score, with nothing in the
    /// body saying a better window was in the same answer.
    #[test]
    fn a_free_window_beats_a_compromise_the_calendar_listed_first() {
        let compromised = FeasibleWindow {
            starts_on: "2026-10-01".into(),
            ends_before: "2026-10-08".into(),
            days: Vec::new(),
            verdict: "needs-travel-day".into(),
            days_needing_travel_day: (1..=7).map(|d| format!("2026-10-0{d}")).collect(),
        };
        let stub = Stub {
            calendar: Ok(vec![compromised, free_window()]),
            ..Default::default()
        };
        let result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        let candidate = &result.candidates[0];
        assert_eq!(candidate.window.verdict, "free");
        assert_eq!(candidate.window.starts_on, "2026-10-05");
        let feasibility = candidate
            .factors
            .iter()
            .find(|f| f.key == "feasibility")
            .expect("a measured window is scored");
        assert_eq!(feasibility.score, 1.0);
        // Both windows are still reported; only the one used is chosen.
        assert_eq!(result.windows.len(), 2);
    }

    /// A scouting feed's `starts_at` is external text. Slicing it at byte 10
    /// panicked inside a multi-byte character, and a panicked job stayed
    /// `Running` for ever because `jobs::open` never evicts one.
    #[test]
    fn an_event_start_that_is_not_an_iso_day_is_skipped_rather_than_panicking() {
        let candidates = vec![candidate("place:a", "Aachen")];
        let at = candidates[0].destination.clone();
        let event = |id: &str, starts_at: &str| CandidateEvent {
            id: id.into(),
            title: "An opportunity".into(),
            starts_at: Some(starts_at.into()),
            url: String::new(),
            latitude: at.latitude,
            longitude: at.longitude,
            distance_km: None,
        };
        let opportunities = vec![
            event("evt:umlaut", "2026-10-0\u{dc}T00:00"),
            event("evt:short", "2026"),
            event("evt:good", "2026-10-06T18:00:00Z"),
        ];
        let found = events_near(&candidates, &opportunities, &free_window());
        let ids: Vec<&str> = found["place:a"].iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["evt:good"]);
    }

    /// places counts its `to` inclusively; calendar's `ends_before` is
    /// exclusive. Sending the exclusive end asked about one day too many, and
    /// the hint read "for 8 days of this window" for a seven-day window.
    #[test]
    fn the_presence_read_asks_about_the_windows_last_day_and_no_further() {
        let stub = Stub::default();
        let _ = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        let asked = stub.presence_asked.borrow();
        assert_eq!(
            asked.as_slice(),
            [("2026-10-05".to_string(), "2026-10-11".to_string())],
            "the window is 2026-10-05 up to but not including 2026-10-12"
        );
    }

    /// The id and the destination are one fact. When the second source's place
    /// wins because it has coordinates, its id has to win too, or
    /// `options[].id` in the adopted payload names a place no `destination` in
    /// the same payload has.
    #[test]
    fn a_merged_candidate_keeps_its_id_and_its_destination_together() {
        let without_coordinates = DestinationCandidate {
            place_id: "place:berlin".into(),
            destination: PlaceRef {
                id: "place:berlin".into(),
                name: "Berlin".into(),
                kind: PlaceKind::City,
                address: None,
                latitude: None,
                longitude: None,
            },
            sources: vec!["plan".into()],
        };
        let with_coordinates = DestinationCandidate {
            place_id: "obsidian-place:berlin".into(),
            destination: place("obsidian-place:berlin", "Berlin", 52.5, 13.4),
            sources: vec!["places".into()],
        };
        let merged = merge_candidates(vec![without_coordinates], vec![with_coordinates]);
        assert_eq!(merged.len(), 1, "one name, one candidate");
        assert!(merged[0].destination.latitude.is_some());
        assert_eq!(
            merged[0].place_id, merged[0].destination.id,
            "the candidate's id must name the destination it carries"
        );
        assert_eq!(merged[0].sources.len(), 2);
    }

    /// A rail fare must not be reported as a flight. The only fare source in
    /// this search is transit, so a mode it cannot price is left unpriced and
    /// said so, rather than priced with somebody else's fare.
    #[test]
    fn a_mode_no_fare_source_covers_is_left_unpriced_and_named() {
        let stub = Stub::default();
        let result = compose(
            &request_by(Some("2026-10"), None, vec![TransportMode::Flight]),
            Vec::new(),
            &stub,
        )
        .expect("a month search with a calendar composes");
        assert!(stub.priced.borrow().is_empty(), "transit was never asked");
        assert_eq!(result.reach.transit, "absent");
        assert!(result
            .degraded
            .iter()
            .any(|d| d == &no_fare_source_for(&TransportMode::Flight)));
        let candidate = &result.candidates[0];
        assert_eq!(candidate.mode, TransportMode::Flight);
        assert_eq!(candidate.estimated_cost_cents, None);
        assert_eq!(candidate.cost_basis, "unpriced");
        assert!(
            candidate.why.iter().any(|w| w.contains("no fare source")),
            "{:?}",
            candidate.why
        );
        assert!(
            !candidate.why.iter().any(|w| w.contains("by train")),
            "a flight search must not be explained with a rail fare: {:?}",
            candidate.why
        );
    }

    /// A trip from Berlin to Berlin is not an answer, and it used to take one
    /// of the few fare probes.
    #[test]
    fn the_origin_is_not_offered_as_its_own_destination() {
        let stub = Stub {
            cities: Ok(vec![
                candidate("place:home", "Home"),
                candidate("place:a", "Aachen"),
                // The same origin under the id another source minted for it.
                candidate("obsidian-place:home", "home"),
            ]),
            ..Default::default()
        };
        let result = compose(&request(Some("2026-10"), None), Vec::new(), &stub)
            .expect("a month search with a calendar composes");
        assert_eq!(result.considered, 1);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].place_id, "place:a");
    }

    /// A candidate nothing could be measured about carries no score, rather
    /// than a `0.0` a reader cannot tell apart from a measured worst.
    #[test]
    fn a_candidate_with_no_measurable_factor_is_unscored_rather_than_zero() {
        assert_eq!(weighted_score(&[]), None);
        let stub = Stub {
            calendar: Err("connection refused".into()),
            opportunities: Err("connection refused".into()),
            fare_cents: None,
            ..Default::default()
        };
        let result = compose(
            &request(None, Some(("2026-10-05", "2026-10-11"))),
            Vec::new(),
            &stub,
        )
        .expect("an explicit window needs no calendar");
        let candidate = &result.candidates[0];
        assert!(candidate.factors.is_empty());
        assert_eq!(candidate.score, None);
    }

    /// The hint says how many and how long, and never who.
    #[test]
    fn the_companion_hint_carries_a_count_and_never_a_name() {
        assert_eq!(
            companion_hint(1, 1),
            "a known companion is near this destination for 1 day of this window"
        );
        assert_eq!(
            companion_hint(3, 5),
            "3 known companions are near this destination for 5 days of this window"
        );
    }

    #[test]
    fn favorable_retrospective_boosts_candidate_score_and_rationale() {
        let mut retrospectives = HashMap::new();
        retrospectives.insert(
            "berlin".to_string(),
            crate::retrospective::DestinationFactor {
                key: "berlin".into(),
                n: 3,
                mean_again: 1.0,
                factor: 1.15,
                median_overrun_bp: None,
                basis: vec!["plan-1".into(), "plan-2".into(), "plan-3".into()],
            },
        );

        let stub = Stub {
            cities: Ok(vec![
                candidate("place:berlin", "Berlin"),
                candidate("place:munich", "Munich"),
            ]),
            fare_cents: Some(5000),
            retrospectives,
            ..Default::default()
        };

        let req = request_by(Some("2026-10"), None, vec![TransportMode::Train]);
        let result = compose(&req, Vec::new(), &stub).expect("search composes");

        assert_eq!(result.revision, "plan-search-v3");
        let berlin = result
            .candidates
            .iter()
            .find(|c| c.place_id == "place:berlin")
            .expect("berlin candidate present");
        let munich = result
            .candidates
            .iter()
            .find(|c| c.place_id == "place:munich")
            .expect("munich candidate present");

        let berlin_retro = berlin
            .factors
            .iter()
            .find(|f| f.key == FACTOR_RETROSPECTIVE.0);
        assert!(
            berlin_retro.is_some(),
            "berlin must have retrospective factor"
        );
        assert!(berlin_retro
            .unwrap()
            .rationale
            .contains("rated favorably across 3 past trip(s)"));
        assert!(berlin
            .why
            .iter()
            .any(|w| w.contains("rated favorably across 3 past trip(s)")));

        let munich_retro = munich
            .factors
            .iter()
            .find(|f| f.key == FACTOR_RETROSPECTIVE.0);
        assert!(
            munich_retro.is_none(),
            "munich without past trips has no retrospective factor"
        );

        // Berlin has higher score than Munich due to favorable past retrospective
        assert!(
            berlin.score.unwrap() > munich.score.unwrap(),
            "favorable retrospective should boost berlin above munich"
        );
    }

    #[test]
    fn custom_soft_weights_are_respected_in_scoring() {
        let custom_weights = SoftWeights {
            budget_fit: 0.70,
            feasibility: 0.10,
            season: 0.10,
            events: 0.10,
            retrospective: 0.0,
        };

        let stub = Stub {
            cities: Ok(vec![candidate("place:berlin", "Berlin")]),
            fare_cents: Some(3000),
            soft_weights: Some(custom_weights),
            ..Default::default()
        };

        let req = request_by(Some("2026-10"), None, vec![TransportMode::Train]);
        let result = compose(&req, Vec::new(), &stub).expect("search composes");
        let c = &result.candidates[0];
        let budget_factor = c.factors.iter().find(|f| f.key == "budget_fit").unwrap();
        // Budget weight dominates heavily when custom weight is 0.70
        assert!(budget_factor.weight > 0.50);
    }

    fn budget_and_events(weights: SoftWeights) -> ScoringInputs {
        ScoringInputs {
            budget_cents: Some(30_000),
            estimated_cost_cents: Some(15_000),
            events: Some(3),
            weights,
            ..Default::default()
        }
    }

    /// A negative weight is clamped to 0 rather than subtracting from the score.
    #[test]
    fn a_negative_soft_weight_counts_as_zero() {
        let inputs = budget_and_events(SoftWeights {
            budget_fit: -0.5,
            events: 0.5,
            ..SoftWeights::default()
        });
        let measured = factors(&inputs);
        let budget = measured.iter().find(|f| f.key == "budget_fit").unwrap();
        let events = measured.iter().find(|f| f.key == "events").unwrap();
        assert_eq!(budget.weight, 0.0);
        assert!((events.weight - 1.0).abs() < 1e-9);
        assert!(weighted_score(&measured).unwrap() > 0.0);
    }

    /// When the measured factors carry no weight, the defaults stand in. Before,
    /// a measured total of 0 scored every candidate 0.
    #[test]
    fn a_zero_measured_weight_total_falls_back_to_the_defaults() {
        let zero = SoftWeights {
            budget_fit: 0.0,
            feasibility: 1.0,
            season: 0.0,
            events: -1.0,
            retrospective: 0.0,
        };
        let measured = factors(&budget_and_events(zero));
        let budget = measured.iter().find(|f| f.key == "budget_fit").unwrap();
        let events = measured.iter().find(|f| f.key == "events").unwrap();
        let expected = WEIGHT_BUDGET_FIT / (WEIGHT_BUDGET_FIT + WEIGHT_EVENTS);
        assert!((budget.weight - expected).abs() < 1e-9, "{measured:?}");
        assert!((budget.weight + events.weight - 1.0).abs() < 1e-9);
        assert!(weighted_score(&measured).unwrap() > 0.0);
    }
}
