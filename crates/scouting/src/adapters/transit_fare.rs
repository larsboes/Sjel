//! Wires `capabilities/transit`'s HAFAS fare-search in as a scored
//! `Opportunity` source (`capabilities/store/README.md`'s correlation section, Phase 2: "wire its
//! fare-search in as a scored source feeding opportunities, not just an
//! on-demand CLI answer"). One-directional dependency -- scouting depends on
//! transit, never the reverse (see Cargo.toml).
//!
//! Unlike every other adapter here, this one also writes a detailed record
//! into transit's own store (`transit.trips`/`trip_legs`, tagged
//! `trigger_reason = "auto"`) as a side effect of `search()` -- the same
//! "persistence as an optional side effect" shape `pipeline::run`'s
//! `store: Option<&mut Store>` already uses, just one level down. Passing
//! `store: None` (e.g. `--no-store` runs, or tests) skips that write; the
//! `Opportunity` results returned to the caller are unaffected either way.
//! A `transit search`/`split` CLI invocation writes to the same
//! `transit.trips` table too, tagged `trigger_reason = "manual"` -- see
//! `transit::store`'s module doc.
//!
//! Origin/destination come from `transit::config::Config`'s
//! `default_from_eva`/`default_to_eva` (the overlay route to watch), not a
//! CLI flag -- this adapter has no "which route" concept of its own,
//! deliberately, same reasoning transit's own CLI already documents for why
//! it has no baked-in station default (see `transit::config`'s doc comment).
//! The search date/time is required via `SearchQuery.date_from` (ISO
//! datetime) -- no invented "tomorrow at 8am" default; same "error with a
//! clear message, don't silently guess" philosophy `main.rs::require()`
//! already applies to `transit`'s own CLI.

use transit::hafas::HafasClient;
use transit::store::TransitStore;
use transit::travel::Journey;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use crate::source::{SearchQuery, SourceAdapter, SourceError};

pub struct TransitFareAdapter {
    from_eva: String,
    to_eva: String,
    store: Option<TransitStore>,
}

impl TransitFareAdapter {
    pub fn new(from_eva: String, to_eva: String, store: Option<TransitStore>) -> Self {
        Self {
            from_eva,
            to_eva,
            store,
        }
    }
}

impl SourceAdapter for TransitFareAdapter {
    fn name(&self) -> &str {
        "transit_fare"
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Trip
    }

    fn rate_limit_per_min(&self) -> u32 {
        // bahn.de's undocumented internal API, same client transit's own CLI
        // uses -- no published rate limit; matches the other network
        // adapters' conservative default in this directory.
        20
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let Some(datetime) = query.date_from.as_deref() else {
            return Err(SourceError::Fetch(
                "transit_fare requires SearchQuery.date_from (ISO datetime, e.g. 2026-08-01T08:00:00) -- no invented default".into(),
            ));
        };

        let client = HafasClient::new();
        // Undiscounted second class, no Deutschlandticket. `search_connections` grew a
        // `&FareOptions` argument in aec53b4 and this call site was not updated, which is
        // why scouting stopped compiling; `FareOptions::default()` is what restores the
        // behaviour it had before, not a new choice. It is also exactly what transit's own
        // HTTP surface serves an unspecified request: every field of `RouteQuery`'s fare
        // trio is `#[serde(default)]` (transit/src/server.rs:83-89).
        //
        // There is deliberately no fare profile in config to read here. A watch that should
        // price against a BahnCard needs that stated per source, and no source declares one
        // yet -- inventing a discount would silently under-report every fare this adapter
        // reports as a bargain.
        let journeys = client
            .search_connections(
                &self.from_eva,
                &self.to_eva,
                datetime,
                &transit::hafas::FareOptions::default(),
            )
            .map_err(|e| SourceError::Fetch(e.to_string()))?;

        if let Some(store) = &self.store {
            for j in &journeys {
                if let Err(e) = store.record_journey(j, &self.from_eva, &self.to_eva, "auto", None)
                {
                    eprintln!("warning: transit_fare could not record trip {}: {e}", j.id);
                }
            }
        }

        let fetched_at = chrono_now();
        Ok(journeys
            .iter()
            .map(|j| journey_to_opportunity(j, &fetched_at))
            .collect())
    }
}

/// Pure conversion, no I/O -- `Journey` (transit's shape) into `Opportunity`
/// (scouting's shape), so a fare-search result flows through the exact same
/// score/rank/dismiss/save pipeline every other source already does.
pub fn journey_to_opportunity(j: &Journey, fetched_at: &str) -> Opportunity {
    let price = j
        .total_price
        .map(|p| format!("€{p:.2}"))
        .unwrap_or_else(|| "price unknown".into());
    let title = format!(
        "{} → {} ({price})",
        j.start_station.name, j.end_station.name
    );

    Opportunity {
        id: format!("trip:transit_fare:{}", j.id),
        opportunity_type: OpportunityType::Trip,
        source: "transit_fare".into(),
        source_kind: SourceKind::Api,
        url: String::new(), // no public URL for a HAFAS journey result
        title,
        starts_at: j.legs.first().map(|l| l.departure_time.clone()),
        ends_at: j.legs.last().map(|l| l.arrival_time.clone()),
        location: Some(j.end_station.name.clone()),
        city: Some(j.end_station.name.clone()),
        country_code: Some("DE".into()), // bahn.de/HAFAS is Germany-only
        latitude: None,
        longitude: None,
        raw: serde_json::to_value(j).unwrap_or(serde_json::Value::Null),
        fetched_at: fetched_at.into(),
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use transit::travel::{Leg, Station};

    fn mk_journey() -> Journey {
        let bonn = Station {
            id: "8000044".into(),
            name: "Bonn Hbf".into(),
            latitude: None,
            longitude: None,
        };
        let berlin = Station {
            id: "8098160".into(),
            name: "Berlin Hbf".into(),
            latitude: None,
            longitude: None,
        };
        Journey {
            reliability: None,
            unscored_legs: Vec::new(),
            ranking: None,
            id: "journey:abc".into(),
            start_station: bonn.clone(),
            end_station: berlin.clone(),
            legs: vec![Leg {
                on_time_probability: None,
                origin: bonn,
                destination: berlin,
                departure_time: "2026-08-01T08:00:00".into(),
                arrival_time: "2026-08-01T12:00:00".into(),
                train_name: "ICE 691".into(),
                train_number: "691".into(),
                train_category: "ICE".into(),
                platform: Some("3".into()),
                is_regional: false,
                scheduled_departure: None,
                realtime_departure: None,
                scheduled_arrival: None,
                realtime_arrival: None,
                // `None`, not a hand-computed CEST offset. These two tests read
                // `starts_at`/`ends_at`, which map from the naive `departure_time` and
                // `arrival_time` above; nothing here exercises UTC arithmetic. `None` is
                // a state production genuinely produces (transit/src/travel.rs: absent
                // when the station's UIC prefix is not in station-time's table), whereas
                // writing "2026-08-01T06:00:00Z" would assert a conversion this fixture
                // does not test and scouting does not depend on station-time to perform.
                departure_utc: None,
                arrival_utc: None,
                cancelled: false,
            }],
            total_duration_minutes: 240,
            total_price: Some(79.90),
            delay_risk_score: None,
            // `None` for the same reason `delay_risk_score` is: this fixture
            // exercises the fare mapping, and scouting reads neither. Absent is
            // a state production genuinely returns — transit fills it only when
            // the route's punctuality history is available.
            arrival_punctuality: None,
        }
    }

    #[test]
    fn journey_to_opportunity_maps_fields() {
        let j = mk_journey();
        let opp = journey_to_opportunity(&j, "123");

        assert_eq!(opp.id, "trip:transit_fare:journey:abc");
        assert_eq!(opp.opportunity_type, OpportunityType::Trip);
        assert_eq!(opp.source, "transit_fare");
        assert_eq!(opp.title, "Bonn Hbf → Berlin Hbf (€79.90)");
        assert_eq!(opp.starts_at.as_deref(), Some("2026-08-01T08:00:00"));
        assert_eq!(opp.ends_at.as_deref(), Some("2026-08-01T12:00:00"));
        assert_eq!(opp.city.as_deref(), Some("Berlin Hbf"));
        assert_eq!(opp.country_code.as_deref(), Some("DE"));
        assert_eq!(opp.fetched_at, "123");
    }

    #[test]
    fn journey_to_opportunity_handles_missing_price() {
        let mut j = mk_journey();
        j.total_price = None;
        let opp = journey_to_opportunity(&j, "123");
        assert!(opp.title.contains("price unknown"));
    }

    #[test]
    fn search_without_date_from_errors_clearly() {
        let adapter = TransitFareAdapter::new("8000044".into(), "8098160".into(), None);
        let query = SearchQuery::default();
        let result = adapter.search(&query);
        assert!(
            result.is_err(),
            "no date_from should error, not silently guess a date"
        );
    }
}
