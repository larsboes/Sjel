//! Multi-source merge ranking: one unified ranked list from N per-source
//! result blocks (see the merge-ranking notes in
//! `capabilities/scouting/README.md` — multi-source
//! orchestration is config-driven; this module is the merge layer on top).
//!
//! Score normalization: **per-source min-max within the batch.** Each
//! source's raw cosine scores sit on their own scale (embedding quality,
//! hash-fallback vs e5, the ×0.1 category-affinity penalty in `score.rs`),
//! so raw scores are not comparable across sources. Min-max maps each
//! source's batch onto [0, 1] using that batch's own min/max. Chosen over
//! z-score because batches are small (often <20 items) and the distribution
//! is non-Gaussian (the affinity penalty produces bimodal clusters, making
//! mean/stddev meaningless), and over pure rank/percentile because min-max
//! preserves relative magnitude within a source instead of flattening it.
//! Rank order within each source is unchanged — normalization only makes
//! scores comparable across sources. Degenerate batches (single item, or all
//! scores equal) get 0.5: with no scale information they land mid-range
//! instead of artificially topping (1.0) or sinking (0.0) the merged list.
//!
//! Cross-source dedup: conservative **exact match on normalized title +
//! calendar date** (lowercased alphanumeric words + first 10 chars of
//! `starts_at`). Deliberately NOT fuzzy embedding distance — a false merge
//! silently loses an event, a missed merge only shows a duplicate. Entries
//! with no date only merge with other no-date entries.

use std::collections::HashMap;

use crate::opportunity::Opportunity;
use crate::score::ScoredOpportunity;

#[derive(Debug)]
pub struct MergedEntry {
    /// Representative copy — the highest-normalized-score one across sources.
    pub scored: ScoredOpportunity,
    /// Score after per-source min-max normalization, in [0, 1].
    pub normalized_score: f64,
    /// Every source id that carried this event; the representative's first.
    pub sources: Vec<String>,
}

/// Merge per-source scored batches into one deduplicated, cross-source ranked
/// list. Input: `(source_id, scored batch)` pairs as produced by
/// `pipeline::run` per source. Output is sorted by normalized score
/// descending (title as deterministic tiebreak).
pub fn merge(per_source: Vec<(String, Vec<ScoredOpportunity>)>) -> Vec<MergedEntry> {
    let mut by_key: HashMap<String, MergedEntry> = HashMap::new();

    for (source_id, batch) in per_source {
        let normalized = normalize_scores(&batch);
        for (scored, norm) in batch.into_iter().zip(normalized) {
            let key = dedup_key(&scored.opportunity);
            match by_key.get_mut(&key) {
                Some(existing) => {
                    if norm > existing.normalized_score {
                        existing.scored = scored;
                        existing.normalized_score = norm;
                        // New representative — its source moves to the front.
                        existing.sources.retain(|s| s != &source_id);
                        existing.sources.insert(0, source_id.clone());
                    } else if !existing.sources.contains(&source_id) {
                        existing.sources.push(source_id.clone());
                    }
                }
                None => {
                    by_key.insert(
                        key,
                        MergedEntry {
                            scored,
                            normalized_score: norm,
                            sources: vec![source_id.clone()],
                        },
                    );
                }
            }
        }
    }

    let mut merged: Vec<MergedEntry> = by_key.into_values().collect();
    merged.sort_by(|a, b| {
        b.normalized_score
            .partial_cmp(&a.normalized_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.scored.opportunity.title.cmp(&b.scored.opportunity.title))
    });
    merged
}

/// Per-source min-max normalization (see module header for the rationale).
fn normalize_scores(batch: &[ScoredOpportunity]) -> Vec<f64> {
    if batch.is_empty() {
        return Vec::new();
    }
    let min = batch.iter().map(|s| s.score).fold(f64::INFINITY, f64::min);
    let max = batch
        .iter()
        .map(|s| s.score)
        .fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON {
        return vec![0.5; batch.len()];
    }
    batch
        .iter()
        .map(|s| (s.score - min) / (max - min))
        .collect()
}

/// Conservative dedup key: normalized title + calendar date. Missing dates
/// hash as an empty date component, so they only collide with other
/// missing-date entries carrying the same title.
fn dedup_key(opp: &Opportunity) -> String {
    let title = normalize_title(&opp.title);
    let date = opp
        .starts_at
        .as_deref()
        .map(|s| s.get(..10).unwrap_or(s))
        .unwrap_or("");
    format!("{title}|{date}")
}

/// Lowercase, split on non-alphanumerics, rejoin with single spaces —
/// "AI  Hackathon — Berlin!" and "ai hackathon berlin" produce the same key.
fn normalize_title(title: &str) -> String {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity::{OpportunityType, SourceKind};

    fn opp(title: &str, starts_at: Option<&str>) -> Opportunity {
        Opportunity {
            id: format!("id:{title}"),
            opportunity_type: OpportunityType::Event,
            source: "s".into(),
            source_kind: SourceKind::Api,
            url: "u".into(),
            title: title.into(),
            starts_at: starts_at.map(|s| s.into()),
            ends_at: None,
            location: None,
            city: None,
            country_code: None,
            latitude: None,
            longitude: None,
            raw: serde_json::Value::Null,
            fetched_at: "t".into(),
        }
    }

    fn scored(title: &str, starts_at: Option<&str>, score: f64) -> ScoredOpportunity {
        ScoredOpportunity {
            opportunity: opp(title, starts_at),
            score,
            rationale: "test".into(),
            matched_focus: None,
        }
    }

    #[test]
    fn normalization_makes_cross_source_scores_comparable() {
        // Source A scores on a ~[0.1, 0.9] scale, source B on ~[0.01, 0.09].
        // Raw-score sorting would put all of A above all of B; per-source
        // min-max puts each source's best at 1.0 and worst at 0.0.
        let merged = merge(vec![
            (
                "a".into(),
                vec![scored("a-top", None, 0.9), scored("a-low", None, 0.1)],
            ),
            (
                "b".into(),
                vec![scored("b-top", None, 0.09), scored("b-low", None, 0.01)],
            ),
        ]);
        assert_eq!(merged.len(), 4);
        assert!((merged[0].normalized_score - 1.0).abs() < 1e-9);
        assert!((merged[1].normalized_score - 1.0).abs() < 1e-9);
        let top_titles: Vec<&str> = merged[..2]
            .iter()
            .map(|m| m.scored.opportunity.title.as_str())
            .collect();
        assert!(top_titles.contains(&"a-top"));
        assert!(top_titles.contains(&"b-top")); // b-top outranks a-low despite raw 0.09 < 0.1
        assert!((merged[2].normalized_score - 0.0).abs() < 1e-9);
        assert!((merged[3].normalized_score - 0.0).abs() < 1e-9);
    }

    #[test]
    fn same_normalized_title_and_date_across_sources_merges_to_one() {
        let merged = merge(vec![
            (
                "rss".into(),
                vec![scored(
                    "AI  Hackathon — Berlin!",
                    Some("2026-09-01T09:00:00Z"),
                    0.8,
                )],
            ),
            (
                "obsidian".into(),
                vec![scored("ai hackathon berlin", Some("2026-09-01"), 0.3)],
            ),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sources.len(), 2);
        assert!(merged[0].sources.contains(&"rss".into()));
        assert!(merged[0].sources.contains(&"obsidian".into()));
    }

    #[test]
    fn different_events_are_kept_apart() {
        // Same title, different date → two entries. Different title → two entries.
        let merged = merge(vec![
            (
                "rss".into(),
                vec![
                    scored("AI Hackathon Berlin", Some("2026-09-01"), 0.8),
                    scored("Rust Meetup Bonn", Some("2026-09-01"), 0.5),
                ],
            ),
            (
                "obsidian".into(),
                vec![scored("AI Hackathon Berlin", Some("2026-10-01"), 0.3)],
            ),
        ]);
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn missing_date_only_merges_with_missing_date() {
        let merged = merge(vec![
            (
                "rss".into(),
                vec![scored("AI Hackathon Berlin", Some("2026-09-01"), 0.8)],
            ),
            (
                "obsidian".into(),
                vec![scored("AI Hackathon Berlin", None, 0.3)],
            ),
        ]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn degenerate_batch_gets_midpoint_score() {
        // Single item and all-equal batches carry no scale info → 0.5.
        let merged = merge(vec![
            ("solo".into(), vec![scored("only event", None, 0.7)]),
            (
                "flat".into(),
                vec![scored("x", None, 0.4), scored("y", None, 0.4)],
            ),
        ]);
        for m in &merged {
            assert!(
                (m.normalized_score - 0.5).abs() < 1e-9,
                "{}: {}",
                m.scored.opportunity.title,
                m.normalized_score
            );
        }
    }
}
