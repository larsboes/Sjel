//! The shared `Opportunity` schema: one type, six kinds (event, scholarship,
//! career, housing, project, literature), one pipeline. Ported from
//! LifeOS-mono's `schemas/opportunity` crate and folded in as a module here
//! (Sjel doctrine rule 5: no second crate for ~90 lines used by one
//! consumer -- DRY/minimal over ceremony).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityType {
    Event,
    Scholarship,
    Career,
    Housing,
    Project,
    Literature,
    /// A scored train-fare journey (`capabilities/transit`'s HAFAS
    /// fare-search wired in as a source -- see `capabilities/store/README.md`
    /// Phase 2 and `adapters/transit_fare.rs`).
    Trip,
}

impl OpportunityType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Scholarship => "scholarship",
            Self::Career => "career",
            Self::Housing => "housing",
            Self::Project => "project",
            Self::Literature => "literature",
            Self::Trip => "trip",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Api,
    Scraper,
    JsonFeed,
    UserImport,
    AiDiscovery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Opportunity {
    pub id: String,
    pub opportunity_type: OpportunityType,
    pub source: String,
    pub source_kind: SourceKind,
    pub url: String,
    pub title: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub location: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    /// Where the thing actually is, when the source said so. Sparse on
    /// purpose: Luma fills it for roughly a quarter of its events, an RSS
    /// item or a vault note never will. A consumer that needs a distance
    /// must treat `None` as "ask the country instead", never as a
    /// coordinate — 0.0/0.0 is a real place in the Gulf of Guinea, and
    /// defaulting to it would put every unlocated opportunity there.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub raw: serde_json::Value,
    pub fetched_at: String,
}

impl Opportunity {
    pub fn fingerprint(&self) -> String {
        let stem = format!("{}|{}|{}", self.source, self.url, self.title);
        let mut h = 0u64;
        for b in stem.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(*b as u64);
        }
        format!("{:016x}", h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_identity() {
        let o = Opportunity {
            id: "evt:example-hackathon-2026".into(),
            opportunity_type: OpportunityType::Event,
            source: "euro_hackathons".into(),
            source_kind: SourceKind::Api,
            url: "https://example.test/hackathon-2026".into(),
            title: "Example Hackathon".into(),
            starts_at: Some("2026-10-23T00:00:00+00:00".into()),
            ends_at: Some("2026-10-25T00:00:00+00:00".into()),
            location: Some("London".into()),
            city: Some("London".into()),
            country_code: Some("GB".into()),
            latitude: Some(51.5072),
            longitude: Some(-0.1276),
            raw: serde_json::json!({"topics": []}),
            fetched_at: "2026-07-03T20:00:00Z".into(),
        };
        let s = serde_json::to_string(&o).unwrap();
        let back: Opportunity = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);
    }

    #[test]
    fn fingerprint_stable_for_same_input() {
        let mk = |title: &str| Opportunity {
            id: "x".into(),
            opportunity_type: OpportunityType::Event,
            source: "s".into(),
            source_kind: SourceKind::Api,
            url: "u".into(),
            title: title.into(),
            starts_at: None,
            ends_at: None,
            location: None,
            city: None,
            country_code: None,
            latitude: None,
            longitude: None,
            raw: serde_json::Value::Null,
            fetched_at: "t".into(),
        };
        assert_eq!(mk("a").fingerprint(), mk("a").fingerprint());
        assert_ne!(mk("a").fingerprint(), mk("b").fingerprint());
    }
}
