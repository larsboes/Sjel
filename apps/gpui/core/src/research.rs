//! Data models and pure reductions for Sjel Research & Local Decision Models (`/research`).
//!
//! Replaces the dashboard's `/research` routes (`/research`, `/research/projects`, `/research/benchmarks`).
//! Provides markdown research articles, external project registers (`upstreams.toml` and `systems.toml`),
//! and model benchmark evaluation runs.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// One external project Sjel borrows from, watches, adopts, or declined (`upstreams.toml`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpstreamRow {
    pub name: String,
    pub url: String,
    pub verdict: String,
    pub license: String,
    pub summary: String,
}

/// One external system running alongside Sjel (`systems.toml`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SystemRow {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    pub kind: String,
    pub local: bool,
    pub why: String,
}

/// Grouping of upstream projects for human inspection.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectGroup {
    pub id: String,
    pub label: String,
    pub blurb: String,
    pub rows: Vec<UpstreamRow>,
}

/// One individual test case evaluated during a benchmark run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkCase {
    pub id: String,
    #[serde(default)]
    pub expected: serde_json::Value,
    #[serde(default)]
    pub got: serde_json::Value,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub ms: i64,
}

/// Latency percentiles in milliseconds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkLatency {
    pub p50: f64,
    pub p95: f64,
}

/// One benchmark result payload.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkResult {
    pub suite: String,
    pub date: String,
    pub backend: String,
    pub model: String,
    pub host: String,
    pub n: i64,
    pub correct: i64,
    pub accuracy: f64,
    pub latency_ms: BenchmarkLatency,
    #[serde(default)]
    pub cases: Vec<BenchmarkCase>,
}

/// One benchmark run with its source filename.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkRun {
    pub suite: String,
    pub file: String,
    pub result: BenchmarkResult,
}

/// A suite of benchmark runs grouped by suite name.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BenchmarkSuite {
    pub name: String,
    pub runs: Vec<BenchmarkRun>,
    pub case_ids: Vec<String>,
}

/// One markdown research article.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResearchArticle {
    pub slug: String,
    pub file: String,
    pub title: String,
    pub body: String,
}

/// Combined view model for the primary `/research` screen.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ResearchView {
    pub articles: Vec<ResearchArticle>,
    pub upstreams: Vec<UpstreamRow>,
    pub systems: Vec<SystemRow>,
    pub suites: Vec<BenchmarkSuite>,
}

// ── Pure Reductions & Parsing ───────────────────────────────────────────────

/// Parse single-line TOML sections and key-values without heavy parser dependencies.
pub fn parse_single_line_toml(content: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_map: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(name) = current_name.take() {
                sections.push((name, std::mem::take(&mut current_map)));
            }
            let name = trimmed[1..trimmed.len() - 1].trim().to_string();
            current_name = Some(name);
            continue;
        }

        if let Some((key, val_raw)) = trimmed.split_once('=') {
            let key = key.trim().to_string();
            let val = val_raw.trim();
            let val_unquoted = if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                if val.len() >= 2 {
                    &val[1..val.len() - 1]
                } else {
                    val
                }
            } else {
                val
            };
            current_map.insert(key, val_unquoted.to_string());
        }
    }

    if let Some(name) = current_name {
        sections.push((name, current_map));
    }

    sections
}

/// Parse `upstreams.toml` content into a list of `UpstreamRow`s.
pub fn parse_upstreams_toml(content: &str) -> Vec<UpstreamRow> {
    parse_single_line_toml(content)
        .into_iter()
        .map(|(name, map)| UpstreamRow {
            name,
            url: map.get("url").cloned().unwrap_or_default(),
            verdict: map.get("verdict").cloned().unwrap_or_default(),
            license: map.get("license").cloned().unwrap_or_default(),
            summary: map.get("summary").cloned().unwrap_or_default(),
        })
        .collect()
}

/// Parse `systems.toml` content into a list of `SystemRow`s.
pub fn parse_systems_toml(content: &str) -> Vec<SystemRow> {
    parse_single_line_toml(content)
        .into_iter()
        .map(|(name, map)| {
            let raw_url = map.get("url").cloned().unwrap_or_default();
            let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
                Some(raw_url)
            } else {
                None
            };
            let local = map.get("local").map(|v| v == "yes").unwrap_or(false);
            SystemRow {
                name,
                url,
                kind: map.get("kind").cloned().unwrap_or_default(),
                local,
                why: map.get("why").cloned().unwrap_or_default(),
            }
        })
        .collect()
}

/// Convert a markdown file and its raw text into a typed `ResearchArticle`.
pub fn to_article(file: &str, markdown: &str) -> ResearchArticle {
    let slug = if file == "README.md" {
        "index".to_string()
    } else {
        file.strip_suffix(".md").unwrap_or(file).to_string()
    };

    let mut title = file.to_string();
    let mut body_lines = Vec::new();
    let mut found_title = false;

    for line in markdown.lines() {
        if !found_title && line.starts_with("# ") {
            title = line[2..].trim().to_string();
            found_title = true;
        } else {
            body_lines.push(line);
        }
    }

    ResearchArticle {
        slug,
        file: file.to_string(),
        title,
        body: body_lines.join("\n").trim().to_string(),
    }
}

/// Group upstreams into human-oriented categories (Built on, Learned from, Watching, Declined).
pub fn group_upstreams(rows: &[UpstreamRow]) -> Vec<ProjectGroup> {
    let mut built_on = Vec::new();
    let mut learned_from = Vec::new();
    let mut watching = Vec::new();
    let mut declined = Vec::new();
    let mut other = Vec::new();

    for row in rows {
        match row.verdict.as_str() {
            "adopt" | "adapt" | "contribute" | "overlay" | "fork" | "build" => {
                built_on.push(row.clone());
            }
            "inspiration" | "quarry" => {
                learned_from.push(row.clone());
            }
            "watch" => {
                watching.push(row.clone());
            }
            "reject" => {
                declined.push(row.clone());
            }
            _ => {
                other.push(row.clone());
            }
        }
    }

    let sort_group = |items: &mut Vec<UpstreamRow>| {
        items.sort_by(|a, b| a.name.cmp(&b.name));
    };
    sort_group(&mut built_on);
    sort_group(&mut learned_from);
    sort_group(&mut watching);
    sort_group(&mut declined);
    sort_group(&mut other);

    let mut groups = vec![
        ProjectGroup {
            id: "built-on".to_string(),
            label: "Built on".to_string(),
            blurb: "Code Sjel uses, contributes to, or carries a reviewed delta for.".to_string(),
            rows: built_on,
        },
        ProjectGroup {
            id: "learned-from".to_string(),
            label: "Learned from".to_string(),
            blurb: "Projects whose ideas or parts Sjel took, without depending on their code.".to_string(),
            rows: learned_from,
        },
        ProjectGroup {
            id: "watching".to_string(),
            label: "Watching".to_string(),
            blurb: "Noted and not yet read. A watch row grants nothing until an audit gives it a verdict.".to_string(),
            rows: watching,
        },
        ProjectGroup {
            id: "declined".to_string(),
            label: "Declined".to_string(),
            blurb: "Read and declined, with the reason kept so nobody has to read them again.".to_string(),
            rows: declined,
        },
    ];

    if !other.is_empty() {
        groups.push(ProjectGroup {
            id: "other".to_string(),
            label: "Other verdicts".to_string(),
            blurb: "Projects with unique verdicts.".to_string(),
            rows: other,
        });
    }

    groups
}

/// Group benchmark runs by suite, sorted with best accuracy first.
pub fn group_suites(runs: &[BenchmarkRun]) -> Vec<BenchmarkSuite> {
    let mut by_suite: BTreeMap<String, Vec<BenchmarkRun>> = BTreeMap::new();
    for run in runs {
        by_suite
            .entry(run.suite.clone())
            .or_default()
            .push(run.clone());
    }

    let mut suites = Vec::new();
    for (name, mut suite_runs) in by_suite {
        // Best accuracy first, then lowest p50 latency
        suite_runs.sort_by(|a, b| {
            b.result
                .accuracy
                .partial_cmp(&a.result.accuracy)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.result
                        .latency_ms
                        .p50
                        .partial_cmp(&b.result.latency_ms.p50)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut case_ids = Vec::new();
        for r in &suite_runs {
            for c in &r.result.cases {
                if !case_ids.contains(&c.id) {
                    case_ids.push(c.id.clone());
                }
            }
        }

        suites.push(BenchmarkSuite {
            name,
            runs: suite_runs,
            case_ids,
        });
    }

    suites
}

/// Build the embedded default `ResearchView`.
pub fn default_research_view() -> ResearchView {
    let upstreams = parse_upstreams_toml(include_str!("../../../../upstreams.toml"));
    let systems = parse_systems_toml(include_str!("../../../../systems.toml"));

    let articles = vec![
        to_article("README.md", include_str!("../../../../research/README.md")),
        to_article(
            "local-decision-models.md",
            include_str!("../../../../research/local-decision-models.md"),
        ),
        to_article(
            "offline-on-the-phone.md",
            include_str!("../../../../research/offline-on-the-phone.md"),
        ),
        to_article(
            "assistant-harness.md",
            include_str!("../../../../research/assistant-harness.md"),
        ),
        to_article(
            "agent-safety.md",
            include_str!("../../../../research/agent-safety.md"),
        ),
        to_article(
            "cloud-models-and-privacy.md",
            include_str!("../../../../research/cloud-models-and-privacy.md"),
        ),
        to_article(
            "on-device-models.md",
            include_str!("../../../../research/on-device-models.md"),
        ),
        to_article(
            "simplicity-and-adoption.md",
            include_str!("../../../../research/simplicity-and-adoption.md"),
        ),
        to_article(
            "why-sjel.md",
            include_str!("../../../../research/why-sjel.md"),
        ),
    ];

    let bench_nimble: BenchmarkResult = serde_json::from_str(include_str!(
        "../../../../research/benchmarks/decisions/results/2026-09-30-ollama-nimble.json"
    ))
    .unwrap_or_default();
    let bench_tev1: BenchmarkResult = serde_json::from_str(include_str!(
        "../../../../research/benchmarks/decisions/results/2026-09-30-ollama-tev1.json"
    ))
    .unwrap_or_default();
    let bench_tev1_08b: BenchmarkResult = serde_json::from_str(include_str!(
        "../../../../research/benchmarks/decisions/results/2026-09-30-ollama-tev1-0.8b.json"
    ))
    .unwrap_or_default();

    let benchmark_runs = vec![
        BenchmarkRun {
            suite: "decisions".to_string(),
            file: "2026-09-30-ollama-nimble.json".to_string(),
            result: bench_nimble,
        },
        BenchmarkRun {
            suite: "decisions".to_string(),
            file: "2026-09-30-ollama-tev1.json".to_string(),
            result: bench_tev1,
        },
        BenchmarkRun {
            suite: "decisions".to_string(),
            file: "2026-09-30-ollama-tev1-0.8b.json".to_string(),
            result: bench_tev1_08b,
        },
    ];

    let suites = group_suites(&benchmark_runs);

    ResearchView {
        articles,
        upstreams,
        systems,
        suites,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line_toml_parser() {
        let raw = r#"
# Comment
[lifeos]
url = "https://github.com/danielmiessler/LifeOS"
verdict = "inspiration"
license = "MIT"
summary = "Personal-AI harness"

[rust-analyzer]
url = "https://github.com/rust-lang/rust-analyzer"
verdict = "inspiration"
"#;
        let parsed = parse_upstreams_toml(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "lifeos");
        assert_eq!(parsed[0].verdict, "inspiration");
        assert_eq!(parsed[0].license, "MIT");
        assert_eq!(parsed[1].name, "rust-analyzer");
    }

    #[test]
    fn test_to_article() {
        let raw = "# Sjel Premise\n\nThis is why Sjel exists.\nAnother paragraph.";
        let art = to_article("why-sjel.md", raw);
        assert_eq!(art.slug, "why-sjel");
        assert_eq!(art.title, "Sjel Premise");
        assert_eq!(art.body, "This is why Sjel exists.\nAnother paragraph.");
    }

    #[test]
    fn test_group_upstreams() {
        let rows = vec![
            UpstreamRow {
                name: "rust-analyzer".into(),
                verdict: "inspiration".into(),
                ..Default::default()
            },
            UpstreamRow {
                name: "gitleaks".into(),
                verdict: "adopt".into(),
                ..Default::default()
            },
            UpstreamRow {
                name: "semgrep".into(),
                verdict: "reject".into(),
                ..Default::default()
            },
            UpstreamRow {
                name: "future-lib".into(),
                verdict: "watch".into(),
                ..Default::default()
            },
        ];

        let groups = group_upstreams(&rows);
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].id, "built-on");
        assert_eq!(groups[0].rows.len(), 1);
        assert_eq!(groups[1].id, "learned-from");
        assert_eq!(groups[1].rows.len(), 1);
        assert_eq!(groups[2].id, "watching");
        assert_eq!(groups[2].rows.len(), 1);
        assert_eq!(groups[3].id, "declined");
        assert_eq!(groups[3].rows.len(), 1);
    }

    #[test]
    fn test_default_research_view_loads_cleanly() {
        let view = default_research_view();
        assert!(!view.articles.is_empty());
        assert!(!view.upstreams.is_empty());
        assert!(!view.systems.is_empty());
        assert!(!view.suites.is_empty());
        assert_eq!(view.suites[0].name, "decisions");
        assert_eq!(view.suites[0].runs.len(), 3);
    }
}
