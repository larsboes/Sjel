//! Deterministic, inspectable evaluation for Feed items.
//!
//! The model supplies summaries and TELOS embeddings; it does not invent the
//! final rank. This module turns stored facts into explicit factors with fixed
//! weights. Revisions make the result cacheable: unchanged content under the
//! same TELOS context and evaluator revision is never evaluated again.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::media;
use crate::relevance::{InterestProfile, RelevanceMatch};
use crate::store::FeedItem;
use crate::travel::{self, TravelContext};

/// v6 adds the learned feedback factor as a fifth entry in the vector, so every
/// stored evaluation restales and is recomputed once. Under the split currency
/// check that is a re-evaluation from stored matches rather than a re-embed,
/// for every item whose relevance revision has not moved.
pub const EVALUATOR_REVISION: &str = "feed-evaluator-v6-feedback";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluationFactorContext {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationFactor {
    pub key: String,
    pub label: String,
    /// Normalized value in the closed interval 0..=1.
    pub score: f64,
    /// Share of the overall score. All factors for this revision sum to 1,
    /// except on a class refusal, where the refused factor's share is withheld
    /// rather than redistributed and the sum is deliberately below 1
    /// (`scale_weights`).
    pub weight: f64,
    pub rationale: String,
    pub context: Option<EvaluationFactorContext>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedEvaluation {
    pub feed_id: String,
    pub overall_score: f64,
    pub explanation: String,
    pub mode: String,
    pub item_revision: String,
    pub context_revision: String,
    pub evaluator_revision: String,
    pub evaluated_at: String,
    pub factors: Vec<EvaluationFactor>,
}

pub fn item_revision(item: &FeedItem) -> String {
    revision_hash(&[
        &item.stream,
        &item.kind,
        &item.url,
        item.title.as_deref().unwrap_or_default(),
        item.author.as_deref().unwrap_or_default(),
        item.summary.as_deref().unwrap_or_default(),
        item.transcript.as_deref().unwrap_or_default(),
    ])
}

/// What decides whether an item must be RE-EMBEDDED.
///
/// The half of the old `context_revision` that is genuinely about the vector
/// space: the lens texts and the two producers. Split out because
/// `relevance_refresh_handler` retains items by `is_current` and hands the
/// retained set straight to `score_items`, so every term in `context_revision`
/// was an embedding trigger — a changed travel snapshot, or a retrained
/// feedback model, would re-embed the whole window for a factor weighted 0.10.
///
/// The last completed full sweep records this in the `relevance-pass` receipt's
/// cursor, so a pass can tell "the vector space moved" from "the ranking inputs
/// moved" without a new column and without an ALTER.
pub fn relevance_revision(
    profiles: &[InterestProfile],
    embedding_producer: Option<&str>,
    reranking_producer: Option<&str>,
) -> String {
    let mut revisions = profiles
        .iter()
        .map(|profile| format!("{}:{}", profile.key, profile.fingerprint))
        .collect::<Vec<_>>();
    revisions.push(format!(
        "embedding:{}",
        embedding_producer.unwrap_or("lexical")
    ));
    revisions.push(format!(
        "reranking:{}",
        reranking_producer.unwrap_or("semantic")
    ));
    revisions.sort();
    revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

/// What decides whether an item must be RE-EVALUATED, from stored matches.
pub fn context_revision(
    profiles: &[InterestProfile],
    embedding_producer: Option<&str>,
    reranking_producer: Option<&str>,
    travel_revision: &str,
    feedback_revision: &str,
) -> String {
    let mut revisions = profiles
        .iter()
        .map(|profile| format!("{}:{}", profile.key, profile.fingerprint))
        .collect::<Vec<_>>();
    // A provider/model change changes the vector space even when the source
    // notes do not. Including it here makes the persisted ledger self-heal on
    // the next normal refresh instead of requiring an undocumented force run.
    revisions.push(format!(
        "embedding:{}",
        embedding_producer.unwrap_or("lexical")
    ));
    revisions.push(format!(
        "reranking:{}",
        reranking_producer.unwrap_or("semantic")
    ));
    revisions.push(format!("travel:{travel_revision}"));
    // The learned model's revision is the literal `none` while it is inert, so
    // accumulating labels does not restale 372 cached evaluations for a factor
    // that counts for nothing.
    revisions.push(format!("feedback:{feedback_revision}"));
    revisions.sort();
    revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Whether a stored evaluation may be left alone.
///
/// `semantic_available` is the fourth condition, and it is not folded into a
/// revision on purpose: making the answering mode part of the hash would mint a
/// second revision per outcome and thrash between them. A row that was written
/// `lexical` while an embedding role is reachable is stale by definition — 525
/// such rows were all written in one pass on 2026-08-30 and have read as
/// current ever since. Passing `false` (no reachable role) keeps them current,
/// so the drain happens over ordinary passes with no force flag.
pub fn is_current(
    stored: Option<&FeedEvaluation>,
    item_revision: &str,
    context_revision: &str,
    semantic_available: bool,
) -> bool {
    stored.is_some_and(|evaluation| {
        let mode_is_final =
            !(semantic_available && matches!(evaluation.mode.as_str(), "lexical" | "unscored"));
        mode_is_final
            && evaluation.item_revision == item_revision
            && evaluation.context_revision == context_revision
            && evaluation.evaluator_revision == EVALUATOR_REVISION
    })
}

/// Whether a stored class REFUSAL may be left alone.
///
/// [`is_current`] treats `unscored` as stale whenever an embedding role answers,
/// because a row written while the embedder was down should drain on the next
/// pass. A class refusal is the opposite case: no reachable embedder can ever
/// upgrade it, so that rule rewrote every c3 row and every one of its factor
/// rows on every pass, forever. The three revision terms still decide currency,
/// so a moved lens, a changed item or a new evaluator revision still restales
/// the refusal.
pub fn refusal_is_current(
    stored: Option<&FeedEvaluation>,
    item_revision: &str,
    context_revision: &str,
) -> bool {
    stored.is_some_and(|evaluation| {
        evaluation.mode == "unscored"
            && evaluation.item_revision == item_revision
            && evaluation.context_revision == context_revision
            && evaluation.evaluator_revision == EVALUATOR_REVISION
    })
}

/// The share the learned feedback factor takes when it is active.
///
/// Taste, not measurement, which is why it is a named constant reported by
/// `GET /feed/evaluation/status`: 15 points of 100 re-orders inside a band and
/// can never outvote the 45-point TELOS factor.
pub const FEEDBACK_WEIGHT: f64 = 0.15;

/// The one weight rule, stated once and reused by both evaluators.
///
/// **A factor whose producer has not run yet carries weight 0, and the
/// remaining factors scale so the sum stays 1.0.** That is what lets an inert
/// learned factor, or an urgency the model rung has not published, leave the
/// arithmetic whole instead of dumping the item to the bottom of its band by a
/// zero it never earned.
///
/// **A class refusal is not that case, and callers must not rescale it.** A
/// refusal is missing evidence, not a free pass. Measured on a copy of the live
/// database: rescaling a refused mail's `interest` share into `category` and
/// `age` -- 1.0 and ~0.6 for an `aktiv` thread -- put all 11 c3 mails at
/// 0.863..0.990 against a maximum of 0.702 for every mail that was actually
/// read, so the eleven threads the class ladder forbids a model to read led the
/// band. Both evaluators therefore skip this call when `refused_class` is true:
/// the refused factor keeps weight 0, every other factor keeps its stated
/// share, the sum stays deliberately below 1.0, and a refused row can never
/// outrank a scored row whose other factors are identical.
///
/// `reserved_keys` names the factors whose weight is a fixed share rather than a
/// share of the base — `feedback` on the feed, `urgency` on mail. Those keep
/// their stated number and the base factors scale into what is left, which is
/// why 0.45/0.25/0.20/0.10 with an active learned factor becomes
/// 0.3825/0.2125/0.17/0.085/0.15 and not five equal fifths of 1.15.
pub(crate) fn scale_weights(factors: &mut [EvaluationFactor], reserved_keys: &[&str]) {
    let is_reserved = |factor: &EvaluationFactor| reserved_keys.contains(&factor.key.as_str());
    let reserved = factors
        .iter()
        .filter(|factor| is_reserved(factor))
        .map(|factor| factor.weight)
        .sum::<f64>();
    let base_total = factors
        .iter()
        .filter(|factor| !is_reserved(factor))
        .map(|factor| factor.weight)
        .sum::<f64>();
    if base_total <= 0.0 {
        return;
    }
    let share = (1.0 - reserved).max(0.0);
    for factor in factors.iter_mut() {
        if !reserved_keys.contains(&factor.key.as_str()) {
            factor.weight = factor.weight / base_total * share;
        }
    }
}

pub fn evaluate(
    item: &FeedItem,
    strongest_match: Option<&RelevanceMatch>,
    context_revision: &str,
    travel_contexts: &[TravelContext],
    refused_class: bool,
    feedback: Option<EvaluationFactor>,
) -> FeedEvaluation {
    let interest_score = strongest_match
        .map(|matched| matched.score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let interest_rationale = if refused_class {
        // Stored as a row rather than as an absence. An item with no
        // evaluation is indistinguishable from one nobody has got to yet; a
        // refusal says who refused and why, and it survives the next pass.
        "Not scored: c3 is never read by a model".to_string()
    } else {
        strongest_match
            .map(|matched| {
                format!(
                    "{} with {:.0}% alignment ({})",
                    matched.profile_label,
                    interest_score * 100.0,
                    match matched.mode.as_str() {
                        "reranked" => "reranked",
                        "semantic" => "semantic",
                        _ => "lexical",
                    }
                )
            })
            .unwrap_or_else(|| "No configured TELOS lens is available".into())
    };

    let age = age_days(&item.day);
    let freshness_score = freshness_score(age);
    let freshness_rationale = match age {
        Some(0) => "Captured today".to_string(),
        Some(1) => "Captured yesterday".to_string(),
        Some(days) => format!("Captured {days} days ago"),
        None => "Capture date cannot be evaluated".to_string(),
    };

    let (evidence_score, evidence_rationale) = evidence_score(item);
    let travel_signal = travel::score_item(item, travel_contexts);
    let mut factors = vec![
        EvaluationFactor {
            key: "interest".into(),
            label: "Interest fit".into(),
            score: if refused_class { 0.0 } else { interest_score },
            weight: if refused_class { 0.0 } else { 0.45 },
            rationale: interest_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "travel".into(),
            label: "Travel relevance".into(),
            score: travel_signal.score,
            weight: 0.25,
            rationale: travel_signal.rationale,
            context: travel_signal
                .context
                .map(|context| EvaluationFactorContext {
                    kind: "trip".into(),
                    id: context.id,
                    label: context.label,
                    date_start: Some(context.date_start),
                    date_end: Some(context.date_end),
                    matched_terms: context.matched_terms,
                }),
        },
        EvaluationFactor {
            key: "freshness".into(),
            label: "Freshness".into(),
            score: freshness_score,
            weight: 0.20,
            rationale: freshness_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "evidence".into(),
            label: "Content evidence".into(),
            score: evidence_score,
            weight: 0.10,
            rationale: evidence_rationale,
            context: None,
        },
    ];
    // The fifth factor. Absent means the model has never been trained; present
    // and inert means it has been, and is still below its gate -- which the
    // reader should be able to see, so it renders at weight 0 with its own
    // rationale rather than vanishing.
    if let Some(feedback) = feedback {
        factors.push(feedback);
    }
    // `feedback` is a reserved share: active, it takes its stated 0.15 and the
    // four base factors scale into the remaining 0.85, so 0.45/0.25/0.20/0.10
    // becomes 0.3825/0.2125/0.17/0.085. The learned signal can move an item at
    // most 15 points of 100 -- it re-orders inside a band and can never outvote
    // the TELOS lenses.
    //
    // A refusal is never rescaled: the item keeps 0.25/0.20/0.10 (plus the
    // learned share when it is active) and caps below what the same item would
    // have scored with an interest match, which is the whole rule in
    // `scale_weights`.
    if !refused_class {
        scale_weights(&mut factors, &["feedback"]);
    }
    let overall_score = factors
        .iter()
        .map(|factor| factor.score * factor.weight)
        .sum::<f64>()
        .clamp(0.0, 1.0);
    // Zero-weight factors are skipped: a refused interest factor scoring 0.0
    // counts for nothing in the score, so reporting it as the largest deduction
    // would be the explanation contradicting the arithmetic.
    let counted = factors
        .iter()
        .filter(|factor| factor.weight > 0.0)
        .collect::<Vec<_>>();
    let strongest = counted
        .iter()
        .max_by(|left, right| left.score.total_cmp(&right.score))
        .expect("the evaluator always has at least one weighted factor");
    let weakest = counted
        .iter()
        .min_by(|left, right| left.score.total_cmp(&right.score))
        .expect("the evaluator always has at least one weighted factor");
    let explanation = format!(
        "Strongest signal: {} ({:.0}%). Largest deduction: {} ({:.0}%).",
        strongest.label,
        strongest.score * 100.0,
        weakest.label,
        weakest.score * 100.0
    );

    FeedEvaluation {
        feed_id: item.id.clone(),
        overall_score,
        explanation,
        mode: match strongest_match {
            // A refusal is `unscored` whatever a stale match once said.
            Some(matched) if !refused_class => matched.mode.clone(),
            _ => "unscored".into(),
        },
        item_revision: item_revision(item),
        context_revision: context_revision.to_string(),
        evaluator_revision: EVALUATOR_REVISION.into(),
        evaluated_at: String::new(),
        factors,
    }
}

/// Evidence is graded, not counted.
///
/// Presence alone scored a stored consent wall exactly like a stored paper,
/// because both put a non-empty string in `transcript`. The pipeline already
/// classifies the difference — `content_status` is derived from the
/// *normalized* body — so the source-text signal takes the share its status
/// earns. The other three stay binary: a title either exists or it does not.
fn evidence_score(item: &FeedItem) -> (f64, String) {
    let (text_earned, text_label) = source_text_evidence(item);
    let signals = [
        (
            "title",
            binary(item.title.as_deref().is_some_and(non_empty)),
            0.20,
        ),
        (
            "author",
            binary(item.author.as_deref().is_some_and(non_empty)),
            0.15,
        ),
        (
            "summary",
            binary(item.summary.as_deref().is_some_and(non_empty)),
            0.30,
        ),
        (text_label, text_earned, 0.35),
    ];
    let score = signals
        .iter()
        .map(|(_, earned, weight)| earned * weight)
        .sum::<f64>();
    let present = signals
        .iter()
        .filter(|(_, earned, _)| *earned > 0.0)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let missing = signals
        .iter()
        .filter(|(_, earned, _)| *earned == 0.0)
        .map(|(label, _, _)| *label)
        .collect::<Vec<_>>();
    let rationale = match (present.is_empty(), missing.is_empty()) {
        // `present` carries the graded label, so this arm cannot claim full
        // source text when the item only stored a card.
        (_, true) => format!("Available: {}", present.join(", ")),
        (true, _) => format!("No usable content yet; missing: {}", missing.join(", ")),
        _ => format!(
            "Available: {}; missing: {}",
            present.join(", "),
            missing.join(", ")
        ),
    };
    (score, rationale)
}

/// What the stored body is worth as a share of its weight, and how to name it
/// in the rationale. `unknown` is the legacy rows written before extraction
/// classified itself: grade those by the same threshold the classifier uses
/// rather than assuming the best case for them.
fn source_text_evidence(item: &FeedItem) -> (f64, &'static str) {
    let Some(text) = item.transcript.as_deref().filter(|t| non_empty(t)) else {
        return (0.0, "source text");
    };
    match item.content_status.as_str() {
        "full" => (1.0, "source text"),
        "thin" => (THIN_TEXT_SHARE, "thin source text"),
        "none" => (0.0, "source text"),
        _ if text.chars().count() >= media::CONTENT_FULL_THRESHOLD => (1.0, "source text"),
        _ => (THIN_TEXT_SHARE, "thin source text"),
    }
}

/// A card, an abstract or a page that normalized down to a stub is real
/// evidence, just not the article. It keeps well under half its weight so a
/// full body always outranks one on this factor.
const THIN_TEXT_SHARE: f64 = 0.4;

fn binary(present: bool) -> f64 {
    if present {
        1.0
    } else {
        0.0
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

pub(crate) fn freshness_score(age: Option<i64>) -> f64 {
    match age {
        None => 0.0,
        Some(days) if days <= 0 => 1.0,
        Some(days) if days <= 7 => interpolate(days, 0, 7, 1.0, 0.90),
        Some(days) if days <= 30 => interpolate(days, 7, 30, 0.90, 0.65),
        Some(days) if days <= 90 => interpolate(days, 30, 90, 0.65, 0.35),
        Some(days) if days <= 365 => interpolate(days, 90, 365, 0.35, 0.10),
        Some(_) => 0.05,
    }
}

fn interpolate(value: i64, start: i64, end: i64, high: f64, low: f64) -> f64 {
    let progress = (value - start) as f64 / (end - start) as f64;
    high + (low - high) * progress
}

pub(crate) fn age_days(day: &str) -> Option<i64> {
    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    age_days_at(day, now_secs)
}

pub(crate) fn age_days_at(day: &str, now_secs: i64) -> Option<i64> {
    let mut parts = day.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let date = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&date) {
        return None;
    }
    let item_days = days_from_civil(year, month, date);
    let now_days = now_secs / 86_400;
    Some((now_days - item_days).max(0))
}

/// Gregorian civil date to days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A length-prefixed SHA-256 over an ordered list of parts.
///
/// `pub` so the mail model rung stamps its `item_revision` with the same hash
/// the feed evaluator uses rather than a second one, and so
/// `mail_model_eval` can recompute it over a frozen corpus fixture. Length
/// prefixes rather than a separator, so no choice of parts can collide.
pub fn revision_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> FeedItem {
        let mut item = FeedItem::new("https://example.com/item", "news", "article");
        item.title = Some("Local-first systems".into());
        item.summary = Some("A bounded summary".into());
        item.transcript = Some("Full source".into());
        item.day = "2026-07-29".into();
        item
    }

    #[test]
    fn revisions_change_only_with_relevant_inputs() {
        let first = item();
        let mut second = first.clone();
        second.status = "keeper".into();
        assert_eq!(item_revision(&first), item_revision(&second));
        second.summary = Some("Changed summary".into());
        assert_ne!(item_revision(&first), item_revision(&second));
    }

    #[test]
    fn context_revision_includes_embedding_vector_space() {
        let profiles = Vec::new();
        assert_ne!(
            context_revision(
                &profiles,
                Some("ollama:nomic-embed-text"),
                None,
                "travel",
                "none"
            ),
            context_revision(
                &profiles,
                Some("omlx:multilingual-embedding"),
                None,
                "travel",
                "none"
            )
        );
    }

    #[test]
    fn context_revision_includes_travel_snapshot() {
        assert_ne!(
            context_revision(&[], None, None, "travel-one", "none"),
            context_revision(&[], None, None, "travel-two", "none")
        );
    }

    #[test]
    fn context_revision_includes_reranking_model() {
        assert_ne!(
            context_revision(
                &[],
                Some("omlx:e5"),
                Some("omlx:reranker-a"),
                "travel",
                "none"
            ),
            context_revision(
                &[],
                Some("omlx:e5"),
                Some("omlx:reranker-b"),
                "travel",
                "none"
            )
        );
    }

    #[test]
    fn score_is_weighted_and_bounded() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.8,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        let evaluation = evaluate(&item, Some(&matched), "context", &[], false, None);
        assert_eq!(evaluation.factors.len(), 4);
        assert!((0.0..=1.0).contains(&evaluation.overall_score));
        assert!(
            (evaluation
                .factors
                .iter()
                .map(|factor| factor.weight)
                .sum::<f64>()
                - 1.0)
                .abs()
                < 1e-9
        );
        assert_eq!(evaluation.mode, "semantic");
        assert_eq!(
            evaluation
                .factors
                .iter()
                .map(|factor| factor.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Interest fit",
                "Travel relevance",
                "Freshness",
                "Content evidence"
            ]
        );
        assert!(evaluation.explanation.starts_with("Strongest signal:"));
    }

    #[test]
    fn content_evidence_grades_the_body_it_actually_stored() {
        let mut full = item();
        full.transcript = Some("a".repeat(media::CONTENT_FULL_THRESHOLD));
        full.content_status = "full".into();

        // Same fields populated, same field count -- a consent wall that
        // normalized down to a stub. Counting presence scored these alike.
        let mut thin = full.clone();
        thin.transcript = Some("Accept all cookies".into());
        thin.content_status = "thin".into();

        let mut none = full.clone();
        none.transcript = None;
        none.content_status = "none".into();

        let score = |item: &FeedItem| evidence_score(item).0;
        assert!(
            score(&full) > score(&thin),
            "a full body must outrank a stub: {} vs {}",
            score(&full),
            score(&thin)
        );
        assert!(
            score(&thin) > score(&none),
            "a stub is still more than nothing"
        );

        assert!(
            evidence_score(&thin).1.contains("thin source text"),
            "the rationale must name what it graded: {}",
            evidence_score(&thin).1
        );

        // A legacy row predating classification is graded by the same
        // threshold the classifier uses, not assumed to be a full body.
        let mut legacy = thin.clone();
        legacy.content_status = "unknown".into();
        assert_eq!(score(&legacy), score(&thin));
    }

    fn stored(mode: &str) -> FeedEvaluation {
        FeedEvaluation {
            feed_id: "id".into(),
            overall_score: 0.5,
            explanation: String::new(),
            mode: mode.into(),
            item_revision: "item".into(),
            context_revision: "context".into(),
            evaluator_revision: EVALUATOR_REVISION.into(),
            evaluated_at: String::new(),
            factors: Vec::new(),
        }
    }

    #[test]
    fn a_lexical_row_is_stale_while_the_embedding_role_answers() {
        // Every revision matches. The only thing that differs is whether an
        // embedding role is answering right now -- which is what drains the 525
        // rows written lexical in one pass, with no force flag and no endpoint.
        let lexical = stored("lexical");
        assert!(!is_current(Some(&lexical), "item", "context", true));
        assert!(is_current(Some(&lexical), "item", "context", false));

        let unscored = stored("unscored");
        assert!(!is_current(Some(&unscored), "item", "context", true));

        let semantic = stored("semantic");
        assert!(is_current(Some(&semantic), "item", "context", true));
        assert!(!is_current(Some(&semantic), "moved", "context", true));
    }

    #[test]
    fn relevance_revision_ignores_the_travel_snapshot() {
        // The whole point of the split: a trip that starts or ends must not
        // re-embed 372 items for a factor weighted 0.10.
        assert_eq!(
            relevance_revision(&[], Some("ollama:bge-m3"), None),
            relevance_revision(&[], Some("ollama:bge-m3"), None)
        );
        assert_ne!(
            context_revision(&[], Some("ollama:bge-m3"), None, "trip-one", "none"),
            context_revision(&[], Some("ollama:bge-m3"), None, "trip-two", "none")
        );
        assert_ne!(
            relevance_revision(&[], Some("ollama:bge-m3"), None),
            relevance_revision(&[], Some("omlx:e5"), None)
        );
    }

    #[test]
    fn a_refused_factor_carries_weight_zero_and_the_rest_are_not_rescaled() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.8,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        let refused = evaluate(&item, Some(&matched), "context", &[], true, None);
        let interest = refused
            .factors
            .iter()
            .find(|factor| factor.key == "interest")
            .expect("the refusal is a row, not an absence");
        assert_eq!(interest.weight, 0.0);
        assert!(interest.rationale.contains("never read by a model"));
        assert_eq!(refused.mode, "unscored");
        // The refused share is withheld, not redistributed: 0.25 + 0.20 + 0.10.
        assert!(
            (refused
                .factors
                .iter()
                .map(|factor| factor.weight)
                .sum::<f64>()
                - 0.55)
                .abs()
                < 1e-9,
            "a refusal withholds the refused share instead of handing it to the survivors"
        );
        // The explanation must not report a factor that counts for nothing.
        assert!(!refused.explanation.contains("Interest fit"));
    }

    /// The rule the rescaled version broke, pinned.
    ///
    /// A refused c3 item and a scored item with identical travel, freshness and
    /// evidence: the refusal must never come out on top, because the only
    /// difference between them is evidence the class ladder forbade reading.
    #[test]
    fn a_refused_item_never_outranks_a_scored_item_with_the_same_other_factors() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.0,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        // The weakest possible scored item: a real interest match at 0.0.
        let scored = evaluate(&item, Some(&matched), "context", &[], false, None);
        let refused = evaluate(&item, None, "context", &[], true, None);
        assert!(
            refused.overall_score <= scored.overall_score,
            "refused {} must not beat scored {}",
            refused.overall_score,
            scored.overall_score
        );
    }

    fn feedback_factor(active: bool) -> EvaluationFactor {
        EvaluationFactor {
            key: "feedback".into(),
            label: "Your decisions".into(),
            score: if active { 0.8 } else { 0.0 },
            weight: if active { FEEDBACK_WEIGHT } else { 0.0 },
            rationale: if active {
                "Your past decisions: kind:github (+)".into()
            } else {
                "Not yet learned: 19 of 50 decisions recorded".into()
            },
            context: None,
        }
    }

    #[test]
    fn weights_sum_to_one_in_every_state_a_refusal_does_not_reach() {
        let item = item();
        let matched = RelevanceMatch {
            profile_key: "p".into(),
            profile_label: "Local AI".into(),
            score: 0.8,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "r".into(),
        };
        let sum = |evaluation: &FeedEvaluation| -> f64 {
            evaluation.factors.iter().map(|factor| factor.weight).sum()
        };
        let weight_of = |evaluation: &FeedEvaluation, key: &str| -> f64 {
            evaluation
                .factors
                .iter()
                .find(|factor| factor.key == key)
                .map(|factor| factor.weight)
                .unwrap_or(0.0)
        };

        // Inert: five factors, the fifth at weight 0, the four base weights
        // unchanged.
        let inert = evaluate(
            &item,
            Some(&matched),
            "context",
            &[],
            false,
            Some(feedback_factor(false)),
        );
        assert_eq!(inert.factors.len(), 5);
        assert!((sum(&inert) - 1.0).abs() < 1e-9);
        assert_eq!(weight_of(&inert, "feedback"), 0.0);
        assert!((weight_of(&inert, "interest") - 0.45).abs() < 1e-9);

        // Active: 0.3825 / 0.2125 / 0.17 / 0.085 / 0.15.
        let active = evaluate(
            &item,
            Some(&matched),
            "context",
            &[],
            false,
            Some(feedback_factor(true)),
        );
        assert!((sum(&active) - 1.0).abs() < 1e-9);
        assert!((weight_of(&active, "feedback") - 0.15).abs() < 1e-9);
        assert!((weight_of(&active, "interest") - 0.3825).abs() < 1e-9);
        assert!((weight_of(&active, "travel") - 0.2125).abs() < 1e-9);
        assert!((weight_of(&active, "freshness") - 0.17).abs() < 1e-9);
        assert!((weight_of(&active, "evidence") - 0.085).abs() < 1e-9);

        // Refused, with the learned factor active: the interest factor drops to
        // zero and its 0.45 is withheld rather than handed to the survivors, so
        // the sum is 0.25 + 0.20 + 0.10 + 0.15 and every other factor keeps the
        // share it would have had.
        let refused = evaluate(
            &item,
            Some(&matched),
            "context",
            &[],
            true,
            Some(feedback_factor(true)),
        );
        assert!((sum(&refused) - 0.70).abs() < 1e-9);
        assert_eq!(weight_of(&refused, "interest"), 0.0);
        assert!((weight_of(&refused, "feedback") - 0.15).abs() < 1e-9);
        assert!((weight_of(&refused, "travel") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn the_explanation_ignores_a_zero_weight_factor() {
        let item = item();
        let inert = evaluate(
            &item,
            None,
            "context",
            &[],
            false,
            Some(feedback_factor(false)),
        );
        // The inert factor scores 0.0 -- the lowest of the five -- and must not
        // be reported as the largest deduction.
        assert!(
            !inert.explanation.contains("Your decisions"),
            "{}",
            inert.explanation
        );
    }

    #[test]
    fn context_revision_includes_the_learned_model() {
        assert_ne!(
            context_revision(&[], None, None, "travel", "none"),
            context_revision(&[], None, None, "travel", "feedback-abc123")
        );
    }

    #[test]
    fn civil_date_epoch_is_stable() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
    }
}
