//! Deterministic Feed quality signals for the human review queue.
//!
//! This module has no inference dependency. It inspects values already stored
//! by extraction, normalization, summarization, and ranking; a flag suggests a
//! review and never starts enrichment or changes the human Feed status.

use crate::config::QualityFlagConfig;
use crate::normalize;
use crate::provenance::StageProvenance;
use crate::store::FeedItem;

#[derive(Debug, Clone, PartialEq)]
pub struct QualityFlag {
    pub signal: String,
    pub reason: String,
    pub evidence: String,
}

impl QualityFlag {
    fn new(signal: &str, reason: impl Into<String>, evidence: impl Into<String>) -> Self {
        Self {
            signal: signal.to_string(),
            reason: reason.into(),
            evidence: evidence.into(),
        }
    }
}

/// Derive independently inspectable review signals from one stored item.
///
/// `has_ranking` means an evaluation row exists. Its model/heuristic score and
/// any self-reported confidence are deliberately absent from the inputs.
pub fn derive(
    item: &FeedItem,
    raw_content: Option<&str>,
    stages: &[StageProvenance],
    has_ranking: bool,
    config: &QualityFlagConfig,
) -> Vec<QualityFlag> {
    let mut flags = Vec::new();
    let extraction = stages.iter().find(|stage| stage.stage == "extraction");
    let extraction_path = item.captured_via.as_deref().unwrap_or("server-fetch");

    match item.content_status.as_str() {
        "none" => flags.push(QualityFlag::new(
            "content_status",
            "content_status fired: extraction retained no readable body",
            format!("content_status=none; extraction_path={extraction_path}"),
        )),
        "unknown" => flags.push(QualityFlag::new(
            "content_status",
            "content_status fired: legacy content has not been classified",
            format!("content_status=unknown; extraction_path={extraction_path}"),
        )),
        _ => {}
    }

    match (raw_content, extraction) {
        (None, _) => flags.push(QualityFlag::new(
            "extraction_path",
            "extraction_path fired: raw extractor output is unavailable for replay",
            format!("captured_via={extraction_path}; raw_content=missing"),
        )),
        (Some(_), Some(stage)) if stage.tier == "legacy" => flags.push(QualityFlag::new(
            "extraction_path",
            "extraction_path fired: extractor provenance is legacy and cannot name its producer",
            format!(
                "captured_via={extraction_path}; tier={}; revision={}",
                stage.tier, stage.revision
            ),
        )),
        _ => {}
    }

    if let (Some(raw), Some(canonical)) = (raw_content, item.transcript.as_deref()) {
        let raw_chars = raw.chars().count();
        if raw_chars > 0 {
            let canonical_chars = canonical.chars().count();
            let retained_percent = percent(canonical_chars, raw_chars);
            if retained_percent < config.minimum_total_retention_percent {
                flags.push(QualityFlag::new(
                    "retention",
                    "retention fired: canonical text falls below the passing corpus envelope",
                    format!(
                        "retained={retained_percent:.1}%; minimum={:.1}%; raw_chars={raw_chars}; canonical_chars={canonical_chars}",
                        config.minimum_total_retention_percent
                    ),
                ));
            } else if retained_percent > config.maximum_total_retention_percent {
                flags.push(QualityFlag::new(
                    "retention",
                    "retention fired: canonical text exceeds the passing corpus envelope",
                    format!(
                        "retained={retained_percent:.1}%; maximum={:.1}%; raw_chars={raw_chars}; canonical_chars={canonical_chars}",
                        config.maximum_total_retention_percent
                    ),
                ));
            }
        }
    }

    if let Some(canonical) = item.transcript.as_deref() {
        let normalized_again = normalize::normalize(canonical);
        let leaked = normalized_again
            .dropped
            .iter()
            .filter(|drop| drop.rule != "blank-run")
            .collect::<Vec<_>>();
        let leaked_lines = leaked.iter().map(|drop| drop.lines).sum::<usize>();
        let content_lines = canonical
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        let leakage_percent = percent(leaked_lines, content_lines.max(1));
        if leakage_percent > config.maximum_boilerplate_leakage_percent {
            let rules = leaked
                .iter()
                .map(|drop| format!("{}:{}", drop.rule, drop.lines))
                .collect::<Vec<_>>()
                .join(",");
            flags.push(QualityFlag::new(
                "boilerplate_leakage",
                "boilerplate_leakage fired: stored canonical text still matches normalization rules",
                format!(
                    "leakage={leakage_percent:.1}%; maximum={:.1}%; matched={rules}",
                    config.maximum_boilerplate_leakage_percent
                ),
            ));
        }
    }

    if item.summary_attempts >= config.summary_attempt_warning {
        flags.push(QualityFlag::new(
            "summary_attempts",
            "summary_attempts fired: summarization is approaching or has reached its retry cap",
            format!(
                "attempts={}; warning_at={}; last_error={}",
                item.summary_attempts,
                config.summary_attempt_warning,
                item.summary_last_error.as_deref().unwrap_or("unknown")
            ),
        ));
    }

    if has_ranking {
        let basis = match item.content_status.as_str() {
            "full" => "full_text",
            "thin" => "abstract",
            _ => "bare_title",
        };
        if basis != "full_text" {
            let ranking = stages.iter().find(|stage| stage.stage == "ranking");
            flags.push(QualityFlag::new(
                "ranking_basis",
                format!("ranking_basis fired: ranking used {basis} rather than full text"),
                format!(
                    "basis={basis}; content_status={}; summary_present={}; ranking_revision={}",
                    item.content_status,
                    item.summary
                        .as_deref()
                        .is_some_and(|summary| !summary.trim().is_empty()),
                    ranking
                        .map(|stage| stage.revision.as_str())
                        .unwrap_or("unknown")
                ),
            ));
        }
    }

    flags
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(content_status: &str, transcript: Option<&str>) -> FeedItem {
        let mut item = FeedItem::new("https://example.com/item", "news", "article");
        item.content_status = content_status.to_string();
        item.transcript = transcript.map(str::to_string);
        item
    }

    fn deterministic_stages() -> Vec<StageProvenance> {
        let mut extraction = StageProvenance::deterministic("extract-v1");
        extraction.stage = "extraction".into();
        let mut ranking = StageProvenance::deterministic("rank-v1");
        ranking.stage = "ranking".into();
        vec![extraction, ranking]
    }

    #[test]
    fn clean_full_text_inside_the_corpus_envelope_has_no_flags() {
        let raw = "x".repeat(100);
        let canonical = "x".repeat(60);
        let item = item("full", Some(&canonical));

        assert!(derive(
            &item,
            Some(&raw),
            &deterministic_stages(),
            true,
            &QualityFlagConfig::default()
        )
        .is_empty());
    }

    #[test]
    fn retention_flags_both_sides_of_the_fixture_envelope() {
        for canonical_chars in [20, 95] {
            let raw = "x".repeat(100);
            let canonical = "x".repeat(canonical_chars);
            let item = item("full", Some(&canonical));
            let flags = derive(
                &item,
                Some(&raw),
                &deterministic_stages(),
                false,
                &QualityFlagConfig::default(),
            );
            assert_eq!(
                flags
                    .iter()
                    .filter(|flag| flag.signal == "retention")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn residual_boilerplate_names_the_rule_as_evidence() {
        let canonical = "Useful sentence.\nAccept all cookies";
        let raw = format!("{canonical}\n{}", "padding ".repeat(8));
        let item = item("full", Some(canonical));
        let flags = derive(
            &item,
            Some(&raw),
            &deterministic_stages(),
            false,
            &QualityFlagConfig::default(),
        );
        let flag = flags
            .iter()
            .find(|flag| flag.signal == "boilerplate_leakage")
            .unwrap();
        assert!(flag.evidence.contains("cookie-notice:1"));
    }

    #[test]
    fn summary_error_and_abstract_ranking_are_independent_signals() {
        let mut item = item("thin", Some("Short abstract."));
        item.summary_attempts = 2;
        item.summary_last_error = Some("timeout".into());
        let raw = format!("{}{}", "Short abstract.", "x".repeat(20));
        let flags = derive(
            &item,
            Some(&raw),
            &deterministic_stages(),
            true,
            &QualityFlagConfig::default(),
        );

        assert!(flags.iter().any(|flag| {
            flag.signal == "summary_attempts" && flag.evidence.contains("last_error=timeout")
        }));
        assert!(flags.iter().any(|flag| {
            flag.signal == "ranking_basis" && flag.evidence.contains("basis=abstract")
        }));
    }

    #[test]
    fn missing_content_and_raw_output_explain_the_extraction_path() {
        let mut item = item("none", None);
        item.captured_via = Some("extension".into());
        let flags = derive(&item, None, &[], true, &QualityFlagConfig::default());

        assert!(flags.iter().any(|flag| flag.signal == "content_status"));
        assert!(flags.iter().any(|flag| {
            flag.signal == "extraction_path" && flag.evidence.contains("captured_via=extension")
        }));
        assert!(flags.iter().any(|flag| {
            flag.signal == "ranking_basis" && flag.evidence.contains("basis=bare_title")
        }));
    }
}
