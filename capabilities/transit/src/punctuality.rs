//! Filling `Journey.delay_risk_score` from `capabilities/punctuality`.
//!
//! Over HTTP, never by linking that crate: a capability depends on another's contract,
//! not its code (CONTRIBUTING.md#schemas-and-dependency-direction). This module is the whole dependency, and it is a
//! one-way one — punctuality knows nothing about transit.
//!
//! Absence degrades, it never fails. If punctuality is not running, or has never
//! ingested, or has no cell for this train at this hour, the score stays `None` and the
//! search returns exactly what it returned before this module existed. A journey search
//! that breaks because a statistics service is down would be a worse product than one
//! without statistics.

use crate::travel::{
    ArrivalPunctuality, Journey, JourneyReliability, Leg, SplitResult, Station,
    TransferReliability, UnscoredLeg,
};
use serde::{Deserialize, Serialize};

/// Where punctuality-server listens.
///
/// The literal mirrors `capabilities/punctuality/service.toml`'s `port`, which is one
/// duplication more than CONTRIBUTING.md#dynamic-paths-and-current-facts likes. It is here rather than hidden because the honest
/// fix is a spine mechanism — service-runner.sh exporting a declared `requires =`
/// sibling's port the way it already exports `SJEL_PORT` for the capability itself — and
/// building that for a single consumer would be inventing a convention from one example.
/// The second capability that needs a sibling's URL is when that gets built.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8085";

pub fn base_url() -> String {
    sjel_config::env_var("SJEL_PUNCTUALITY_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

#[derive(Debug, Clone, Serialize)]
struct StopQuery {
    eva: String,
    train_type: String,
    hour: u8,
    weekend: bool,
    /// The exceedance threshold to answer at. Omitted, the reply carries the
    /// six-minute figure it always did; set, it answers the question this
    /// particular transfer actually poses.
    #[serde(skip_serializing_if = "Option::is_none")]
    at_least_minutes: Option<i32>,
}

#[derive(Debug, Serialize)]
struct LookupBody {
    stops: Vec<StopQuery>,
}

/// Punctuality's cell, in full.
///
/// This used to declare `share_late_6` alone, so six of the seven fields the
/// endpoint returns were discarded at deserialization -- `n` among them, which is
/// what tells a measurement over four thousand observations from one over thirty-one.
#[derive(Debug, Deserialize)]
struct StopStats {
    station_name: Option<String>,
    train_type: String,
    hour: i16,
    #[serde(default)]
    weekend: bool,
    n: i64,
    mean_delay: f32,
    p50: i16,
    p90: i16,
    /// Share of this train type's stops at this station in this hour that were at least
    /// six minutes off schedule. Six is DB's own punctuality threshold.
    share_late_6: f32,
    cancel_rate: f32,
    /// The exceedance at the threshold this query asked for.
    ///
    /// Punctuality's three states survive the round trip and mean different
    /// things. Absent: nobody asked. Explicit `null`: asked, and the row predates
    /// the stored histogram, so it cannot be answered. A number: answered.
    /// Flattening the middle case into absence would let a caller read silence as
    /// zero risk, which is the whole failure the endpoint's own comment names.
    #[serde(default)]
    share_delay_at_least: Option<Option<f64>>,
}

impl From<StopStats> for ArrivalPunctuality {
    fn from(s: StopStats) -> Self {
        Self {
            station_name: s.station_name,
            train_type: s.train_type,
            hour: s.hour,
            weekend: s.weekend,
            n: s.n,
            mean_delay: s.mean_delay,
            p50: s.p50,
            p90: s.p90,
            share_late_6: s.share_late_6,
            cancel_rate: s.cancel_rate,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    stats: Vec<Option<StopStats>>,
}

/// The EVA number inside a station id.
///
/// `travel::Station::id` carries two different formats depending on which HAFAS endpoint
/// produced it, despite the type's own comment claiming it is always an EVA code:
/// `/reiseloesung/orte` (suggest) answers a bare `"8000044"`, while a journey search
/// answers the full location string
/// `A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@`, where the EVA is
/// the `L=` field. Reading `id` as an EVA works for one of those and silently finds
/// nothing for the other.
pub fn eva_of(station_id: &str) -> Option<String> {
    if !station_id.is_empty() && station_id.chars().all(|c| c.is_ascii_digit()) {
        return Some(station_id.to_string());
    }
    station_id
        .split('@')
        .find_map(|part| part.strip_prefix("L="))
        .filter(|eva| !eva.is_empty() && eva.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// Local hour and weekend flag from a HAFAS `YYYY-MM-DDTHH:MM:SS` timestamp.
///
/// Parsed by position rather than with a date library: HAFAS emits local time in a fixed
/// layout, and punctuality's own timestamps are local wall-clock too, so no conversion
/// is wanted on either side — only the same clock read the same way.
pub fn hour_and_weekend(iso: &str) -> Option<(u8, bool)> {
    let (date, time) = iso.split_once('T')?;
    let hour: u8 = time.get(0..2)?.parse().ok()?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if hour > 23 {
        return None;
    }
    // Sakamoto's algorithm: day of week without a calendar dependency, 0 = Sunday.
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let idx = usize::try_from(m - 1).ok()?;
    let dow = (y + y / 4 - y / 100 + y / 400 + T.get(idx)? + d) % 7;
    Some((hour, dow == 0 || dow == 6))
}

/// punctuality's `train_type` for a leg: the label the train is announced under,
/// read off `train_name`.
///
/// Neither backend's product class is this key, and using one was the bug. Measured
/// 2026-08-20 over 104 legs on 16 routes, both backends, against the 109 distinct
/// `train_type` values in the ingested cells:
///
/// - dbweb's `verkehrsmittel.kategorie` is HAFAS's class code. It reports `DRB` for
///   BOTH the RE5 (`28510`) and the RB26 (`33602`), and punctuality holds RE and RB as
///   separate populations of 10.8M and 14.4M observations. So no `DRB -> RB` table can
///   be right: the mapping the field would need does not exist, because the field
///   already threw the distinction away.
/// - dbnav's `produktGattung` collapses the same way -- `RB` for an RE5, `IC_EC` for
///   both IC and EC. It found a cell, which is worse than finding none: it answered an
///   RE journey with RB statistics.
/// - `train_name` carries the label itself, and the label IS the vocabulary. All
///   eleven prefixes observed (ICE, RE, IC, S, RB, EC, RJ, FEX, FLX, EUR, ECE) exist
///   in the ingested cells; none needed translating.
///
/// `None` when nothing is left after the trailing number, which is exactly dbweb's
/// regional case: it names those trains `"28510"` and carries no label at all. That is
/// honest absence, and it is reported as `Journey::unscored_legs` rather than passed
/// off as a missing cell.
pub fn train_type_of(leg: &Leg) -> Option<&str> {
    let label = leg
        .train_name
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end();
    (!label.is_empty()).then_some(label)
}

/// Six minutes is DB's own punctuality threshold, and the one `share_late_6`
/// reports. Final arrival is judged against it so this number means the same
/// thing the rest of the system already means by "late".
const ARRIVAL_THRESHOLD_MINUTES: i32 = 6;

/// Punctuality refuses a lookup over 200 stops. One journey now costs one query
/// per leg plus one per transfer plus its own, so a search returning several
/// multi-leg journeys reaches that on its own -- and a refused batch would take
/// the `delay_risk_score` that used to work down with it.
const MAX_STOPS_PER_LOOKUP: usize = 200;

/// What a position in the batched lookup is being asked for.
enum Want {
    /// The journey's destination cell: `delay_risk_score`, `arrival_punctuality`,
    /// and the final on-time term of the product.
    JourneyArrival(usize),
    /// One leg's own destination, for `Leg::on_time_probability`.
    LegArrival { journey: usize, leg: usize },
    /// The transfer after leg `leg`, asked at that transfer's own buffer.
    Transfer {
        journey: usize,
        station: Station,
        buffer: i64,
    },
}

/// POSTs `/lookup`, in chunks, answering `None` only when the service could not be
/// reached at all. Absence of a cell is an inner `None` and stays distinguishable.
fn lookup(stops: Vec<StopQuery>) -> Option<Vec<Option<StopStats>>> {
    // Short on purpose: this is an enhancement on a localhost service. Waiting on
    // it would make a journey search slower than not having the number at all.
    let client = sjel_http::client(
        sjel_http::Purpose::new("transit-punctuality"),
        std::time::Duration::from_secs(3),
    )
    .ok()?;
    let url = format!("{}/lookup", base_url());
    let mut out = Vec::with_capacity(stops.len());
    for chunk in stops.chunks(MAX_STOPS_PER_LOOKUP) {
        let body = LookupBody {
            stops: chunk.to_vec(),
        };
        let request =
            sjel_server::InboundAuth::with_loopback_auth(client.post(&url).json(&body), &url);
        let response = request
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<LookupResponse>())
            .ok()?;
        out.extend(response.stats);
    }
    Some(out)
}

/// Scheduled minutes between arriving on one leg and departing on the next.
///
/// From the UTC instants X1 attached, so it stays right across a zone boundary.
/// `None` when either instant is absent, which is what keeps an unknown buffer from
/// being scored as a comfortable one.
fn buffer_minutes(arriving: &Leg, departing: &Leg) -> Option<i64> {
    let arrival = arriving.arrival_utc.as_ref()?;
    let departure = departing.departure_utc.as_ref()?;
    station_time::duration_between(arrival, "", departure, "").map(|d| d.num_minutes())
}

/// The measured floor: catching every transfer, and the last leg then arriving on time.
///
/// Pure, so the composition is testable without a punctuality server -- the same
/// reason `cheapest_split` is a free function. `None` the moment any term is missing:
/// a product with a hole in it is not a floor, it is a smaller number wearing one.
pub fn compose_reliability(
    transfers: Vec<TransferReliability>,
    final_leg_on_time: Option<(f64, i64)>,
    expected_transfers: usize,
) -> Option<JourneyReliability> {
    if transfers.len() != expected_transfers {
        return None;
    }
    let (final_on_time, final_n) = final_leg_on_time?;
    let mut probability = final_on_time;
    let mut min_sample = final_n;
    for t in &transfers {
        probability *= t.catch_probability;
        min_sample = min_sample.min(t.n);
    }
    Some(JourneyReliability {
        probability,
        threshold_minutes: ARRIVAL_THRESHOLD_MINUTES,
        final_leg_on_time: final_on_time,
        transfers,
        min_sample,
    })
}

/// Fills `delay_risk_score`, every leg's `on_time_probability`, and the journey's
/// composed `reliability` -- from one batched lookup.
///
/// The journey-level score is unchanged: the share of stops of the arriving train's
/// type, at the journey's destination, in the arrival hour, that ran at least six
/// minutes off schedule. It is still emphatically NOT the probability the trip works
/// out, and consumers that read it keep reading exactly what they read before.
///
/// `reliability` is the number that does try to answer that, and only as far as this
/// data can: the product of catching each transfer at its own buffer and then arriving
/// within six minutes. Every factor is an exceedance read off the stored histogram at
/// the threshold that transfer actually has -- no fitted curve, no constants. What it
/// assumes is on `JourneyReliability` itself.
pub fn enrich(journeys: &mut [Journey]) {
    if journeys.is_empty() {
        return;
    }
    let mut wants: Vec<Want> = Vec::new();
    let mut stops: Vec<StopQuery> = Vec::new();
    // A property of the response, not of the punctuality service: it is filled and
    // written back even when the lookup never happens or fails.
    let mut unscored: Vec<Vec<UnscoredLeg>> = vec![Vec::new(); journeys.len()];

    for (i, j) in journeys.iter().enumerate() {
        if let (Some(last), Some(eva)) = (
            j.legs.last(),
            j.legs.last().and_then(|last| {
                eva_of(&j.end_station.id).or_else(|| eva_of(&last.destination.id))
            }),
        ) {
            if let (Some((hour, weekend)), Some(train_type)) =
                (hour_and_weekend(&last.arrival_time), train_type_of(last))
            {
                wants.push(Want::JourneyArrival(i));
                stops.push(StopQuery {
                    eva,
                    train_type: train_type.to_string(),
                    hour,
                    weekend,
                    at_least_minutes: None,
                });
            }
        }

        for (k, leg) in j.legs.iter().enumerate() {
            // Recorded before the other two guards on purpose: a leg with no label is
            // unscorable whatever its station id or timestamp look like, and that is
            // the fact worth reporting. An unparseable eva or arrival time is a
            // different failure and stays a plain skip.
            let Some(train_type) = train_type_of(leg) else {
                unscored[i].push(UnscoredLeg {
                    leg_index: k,
                    train_name: leg.train_name.clone(),
                    train_category: leg.train_category.clone(),
                });
                continue;
            };
            let Some(eva) = eva_of(&leg.destination.id) else {
                continue;
            };
            let Some((hour, weekend)) = hour_and_weekend(&leg.arrival_time) else {
                continue;
            };
            wants.push(Want::LegArrival { journey: i, leg: k });
            stops.push(StopQuery {
                eva,
                train_type: train_type.to_string(),
                hour,
                weekend,
                at_least_minutes: None,
            });
        }

        for pair in j.legs.windows(2) {
            let (arriving, departing) = (&pair[0], &pair[1]);
            let Some(buffer) = buffer_minutes(arriving, departing) else {
                continue;
            };
            let Some(eva) = eva_of(&arriving.destination.id) else {
                continue;
            };
            let Some((hour, weekend)) = hour_and_weekend(&arriving.arrival_time) else {
                continue;
            };
            let Some(train_type) = train_type_of(arriving) else {
                continue;
            };
            wants.push(Want::Transfer {
                journey: i,
                station: arriving.destination.clone(),
                buffer,
            });
            stops.push(StopQuery {
                eva,
                train_type: train_type.to_string(),
                hour,
                weekend,
                // The threshold IS the buffer: a train that loses the whole margin
                // has lost the connection. Counting a train exactly `buffer` late as
                // missing it is the conservative call, which is what keeps the
                // composed number a floor rather than a best guess.
                at_least_minutes: Some(i32::try_from(buffer).unwrap_or(i32::MAX)),
            });
        }
    }

    for (journey, legs) in journeys.iter_mut().zip(unscored) {
        journey.unscored_legs = legs;
    }

    if stops.is_empty() {
        return;
    }
    let Some(stats) = lookup(stops) else { return };

    // Collected before anything is written back, because the composition needs a
    // journey's transfers together and the responses arrive interleaved.
    let mut transfers: Vec<Vec<TransferReliability>> = vec![Vec::new(); journeys.len()];
    let mut final_on_time: Vec<Option<(f64, i64)>> = vec![None; journeys.len()];

    for (want, stat) in wants.into_iter().zip(stats) {
        let Some(stat) = stat else { continue };
        match want {
            Want::JourneyArrival(i) => {
                let on_time = 1.0 - stat.share_late_6 as f64;
                final_on_time[i] = Some((on_time, stat.n));
                // Both, from one lookup: the flattened score every existing consumer
                // reads, and the cell it came from for the ones that want the sample
                // size with it.
                journeys[i].delay_risk_score = Some(stat.share_late_6 as f64);
                journeys[i].arrival_punctuality = Some(stat.into());
            }
            Want::LegArrival { journey, leg } => {
                if let Some(l) = journeys[journey].legs.get_mut(leg) {
                    l.on_time_probability = Some(1.0 - stat.share_late_6 as f64);
                }
            }
            Want::Transfer {
                journey,
                station,
                buffer,
            } => {
                // Outer `None` means nobody asked, which cannot happen here. Inner
                // `None` means the row predates the stored histogram, so this
                // transfer genuinely cannot be scored -- and a journey missing one
                // transfer term gets no composed number at all rather than a product
                // over the transfers that happened to answer.
                let Some(Some(share)) = stat.share_delay_at_least else {
                    continue;
                };
                transfers[journey].push(TransferReliability {
                    station,
                    buffer_minutes: buffer,
                    catch_probability: 1.0 - share,
                    n: stat.n,
                });
            }
        }
    }

    for (i, journey) in journeys.iter_mut().enumerate() {
        // How many transfers the journey HAS, not how many answered. The count is
        // what makes a missing term fail the composition instead of shrinking it.
        let expected = journey.legs.len().saturating_sub(1);
        journey.reliability = compose_reliability(
            std::mem::take(&mut transfers[i]),
            final_on_time[i],
            expected,
        );
    }
}

/// Same enrichment for a split-ticket result's segments.
///
/// A segment now carries its journey beside the train-match verdict, so the
/// journeys are lifted out, enriched as a slice, and put back rather than
/// enriching in place: `enrich` takes `&mut [Journey]` and keeping it that way
/// leaves it usable by every caller that has no split-ticket in hand.
pub fn enrich_split(result: &mut SplitResult) {
    let mut journeys: Vec<_> = result.segments.iter().map(|s| s.journey.clone()).collect();
    enrich(&mut journeys);
    for (segment, journey) in result.segments.iter_mut().zip(journeys) {
        segment.journey = journey;
    }
    enrich_boundaries(result);
}

/// Fills each contract boundary's `incoming_share_late_6`: how often the
/// arriving train's type ran >= 6 minutes late at that station in that hour.
/// Context for the transfer buffer, and deliberately not a probability of making
/// it: this is the six-minute figure, which answers a different question than a
/// four-minute buffer poses. `enrich` asks each transfer at its own buffer and
/// composes those into `Journey::reliability`; a contract boundary is about who
/// owes you a refund when it goes wrong, which no exceedance answers. Same
/// degradation contract: punctuality being absent leaves the boundaries as
/// the solver built them.
fn enrich_boundaries(result: &mut SplitResult) {
    let mut targets: Vec<usize> = Vec::new();
    let mut stops: Vec<StopQuery> = Vec::new();
    for (idx, boundary) in result.contract_boundaries.iter().enumerate() {
        // Boundary i sits between segments i and i+1; the arriving train is
        // segment i's last leg.
        let Some(arriving) = result.segments.get(idx).and_then(|s| s.journey.legs.last()) else {
            continue;
        };
        let Some(eva) = eva_of(&boundary.station.id) else {
            continue;
        };
        let Some((hour, weekend)) = hour_and_weekend(&arriving.arrival_time) else {
            continue;
        };
        let Some(train_type) = train_type_of(arriving) else {
            continue;
        };
        targets.push(idx);
        stops.push(StopQuery {
            eva,
            train_type: train_type.to_string(),
            hour,
            weekend,
            // The six-minute figure, deliberately: this field is context for a
            // buffer, not a probability of catching it. The buffer's own threshold
            // is asked in `enrich`, where the answer becomes one.
            at_least_minutes: None,
        });
    }
    if stops.is_empty() {
        return;
    }
    let Some(stats) = lookup(stops) else { return };
    for (idx, stat) in targets.into_iter().zip(stats) {
        if let Some(stat) = stat {
            result.contract_boundaries[idx].incoming_share_late_6 = Some(stat.share_late_6 as f64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eva_comes_out_of_both_station_id_formats() {
        // The suggest endpoint's bare form.
        assert_eq!(eva_of("8000044").as_deref(), Some("8000044"));
        // The journey-search location string, verified against a live response.
        assert_eq!(
            eva_of("A=1@O=Bonn Hbf@X=7097136@Y=50732008@U=80@L=8000044@i=U×008015485@").as_deref(),
            Some("8000044")
        );
        assert_eq!(
            eva_of("A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@").as_deref(),
            Some("8000207")
        );
    }

    #[test]
    fn a_station_id_without_an_eva_yields_nothing() {
        // Better no lookup than a lookup on a wrong key: a wrong key returns a real
        // statistic for a different station, which is worse than no statistic.
        assert_eq!(eva_of(""), None);
        assert_eq!(eva_of("A=1@O=Somewhere@X=1@Y=2@"), None);
        assert_eq!(eva_of("A=1@O=X@L=@i=1@"), None);
        assert_eq!(eva_of("A=1@O=X@L=abc@i=1@"), None);
    }

    #[test]
    fn weekday_matches_the_calendar() {
        // 2026-07-28 is a Tuesday, 2026-08-01 a Saturday, 2026-08-02 a Sunday.
        assert_eq!(hour_and_weekend("2026-07-28T14:32:00"), Some((14, false)));
        assert_eq!(hour_and_weekend("2026-08-01T09:00:00"), Some((9, true)));
        assert_eq!(hour_and_weekend("2026-08-02T23:59:00"), Some((23, true)));
        // January and February take Sakamoto's previous-year branch.
        assert_eq!(hour_and_weekend("2026-01-01T00:00:00"), Some((0, false))); // Thursday
        assert_eq!(hour_and_weekend("2026-02-28T12:00:00"), Some((12, true))); // Saturday
                                                                               // A leap year's 29 February -- 2024-02-29 was a Thursday.
        assert_eq!(hour_and_weekend("2024-02-29T08:00:00"), Some((8, false)));
    }

    #[test]
    fn a_malformed_timestamp_yields_nothing() {
        assert_eq!(hour_and_weekend("2026-07-28"), None);
        assert_eq!(hour_and_weekend(""), None);
        assert_eq!(hour_and_weekend("2026-07-28T99:00:00"), None);
        assert_eq!(hour_and_weekend("not-a-date T12:00:00"), None);
    }

    #[test]
    fn enrichment_of_nothing_does_nothing() {
        // Guards the early return: an empty search must not open a connection.
        let mut none: Vec<Journey> = Vec::new();
        enrich(&mut none);
        assert!(none.is_empty());
    }

    fn station(name: &str, eva: &str) -> Station {
        Station {
            id: eva.to_string(),
            name: name.to_string(),
            latitude: None,
            longitude: None,
        }
    }

    fn transfer(name: &str, buffer: i64, catch: f64, n: i64) -> TransferReliability {
        TransferReliability {
            station: station(name, "8000207"),
            buffer_minutes: buffer,
            catch_probability: catch,
            n,
        }
    }

    /// A direct journey has nothing to catch, so its reliability is exactly the
    /// chance it arrives on time -- not a product over an empty set dressed up as
    /// something more.
    #[test]
    fn a_direct_journey_is_just_its_arrival() {
        let r = compose_reliability(Vec::new(), Some((0.82, 4100)), 0).expect("composable");
        assert_eq!(r.probability, 0.82);
        assert_eq!(r.final_leg_on_time, 0.82);
        assert_eq!(r.min_sample, 4100);
        assert_eq!(r.threshold_minutes, 6);
        assert!(r.transfers.is_empty());
    }

    /// Every transfer multiplies, and the sample reported is the thinnest cell in
    /// the product -- the number is only as measured as its weakest term.
    #[test]
    fn transfers_multiply_and_the_thinnest_cell_is_the_one_reported() {
        let r = compose_reliability(
            vec![
                transfer("Köln Hbf", 12, 0.9, 5000),
                transfer("Hamm", 4, 0.5, 31),
            ],
            Some((0.8, 4100)),
            2,
        )
        .expect("composable");
        assert!(
            (r.probability - 0.36).abs() < 1e-9,
            "0.9 * 0.5 * 0.8, got {}",
            r.probability
        );
        assert_eq!(r.min_sample, 31, "not the 4100 the arrival rests on");
    }

    /// The failure this exists to prevent: a journey with two transfers where only
    /// one could be scored must produce no number at all.
    ///
    /// Multiplying the transfers that happened to answer would return a *higher*
    /// reliability than the journey has, and it would look identical to a journey
    /// that genuinely only had one transfer. Absence is the honest answer, and it
    /// is the same contract every other field in this module keeps.
    #[test]
    fn a_transfer_that_could_not_be_scored_costs_the_whole_number() {
        let one_answered = vec![transfer("Köln Hbf", 12, 0.9, 5000)];
        assert!(
            compose_reliability(one_answered.clone(), Some((0.8, 4100)), 2).is_none(),
            "two transfers, one term: no number"
        );
        assert!(
            compose_reliability(one_answered, Some((0.8, 4100)), 1).is_some(),
            "one transfer, one term: composable"
        );
    }

    /// No arrival cell, no product. A journey whose destination has no history is
    /// not a reliable journey, it is an unmeasured one.
    #[test]
    fn an_unknown_arrival_leaves_the_journey_unscored() {
        assert!(compose_reliability(vec![transfer("Köln Hbf", 12, 0.9, 5000)], None, 1).is_none());
    }

    /// The buffer is read from the UTC instants, never the naive local strings.
    ///
    /// Köln 09:28 CEST to London 09:45 BST subtracts to 17 minutes as wall clock and
    /// is really 77. A buffer wrong by a zone delta asks the histogram the wrong
    /// question, and asks it confidently.
    #[test]
    fn the_buffer_comes_from_the_utc_instants_not_the_wall_clock() {
        let mut arriving = leg_at("2026-09-15T09:28:00", Some("2026-09-15T07:28:00Z"));
        let departing = leg_at("2026-09-15T09:45:00", Some("2026-09-15T08:45:00Z"));
        assert_eq!(buffer_minutes(&arriving, &departing), Some(77));

        // Same clock, same zone: the ordinary case still reads straight.
        let same_zone = leg_at("2026-09-15T09:45:00", Some("2026-09-15T07:45:00Z"));
        assert_eq!(buffer_minutes(&arriving, &same_zone), Some(17));

        // An absent instant is an unknown buffer, and an unknown buffer is never
        // scored as a comfortable one.
        arriving.arrival_utc = None;
        assert_eq!(buffer_minutes(&arriving, &departing), None);
    }

    fn leg_at(local: &str, utc: Option<&str>) -> Leg {
        Leg {
            origin: station("Köln Hbf", "8000207"),
            destination: station("Köln Hbf", "8000207"),
            departure_time: local.to_string(),
            arrival_time: local.to_string(),
            scheduled_departure: None,
            realtime_departure: None,
            scheduled_arrival: None,
            realtime_arrival: None,
            departure_utc: utc.map(str::to_string),
            arrival_utc: utc.map(str::to_string),
            cancelled: false,
            train_name: "ICE 857".into(),
            train_number: "857".into(),
            train_category: "ICE".into(),
            platform: None,
            is_regional: false,
            on_time_probability: None,
        }
    }

    fn named(train_name: &str, train_category: &str) -> Leg {
        let mut leg = leg_at("2026-08-24T09:04:00", Some("2026-08-24T07:04:00Z"));
        leg.train_name = train_name.into();
        leg.train_category = train_category.into();
        leg
    }

    /// Every pair here was read off a live search on 2026-08-20, and every type on
    /// the right exists in the ingested cells. The two `DRB` rows are the whole
    /// argument against a category table: one product class, two populations
    /// punctuality holds apart.
    #[test]
    fn the_train_type_is_the_label_not_the_product_class() {
        // dbnav: the label is right where the class is wrong. `produktGattung` says
        // `RB` for an RE5, and RE and RB are 10.8M and 14.4M separate observations.
        assert_eq!(train_type_of(&named("RE5", "RB")), Some("RE"));
        assert_eq!(train_type_of(&named("RB26", "RB")), Some("RB"));
        assert_eq!(train_type_of(&named("S12", "SBAHN")), Some("S"));
        // `IC_EC` collapses two types the cells keep apart, 408k and 35k.
        assert_eq!(train_type_of(&named("IC 2067", "IC_EC")), Some("IC"));
        assert_eq!(train_type_of(&named("EC 135", "IC_EC")), Some("EC"));
        assert_eq!(train_type_of(&named("ICE 1022", "ICE")), Some("ICE"));
        assert_eq!(train_type_of(&named("FLX 1237", "IR")), Some("FLX"));

        // dbweb: same trains, HAFAS class codes, and the same right answer from the
        // label wherever there is one.
        assert_eq!(train_type_of(&named("ICE 857", "ICE")), Some("ICE"));
        assert_eq!(train_type_of(&named("S6", "DBS")), Some("S"));
        assert_eq!(train_type_of(&named("EUR 9411", "THA")), Some("EUR"));
    }

    /// dbweb names a regional train by its bare number, so there is nothing to read a
    /// type off. `None` is the whole point: a guess here would answer an RE journey
    /// with whatever population the class code happened to collapse into.
    #[test]
    fn a_bare_number_yields_no_type_rather_than_a_guess() {
        // The RE5 and the RB26 Bonn -> Köln, exactly as dbweb returns them.
        assert_eq!(train_type_of(&named("28510", "DRB")), None);
        assert_eq!(train_type_of(&named("33602", "DRB")), None);
        assert_eq!(train_type_of(&named("16511", "NRE")), None);
        assert_eq!(train_type_of(&named("", "DRB")), None);
        assert_eq!(train_type_of(&named("   ", "DRB")), None);
    }

    /// The distinction the field exists to make: nobody asked, and which legs.
    /// Filled before the lookup, so it survives punctuality being down -- which is
    /// also what makes this assertable without a running service.
    #[test]
    fn an_unscorable_leg_is_reported_rather_than_left_silent() {
        let mut journeys = vec![Journey {
            id: "j".into(),
            start_station: station("Bonn Hbf", "8000044"),
            end_station: station("Köln Hbf", "8000207"),
            legs: vec![named("28510", "DRB"), named("ICE 857", "ICE")],
            total_duration_minutes: 60,
            total_price: None,
            delay_risk_score: None,
            arrival_punctuality: None,
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
        }];

        enrich(&mut journeys);

        assert_eq!(journeys[0].unscored_legs.len(), 1);
        let unscored = &journeys[0].unscored_legs[0];
        assert_eq!(unscored.leg_index, 0);
        assert_eq!(unscored.train_name, "28510");
        assert_eq!(unscored.train_category, "DRB");
        // The labelled leg is not in the list; absence of a cell is a different state
        // from absence of a question, and only the second one lands here.
        assert!(journeys[0]
            .unscored_legs
            .iter()
            .all(|u| u.train_name != "ICE 857"));
    }
}
