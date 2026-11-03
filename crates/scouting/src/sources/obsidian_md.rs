//! Obsidian markdown adapter — reads typed opportunity notes from a directory
//! or one exact markdown file.
//!
//! This replaces the retired Python event extractor: instead of a separate script writing
//! intermediate JSON, the Rust pipeline reads event files directly. The convention
//! (frontmatter fields, directory structure) is the same as the original TELOS/Atlas vault
//! convention, but nothing here references "atlas" or "vault" as a hardcoded name — it's
//! "a directory of markdown files with frontmatter."
//!
//! Frontmatter fields read:
//!   - `type`           → must match the configured opportunity type
//!   - `summary`        → opportunity title
//!   - `start` / `intake_start` / `required_start` → start date
//!   - `end` / `deadline` → end date
//!   - `location`       → free-text location
//!   - `category`       → event category
//!   - `source_url` / `sources` → canonical URL
//!
//! Scholarship notes are fail-closed: only `eligibility: eligible` enters the
//! actionable ranking, and the four timing fields which prevent intake/payment
//! confusion must all be present. Incomplete or ineligible notes remain visible
//! in the Obsidian Base, but are not surfaced by the scouting pipeline.
//!
//! Description is extracted from the `## About` section of the body, or the first
//! non-heading paragraph if no `## About` exists.

use std::path::{Path, PathBuf};

use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
use crate::source::{SearchQuery, SourceAdapter, SourceError};
use markdown_root::MarkdownRoot;

#[allow(dead_code)]
pub struct ObsidianMarkdownSource {
    id: String,
    root: MarkdownRoot,
    opportunities_glob: String,
    opportunity_type: OpportunityType,
}

impl ObsidianMarkdownSource {
    pub fn new(
        id: String,
        root: PathBuf,
        opportunities_glob: String,
        opportunity_type: OpportunityType,
    ) -> Result<Self, String> {
        // `//libs/markdown-root` owns "is this a usable root", and from here on
        // "is this file actually inside it" — the second question this adapter
        // never asked, so a declared glob of `../../.ssh` used to resolve.
        let root = MarkdownRoot::declare(root).map_err(|e| e.to_string())?;
        Ok(Self {
            id,
            root,
            opportunities_glob,
            opportunity_type,
        })
    }
}

impl SourceAdapter for ObsidianMarkdownSource {
    fn name(&self) -> &str {
        &self.id
    }

    fn opportunity_type(&self) -> OpportunityType {
        self.opportunity_type
    }

    fn rate_limit_per_min(&self) -> u32 {
        0 // local file reads, no rate limit
    }

    fn search(&self, _query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        let mut opportunities = Vec::new();
        let paths = self
            .root
            .markdown_files(&self.opportunities_glob)
            .map_err(|e| SourceError::Fetch(format!("source '{}': {e}", self.id)))?;
        for path in paths {
            match parse_opportunity_file(&path, self.opportunity_type) {
                Ok(Some(opp)) => opportunities.push(opp),
                Ok(None) => {} // skip (wrong type, no content, etc.)
                Err(e) => {
                    // Log but don't fail the whole batch
                    eprintln!("  warn: skipping {}: {e}", path.display());
                }
            }
        }

        Ok(opportunities)
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_opportunity_file(
    path: &Path,
    expected_type: OpportunityType,
) -> Result<Option<Opportunity>, String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let fm = parse_frontmatter(&body)?;

    // Event notes historically allowed an omitted type. New opportunity kinds
    // must be explicit so an arbitrary vault note can never become a candidate.
    if let Some(t) = fm.get("type") {
        if t != expected_type.as_str() {
            return Ok(None);
        }
    } else if expected_type != OpportunityType::Event {
        return Ok(None);
    }

    if expected_type == OpportunityType::Scholarship && !scholarship_is_actionable(&fm, path) {
        return Ok(None);
    }

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let title = fm
        .get("summary")
        .or_else(|| fm.get("title"))
        .cloned()
        .unwrap_or_else(|| stem.clone());

    let location = fm.get("location").cloned();
    let (city, country_code) = parse_location(&location);

    let sources = fm
        .get("source_url")
        .or_else(|| fm.get("sources"))
        .cloned()
        .unwrap_or_default();
    let url = parse_first_url(&sources).unwrap_or_default();

    let starts_at = ["start", "intake_start", "required_start"]
        .iter()
        .find_map(|key| fm.get(*key))
        .map(String::as_str)
        .and_then(normalize_date);
    let ends_at = ["end", "deadline"]
        .iter()
        .find_map(|key| fm.get(*key))
        .map(String::as_str)
        .and_then(normalize_date);

    let description = extract_description(&body, &stem);

    let category = fm.get("category").cloned().unwrap_or_default();

    let mut raw = serde_json::Map::new();
    for (key, value) in &fm {
        raw.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    raw.insert("category".into(), serde_json::Value::String(category));
    raw.insert("description".into(), serde_json::Value::String(description));
    raw.insert("filename".into(), serde_json::Value::String(stem.clone()));
    raw.insert(
        "vault_path".into(),
        serde_json::Value::String(path.to_string_lossy().into_owned()),
    );

    let prefix = match expected_type {
        OpportunityType::Event => "evt",
        OpportunityType::Scholarship => "sch",
        OpportunityType::Career => "career",
        OpportunityType::Housing => "housing",
        OpportunityType::Project => "project",
        OpportunityType::Literature => "literature",
        OpportunityType::Trip => "trip",
    };
    let opportunity_id = format!("{prefix}:obsidian:{stem}");

    let opp = Opportunity {
        id: opportunity_id,
        opportunity_type: expected_type,
        source: "obsidian_markdown".into(),
        source_kind: SourceKind::UserImport,
        url,
        title,
        starts_at,
        ends_at,
        location: location.clone(),
        city,
        country_code,
        latitude: None,
        longitude: None,
        raw: serde_json::Value::Object(raw),
        fetched_at: chrono_now(),
    };

    Ok(Some(opp))
}

fn scholarship_is_actionable(
    frontmatter: &std::collections::HashMap<String, String>,
    path: &Path,
) -> bool {
    if frontmatter.get("eligibility").map(String::as_str) != Some("eligible") {
        return false;
    }

    const REQUIRED_REVIEW_FIELDS: [&str; 7] = [
        "status",
        "deadline",
        "source_url",
        "required_start",
        "employment_compatible",
        "deferral",
        "payment_start",
    ];
    let missing: Vec<&str> = REQUIRED_REVIEW_FIELDS
        .iter()
        .copied()
        .filter(|key| {
            frontmatter
                .get(*key)
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
        })
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "  warn: skipping scholarship {}: eligibility review missing {}",
            path.display(),
            missing.join(", ")
        );
        return false;
    }

    if matches!(
        frontmatter.get("status").map(String::as_str),
        Some("won" | "lost" | "ineligible" | "expired")
    ) {
        return false;
    }
    if frontmatter
        .get("employment_compatible")
        .map(|value| value.eq_ignore_ascii_case("yes"))
        != Some(true)
    {
        eprintln!(
            "  warn: skipping scholarship {}: employment_compatible must be yes",
            path.display()
        );
        return false;
    }
    true
}

/// Frontmatter parsing moved to `//libs/markdown-root` when calendar's event
/// importer needed the same format read the same way. The fields below are
/// still this adapter's own — the lib knows what a frontmatter block is, never
/// what `eligibility` means.
use markdown_root::frontmatter as parse_frontmatter;

fn parse_location(location: &Option<String>) -> (Option<String>, Option<String>) {
    match location {
        None => (None, None),
        Some(loc) => {
            let parts: Vec<&str> = loc.split(',').collect();
            let city = parts.first().map(|s| s.trim().to_string());
            let country = if parts.len() > 1 {
                let last = parts.last().unwrap().trim();
                if last.len() <= 3 {
                    Some(last.to_string())
                } else {
                    None
                }
            } else {
                None
            };
            (city, country)
        }
    }
}

fn parse_first_url(sources: &str) -> Option<String> {
    // Sources might be a comma-separated list of URLs
    sources
        .split(',')
        .map(|s| s.trim())
        .find(|s| s.starts_with("http"))
        .map(|s| s.to_string())
        .or_else(|| {
            if sources.starts_with("http") {
                Some(sources.to_string())
            } else {
                None
            }
        })
}

fn normalize_date(date: &str) -> Option<String> {
    let d = date.trim();
    if d.is_empty() {
        return None;
    }
    // If no time component, add midnight UTC
    if d.len() <= 10 && d.contains('-') {
        Some(format!("{}T00:00:00+00:00", d))
    } else {
        Some(d.to_string())
    }
}

fn extract_description(body: &str, _stem: &str) -> String {
    let end = body[3..].find("---").map(|i| i + 6).unwrap_or(0);
    let content = &body[end..];

    // First try ## About section
    let lines: Vec<&str> = content.lines().collect();
    let mut in_about = false;
    let mut about_parts: Vec<&str> = Vec::new();

    for line in &lines {
        let stripped = line.trim();
        if stripped.starts_with("## About") || stripped.starts_with("# About") {
            in_about = true;
            continue;
        }
        if in_about {
            if stripped.starts_with("##") || stripped.starts_with("---") {
                break;
            }
            if !stripped.is_empty() && !stripped.starts_with('[') && !stripped.starts_with('!') {
                about_parts.push(stripped);
            }
        }
    }

    if !about_parts.is_empty() {
        let desc = about_parts.join(" ");
        if desc.len() > 300 {
            return desc[..300].to_string();
        }
        return desc;
    }

    // Fallback: first non-empty, non-heading line after frontmatter
    for line in content.lines() {
        let stripped = line.trim();
        if !stripped.is_empty()
            && !stripped.starts_with('#')
            && !stripped.starts_with('[')
            && !stripped.starts_with('!')
            && !stripped.starts_with("---")
            && !stripped.starts_with('|')
        {
            return stripped.chars().take(200).collect();
        }
    }

    String::new()
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
    use std::fs;

    fn write_test_event(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn parses_frontmatter_basic() {
        let md = "---\nsummary: \"AI Hackathon Berlin\"\nstart: \"2026-07-15\"\nend: \"2026-07-17\"\nlocation: \"Berlin, DE\"\ncategory: \"tech\"\ntype: \"event\"\n---\n\n## About\nA weekend of building.";

        let fm = parse_frontmatter(md).unwrap();
        assert_eq!(fm.get("summary").unwrap(), "AI Hackathon Berlin");
        assert_eq!(fm.get("start").unwrap(), "2026-07-15");
        assert_eq!(fm.get("location").unwrap(), "Berlin, DE");
        assert_eq!(fm.get("type").unwrap(), "event");
    }

    #[test]
    fn skips_non_event_types() {
        let dir = std::env::temp_dir().join("axon_obsidian_test_skip");
        fs::create_dir_all(&dir).unwrap();
        write_test_event(
            &dir,
            "MOC.md",
            "---\ntype: moc\nsummary: \"Hub\"\n---\n\nLinks.",
        );

        let source = ObsidianMarkdownSource::new(
            "test".into(),
            dir.clone(),
            "*.md".into(),
            OpportunityType::Event,
        )
        .unwrap();
        let opps = source.search(&SearchQuery::default()).unwrap();
        assert!(opps.is_empty(), "MOC type should be filtered out");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_event_files() {
        let dir = std::env::temp_dir().join("axon_obsidian_test_read");
        fs::create_dir_all(&dir).unwrap();

        write_test_event(&dir, "2026-07-berlin-ai-hack.md",
            "---\nsummary: \"Berlin AI Hackathon\"\nstart: \"2026-07-15\"\nend: \"2026-07-17\"\nlocation: \"Berlin, DE\"\ncategory: \"tech\"\ntype: event\nsources:\n  - \"https://example.com/event1\"\n  - \"https://example.com/event2\"\n---\n\n## About\nA weekend of building AI projects together.\n\n## Details\nMore info here."
        );

        write_test_event(&dir, "munich-meetup.md",
            "---\nsummary: \"Munich Tech Meetup\"\nstart: \"2026-08-01\"\nlocation: \"Munich, DE\"\ncategory: \"meetup\"\ntype: event\nsources: \"https://meetup.com/xyz\"\n---\n\n## About\nMonthly meetup."
        );

        let source = ObsidianMarkdownSource::new(
            "test".into(),
            dir.clone(),
            "*.md".into(),
            OpportunityType::Event,
        )
        .unwrap();
        let opps = source.search(&SearchQuery::default()).unwrap();
        assert_eq!(opps.len(), 2);

        // Check first event
        let berlin = opps.iter().find(|o| o.title.contains("Berlin")).unwrap();
        assert_eq!(berlin.city.as_deref(), Some("Berlin"));
        assert_eq!(berlin.country_code.as_deref(), Some("DE"));
        assert!(berlin.url.contains("example.com/event1"));

        // Check second
        let munich = opps.iter().find(|o| o.title.contains("Munich")).unwrap();
        assert_eq!(
            munich.starts_at.as_deref(),
            Some("2026-08-01T00:00:00+00:00")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_frontmatter_returns_empty_map() {
        let md = "Just a paragraph.\n\nNo frontmatter.";
        let fm = parse_frontmatter(md).unwrap();
        assert!(fm.is_empty());
    }

    #[test]
    fn extracts_about_section() {
        let md = "---\nsummary: Test\n---\n\n## About\nThis is the description.\nMore lines.\n\n## Other\nNot included.";

        let dir = std::env::temp_dir().join("axon_obsidian_test_about");
        fs::create_dir_all(&dir).unwrap();
        write_test_event(&dir, "test.md", md);

        let source = ObsidianMarkdownSource::new(
            "test".into(),
            dir.clone(),
            "*.md".into(),
            OpportunityType::Event,
        )
        .unwrap();
        let opps = source.search(&SearchQuery::default()).unwrap();
        assert_eq!(opps.len(), 1);
        assert!(opps[0]
            .raw
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("description"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scholarship_requires_completed_hard_filter_review() {
        let dir = std::env::temp_dir().join("axon_obsidian_test_scholarship");
        fs::create_dir_all(&dir).unwrap();
        write_test_event(
            &dir,
            "eligible.md",
            "---\ntype: scholarship\nsummary: \"Feasible 2027 scholarship\"\n\
             status: radar\neligibility: eligible\nrequired_start: 2027-10-01\n\
             employment_compatible: yes\ndeferral: not-applicable\n\
             payment_start: 2027-10-01\ndeadline: 2027-03-01\n\
             source_url: \"https://example.com/scholarship\"\n---\n\n## About\nAI funding.",
        );
        write_test_event(
            &dir,
            "unreviewed.md",
            "---\ntype: scholarship\nsummary: \"Missing timing review\"\n\
             status: radar\neligibility: eligible\nsource_url: \"https://example.com/missing\"\n---\n",
        );
        write_test_event(
            &dir,
            "ineligible.md",
            "---\ntype: scholarship\nsummary: \"Wrong intake\"\n\
             status: ineligible\neligibility: ineligible\ndeadline: 2026-07-31\n\
             source_url: \"https://example.com/ineligible\"\nrequired_start: 2026-10-01\n\
             employment_compatible: no\ndeferral: no\npayment_start: 2027-01-01\n---\n",
        );
        write_test_event(
            &dir,
            "inconsistent.md",
            "---\ntype: scholarship\nsummary: \"Explicitly incompatible\"\n\
             status: radar\neligibility: eligible\ndeadline: 2027-03-01\n\
             source_url: \"https://example.com/inconsistent\"\nrequired_start: 2027-10-01\n\
             employment_compatible: no\ndeferral: not-applicable\n\
             payment_start: 2027-10-01\n---\n",
        );

        let source = ObsidianMarkdownSource::new(
            "scholarships".into(),
            dir.clone(),
            "*.md".into(),
            OpportunityType::Scholarship,
        )
        .unwrap();
        let opps = source.search(&SearchQuery::default()).unwrap();
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].opportunity_type, OpportunityType::Scholarship);
        assert_eq!(
            opps[0].ends_at.as_deref(),
            Some("2027-03-01T00:00:00+00:00")
        );
        assert_eq!(
            opps[0]
                .raw
                .get("employment_compatible")
                .and_then(|v| v.as_str()),
            Some("yes")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn supports_one_exact_markdown_file() {
        let dir = std::env::temp_dir().join("axon_obsidian_test_exact_file");
        fs::create_dir_all(&dir).unwrap();
        write_test_event(
            &dir,
            "one.md",
            "---\ntype: event\nsummary: \"One event\"\n---\n",
        );
        write_test_event(
            &dir,
            "ignored.md",
            "---\ntype: event\nsummary: \"Ignored event\"\n---\n",
        );

        let source = ObsidianMarkdownSource::new(
            "test".into(),
            dir.clone(),
            "one.md".into(),
            OpportunityType::Event,
        )
        .unwrap();
        let opps = source.search(&SearchQuery::default()).unwrap();
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].title, "One event");

        fs::remove_dir_all(&dir).ok();
    }
}
