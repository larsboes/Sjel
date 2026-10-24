//! Train connections from transit: station lookup, the direct search, and the split-ticket
//! calculation for the same route. The wire types follow `capabilities/transit/src/travel.rs`;
//! only what the page draws is read. The web page this replaces is
//! `dashboard/src/routes/travel/connections/+page.svelte`.

use chrono::{Local, NaiveDate, NaiveTime};
use serde::Deserialize;

use super::Source;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Station {
    /// EVA code, e.g. `8000105`.
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Leg {
    pub origin: Station,
    pub destination: Station,
    /// The time to plan around: real-time where HAFAS has one, scheduled otherwise.
    pub departure_time: String,
    pub arrival_time: String,
    #[serde(default)]
    pub scheduled_departure: Option<String>,
    #[serde(default)]
    pub realtime_departure: Option<String>,
    #[serde(default)]
    pub cancelled: bool,
    pub train_name: String,
    pub platform: Option<String>,
    /// Covered by the Deutschland-Ticket.
    pub is_regional: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Journey {
    pub id: String,
    pub start_station: Station,
    pub end_station: Station,
    pub legs: Vec<Leg>,
    pub total_duration_minutes: u32,
    /// `None` is "not priced here" (regional legs come back without a fare), never free.
    pub total_price: Option<f64>,
    /// The share of this train type's stops at the destination, in the arrival hour, that ran at
    /// least six minutes late (`capabilities/punctuality`). Not the chance the trip works out.
    pub delay_risk_score: Option<f64>,
    #[serde(default)]
    pub reliability: Option<Reliability>,
    /// Where the journey placed against the traveller's weights. Absent: transit's own order.
    #[serde(default)]
    pub ranking: Option<Ranking>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Reliability {
    /// How likely the whole journey holds together, from measured history.
    pub probability: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Ranking {
    pub rank: usize,
    pub factors: Vec<RankFactor>,
    /// `profile` or `request`: where the weights came from.
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RankFactor {
    pub label: String,
    pub weight: f64,
    pub rationale: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainMatch {
    Exact,
    Partial,
    Different,
    Unknown,
}

impl TrainMatch {
    /// Why a ticket may not be for the train you ride. `None` when it is.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Exact => None,
            Self::Partial => Some("partial train match"),
            Self::Different => Some("different train"),
            Self::Unknown => Some("train unknown"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SplitSegment {
    pub journey: Journey,
    pub train_match: TrainMatch,
    /// The trains the direct journey uses over this hop, in route order.
    pub expected_trains: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Partial,
    Low,
}

impl Confidence {
    /// Why the chain might not be buyable as shown. Each segment is priced by its own fare
    /// search, and nothing forces that search to return the train you are on.
    pub fn note(self) -> Option<&'static str> {
        match self {
            Self::Exact => None,
            Self::Partial => Some(
                "Some tickets could not be matched to the exact train on this route, or a fare \
                 lookup failed. Check each ticket before buying.",
            ),
            Self::Low => Some(
                "At least one ticket is priced for a different train than this journey uses. \
                 Buying this chain does not buy a seat on this trip.",
            ),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SplitResult {
    pub segments: Vec<SplitSegment>,
    /// `None` when the direct fare is unknown, which is not the same as free.
    pub original_price: Option<f64>,
    pub split_price: f64,
    pub savings: Option<f64>,
    pub confidence: Confidence,
    /// Stop pairs the solver wanted a fare for and got none.
    pub unpriced_pairs: usize,
    pub queried_pairs: usize,
}

impl SplitResult {
    pub fn saved_percent(&self) -> Option<i64> {
        let (savings, original) = (self.savings?, self.original_price?);
        (original > 0.0).then(|| (savings / original * 100.0).round() as i64)
    }
}

/// The presets transit knows (`capabilities/transit/src/ranking.rs`, `PRIORITIES`): the id is the
/// contract, the label and hint are this page's.
pub const PRIORITIES: [(&str, &str, &str); 5] = [
    ("balanced", "Balanced", "no term dominates"),
    ("cheapest", "Cheapest", "fare first"),
    ("fastest", "Fastest", "door to door"),
    ("fewest_changes", "Fewest changes", "direct where possible"),
    ("reliable", "Most reliable", "measured, not fitted"),
];

/// What one search asked for instead of the profile's usual weights. Transit refuses more than
/// one, so a typed phrase beats a pressed preset: it is the more specific thing to say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rank {
    pub priority: Option<&'static str>,
    pub phrase: String,
}

/// One connection search: two picked stations and a local date and time.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub from: Station,
    pub to: Station,
    pub date: String,
    pub time: String,
    pub rank: Rank,
}

impl Query {
    /// `YYYY-MM-DDTHH:MM:00`, or why the date or time cannot be read.
    pub fn when(&self) -> Result<String, String> {
        let date = NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d")
            .map_err(|_| "Write the date as 2026-10-25.".to_owned())?;
        let time = NaiveTime::parse_from_str(self.time.trim(), "%H:%M")
            .map_err(|_| "Write the time as 10:00.".to_owned())?;
        Ok(format!(
            "{}T{}:00",
            date.format("%Y-%m-%d"),
            time.format("%H:%M")
        ))
    }

    pub fn search_path(&self) -> Result<String, String> {
        let when = self.when()?;
        let mut pairs = vec![
            ("from", self.from.id.as_str()),
            ("to", self.to.id.as_str()),
            ("time", when.as_str()),
        ];
        let phrase = self.rank.phrase.trim();
        if !phrase.is_empty() {
            pairs.push(("phrase", phrase));
        } else if let Some(priority) = self.rank.priority {
            pairs.push(("priority", priority));
        }
        Ok(path("/transit/api/search", &pairs))
    }

    pub fn split_path(&self) -> Result<String, String> {
        let when = self.when()?;
        Ok(path(
            "/transit/api/split",
            &[
                ("from", &self.from.id),
                ("to", &self.to.id),
                ("time", &when),
            ],
        ))
    }
}

pub fn suggest_path(text: &str) -> String {
    path("/transit/api/suggest", &[("q", text.trim())])
}

fn path(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://localhost").expect("static base");
    url.set_path(base);
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// The date the form opens on.
pub fn tomorrow() -> String {
    (Local::now().date_naive() + chrono::Days::new(1))
        .format("%Y-%m-%d")
        .to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// Transit's order, or the ranking's when one came back.
    #[default]
    Ranked,
    Price,
    Duration,
    Departure,
}

impl Sort {
    pub const ALL: [Self; 4] = [Self::Ranked, Self::Price, Self::Duration, Self::Departure];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ranked => "Ranked",
            Self::Price => "Price",
            Self::Duration => "Duration",
            Self::Departure => "Departure",
        }
    }
}

/// The journeys in `sort` order. The sort is stable, so ties and missing values keep transit's
/// order, and a missing price or rank goes last.
pub fn ordered(journeys: &[Journey], sort: Sort) -> Vec<&Journey> {
    let mut rows: Vec<&Journey> = journeys.iter().collect();
    match sort {
        Sort::Ranked => rows.sort_by_key(|j| j.ranking.as_ref().map_or(usize::MAX, |r| r.rank)),
        Sort::Duration => rows.sort_by_key(|j| j.total_duration_minutes),
        Sort::Departure => rows.sort_by(|a, b| departure(a).cmp(departure(b))),
        Sort::Price => rows.sort_by(|a, b| {
            let price = |j: &Journey| j.total_price.unwrap_or(f64::INFINITY);
            price(a).total_cmp(&price(b))
        }),
    }
    rows
}

fn departure(journey: &Journey) -> &str {
    journey
        .legs
        .first()
        .map_or("", |l| l.departure_time.as_str())
}

impl Journey {
    pub fn changes(&self) -> usize {
        self.legs.len().saturating_sub(1)
    }

    pub fn departs(&self) -> &str {
        self.legs.first().map_or("", |l| clock(&l.departure_time))
    }

    pub fn arrives(&self) -> &str {
        self.legs.last().map_or("", |l| clock(&l.arrival_time))
    }

    /// Minutes the first leg leaves after its timetable, when HAFAS sent both times.
    pub fn delay_minutes(&self) -> Option<i64> {
        let leg = self.legs.first()?;
        let parse = |t: &Option<String>| chrono::DateTime::parse_from_rfc3339(t.as_deref()?).ok();
        let late = parse(&leg.realtime_departure)? - parse(&leg.scheduled_departure)?;
        Some(late.num_minutes()).filter(|m| *m != 0)
    }
}

/// `HH:MM` out of an ISO 8601 date-time, as transit's local time says it.
pub fn clock(iso: &str) -> &str {
    iso.get(11..16).unwrap_or(iso)
}

pub fn duration(minutes: u32) -> String {
    match minutes / 60 {
        0 => format!("{minutes} min"),
        hours => format!("{hours} h {} min", minutes % 60),
    }
}

pub fn price(value: Option<f64>) -> String {
    value.map_or("—".into(), |v| format!("{v:.2} €"))
}

/// `NN% late`: the share of comparable arrivals at least six minutes late. Named "late" and not
/// "risk", because it says nothing about catching a transfer.
pub fn late(score: Option<f64>) -> Option<String> {
    score.map(|s| format!("{:.0}% late", s * 100.0))
}

impl Source {
    /// Stations whose name starts like `text`.
    pub async fn stations(&self, text: &str) -> Result<Vec<Station>, String> {
        self.transit(&suggest_path(text)).await
    }

    pub async fn connections(&self, query: &Query) -> Result<Vec<Journey>, String> {
        self.transit(&query.search_path()?).await
    }

    /// Transit answers 404 when no cheaper combination exists: an outcome, which the caller
    /// shows instead of the split, never a reason to drop the direct results.
    pub async fn split(&self, query: &Query) -> Result<SplitResult, String> {
        self.transit(&query.split_path()?).await
    }

    async fn transit<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        match self {
            Self::Shell(shell) => shell.transit(path).await.map_err(|e| e.to_string()),
            Self::Device(client) => client.get(path).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(id: &str) -> Station {
        Station {
            id: id.into(),
            name: format!("Station {id}"),
        }
    }

    fn journey(
        id: &str,
        departs: &str,
        minutes: u32,
        price: Option<f64>,
        rank: Option<usize>,
    ) -> Journey {
        Journey {
            id: id.into(),
            start_station: station("1"),
            end_station: station("2"),
            legs: vec![Leg {
                origin: station("1"),
                destination: station("2"),
                departure_time: format!("2026-10-25T{departs}:00+02:00"),
                arrival_time: "2026-10-25T12:00:00+02:00".into(),
                scheduled_departure: Some(format!("2026-10-25T{departs}:00+02:00")),
                realtime_departure: Some(format!("2026-10-25T{departs}:00+02:00")),
                cancelled: false,
                train_name: "ICE 1".into(),
                platform: None,
                is_regional: false,
            }],
            total_duration_minutes: minutes,
            total_price: price,
            delay_risk_score: None,
            reliability: None,
            ranking: rank.map(|rank| Ranking {
                rank,
                factors: Vec::new(),
                source: "profile".into(),
            }),
        }
    }

    /// One real round trip through this Mac's shell and transit: Köln Hbf to Frankfurt(Main)Hbf,
    /// tomorrow at 10:00. Prints counts only.
    #[test]
    #[ignore = "needs this machine's Sjel and transit"]
    fn live_search_reads_transit() {
        let endpoint = std::env::var("SJEL_STATUS_URL")
            .unwrap_or_else(|_| crate::shell::DEFAULT_ENDPOINT.to_owned());
        let source = Source::Shell(std::sync::Arc::new(
            crate::shell::Shell::new(&endpoint).unwrap(),
        ));
        let (stations, journeys, split) =
            futures::executor::block_on(crate::shell::run(async move {
                let stations = source.stations("Köln Hbf").await;
                let query = Query {
                    from: station("8000207"),
                    to: station("8000105"),
                    date: tomorrow(),
                    time: "10:00".into(),
                    rank: Rank::default(),
                };
                (
                    stations,
                    source.connections(&query).await,
                    source.split(&query).await,
                )
            }))
            .unwrap();
        let stations = stations.unwrap();
        let journeys = journeys.unwrap();
        println!(
            "stations {} · journeys {} · first {} {} · split {:?}",
            stations.len(),
            journeys.len(),
            journeys.first().map_or("", |j| j.departs()),
            journeys
                .first()
                .map_or(String::new(), |j| duration(j.total_duration_minutes)),
            split.map(|s| (s.segments.len(), s.confidence, s.split_price)),
        );
        assert!(stations.iter().any(|s| s.id == "8000207"));
        assert!(!journeys.is_empty());
    }

    /// A split as transit serializes it (`capabilities/transit/src/travel.rs`), with the fields
    /// this page does not read left in.
    #[test]
    fn reads_transit_s_split_with_fields_it_ignores() {
        let leg = r#"{"origin":{"id":"8000207","name":"Köln Hbf","latitude":50.9,"longitude":6.9},
            "destination":{"id":"8000105","name":"Frankfurt(Main)Hbf","latitude":null,"longitude":null},
            "departure_time":"2026-10-25T10:04:00+01:00","arrival_time":"2026-10-25T11:09:00+01:00",
            "scheduled_departure":"2026-10-25T10:04:00+01:00","realtime_departure":null,
            "scheduled_arrival":null,"realtime_arrival":null,"departure_utc":"2026-10-25T09:04:00Z",
            "cancelled":false,"train_name":"ICE 523","train_number":"523","train_category":"ICE",
            "platform":"5","is_regional":false,"on_time_probability":0.8}"#;
        let journey = format!(
            r#"{{"id":"j1","start_station":{{"id":"8000207","name":"Köln Hbf","latitude":null,"longitude":null}},
            "end_station":{{"id":"8000105","name":"Frankfurt(Main)Hbf","latitude":null,"longitude":null}},
            "legs":[{leg}],"total_duration_minutes":65,"total_price":null,"delay_risk_score":0.21,
            "arrival_punctuality":{{"station_name":null,"train_type":"ICE","hour":11,"weekend":true,
            "n":400,"mean_delay":3.1,"p50":1,"p90":9,"share_late_6":0.21,"cancel_rate":0.01}},
            "reliability":{{"probability":0.74,"threshold_minutes":6,"final_leg_on_time":0.8,
            "transfers":[],"min_sample":30}},"unscored_legs":[{{"leg_index":0,"train_name":"RE 5","train_category":"RE"}}],
            "ranking":{{"score":0.6,"rank":1,"factors":[{{"key":"price","label":"Price","score":0.5,
            "weight":0.4,"rationale":"not priced"}}],"weights":{{"price":0.4,"duration":0.3,
            "changes":0.2,"reliability":0.1}},"source":"profile"}}}}"#
        );
        let body = format!(
            r#"{{"original_price":59.9,"split_price":41.8,"savings":18.1,
            "segments":[{{"journey":{journey},"train_match":"different","expected_trains":["ICE 523"]}}],
            "contract_boundaries":[],"confidence":"low","unpriced_pairs":2,"queried_pairs":9}}"#
        );
        let split: SplitResult = serde_json::from_str(&body).unwrap();
        assert_eq!(split.confidence, Confidence::Low);
        assert_eq!(split.saved_percent(), Some(30));
        let segment = &split.segments[0];
        assert_eq!(segment.train_match, TrainMatch::Different);
        let journey = &segment.journey;
        assert_eq!((journey.departs(), journey.arrives()), ("10:04", "11:09"));
        assert_eq!(journey.total_price, None);
        assert_eq!(journey.ranking.as_ref().map(|r| r.rank), Some(1));
        assert_eq!(
            journey.reliability.as_ref().map(|r| r.probability),
            Some(0.74)
        );
        assert_eq!(
            journey.delay_minutes(),
            None,
            "no real-time departure, no delay"
        );
    }

    #[test]
    fn sorts_put_missing_values_last_and_keep_ties_in_order() {
        let rows = [
            journey("a", "10:30", 90, None, Some(2)),
            journey("b", "10:00", 120, Some(29.9), None),
            journey("c", "11:00", 90, Some(19.9), Some(1)),
        ];
        let ids = |sort| -> String { ordered(&rows, sort).iter().map(|j| j.id.as_str()).collect() };
        assert_eq!(ids(Sort::Ranked), "cab");
        assert_eq!(ids(Sort::Price), "cba");
        assert_eq!(ids(Sort::Duration), "acb");
        assert_eq!(ids(Sort::Departure), "bac");
    }

    #[test]
    fn a_query_encodes_its_time_and_one_override() {
        let mut query = Query {
            from: station("8000207"),
            to: station("8000105"),
            date: "2026-10-25".into(),
            time: "9:05".into(),
            rank: Rank {
                priority: Some("cheapest"),
                phrase: String::new(),
            },
        };
        assert_eq!(
            query.search_path().unwrap(),
            "/transit/api/search?from=8000207&to=8000105&time=2026-10-25T09%3A05%3A00&priority=cheapest"
        );
        query.rank.phrase = " schnell und zuverlässig ".into();
        assert!(query
            .search_path()
            .unwrap()
            .ends_with("&phrase=schnell+und+zuverl%C3%A4ssig"));
        assert_eq!(
            query.split_path().unwrap(),
            "/transit/api/split?from=8000207&to=8000105&time=2026-10-25T09%3A05%3A00"
        );
        query.date = "25.10.2026".into();
        assert_eq!(query.when().unwrap_err(), "Write the date as 2026-10-25.");
        assert_eq!(suggest_path("Köln"), "/transit/api/suggest?q=K%C3%B6ln");
    }

    #[test]
    fn a_journey_reads_as_times_delay_and_money() {
        let mut j = journey("a", "10:04", 125, Some(49.9), None);
        j.legs[0].realtime_departure = Some("2026-10-25T10:11:00+02:00".into());
        assert_eq!((j.departs(), j.arrives()), ("10:04", "12:00"));
        assert_eq!(j.delay_minutes(), Some(7));
        assert_eq!(duration(125), "2 h 5 min");
        assert_eq!(duration(45), "45 min");
        assert_eq!(price(Some(49.9)), "49.90 €");
        assert_eq!(price(None), "—");
        assert_eq!(late(Some(0.234)).as_deref(), Some("23% late"));
    }

    #[test]
    fn a_split_names_its_saving_only_against_a_known_fare() {
        let split = |original: Option<f64>, savings: Option<f64>| SplitResult {
            segments: Vec::new(),
            original_price: original,
            split_price: 30.0,
            savings,
            confidence: Confidence::Exact,
            unpriced_pairs: 0,
            queried_pairs: 4,
        };
        assert_eq!(split(Some(40.0), Some(10.0)).saved_percent(), Some(25));
        assert_eq!(split(None, None).saved_percent(), None);
        assert_eq!(split(Some(0.0), Some(0.0)).saved_percent(), None);
    }
}
