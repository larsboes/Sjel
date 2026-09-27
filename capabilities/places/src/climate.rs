//! Climate normals per place per month, from the Open-Meteo *archive* endpoint,
//! cached permanently in `places_climate_normals` (README D5,
//! `upstreams.toml` [open-meteo-api]).
//!
//! Modelled on `geocode.rs`, and the four constraints it enforces rather than
//! remembers are enforced here too:
//!
//! - **A stored place never egresses again** (ISA PLC-14): the stored rows are
//!   read before a network client exists for the request.
//! - **A courtesy throttle** spaces provider requests process-globally, whatever
//!   thread asks, because the endpoint is free and keyless.
//! - **The request carries a rounded coordinate pair and a fixed period only**
//!   (ISA PLC-13): six query parameters, no plan date, no person, no amount.
//! - **A provider error writes no row** and its `Display` is stripped of the
//!   request URL, because that URL *is* a coordinate (ISA PLC-13).
//!
//! What is data and what is code: the twelve rows are measurements and are
//! stored; `best_months` is a rule and is computed on read, so changing the rule
//! never leaves twelve stale verdicts per place behind.

use crate::store::{MonthlyNormal, NormalsMeta, Place, PlacesStore};
use serde::Deserialize;
use std::time::{Duration, Instant};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The one place the provider endpoint is named. The word that matters is
/// *archive*: it answers with daily observations over a closed historical
/// window, which is what a normal is computed from — not a forecast
/// (PRD §8.2, "climate normals, not forecast").
pub const ARCHIVE_URL: &str = "https://archive-api.open-meteo.com/v1/archive";

/// Attribution is a licence obligation, not decoration: Open-Meteo publishes the
/// archive data under CC BY 4.0 (`upstreams.toml [open-meteo-api]`). Every
/// response carries this string so the obligation travels with the data.
pub const ATTRIBUTION: &str = "Open-Meteo, CC BY 4.0";

/// A day counts as a rain day at or above this much precipitation. One
/// millimetre is the threshold the published rule names, so it lives here as a
/// constant rather than as a literal inside the fold.
pub const RAIN_DAY_MM: f64 = 1.0;

/// How many complete calendar years one normal is folded from.
pub const NORMALS_YEARS: i64 = 10;

/// How far a bare coordinate may sit from a registered place before the answer
/// is a stated refusal rather than a match.
///
/// ASSUMPTION, not a measurement. The companion register uses 30 km for a
/// person's home (`backfill.rs`), which is an address-scale claim; this is a
/// regional one, so it is looser. Every response carries `distance_km` and the
/// matched place precisely so a wrong match is visible rather than silent.
pub const CLIMATE_MATCH_RADIUS_KM: f64 = 60.0;

/// Decimal places the outbound coordinate is rounded to.
///
/// BOUND, not a measurement. Two decimals is roughly 1.1 km — coarser than a
/// street address, which is the property that matters, and the cache is
/// permanent so an over-precise egress cannot be taken back. Open-Meteo's own
/// grid resolution was NOT verified here and no claim is made about it; if a
/// later measurement shows the grid is coarser than 1.1 km, this constant should
/// go coarser with it and this is the only place that changes. The geocoder
/// rounds to four decimals for an address-scale reverse lookup and says why
/// (`geocode.rs`); this is the regional-scale case.
pub const EGRESS_DECIMALS: usize = 2;

/// The published sentence the `best_month` flag implements. Served in every
/// response so a reader never has to guess which rule produced the flag.
pub const BEST_MONTHS_RULE: &str =
    "t_max_mean 18-28 C and rain_days_mean below this place's own twelve-month median";

const SOURCE: &str = "open-meteo-archive";
const MIN_REQUEST_SPACING: Duration = Duration::from_secs(1);
/// Ten years of daily rows is a larger body than a geocode answer, so the
/// timeout is longer than `geocode.rs`'s twenty seconds rather than the same.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `SJEL_PLACES_OPEN_METEO_URL` overrides the endpoint for the CLI verb only.
/// `fetch_normals` takes its URL as a parameter instead, so the tests inject a
/// stub without touching process env — the reason `geocode.rs`'s own db_tests
/// pass a URL explicitly under cargo's thread-parallel harness.
pub fn archive_url() -> String {
    sjel_config::env_var("SJEL_PLACES_OPEN_METEO_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| ARCHIVE_URL.to_string())
}

/// One day as the archive reports it. Seconds for the two durations, because
/// that is the provider's unit; the fold converts, so nothing downstream has to
/// remember which unit a column is in.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyRow {
    pub date: String,
    pub t_max: Option<f64>,
    pub t_min: Option<f64>,
    pub precipitation_mm: Option<f64>,
    pub daylight_seconds: Option<f64>,
    pub sunshine_seconds: Option<f64>,
}

/// The archive response, narrowed to what the fold reads. Deserializing a narrow
/// shape rather than a `Value` is what makes an unexpected body an error here
/// instead of twelve rows of nulls in the table.
#[derive(Debug, Deserialize)]
struct ArchiveResponse {
    daily: ArchiveDaily,
}

#[derive(Debug, Deserialize)]
struct ArchiveDaily {
    time: Vec<String>,
    #[serde(default)]
    temperature_2m_max: Vec<Option<f64>>,
    #[serde(default)]
    temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    precipitation_sum: Vec<Option<f64>>,
    #[serde(default)]
    daylight_duration: Vec<Option<f64>>,
    #[serde(default)]
    sunshine_duration: Vec<Option<f64>>,
}

/// The ten most recent COMPLETE calendar years, as `(start, end, years)`.
///
/// Complete on purpose: folding a partial current year would weight whichever
/// months have already happened, and the difference would be invisible in the
/// stored row. The current year is therefore never included, whatever the date.
pub fn normals_period(today: &str) -> (String, String, i64) {
    let year: i64 = today.get(..4).and_then(|y| y.parse().ok()).unwrap_or(1970);
    let end_year = year - 1;
    let start_year = end_year - NORMALS_YEARS + 1;
    (
        format!("{start_year}-01-01"),
        format!("{end_year}-12-31"),
        NORMALS_YEARS,
    )
}

#[derive(Default)]
struct MonthAccumulator {
    days: i64,
    t_max_sum: f64,
    t_max_days: i64,
    t_min_sum: f64,
    t_min_days: i64,
    precipitation_sum: f64,
    rain_days: i64,
    precipitation_years: Vec<String>,
    daylight_sum: f64,
    daylight_days: i64,
    sunshine_sum: f64,
    sunshine_days: i64,
}

fn mean(sum: f64, count: i64) -> Option<f64> {
    (count > 0).then(|| sum / count as f64)
}

/// Fold daily observations into at most twelve rows.
///
/// The units, stated once here because nothing downstream can infer them:
/// `t_max_mean`/`t_min_mean` are degrees Celsius averaged per day;
/// `daylight_hours_mean`/`sunshine_hours_mean` are hours per day;
/// `rain_days_mean` is the count of days at or above `RAIN_DAY_MM` in a typical
/// year's instance of that month; `precipitation_mm_mean` is that month's total
/// in a typical year. The two per-year figures divide by the number of distinct
/// years that carried a precipitation reading, so a month present in six of ten
/// years reports a six-year average rather than a suppressed one.
///
/// A month with no rows at all is ABSENT from the result. It is not a row of
/// zeroes: a guess that looks like a measurement is worse than a blank
/// (Packs/travel/ISA.md).
pub fn aggregate_daily(days: &[DailyRow]) -> Vec<MonthlyNormal> {
    let mut months: Vec<MonthAccumulator> = (0..12).map(|_| MonthAccumulator::default()).collect();
    for day in days {
        let Some(month) = day
            .date
            .get(5..7)
            .and_then(|m| m.parse::<usize>().ok())
            .filter(|m| (1..=12).contains(m))
        else {
            continue;
        };
        let year = day.date.get(..4).unwrap_or("").to_string();
        let slot = &mut months[month - 1];
        slot.days += 1;
        if let Some(value) = day.t_max {
            slot.t_max_sum += value;
            slot.t_max_days += 1;
        }
        if let Some(value) = day.t_min {
            slot.t_min_sum += value;
            slot.t_min_days += 1;
        }
        if let Some(value) = day.precipitation_mm {
            slot.precipitation_sum += value;
            if value >= RAIN_DAY_MM {
                slot.rain_days += 1;
            }
            if !slot.precipitation_years.contains(&year) {
                slot.precipitation_years.push(year);
            }
        }
        if let Some(value) = day.daylight_seconds {
            slot.daylight_sum += value;
            slot.daylight_days += 1;
        }
        if let Some(value) = day.sunshine_seconds {
            slot.sunshine_sum += value;
            slot.sunshine_days += 1;
        }
    }

    months
        .into_iter()
        .enumerate()
        .filter(|(_, slot)| slot.days > 0)
        .map(|(index, slot)| {
            let years = slot.precipitation_years.len() as i64;
            MonthlyNormal {
                month: index as u32 + 1,
                t_max_mean: mean(slot.t_max_sum, slot.t_max_days),
                t_min_mean: mean(slot.t_min_sum, slot.t_min_days),
                rain_days_mean: mean(slot.rain_days as f64, years),
                precipitation_mm_mean: mean(slot.precipitation_sum, years),
                daylight_hours_mean: mean(slot.daylight_sum / 3600.0, slot.daylight_days),
                sunshine_hours_mean: mean(slot.sunshine_sum / 3600.0, slot.sunshine_days),
                days_observed: slot.days,
            }
        })
        .collect()
}

/// The month numbers `BEST_MONTHS_RULE` flags.
///
/// A month qualifies only when BOTH measures exist and both clauses hold. A
/// place with no precipitation data flags nothing, because half a rule is not
/// the published rule and a reader cannot tell the two apart from the flag.
pub fn best_months(months: &[MonthlyNormal]) -> Vec<u32> {
    let mut rain: Vec<f64> = months.iter().filter_map(|m| m.rain_days_mean).collect();
    if rain.is_empty() {
        return Vec::new();
    }
    rain.sort_by(f64::total_cmp);
    let median = if rain.len() % 2 == 1 {
        rain[rain.len() / 2]
    } else {
        (rain[rain.len() / 2 - 1] + rain[rain.len() / 2]) / 2.0
    };
    months
        .iter()
        .filter(|month| {
            month.t_max_mean.is_some_and(|t| (18.0..=28.0).contains(&t))
                && month.rain_days_mean.is_some_and(|r| r < median)
        })
        .map(|month| month.month)
        .collect()
}

/// How a bare coordinate was turned into a registered place, or why it was not.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// The coordinate IS a registry row's coordinate, to four decimals. This is
    /// the row `backfill travelers` minted from the same `f64` a trips
    /// destination carries, resolved server-side in one call rather than by the
    /// caller recomputing `external_ref` and listing every registry row.
    Registry { place: Place, distance_km: f64 },
    /// The nearest registered place that actually carries normals, within
    /// `CLIMATE_MATCH_RADIUS_KM`.
    Nearest { place: Place, distance_km: f64 },
    /// Nothing close enough. A stated refusal, never the nearest anything.
    Unmatched { reason: String },
}

impl Resolution {
    pub fn resolved_by(&self) -> Option<&'static str> {
        match self {
            Self::Registry { .. } => Some("registry"),
            Self::Nearest { .. } => Some("nearest"),
            Self::Unmatched { .. } => None,
        }
    }

    pub fn place(&self) -> Option<&Place> {
        match self {
            Self::Registry { place, .. } | Self::Nearest { place, .. } => Some(place),
            Self::Unmatched { .. } => None,
        }
    }
}

/// The exact registry row when it can answer, then nearest-with-normals, then
/// the exact row anyway, then a stated refusal.
///
/// The order is the whole rule. An exact-coordinate row IS the place the caller
/// means, so it answers first — but only when it carries normals. Only cities
/// are fetched by default (`main.rs`; a venue's climate is its city's climate
/// and 134 venue coordinates are not worth egressing), and the reason that is
/// safe is precisely that a station or a venue is served from a city within
/// 60 km. A registry step that short-circuits on a row with no normals defeats
/// its own justification: it hands back an empty strip for exactly the rows the
/// default fetch skips, and tells the operator to run the command that skips
/// them.
///
/// The exact row still wins over nothing (step 3), so the empty state names the
/// right place, and a distance match is always reported as `nearest` with its
/// distance, so a wrong 60 km match is visible rather than silent.
pub fn resolve_at(places: &[(Place, bool)], latitude: f64, longitude: f64) -> Resolution {
    let same = |a: f64, b: f64| format!("{a:.4}") == format!("{b:.4}");
    let exact = places.iter().find(|(place, _)| {
        place
            .latitude
            .zip(place.longitude)
            .is_some_and(|(lat, lon)| same(lat, latitude) && same(lon, longitude))
    });
    if let Some((place, true)) = exact {
        return Resolution::Registry {
            place: place.clone(),
            distance_km: 0.0,
        };
    }
    let nearest = places
        .iter()
        .filter(|(_, has_normals)| *has_normals)
        .filter_map(|(place, _)| {
            let (lat, lon) = place.latitude.zip(place.longitude)?;
            Some((
                place,
                crate::backfill::haversine_km((lat, lon), (latitude, longitude)),
            ))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match nearest {
        Some((place, distance_km)) if distance_km <= CLIMATE_MATCH_RADIUS_KM => {
            Resolution::Nearest {
                place: place.clone(),
                distance_km,
            }
        }
        // Nothing carries normals nearby. The exact row is still the right place
        // to name, and the strip's empty state names the fetch verb for it.
        _ => match exact {
            Some((place, _)) => Resolution::Registry {
                place: place.clone(),
                distance_km: 0.0,
            },
            None => Resolution::Unmatched {
                reason: format!(
                    "no registered place with normals within {CLIMATE_MATCH_RADIUS_KM:.0} km"
                ),
            },
        },
    }
}

/// Last provider-request instant, process-global: two fetchers in one process
/// share the same public endpoint, so they share the same budget.
fn last_request() -> &'static std::sync::Mutex<Option<Instant>> {
    static LAST: std::sync::OnceLock<std::sync::Mutex<Option<Instant>>> =
        std::sync::OnceLock::new();
    LAST.get_or_init(|| std::sync::Mutex::new(None))
}

fn throttle() {
    let mut last = last_request()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < MIN_REQUEST_SPACING {
            std::thread::sleep(MIN_REQUEST_SPACING - elapsed);
        }
    }
    *last = Some(Instant::now());
}

/// What one `fetch_normals` call did.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchOutcome {
    pub months: usize,
    /// True when the stored rows answered and nothing left the host.
    pub cached: bool,
}

/// Fetch one place's normals, or serve the stored ones.
///
/// `url` is a parameter, never an env read, so parallel tests inject a stub
/// without racing on process env. `force` is the refresh path a permanent cache
/// needs; without it a place egresses at most once (ISA PLC-14).
///
/// A `kind = 'address'` place is refused before a request is built (ISA PLC-15):
/// a home address has no business in a climate request at any precision, and its
/// city row answers the same question.
pub fn fetch_normals(
    store: &PlacesStore,
    place: &Place,
    url: &str,
    today: &str,
    force: bool,
) -> Fallible<FetchOutcome> {
    if place.kind == "address" {
        return Err(format!(
            "refusing to fetch climate for an address-kind place ({}); \
             fetch its city instead",
            place.id
        )
        .into());
    }
    let (Some(latitude), Some(longitude)) = (place.latitude, place.longitude) else {
        return Err(format!("place {} carries no coordinate", place.id).into());
    };

    // Stored first: a place with rows answers without a network client ever
    // being built (PLC-14).
    if !force {
        let stored = store.climate_get(&place.id)?;
        if !stored.is_empty() {
            return Ok(FetchOutcome {
                months: stored.len(),
                cached: true,
            });
        }
    }

    let (period_start, period_end, years) = normals_period(today);
    throttle();
    let client = sjel_http::client(sjel_http::Purpose::new("places-climate"), REQUEST_TIMEOUT)?;
    // Exactly six parameters, and PLC-13's falsifier is a seventh: the plan's
    // dates, a traveler, an amount and a place name all have nowhere to go here.
    let response = client
        .get(url)
        .query(&[
            ("latitude", format!("{:.*}", EGRESS_DECIMALS, latitude)),
            ("longitude", format!("{:.*}", EGRESS_DECIMALS, longitude)),
            ("start_date", period_start.clone()),
            ("end_date", period_end.clone()),
            (
                "daily",
                "temperature_2m_max,temperature_2m_min,precipitation_sum,\
                 daylight_duration,sunshine_duration"
                    .to_string(),
            ),
            ("timezone", "auto".to_string()),
        ])
        // `without_url`: reqwest's error Display appends the request URL, and
        // that URL is a coordinate pair. Fetch errors reach stderr verbatim
        // (main.rs), and a coordinate in a log line is what PLC-13 exists to
        // prevent.
        .send()
        .map_err(reqwest::Error::without_url)?;
    let status = response.status();
    if !status.is_success() {
        // NOT cached. The table is the cache and it is permanent, so a stored
        // failure would turn one bad minute into a place that can never be
        // resolved (the geocode cache's own do-not-cache-errors rule).
        return Err(format!("open-meteo answered {status}").into());
    }
    let body: ArchiveResponse = response.json().map_err(reqwest::Error::without_url)?;
    let days = daily_rows(&body.daily);
    let months = aggregate_daily(&days);
    if months.is_empty() {
        return Err("open-meteo returned no daily observations".into());
    }
    let meta = NormalsMeta {
        years_covered: years,
        period_start,
        period_end,
        source: SOURCE.to_string(),
        fetched_at: today.to_string(),
    };
    let written = store.climate_put(&place.id, &months, &meta)?;
    Ok(FetchOutcome {
        months: written,
        cached: false,
    })
}

fn daily_rows(daily: &ArchiveDaily) -> Vec<DailyRow> {
    let at = |values: &[Option<f64>], index: usize| values.get(index).copied().flatten();
    daily
        .time
        .iter()
        .enumerate()
        .map(|(index, date)| DailyRow {
            date: date.clone(),
            t_max: at(&daily.temperature_2m_max, index),
            t_min: at(&daily.temperature_2m_min, index),
            precipitation_mm: at(&daily.precipitation_sum, index),
            daylight_seconds: at(&daily.daylight_duration, index),
            sunshine_seconds: at(&daily.sunshine_duration, index),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str, t_max: f64, precipitation: f64) -> DailyRow {
        DailyRow {
            date: date.into(),
            t_max: Some(t_max),
            t_min: Some(t_max - 8.0),
            precipitation_mm: Some(precipitation),
            daylight_seconds: Some(9.0 * 3600.0),
            sunshine_seconds: Some(4.0 * 3600.0),
        }
    }

    fn month(number: u32, t_max: f64, rain_days: f64) -> MonthlyNormal {
        MonthlyNormal {
            month: number,
            t_max_mean: Some(t_max),
            t_min_mean: Some(t_max - 8.0),
            rain_days_mean: Some(rain_days),
            precipitation_mm_mean: Some(rain_days * 4.0),
            daylight_hours_mean: Some(11.0),
            sunshine_hours_mean: Some(5.0),
            days_observed: 300,
        }
    }

    #[test]
    fn the_period_is_the_ten_most_recent_complete_calendar_years() {
        assert_eq!(
            normals_period("2026-09-05"),
            ("2016-01-01".into(), "2025-12-31".into(), 10)
        );
        // The current year is never included, whatever the date inside it.
        assert_eq!(normals_period("2026-01-01").1, "2025-12-31");
    }

    #[test]
    fn monthly_normals_fold_ten_years_of_daily_rows() {
        let days = vec![
            day("2023-01-01", 10.0, 2.0),
            day("2023-01-02", 12.0, 0.9),
            day("2024-01-01", 8.0, 5.0),
            day("2024-01-02", 14.0, 0.0),
            day("2025-01-01", 6.0, 1.0),
            day("2025-01-02", 16.0, 0.0),
        ];
        let folded = aggregate_daily(&days);
        assert_eq!(folded.len(), 1);
        let january = &folded[0];
        assert_eq!(january.month, 1);
        assert_eq!(january.days_observed, 6);
        assert_eq!(january.t_max_mean, Some(11.0));
        assert_eq!(january.t_min_mean, Some(3.0));
        // 2.0, 5.0 and 1.0 clear the 1.0 mm bar; the 0.9 mm day does not.
        assert_eq!(
            january.rain_days_mean,
            Some(1.0),
            "three rain days / 3 years"
        );
        assert_eq!(january.precipitation_mm_mean, Some(8.9 / 3.0));
        // Seconds in, hours per day out.
        assert_eq!(january.daylight_hours_mean, Some(9.0));
        assert_eq!(january.sunshine_hours_mean, Some(4.0));
    }

    #[test]
    fn a_month_with_no_observed_days_is_absent_not_zero() {
        let days = vec![day("2024-01-05", 5.0, 0.0), day("2024-03-05", 12.0, 0.0)];
        let folded = aggregate_daily(&days);
        assert_eq!(
            folded.iter().map(|m| m.month).collect::<Vec<_>>(),
            vec![1, 3],
            "February had no rows, so it must be absent rather than a row of zeroes"
        );
        assert!(folded.iter().all(|m| m.days_observed > 0));
    }

    #[test]
    fn a_measure_the_provider_omits_stays_null() {
        let days = vec![DailyRow {
            date: "2024-06-01".into(),
            t_max: Some(24.0),
            t_min: Some(14.0),
            precipitation_mm: Some(0.0),
            daylight_seconds: None,
            sunshine_seconds: None,
        }];
        let folded = aggregate_daily(&days);
        assert_eq!(folded[0].sunshine_hours_mean, None, "null, never 0.0");
        assert_eq!(folded[0].daylight_hours_mean, None);
        assert_eq!(folded[0].days_observed, 1);
    }

    #[test]
    fn best_months_applies_the_stated_rule() {
        // Rain days sorted: 2,2,2,2,4,4,6,8,10,10,12,12 -> median (4+6)/2 = 5.
        let months: Vec<MonthlyNormal> = vec![
            month(1, 22.0, 2.0),  // in range, dry -> flagged
            month(2, 30.0, 2.0),  // too hot
            month(3, 22.0, 10.0), // in range, wetter than the median
            month(4, 17.0, 2.0),  // too cold
            month(5, 28.0, 4.0),  // the top of the range is inclusive
            month(6, 24.0, 12.0),
            month(7, 24.0, 6.0), // in range, just above the median
            month(8, 24.0, 8.0),
            month(9, 24.0, 10.0),
            month(10, 24.0, 12.0),
            month(11, 24.0, 2.0), // flagged
            month(12, 24.0, 4.0), // flagged
        ];
        assert_eq!(best_months(&months), vec![1, 5, 11, 12]);
    }

    #[test]
    fn a_place_with_no_precipitation_data_flags_nothing() {
        let mut months = vec![month(6, 22.0, 3.0)];
        months[0].rain_days_mean = None;
        assert!(
            best_months(&months).is_empty(),
            "half the published rule is not the published rule"
        );
    }

    fn place(id: &str, latitude: f64, longitude: f64) -> Place {
        Place {
            id: id.into(),
            name: id.into(),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "test".into(),
            external_ref: None,
        }
    }

    #[test]
    fn a_matching_coordinate_resolves_to_the_registry_row_before_any_distance_search() {
        // The registry row sits at the exact coordinate a trips PlaceRef carries
        // and HAS normals, so no distance search happens at all.
        let places = vec![
            (place("exact", 48.208_49, 16.372_08), true),
            (place("nearer_with_normals", 48.250_00, 16.372_08), true),
        ];
        let resolution = resolve_at(&places, 48.208_49, 16.372_08);
        assert_eq!(resolution.resolved_by(), Some("registry"));
        assert_eq!(resolution.place().unwrap().id, "exact");
        match resolution {
            Resolution::Registry { distance_km, .. } => assert_eq!(distance_km, 0.0),
            other => panic!("expected a registry hit, got {other:?}"),
        }
    }

    /// The station case, measured on live shapes: a plan destination whose exact
    /// registry row is a `station`, which the default fetch verb never covers.
    /// Short-circuiting on it left that destination with a permanently empty
    /// strip; the city 5 km away is the answer the fetch policy assumes.
    #[test]
    fn an_exact_row_without_normals_yields_to_a_nearby_one_that_has_them() {
        let places = vec![
            (place("exact_station", 48.208_49, 16.372_08), false),
            (place("city_with_normals", 48.250_00, 16.372_08), true),
        ];
        let resolution = resolve_at(&places, 48.208_49, 16.372_08);
        assert_eq!(resolution.resolved_by(), Some("nearest"));
        assert_eq!(resolution.place().unwrap().id, "city_with_normals");

        // With nothing else in range the exact row is still named, so the empty
        // state is about the right place rather than about a refusal.
        let alone = vec![(place("exact_station", 48.208_49, 16.372_08), false)];
        let fallback = resolve_at(&alone, 48.208_49, 16.372_08);
        assert_eq!(fallback.resolved_by(), Some("registry"));
        assert_eq!(fallback.place().unwrap().id, "exact_station");
    }

    #[test]
    fn a_coordinate_with_no_registry_row_resolves_to_the_nearest_place_with_normals_or_to_nothing()
    {
        let places = vec![
            (place("ten_km", 48.298_0, 16.372_0), true),
            (place("hundred_km", 49.108_0, 16.372_0), true),
            (
                place("next_door_without_normals", 48.209_0, 16.372_0),
                false,
            ),
        ];
        let near = resolve_at(&places, 48.208_0, 16.372_0);
        assert_eq!(near.resolved_by(), Some("nearest"));
        assert_eq!(near.place().unwrap().id, "ten_km");
        match near {
            Resolution::Nearest { distance_km, .. } => {
                assert!((5.0..15.0).contains(&distance_km), "got {distance_km} km")
            }
            other => panic!("expected a nearest hit, got {other:?}"),
        }

        // 200 km from both: a stated refusal, not the nearest anything.
        let far = resolve_at(&places, 46.400_0, 16.372_0);
        assert_eq!(far.resolved_by(), None);
        assert!(far.place().is_none());
        match far {
            Resolution::Unmatched { reason } => assert!(reason.contains("60 km"), "{reason}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

/// Provider behaviour against a real store and a local stub endpoint. The URL is
/// passed explicitly, never through process env, for the reason `geocode.rs`'s
/// db_tests state: three tests mutating one global under a thread-parallel
/// harness is a flake, not a test.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::geocode::db_tests::stub_with;
    use crate::store::db_tests::open_test_store;
    use std::sync::atomic::Ordering;

    fn archive_body() -> String {
        // Two Januaries and one July, enough to prove the write shape.
        r#"{"latitude":48.2,"longitude":16.37,"daily":{
            "time":["2024-01-01","2024-01-02","2025-01-01","2024-07-01"],
            "temperature_2m_max":[4.0,6.0,2.0,26.0],
            "temperature_2m_min":[-1.0,1.0,-3.0,16.0],
            "precipitation_sum":[2.0,0.0,3.0,0.5],
            "daylight_duration":[30600,30600,30600,57600],
            "sunshine_duration":[7200,7200,7200,36000]}}"#
            .to_string()
    }

    fn city(id: &str) -> Place {
        Place {
            id: id.into(),
            name: "Synthetic City".into(),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: Some("at".into()),
            latitude: Some(48.208_49),
            longitude: Some(16.372_08),
            source: "test".into(),
            external_ref: Some(format!("test:{id}")),
        }
    }

    #[test]
    fn the_archive_request_carries_only_a_rounded_coordinate_and_the_fixed_period() {
        let (store, _path) = open_test_store("climatequery");
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = captured.clone();
        let (url, _hits) = stub_with(move |request| {
            let line = request.lines().next().unwrap_or_default().to_string();
            *sink.lock().unwrap() = line;
            archive_body()
        });
        let place = city("place_query");
        store.upsert_place(&place, "2026-09-05").unwrap();

        fetch_normals(&store, &place, &url, "2026-09-05", false).unwrap();

        let line = captured.lock().unwrap().clone();
        let query = line
            .split_whitespace()
            .nth(1)
            .and_then(|target| target.split_once('?').map(|(_, q)| q.to_string()))
            .expect("the stub captured a query string");
        let keys: Vec<&str> = query
            .split('&')
            .map(|pair| pair.split('=').next().unwrap_or_default())
            .collect();
        assert_eq!(
            keys,
            vec![
                "latitude",
                "longitude",
                "start_date",
                "end_date",
                "daily",
                "timezone"
            ],
            "PLC-13: a seventh parameter is the falsifier — {query}"
        );
        // Two decimals exactly, on both coordinates.
        assert!(query.contains("latitude=48.21"), "{query}");
        assert!(query.contains("longitude=16.37"), "{query}");
        // The fixed ten-year window, not any caller-supplied date.
        assert!(query.contains("start_date=2016-01-01"), "{query}");
        assert!(query.contains("end_date=2025-12-31"), "{query}");

        let stored = store.climate_get(&place.id).unwrap();
        assert_eq!(
            stored.len(),
            2,
            "January and July, not twelve rows of zeroes"
        );
        let meta = store.climate_meta(&place.id).unwrap().unwrap();
        assert_eq!(meta.period_start, "2016-01-01");
        assert_eq!(meta.years_covered, 10);
    }

    #[test]
    fn stored_normals_answer_without_a_provider_request() {
        let (store, _path) = open_test_store("climatecached");
        let (url, hits) = stub_with(move |_| archive_body());
        let place = city("place_cached");
        store.upsert_place(&place, "2026-09-05").unwrap();

        let first = fetch_normals(&store, &place, &url, "2026-09-05", false).unwrap();
        assert!(!first.cached);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Port 1 on loopback: nothing listens. A stored place must not need it.
        let second = fetch_normals(
            &store,
            &place,
            "http://127.0.0.1:1/archive",
            "2026-09-05",
            false,
        )
        .expect("stored rows answer without a client");
        assert!(
            second.cached,
            "PLC-14: a stored place egresses at most once"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_provider_error_writes_no_row() {
        let (store, _path) = open_test_store("climateerror");
        // Port 1 on loopback: the connection is refused, which is the same
        // do-not-cache path a 500 takes.
        let place = city("place_error");
        store.upsert_place(&place, "2026-09-05").unwrap();
        let error = fetch_normals(
            &store,
            &place,
            "http://127.0.0.1:1/archive",
            "2026-09-05",
            false,
        )
        .expect_err("a refused connection must surface as an error")
        .to_string();
        assert!(
            !error.contains("48.21") && !error.contains("16.37"),
            "the error must not carry the coordinate: {error}"
        );
        assert!(
            store.climate_get(&place.id).unwrap().is_empty(),
            "one bad minute must not freeze a place permanently"
        );
    }

    #[test]
    fn an_address_kind_place_is_refused_before_a_request_is_built() {
        let (store, _path) = open_test_store("climateaddress");
        let (url, hits) = stub_with(move |_| archive_body());
        let mut place = city("place_address");
        place.kind = "address".into();
        store.upsert_place(&place, "2026-09-05").unwrap();

        let error = fetch_normals(&store, &place, &url, "2026-09-05", false)
            .expect_err("PLC-15: an address coordinate never reaches the provider")
            .to_string();
        assert!(error.contains("city"), "the refusal names the fix: {error}");
        assert_eq!(hits.load(Ordering::SeqCst), 0, "no request was built");
    }
}
