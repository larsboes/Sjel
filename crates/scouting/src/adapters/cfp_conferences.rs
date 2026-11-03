use std::collections::HashMap;
use std::path::PathBuf;

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};

use crate::source::{SearchQuery, SourceAdapter, SourceError};

const CONFERENCES_URL: &str =
    "https://raw.githubusercontent.com/abhshkdz/ai-deadlines/gh-pages/_data/conferences.yml";

pub struct CfpConferencesAdapter {
    pub cache_dir: Option<PathBuf>,
}

impl CfpConferencesAdapter {
    pub fn new() -> Self {
        Self { cache_dir: None }
    }

    pub fn with_cache(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
        }
    }

    fn fetch(&self) -> Result<String, SourceError> {
        if let Some(ref dir) = self.cache_dir {
            let cache_path = dir.join("conferences.yml");
            if cache_path.exists() {
                return std::fs::read_to_string(&cache_path)
                    .map_err(|e| SourceError::Fetch(format!("cache read: {e}")));
            }
        }

        let body = crate::http::get_checked(CONFERENCES_URL)?;

        if let Some(ref dir) = self.cache_dir {
            std::fs::create_dir_all(dir).ok();
            std::fs::write(dir.join("conferences.yml"), &body).ok();
        }

        Ok(body)
    }

    fn parse_conferences(&self, yaml: &str) -> Vec<ConferenceEntry> {
        let mut entries = Vec::new();
        let mut current: HashMap<String, String> = HashMap::new();

        for line in yaml.lines() {
            if line.starts_with("- title:") {
                if !current.is_empty() {
                    entries.push(ConferenceEntry::from_map(&current));
                    current.clear();
                }
                let val = line
                    .trim_start_matches("- title:")
                    .trim()
                    .trim_matches('\'');
                current.insert("title".into(), val.to_string());
            } else if let Some((key, value)) = line.split_once(':') {
                let k = key.trim();
                if !k.is_empty() && !k.starts_with('-') {
                    let v = value.trim().trim_matches('\'').to_string();
                    current.insert(k.to_string(), v);
                }
            }
        }
        if !current.is_empty() {
            entries.push(ConferenceEntry::from_map(&current));
        }

        entries
    }

    fn normalize(&self, conf: &ConferenceEntry, fetched_at: &str) -> Opportunity {
        let id = format!(
            "cfp:conference:{}",
            conf.id.as_deref().unwrap_or(&conf.title)
        );
        let deadline = conf.deadline.clone();
        let date_str = format!(
            "{} {}",
            conf.start.as_deref().unwrap_or(""),
            conf.end.as_deref().unwrap_or("")
        );
        let place = conf.place.as_deref().unwrap_or("Online/Unknown");
        let sub = conf.sub.as_deref().unwrap_or("ML");
        let url = conf.link.as_deref().unwrap_or("").to_string();
        let title_text = if sub == "ML" || sub == "AI" {
            format!("{} ({})", conf.title, sub)
        } else {
            conf.title.clone()
        };

        let raw_value = serde_json::json!({
            "title": conf.title,
            "deadline": deadline,
            "sub": sub,
            "place": place,
            "date": date_str,
            "link": url,
            "hindex": conf.hindex,
        });

        Opportunity {
            id,
            opportunity_type: OpportunityType::Scholarship,
            source: "cfp_conferences".into(),
            source_kind: SourceKind::Scraper,
            url,
            title: title_text,
            starts_at: deadline.clone(),
            ends_at: None,
            location: Some(place.to_string()),
            city: conf.place.clone().or_else(|| Some("Unknown".into())),
            country_code: None,
            latitude: None,
            longitude: None,
            raw: raw_value,
            fetched_at: fetched_at.into(),
        }
    }
}

struct ConferenceEntry {
    title: String,
    id: Option<String>,
    deadline: Option<String>,
    place: Option<String>,
    link: Option<String>,
    sub: Option<String>,
    start: Option<String>,
    end: Option<String>,
    hindex: Option<i64>,
}

impl ConferenceEntry {
    fn from_map(map: &HashMap<String, String>) -> Self {
        Self {
            title: map.get("title").cloned().unwrap_or_default(),
            id: map.get("id").cloned(),
            deadline: map.get("deadline").cloned(),
            place: map.get("place").cloned(),
            link: map.get("link").cloned(),
            sub: map.get("sub").cloned(),
            start: map.get("start").cloned(),
            end: map.get("end").cloned(),
            hindex: map.get("hindex").and_then(|v| v.parse().ok()),
        }
    }
}

impl Default for CfpConferencesAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceAdapter for CfpConferencesAdapter {
    fn name(&self) -> &str {
        "cfp_conferences"
    }

    fn opportunity_type(&self) -> OpportunityType {
        OpportunityType::Scholarship
    }

    fn rate_limit_per_min(&self) -> u32 {
        30
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let fetched_at = chrono_now();
        let yaml = self.fetch()?;
        let mut confs = self.parse_conferences(&yaml);

        if !query.query.is_empty() {
            let q = query.query.to_lowercase();
            confs.retain(|c| {
                c.title.to_lowercase().contains(&q)
                    || c.sub
                        .as_deref()
                        .map(|s| s.to_lowercase().contains(&q))
                        .unwrap_or(false)
            });
        }
        if let Some(ref loc) = query.location {
            let l = loc.to_lowercase();
            confs.retain(|c| {
                c.place
                    .as_deref()
                    .map(|p| p.to_lowercase().contains(&l))
                    .unwrap_or(false)
            });
        }
        if confs.len() > query.limit {
            confs.truncate(query.limit);
        }

        Ok(confs
            .into_iter()
            .map(|c| self.normalize(&c, &fetched_at))
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

    #[test]
    fn parses_conference_yaml() {
        let yaml = r#"- title: NeurIPS
  year: 2025
  id: neurips25
  deadline: '2025-05-15 23:59:59'
  place: Vancouver, Canada
  link: https://neurips.cc
  sub: ML
  start: 2025-12-01
  end: 2025-12-07
  hindex: 100

- title: ICML
  year: 2025
  id: icml25
  deadline: '2025-01-30 23:59:59'
  place: Vienna, Austria
  link: https://icml.cc
  sub: ML
  start: 2025-07-14
  end: 2025-07-20
  hindex: 90
"#;
        let adapter = CfpConferencesAdapter::new();
        let confs = adapter.parse_conferences(yaml);
        assert_eq!(confs.len(), 2);
        assert_eq!(confs[0].title, "NeurIPS");
        assert_eq!(confs[1].title, "ICML");
    }

    #[test]
    fn normalizes_to_scholarship_type() {
        let yaml = r#"- title: NeurIPS
  year: 2025
  deadline: '2025-05-15 23:59:59'
  place: Vancouver, Canada
  link: https://neurips.cc
  sub: ML
"#;
        let adapter = CfpConferencesAdapter::new();
        let confs = adapter.parse_conferences(yaml);
        let opp = adapter.normalize(&confs[0], "12345");
        assert_eq!(opp.opportunity_type, OpportunityType::Scholarship);
        assert_eq!(opp.source, "cfp_conferences");
        assert!(opp.id.starts_with("cfp:conference:"));
        assert_eq!(opp.title, "NeurIPS (ML)");
    }
}
