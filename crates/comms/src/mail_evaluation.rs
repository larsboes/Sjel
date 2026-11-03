//! Deterministic, inspectable evaluation for mail proposals.
//!
//! Parallel structure to `evaluation`, not a fifth factor inside it: the feed
//! evaluator reads a `FeedItem` and a trip snapshot, and a triage row has
//! neither. What it shares is everything that makes a stored evaluation
//! trustworthy — the same nine columns, the same currency check, the same tier
//! gate, the same normalized factor table, and the same rule that a factor whose
//! producer has not run carries weight 0 while the rest scale to 1.0 (a class
//! refusal excepted: it is never rescaled).
//!
//! Three factors, and one reserved slot:
//!
//! | key       | weight | source                                              |
//! |-----------|--------|-----------------------------------------------------|
//! | interest  | 0.55   | the strongest stored `{prefix}_triage_relevance` row |
//! | category  | 0.30   | the `rules` stream the sweep assigned                |
//! | age       | 0.15   | the Feed's own freshness curve                       |
//! | urgency   | 0.25   | reserved for the model rung, absent until it runs    |
//!
//! There is **no correspondent factor**, and that is a ruling rather than an
//! omission. `people_registry::is_known_person` compares one whitespace-free
//! token, so it is false for every address; and PRD Q72 rule 2 already settled
//! that the registry is asked per token of subject and snippet, never over the
//! sender field. Reversing a dated ruling to win a quarter of a mail ranking is
//! not a trade this module gets to make.
//!
//! **No rationale here ever quotes a stored mail field.** `intake` redacts
//! subject and snippet for c2 and c3 and deliberately keeps the sender
//! unredacted, so a rationale that quoted any of the three would put a real
//! address, subject or snippet into a surface, a log or a receipt.

use crate::evaluation::{self, EvaluationFactor, EvaluationFactorContext, FeedEvaluation};
use crate::relevance::{InterestProfile, RelevanceMatch};
use crate::store::{ModelVerdict, TriageItem};

/// v1: interest, category and age, with a reserved urgency slot.
pub const MAIL_EVALUATOR_REVISION: &str = "mail-evaluator-v1";

/// May the model rung's urgency rank a mail?
///
/// ONE answer for two readers: this gate, and `TriageModelOut::urgency_validated`
/// on the wire (`server/contracts.rs`). A second copy is how the surface and the
/// score come to disagree about whether a number is trustworthy.
///
/// `false`, and not a config key. A number that reorders the operator's ladder
/// passes the same door the model's category does, and the door here is a
/// measurement: `mail_model_eval::Report::urgency_band_error` over the frozen
/// corpus, against an `acceptance.max_urgency_band_error` the corpus states.
/// Flipping this to `true` is the act of having measured that, so it is a
/// source edit that a reviewer sees, not an overlay value that changes ranking
/// silently.
pub const URGENCY_VALIDATED: bool = false;

/// The share the model rung's urgency takes when it has published one. Taste,
/// like [`FEEDBACK_WEIGHT`], and a named constant for the same reason: it is one
/// edit and one pass to change, and the status endpoint reports it.
pub const URGENCY_WEIGHT: f64 = 0.25;

const INTEREST_WEIGHT: f64 = 0.55;
const CATEGORY_WEIGHT: f64 = 0.30;
const AGE_WEIGHT: f64 = 0.15;

/// What a mail category is worth as a ranking signal.
///
/// Said plainly for the record: on the `aktiv` band — the only band Home ranks —
/// this is a constant 1.0, so ordering there is carried by interest, age and,
/// when it exists, urgency.
fn category_score(stream: &str) -> (f64, &'static str) {
    match stream {
        "aktiv" => (1.0, "Active correspondence"),
        "issue" => (1.0, "Something to act on"),
        "steuern" => (0.6, "Tax record"),
        "belege" => (0.6, "Receipt"),
        "sonstiges" => (0.4, "Uncategorised"),
        "feed" => (0.2, "Newsletter or feed"),
        "werbung" => (0.0, "Advertising"),
        _ => (0.4, "Uncategorised"),
    }
}

/// What the item itself says, hashed. Deliberately NOT the status: a human
/// decision about a mail is not a change to the mail, and folding it in would
/// restale an evaluation every time somebody archived something.
pub fn item_revision(item: &TriageItem) -> String {
    evaluation::revision_hash(&[
        item.from_addr.as_deref().unwrap_or_default(),
        item.subject.as_deref().unwrap_or_default(),
        item.snippet.as_deref().unwrap_or_default(),
        &item.stream,
        &item.data_class,
    ])
}

/// The ranking inputs. No travel term and no feedback term: mail gets no learned
/// factor tonight, and the interaction ledger is feed-only.
pub fn context_revision(
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
    evaluation::revision_hash(&revisions.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Whether a stored mail evaluation may be left alone. Same four conditions the
/// feed uses, for the same reasons.
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
            && evaluation.evaluator_revision == MAIL_EVALUATOR_REVISION
    })
}

/// Whether a stored class refusal may be left alone. Same rule and same reason
/// as [`evaluation::refusal_is_current`], against this evaluator's revision.
pub fn refusal_is_current(
    stored: Option<&FeedEvaluation>,
    item_revision: &str,
    context_revision: &str,
) -> bool {
    stored.is_some_and(|evaluation| {
        evaluation.mode == "unscored"
            && evaluation.item_revision == item_revision
            && evaluation.context_revision == context_revision
            && evaluation.evaluator_revision == MAIL_EVALUATOR_REVISION
    })
}

/// What the model rung published about one mail, when it has run.
///
/// The rung stores urgency in a column or table it owns; this reads it as the
/// `urgency` factor's input, so the number moves `score_bp` THROUGH the
/// evaluator rather than competing with it on the wire. A reader who asks why a
/// mail is at the top gets four bars, one of which is the model's.
#[derive(Debug, Clone, PartialEq)]
pub struct UrgencySignal {
    /// 0..=1.
    pub score: f64,
    /// The rung's own words. Never a quotation of a stored mail field.
    pub rationale: String,
}

/// The urgency factor's input, read from one stored verdict.
///
/// `None` -- weight 0, and the other three factors scale to 1.0 -- for each of
/// four different absences, deliberately collapsed into one: urgency is not
/// validated yet, the rung stored no verdict for this mail, the verdict carries
/// no `urgency_bp`, or it carries one with no sentence to explain it. The last
/// is the one worth naming: a bar with no rationale under it is a number the
/// reader cannot check, and this evaluator's whole claim is that every weighted
/// factor says why.
///
/// `validated` is a parameter rather than a read of [`URGENCY_VALIDATED`] so
/// that both sides of the gate are reachable from a test. Callers pass the
/// constant.
///
/// The rationale is the rung's own sentence, which `mail_model` already
/// redacted against the higher of the mail's class and the class its model
/// stream implies. This function never reads a stored mail field.
pub fn urgency_from_verdict(
    verdict: Option<&ModelVerdict>,
    validated: bool,
) -> Option<UrgencySignal> {
    if !validated {
        return None;
    }
    let verdict = verdict?;
    let urgency_bp = verdict.urgency_bp?;
    let rationale = verdict
        .urgency_rationale
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(UrgencySignal {
        // Basis points in the store, 0..=1 in the evaluator. The clamp is not
        // decoration: `urgency_bp` is an integer column, so a row written before
        // `mail_model::clamp_bp` existed can hold anything.
        score: (urgency_bp as f64 / 10_000.0).clamp(0.0, 1.0),
        rationale: rationale.to_string(),
    })
}

/// Evaluate one mail.
///
/// `refused_class` is the c3 state: the item reached no model and no lexical
/// scorer, so the interest factor carries weight 0 with a rationale that says
/// so, and the remaining factors keep their stated shares. The weights of a
/// refused mail sum to 0.45, not to 1.0, and that is the point -- a refusal is
/// missing evidence, so the mail is ranked on what is left rather than
/// promoted above every mail that was actually read. See
/// `evaluation::scale_weights` for what the rescaled version measured.
pub fn evaluate(
    item: &TriageItem,
    strongest_match: Option<&RelevanceMatch>,
    urgency: Option<&UrgencySignal>,
    context_revision: &str,
    refused_class: bool,
) -> FeedEvaluation {
    let interest_score = strongest_match
        .map(|matched| matched.score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let interest_rationale = if refused_class {
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

    let (category_score, category_label) = category_score(&item.stream);
    // The day the mail arrived, from the stored timestamp's date part only. The
    // time of day is not a ranking input and there is no reason to carry it.
    let day = item
        .internal_date_text
        .as_deref()
        .unwrap_or_default()
        .get(..10)
        .unwrap_or_default();
    let age = evaluation::age_days(day);
    let age_score = evaluation::freshness_score(age);
    let age_rationale = match age {
        Some(0) => "Arrived today".to_string(),
        Some(1) => "Arrived yesterday".to_string(),
        Some(days) => format!("Arrived {days} days ago"),
        None => "Arrival date cannot be evaluated".to_string(),
    };

    let mut factors = vec![
        EvaluationFactor {
            key: "interest".into(),
            label: "Interest fit".into(),
            score: if refused_class { 0.0 } else { interest_score },
            weight: if refused_class { 0.0 } else { INTEREST_WEIGHT },
            rationale: interest_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "category".into(),
            label: "Category".into(),
            score: category_score,
            weight: CATEGORY_WEIGHT,
            // The category the rules stream assigned, named. Never the subject
            // or the sender that produced it.
            rationale: format!("{category_label} ({})", item.stream),
            context: None,
        },
        EvaluationFactor {
            key: "age".into(),
            label: "Age".into(),
            score: age_score,
            weight: AGE_WEIGHT,
            rationale: age_rationale,
            context: None,
        },
        EvaluationFactor {
            key: "urgency".into(),
            label: "Urgency".into(),
            score: urgency
                .map(|signal| signal.score.clamp(0.0, 1.0))
                .unwrap_or(0.0),
            // Absent means weight 0, which is what makes the slot reservable:
            // the other three scale to 1.0 until the rung publishes one.
            weight: if urgency.is_some() {
                URGENCY_WEIGHT
            } else {
                0.0
            },
            rationale: urgency
                .map(|signal| signal.rationale.clone())
                .unwrap_or_else(|| "No model rung has judged this yet".into()),
            context: urgency.map(|_| EvaluationFactorContext {
                kind: "urgency".into(),
                id: item.id.clone(),
                label: "Model rung".into(),
                date_start: None,
                date_end: None,
                matched_terms: Vec::new(),
            }),
        },
    ];
    // `urgency` is a reserved share: it keeps its stated 0.25 and the other
    // three scale into the remaining 0.75, rather than every factor taking an
    // equal quarter of 1.25.
    //
    // A refusal is never rescaled. Redistributing the refused 0.55 into
    // `category` and `age` -- 1.0 and ~0.6 for an `aktiv` thread -- scored every
    // c3 mail above every mail that was read; see `evaluation::scale_weights`
    // for the measurement. A refused mail keeps 0.30/0.15 and caps at 0.45.
    if !refused_class {
        evaluation::scale_weights(&mut factors, &["urgency"]);
    }

    let overall_score = factors
        .iter()
        .map(|factor| factor.score * factor.weight)
        .sum::<f64>()
        .clamp(0.0, 1.0);
    let counted = factors
        .iter()
        .filter(|factor| factor.weight > 0.0)
        .collect::<Vec<_>>();
    let strongest = counted
        .iter()
        .max_by(|left, right| left.score.total_cmp(&right.score))
        .expect("the mail evaluator always has at least one weighted factor");
    let weakest = counted
        .iter()
        .min_by(|left, right| left.score.total_cmp(&right.score))
        .expect("the mail evaluator always has at least one weighted factor");
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
            Some(matched) if !refused_class => matched.mode.clone(),
            _ => "unscored".into(),
        },
        item_revision: item_revision(item),
        context_revision: context_revision.to_string(),
        evaluator_revision: MAIL_EVALUATOR_REVISION.into(),
        evaluated_at: String::new(),
        factors,
    }
}

/// The overall score in basis points, 0..=10000.
///
/// ONE writer, one meaning: `TriageOut.score_bp` is this and nothing else. The
/// unit is the one `places_person_places.confidence_bp` already uses (PRD Q73),
/// so a reader comparing two ranked surfaces is comparing the same kind of
/// number.
pub fn score_bp(evaluation: &FeedEvaluation) -> i32 {
    overall_bp(evaluation.overall_score)
}

/// The conversion itself, for the one caller that holds the score without the
/// evaluation around it.
///
/// `TriageOut::from_store` reads `(overall, evaluated_at)` out of a grouped
/// query rather than a `FeedEvaluation`, and used to restate the clamp and the
/// rounding inline -- two copies of one rule, free to drift, under a doc comment
/// naming this module as the single writer.
pub fn overall_bp(overall: f64) -> i32 {
    (overall.clamp(0.0, 1.0) * 10_000.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(stream: &str, class: &str) -> TriageItem {
        let mut item = crate::store::db_tests::mk_triage("thread:eval", stream);
        item.data_class = class.into();
        item.internal_date_text = Some("2026-09-04 08:00:00+00:00".into());
        item
    }

    fn matched(score: f64) -> RelevanceMatch {
        RelevanceMatch {
            profile_key: "lens".into(),
            profile_label: "Systems".into(),
            score,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "revision".into(),
        }
    }

    fn weight_of(evaluation: &FeedEvaluation, key: &str) -> f64 {
        evaluation
            .factors
            .iter()
            .find(|factor| factor.key == key)
            .map(|factor| factor.weight)
            .unwrap_or(0.0)
    }

    fn weights_sum(evaluation: &FeedEvaluation) -> f64 {
        evaluation.factors.iter().map(|factor| factor.weight).sum()
    }

    /// A stored verdict, with only the fields this gate reads set.
    fn verdict(urgency_bp: Option<i64>, urgency_rationale: Option<&str>) -> ModelVerdict {
        ModelVerdict {
            triage_id: "thread:eval".into(),
            mode: "shadow".into(),
            state: "generated".into(),
            rule_decided_by: "fallback".into(),
            rule_stream: "aktiv".into(),
            model_stream: Some("issue".into()),
            confidence_bp: Some(8_000),
            urgency_bp,
            rationale: Some("the model's sentence".into()),
            urgency_rationale: urgency_rationale.map(str::to_string),
            redactions: 0,
            data_class: "c1".into(),
            redaction_class: "c1".into(),
            producer: "synthetic-producer".into(),
            item_revision: "revision-1".into(),
            prompt_revision: "prompt-1".into(),
            classification_version: "mail-model-v1".into(),
            attempts: 0,
            last_error: None,
            next_attempt: None,
            held_reason: None,
            applied_at: None,
        }
    }

    /// The gate is the whole point of the reserved slot: an unvalidated urgency
    /// is not a small urgency, it is no urgency at all.
    #[test]
    fn an_unvalidated_urgency_never_reaches_the_evaluator() {
        let stored = verdict(Some(7_500), Some("A date is named."));
        assert_eq!(urgency_from_verdict(Some(&stored), false), None);

        let item = mail("aktiv", "c1");
        let ungated = urgency_from_verdict(Some(&stored), true).expect("the validated signal");
        let evaluation = evaluate(&item, Some(&matched(0.7)), Some(&ungated), "context", false);
        assert!((weight_of(&evaluation, "urgency") - URGENCY_WEIGHT).abs() < 1e-9);
        assert!((weights_sum(&evaluation) - 1.0).abs() < 1e-9);
    }

    /// Basis points in the store, 0..=1 in the evaluator, and the rung's own
    /// sentence carried through as the factor's rationale.
    #[test]
    fn a_validated_verdict_becomes_a_bounded_signal_with_its_reason() {
        let signal =
            urgency_from_verdict(Some(&verdict(Some(7_500), Some("A date is named."))), true)
                .expect("the signal");
        assert!((signal.score - 0.75).abs() < 1e-9);
        assert_eq!(signal.rationale, "A date is named.");

        // An out-of-range column is clamped, not trusted: `urgency_bp` is a
        // plain integer and a row predating `mail_model::clamp_bp` can hold
        // anything.
        let high = urgency_from_verdict(Some(&verdict(Some(99_999), Some("Overdue."))), true)
            .expect("the clamped signal");
        assert!((high.score - 1.0).abs() < 1e-9);
    }

    /// Four different absences, one answer. The last is the one worth a test: a
    /// weighted bar with no sentence under it is a number nobody can check.
    #[test]
    fn an_urgency_with_no_reason_is_not_carried() {
        assert_eq!(urgency_from_verdict(None, true), None);
        assert_eq!(
            urgency_from_verdict(Some(&verdict(None, Some("x"))), true),
            None
        );
        assert_eq!(
            urgency_from_verdict(Some(&verdict(Some(6_000), None)), true),
            None
        );
        assert_eq!(
            urgency_from_verdict(Some(&verdict(Some(6_000), Some("   "))), true),
            None
        );

        // And an absent signal leaves the slot reserved rather than scoring 0.
        let item = mail("aktiv", "c1");
        let evaluation = evaluate(&item, Some(&matched(0.7)), None, "context", false);
        assert_eq!(weight_of(&evaluation, "urgency"), 0.0);
    }

    #[test]
    fn mail_weights_sum_to_one_with_and_without_urgency() {
        let item = mail("aktiv", "c1");
        let without = evaluate(&item, Some(&matched(0.7)), None, "context", false);
        assert!((weights_sum(&without) - 1.0).abs() < 1e-9);
        assert!((weight_of(&without, "interest") - INTEREST_WEIGHT).abs() < 1e-9);
        assert_eq!(weight_of(&without, "urgency"), 0.0);

        let urgent = UrgencySignal {
            score: 0.9,
            rationale: "The rung read a deadline in the body".into(),
        };
        let with = evaluate(&item, Some(&matched(0.7)), Some(&urgent), "context", false);
        assert!((weights_sum(&with) - 1.0).abs() < 1e-9);
        // 0.4125 / 0.225 / 0.1125 / 0.25
        assert!(
            (weight_of(&with, "interest") - INTEREST_WEIGHT * (1.0 - URGENCY_WEIGHT)).abs() < 1e-9
        );
        assert!((weight_of(&with, "urgency") - URGENCY_WEIGHT).abs() < 1e-9);
        assert!(with.overall_score > without.overall_score);

        for evaluation in [&without, &with] {
            for factor in &evaluation.factors {
                assert!(
                    (0.0..=1.0).contains(&factor.score),
                    "the CHECK constraint refuses anything else: {}",
                    factor.score
                );
                assert!((0.0..=1.0).contains(&factor.weight));
            }
            assert!((0.0..=1.0).contains(&evaluation.overall_score));
        }

        // A refusal costs the interest factor its weight and withholds the
        // share rather than redistributing it: 0.30 + 0.15, never 1.0.
        let refused = evaluate(&mail("aktiv", "c3"), None, None, "context", true);
        assert_eq!(weight_of(&refused, "interest"), 0.0);
        assert!((weights_sum(&refused) - 0.45).abs() < 1e-9);
        assert!((weight_of(&refused, "category") - CATEGORY_WEIGHT).abs() < 1e-9);
        assert!((weight_of(&refused, "age") - AGE_WEIGHT).abs() < 1e-9);
        assert_eq!(refused.mode, "unscored");
        assert!(!refused.explanation.contains("Interest fit"));
    }

    /// The defect this rule exists to prevent, measured on a copy of the live
    /// database on 2026-09-06: with the refused share rescaled into `category`
    /// and `age`, all 11 c3 threads scored 0.863..0.990 while the best mail
    /// anybody had actually read reached 0.702, so Home's mail band was led by
    /// the eleven most sensitive threads on the machine.
    #[test]
    fn a_refused_mail_never_outranks_a_mail_that_was_read() {
        let item = mail("aktiv", "c1");
        // The weakest scored mail there is: an `aktiv` thread that arrived
        // today and matched a lens at 0.0.
        let scored = evaluate(&item, Some(&matched(0.0)), None, "context", false);
        let refused = evaluate(&mail("aktiv", "c3"), None, None, "context", true);
        assert!(
            refused.overall_score <= scored.overall_score,
            "refused {} must not beat scored {}",
            refused.overall_score,
            scored.overall_score
        );
        assert!(refused.overall_score <= 0.45);
    }

    #[test]
    fn the_rationale_never_quotes_a_stored_mail_field() {
        let mut item = mail("aktiv", "c2");
        // What `intake` actually stores for an escalated row.
        item.subject = Some("[redacted]".into());
        item.snippet = Some("[redacted]".into());
        item.from_addr = Some("someone@example.invalid".into());
        let evaluation = evaluate(&item, Some(&matched(0.4)), None, "context", false);
        for factor in &evaluation.factors {
            for forbidden in ["redacted", "example.invalid", "someone@"] {
                assert!(
                    !factor.rationale.contains(forbidden),
                    "a rationale must not quote a stored mail field: {}",
                    factor.rationale
                );
            }
        }
        assert!(!evaluation.explanation.contains("example.invalid"));
    }

    #[test]
    fn the_category_is_a_constant_on_the_band_home_ranks() {
        // Stated in the design and pinned here: ordering inside `aktiv` cannot
        // come from the category, so it has to come from the other factors.
        let active = evaluate(&mail("aktiv", "c1"), Some(&matched(0.9)), None, "c", false);
        let quiet = evaluate(&mail("aktiv", "c1"), Some(&matched(0.1)), None, "c", false);
        assert!(active.overall_score > quiet.overall_score);
        let advertising = evaluate(
            &mail("werbung", "c1"),
            Some(&matched(0.9)),
            None,
            "c",
            false,
        );
        assert!(advertising.overall_score < active.overall_score);
    }

    #[test]
    fn score_bp_is_basis_points() {
        let mut evaluation = evaluate(&mail("aktiv", "c1"), Some(&matched(0.5)), None, "c", false);
        evaluation.overall_score = 0.6234;
        assert_eq!(score_bp(&evaluation), 6234);
        evaluation.overall_score = 1.0;
        assert_eq!(score_bp(&evaluation), 10_000);
        evaluation.overall_score = 0.0;
        assert_eq!(score_bp(&evaluation), 0);
    }

    #[test]
    fn a_status_change_does_not_restale_an_evaluation() {
        let mut item = mail("aktiv", "c1");
        let before = item_revision(&item);
        item.status = "archived".into();
        assert_eq!(
            before,
            item_revision(&item),
            "a human decision is not content"
        );
        item.subject = Some("something else".into());
        assert_ne!(before, item_revision(&item));
    }

    #[test]
    fn a_lexical_mail_row_is_stale_while_the_embedding_role_answers() {
        let stored = evaluate(&mail("aktiv", "c1"), None, None, "context", false);
        let item_revision = stored.item_revision.clone();
        assert!(!is_current(Some(&stored), &item_revision, "context", true));
        assert!(is_current(Some(&stored), &item_revision, "context", false));
    }

    /// Mail gets no learned factor tonight: the interaction ledger is feed-only.
    /// Pinned, because the two evaluators otherwise look interchangeable.
    #[test]
    fn mail_has_no_learned_factor() {
        let evaluation = evaluate(&mail("aktiv", "c1"), Some(&matched(0.5)), None, "c", false);
        assert!(evaluation
            .factors
            .iter()
            .all(|factor| factor.key != "feedback"));
    }
}
