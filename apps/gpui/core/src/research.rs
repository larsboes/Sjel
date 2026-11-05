//! Data models and pure reductions for Sjel Research & Local Decision Models (`/research`).
//!
//! Replaces the dashboard's `/research` routes (`/research`, `/research/projects`, `/research/benchmarks`).
//! Provides markdown research articles, external project registers (`upstreams.toml` and `systems.toml`),
//! and model benchmark evaluation runs.

pub use status_api::{
    group_suites, group_upstreams, parse_single_line_toml, parse_systems_toml,
    parse_upstreams_toml, to_article, BenchmarkCase, BenchmarkLatency, BenchmarkResult,
    BenchmarkRun, BenchmarkSuite, ProjectGroup, ResearchArticle, ResearchView, SystemRow,
    UpstreamRow,
};

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
