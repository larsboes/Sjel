//! Live-verified adapter (see README Gotchas) -- probed against the real
//! hacktrack-eu API during the original build. The other three adapters in
//! this directory are ported but not similarly live-verified; be skeptical
//! of them until you've run each one for real.

use std::fs;
use std::path::PathBuf;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use serde::{Deserialize, Serialize};

use crate::source::{SearchQuery, SourceAdapter, SourceError};

const BASE_URL: &str = "https://hacktrack-eu.vercel.app/api";

#[derive(Debug, Deserialize, Serialize)]
struct HackathonsResponse {
    data: Vec<RawHackathon>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawHackathon {
    id: String,
    name: String,
    city: Option<String>,
    country_code: Option<String>,
    date_start: Option<String>,
    date_end: Option<String>,
    topics: Vec<String>,
    url: Option<String>,
    status: Option<String>,
}

pub struct EuroHackathonsAdapter {
    pub api_base: String,
    pub cache_dir: Option<PathBuf>,
}

impl Default for EuroHackathonsAdapter {
    fn default() -> Self {
        Self {
            api_base: BASE_URL.into(),
            cache_dir: None,
        }
    }
}

impl EuroHackathonsAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cache(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
            ..Default::default()
        }
    }

    fn fetch_upcoming(&self) -> Result<Vec<RawHackathon>, SourceError> {
        if let Some(ref dir) = self.cache_dir {
            let path = dir.join("upcoming.json");
            if path.exists() {
                let body = fs::read_to_string(&path)
                    .map_err(|e| SourceError::Fetch(format!("cache read {path:?}: {e}")))?;
                return self.parse(&body);
            }
        }

        let url = format!("{}/hackathons?status=upcoming", self.api_base);
        let body = crate::http::get_checked(&url)?;
        self.parse(&body)
    }

    fn parse(&self, body: &str) -> Result<Vec<RawHackathon>, SourceError> {
        let resp: HackathonsResponse = serde_json::from_str(body)
            .map_err(|e| SourceError::Parse(format!("JSON decode: {e}")))?;
        Ok(resp.data)
    }

    fn normalize(&self, raw: RawHackathon, fetched_at: &str) -> Opportunity {
        let raw_value = serde_json::to_value(&raw).unwrap_or(serde_json::Value::Null);
        let url = raw.url.unwrap_or_default();
        let city = raw.city;
        let country_code = raw.country_code;
        let title = raw.name;
        let opportunity_id = format!("evt:euro_hackathons:{}", raw.id);
        let location = city.clone();

        Opportunity {
            id: opportunity_id,
            opportunity_type: OpportunityType::Event,
            source: "euro_hackathons".into(),
            source_kind: SourceKind::Api,
            url,
            title,
            starts_at: raw.date_start,
            ends_at: raw.date_end,
            location,
            city,
            country_code,
            latitude: None,
            longitude: None,
            raw: raw_value,
            fetched_at: fetched_at.into(),
        }
    }
}

impl SourceAdapter for EuroHackathonsAdapter {
    fn name(&self) -> &str {
        "euro_hackathons"
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Event
    }

    fn rate_limit_per_min(&self) -> u32 {
        10
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let fetched_at = chrono_now();
        let hackathons = self.fetch_upcoming()?;

        let mut filtered: Vec<RawHackathon> = hackathons;
        if let Some(ref loc) = query.location {
            let loc = loc.to_lowercase();
            filtered.retain(|h| {
                h.city
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&loc)
                    || h.country_code
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&loc)
            });
        }
        if filtered.len() > query.limit {
            filtered.truncate(query.limit);
        }

        Ok(filtered
            .into_iter()
            .map(|h| self.normalize(h, &fetched_at))
            .collect())
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
    use std::io::Write;

    #[test]
    fn parses_cached_fixture() {
        let dir = std::env::temp_dir().join("axon_scouting_test_cache");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upcoming.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(br#"{"data":[{"id":"abc","name":"Test Hack","city":"Berlin","country_code":"DE","date_start":"2026-10-23T00:00:00+00:00","date_end":null,"topics":["AI"],"url":"https://test.example","status":"upcoming"}]}"#).unwrap();

        let adapter = EuroHackathonsAdapter::with_cache(dir.clone());
        let query = SearchQuery::default();
        let opps = adapter.search(&query).unwrap();
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].title, "Test Hack");
        assert_eq!(opps[0].city.as_deref(), Some("Berlin"));
        assert_eq!(opps[0].country_code.as_deref(), Some("DE"));
        assert_eq!(opps[0].opportunity_type, OpportunityType::Event);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn filters_by_location() {
        let dir = std::env::temp_dir().join("axon_scouting_test_loc");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upcoming.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(br#"{"data":[{"id":"a","name":"London Hack","city":"London","country_code":"GB","date_start":null,"date_end":null,"topics":[],"url":null,"status":"upcoming"},{"id":"b","name":"Berlin Hack","city":"Berlin","country_code":"DE","date_start":null,"date_end":null,"topics":[],"url":null,"status":"upcoming"}]}"#).unwrap();

        let adapter = EuroHackathonsAdapter::with_cache(dir.clone());
        let query = SearchQuery {
            location: Some("berlin".into()),
            ..Default::default()
        };
        let opps = adapter.search(&query).unwrap();
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].title, "Berlin Hack");

        fs::remove_dir_all(&dir).ok();
    }
}
