//! The fifth evaluator factor: what the operator's own decisions imply.
//!
//! L2-regularised logistic regression over an explicit feature vector, trained
//! on keep-versus-dismiss from `comms_feed_interactions`, with time decay. It is
//! deliberately small, deliberately explicit and deliberately inert until it has
//! evidence worth acting on.
//!
//! **No slot carries text.** The author is one of sixteen hash buckets; kind,
//! source and lens are categorical ids; the lens scores are numbers. A feature
//! vector cannot reconstruct a title, and a stored model cannot leak one.
//!
//! **The class ladder governs training.** An item that fails
//! `content_item::local_prompt_allowed` contributes no label and no feature
//! vector. See `store::feedback::training_labels`.
//!
//! **The gate is the defence, and it is not decoration.** Measured on the live
//! store: 17 keeper, 2 dismissed, 353 new — and zero of 185 stored arXiv items
//! has ever been kept. A model fitted on 19 labels over ~50 features learns
//! "arXiv is never kept" and buries the largest source in the feed. So the
//! factor renders at weight 0 with the rationale "not yet learned" until three
//! measured conditions hold, and while it is inert its revision is the literal
//! `none` — so accumulating labels does not restale 372 cached evaluations for a
//! factor that counts for nothing.
//!
//! **A full refit, never an incremental update.** ≤372 rows over ~50 features is
//! microseconds, and incremental weights depend on arrival order, which cannot
//! be rederived from the ledger. That would make the revision unusable as a
//! cache key — the property the whole snapshot mechanism rests on.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::evaluation::EvaluationFactor;
use crate::relevance::{InterestProfile, RelevanceMatch};
use crate::store::{FeedItem, FeedbackLabel};

/// Bump this when the feature vector's shape or meaning changes. A stored model
/// whose feature revision differs is stale by definition and retrains.
pub const FEATURE_REVISION: &str = "feedback-features-v1";

/// The revision an inert model reports. A literal, not a hash: it must not move
/// while the factor counts for nothing, or every new label would restale every
/// cached evaluation.
pub const INERT_REVISION: &str = "none";

// The gate. Taste rather than measurement, which is exactly why each one is a
// named constant reported by `GET /feed/evaluation/status`: changing any of them
// is one edit and one retrain, and a reader can check the claim against the
// number.
/// How many decisions before a learned factor may count at all.
pub const MIN_LABELS: usize = 50;
/// How many of the SMALLER class. 48 keeps and 2 dismissals is not evidence
/// about dismissal; it is evidence about one afternoon.
pub const MIN_MINORITY: usize = 10;
/// Held-out AUC on a time-ordered split. 0.5 is a coin.
pub const MIN_AUC: f64 = 0.65;

/// Time decay: a decision made this long ago counts half as much as one made
/// today.
const HALF_LIFE_DAYS: f64 = 90.0;
/// The floor a decayed sample cannot fall below. An August keep still says
/// something, and a decision seeded from `status` with no date sits here.
const DECAY_FLOOR: f64 = 0.1;

const GRADIENT_STEPS: usize = 500;
const LEARNING_RATE: f64 = 0.1;
const L2_LAMBDA: f64 = 1.0;
/// The newest 30% of decisions, by time, are the holdout.
const HOLDOUT_SHARE: f64 = 0.3;
const AUTHOR_BUCKETS: usize = 16;

/// The nine kinds `{prefix}_feed_items.kind` admits. Fixed, so a feed with no
/// podcasts yet still has a podcast slot and the vocabulary does not move when
/// one arrives.
const KINDS: [&str; 9] = [
    "youtube",
    "instagram",
    "podcast",
    "article",
    "mail",
    "github",
    "arxiv",
    "reddit",
    "huggingface",
];

const CONTENT_STATUSES: [&str; 4] = ["full", "thin", "none", "unknown"];
const FRESHNESS_BUCKETS: [&str; 5] = ["0-1d", "2-7d", "8-30d", "31-90d", "90d+"];
const HOUR_BUCKETS: [&str; 4] = ["night", "morning", "afternoon", "evening"];

/// The vocabulary a vector is built against.
///
/// Stored with the model, not only the weights: a new TELOS lens or a new feed
/// source changes the vocabulary, and a model whose stored names differ from the
/// ones computed now is stale by definition. That is an invalidation rule rather
/// than a guess about which piece of configuration moved.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureSpace {
    /// Declared feed source ids, sorted. One extra slot for "some other source".
    pub sources: Vec<String>,
    /// TELOS lens labels, sorted. One extra slot for "no lens matched".
    pub lenses: Vec<String>,
}

impl FeatureSpace {
    pub fn new(mut sources: Vec<String>, profiles: &[InterestProfile]) -> Self {
        sources.sort();
        sources.dedup();
        let mut lenses = profiles
            .iter()
            .map(|profile| profile.label.clone())
            .collect::<Vec<_>>();
        lenses.sort();
        lenses.dedup();
        Self { sources, lenses }
    }

    /// Every slot's name, in vector order. The names ARE the contract: a stored
    /// model is only reusable while these match.
    pub fn names(&self) -> Vec<String> {
        let mut names = vec!["bias".to_string()];
        names.extend(KINDS.iter().map(|kind| format!("kind:{kind}")));
        names.extend(self.sources.iter().map(|id| format!("source:{id}")));
        names.push("source:other".into());
        names.extend(self.lenses.iter().map(|lens| format!("lens:{lens}")));
        names.push("lens:none".into());
        names.push("lens_top_score".into());
        names.push("lens_mean_score".into());
        names.extend(
            CONTENT_STATUSES
                .iter()
                .map(|status| format!("content:{status}")),
        );
        names.extend(
            FRESHNESS_BUCKETS
                .iter()
                .map(|bucket| format!("age:{bucket}")),
        );
        names.extend((0..AUTHOR_BUCKETS).map(|bucket| format!("author:{bucket}")));
        names.extend(HOUR_BUCKETS.iter().map(|hour| format!("hour:{hour}")));
        names
    }

    pub fn len(&self) -> usize {
        self.names().len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

/// One item's inputs, as the vector builder needs them.
pub struct FeatureInput<'a> {
    pub item: &'a FeedItem,
    pub matches: &'a [RelevanceMatch],
    /// The collector source this item arrived from, when it has one.
    pub source_id: Option<&'a str>,
    /// Optional epoch seconds to evaluate freshness against. When None, current wall-clock is used.
    pub now: Option<i64>,
}

fn one_hot(vector: &mut [f64], base: usize, index: Option<usize>, fallback: usize) {
    match index {
        Some(offset) => vector[base + offset] = 1.0,
        None => vector[base + fallback] = 1.0,
    }
}

fn freshness_bucket(day: &str, now: Option<i64>) -> usize {
    let days = match now {
        Some(secs) => crate::evaluation::age_days_at(day, secs),
        None => crate::evaluation::age_days(day),
    };
    match days {
        Some(days) if days <= 1 => 0,
        Some(days) if days <= 7 => 1,
        Some(days) if days <= 30 => 2,
        Some(days) if days <= 90 => 3,
        _ => 4,
    }
}

/// The author, as one of sixteen buckets. Never the name.
fn author_bucket(author: Option<&str>) -> usize {
    let Some(author) = author.map(str::trim).filter(|value| !value.is_empty()) else {
        return 0;
    };
    let mut hasher = Sha256::new();
    hasher.update(author.to_lowercase().as_bytes());
    let digest = hasher.finalize();
    (digest[0] as usize) % AUTHOR_BUCKETS
}

/// The hour the item was captured, as one of four parts of the day.
fn hour_bucket(created_at: &str) -> usize {
    let hour = created_at
        .get(11..13)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(12);
    match hour {
        0..=5 => 0,
        6..=11 => 1,
        12..=17 => 2,
        _ => 3,
    }
}

/// Build one feature vector. Pure, and the only place a slot's meaning is
/// decided.
pub fn features(space: &FeatureSpace, input: &FeatureInput) -> Vec<f64> {
    let mut vector = vec![0.0; space.len()];
    let mut cursor = 0usize;
    vector[cursor] = 1.0; // bias
    cursor += 1;

    one_hot(
        &mut vector,
        cursor,
        KINDS.iter().position(|kind| *kind == input.item.kind),
        // An unknown kind lands on `article`, the generic one, rather than
        // widening the vocabulary at score time.
        3,
    );
    cursor += KINDS.len();

    let source_slots = space.sources.len() + 1;
    one_hot(
        &mut vector,
        cursor,
        input
            .source_id
            .and_then(|id| space.sources.iter().position(|known| known == id)),
        space.sources.len(),
    );
    cursor += source_slots;

    let lens_slots = space.lenses.len() + 1;
    let top = input.matches.first();
    one_hot(
        &mut vector,
        cursor,
        top.and_then(|matched| {
            space
                .lenses
                .iter()
                .position(|label| *label == matched.profile_label)
        }),
        space.lenses.len(),
    );
    cursor += lens_slots;

    vector[cursor] = top
        .map(|matched| matched.score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    cursor += 1;
    vector[cursor] = if input.matches.is_empty() {
        0.0
    } else {
        input
            .matches
            .iter()
            .map(|matched| matched.score.clamp(0.0, 1.0))
            .sum::<f64>()
            / input.matches.len() as f64
    };
    cursor += 1;

    one_hot(
        &mut vector,
        cursor,
        CONTENT_STATUSES
            .iter()
            .position(|status| *status == input.item.content_status),
        3,
    );
    cursor += CONTENT_STATUSES.len();

    vector[cursor + freshness_bucket(&input.item.day, input.now)] = 1.0;
    cursor += FRESHNESS_BUCKETS.len();

    vector[cursor + author_bucket(input.item.author.as_deref())] = 1.0;
    cursor += AUTHOR_BUCKETS;

    vector[cursor + hour_bucket(&input.item.created_at)] = 1.0;
    vector
}

/// What one training row is: a vector, a label, and how much it counts.
#[derive(Debug, Clone)]
pub struct TrainingRow {
    pub features: Vec<f64>,
    pub kept: bool,
    /// Epoch seconds of the decision, for the time-ordered split and the decay.
    pub decided_at: i64,
    /// `true` when the date came from `status` rather than the ledger, so the
    /// trainer weights it at the decay floor: the decision is real, its date is
    /// not.
    pub undated: bool,
}

impl TrainingRow {
    pub fn from_label(label: &FeedbackLabel, features: Vec<f64>) -> Self {
        Self {
            features,
            kept: label.kept,
            decided_at: label.decided_at,
            undated: label.seeded_from_status,
        }
    }
}

/// The stored model, exactly as the snapshot payload carries it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackModel {
    pub feature_revision: String,
    pub feature_names: Vec<String>,
    pub weights: Vec<f64>,
    pub samples: SampleCounts,
    pub holdout: Holdout,
    pub active: bool,
    pub gate_reason: String,
    pub trained_at: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct SampleCounts {
    pub kept: usize,
    pub dismissed: usize,
    pub total: usize,
    pub seeded_from_status: usize,
    pub skipped_class: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Holdout {
    pub n: usize,
    pub auc: f64,
}

impl FeedbackModel {
    /// `feedback-<sha256(payload)[..16]>` when active, the literal `none` when
    /// not. The revision is a cache key: an inert model must not move it.
    pub fn revision(&self) -> String {
        if !self.active {
            return INERT_REVISION.to_string();
        }
        let payload = serde_json::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        format!(
            "feedback-{:.16}",
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }

    /// The probability this item would be kept, in 0..=1.
    pub fn predict(&self, features: &[f64]) -> f64 {
        sigmoid(dot(&self.weights, features))
    }

    /// The two strongest signed contributions, named. This is what Principle 5
    /// asks of a ranked surface: the factor may re-rank, and it must be able to
    /// say which of its own signals did it.
    pub fn explain(&self, features: &[f64]) -> String {
        let mut contributions = self
            .weights
            .iter()
            .zip(features)
            .enumerate()
            // The bias explains nothing about this item.
            .filter(|(index, _)| *index != 0)
            .map(|(index, (weight, value))| (index, weight * value))
            .filter(|(_, contribution)| contribution.abs() > 1e-6)
            .collect::<Vec<_>>();
        contributions.sort_by(|left, right| {
            right
                .1
                .abs()
                .partial_cmp(&left.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let named = contributions
            .iter()
            .take(2)
            .map(|(index, contribution)| {
                let name = self
                    .feature_names
                    .get(*index)
                    .map(String::as_str)
                    .unwrap_or("unknown");
                let direction = if *contribution >= 0.0 { "+" } else { "−" };
                format!("{name} ({direction})")
            })
            .collect::<Vec<_>>();
        if named.is_empty() {
            "No signal in your past decisions points either way".into()
        } else {
            format!("Your past decisions: {}", named.join(", "))
        }
    }

    /// The factor as the evaluator renders it. Weight 0 while inert, and the
    /// rationale says how far off the gate is rather than pretending to a score.
    pub fn factor(&self, features: Option<&[f64]>, weight: f64) -> EvaluationFactor {
        if !self.active {
            return EvaluationFactor {
                key: "feedback".into(),
                label: "Your decisions".into(),
                score: 0.0,
                weight: 0.0,
                rationale: format!(
                    "Not yet learned: {} of {MIN_LABELS} decisions recorded",
                    self.samples.total
                ),
                context: None,
            };
        }
        let features = features.unwrap_or(&[]);
        EvaluationFactor {
            key: "feedback".into(),
            label: "Your decisions".into(),
            score: self.predict(features).clamp(0.0, 1.0),
            weight,
            rationale: self.explain(features),
            context: None,
        }
    }
}

fn sigmoid(value: f64) -> f64 {
    1.0 / (1.0 + (-value).exp())
}

fn dot(weights: &[f64], features: &[f64]) -> f64 {
    weights
        .iter()
        .zip(features)
        .map(|(weight, value)| weight * value)
        .sum()
}

/// How much a decision of a given age counts, relative to one made today.
fn decay(decided_at: i64, now: i64, undated: bool) -> f64 {
    if undated {
        // The decision is real; its date is not. Counting it at full weight
        // would date every pre-ledger keep to the moment the item was captured,
        // which for 19 of them is weeks off.
        return DECAY_FLOOR;
    }
    let age_days = ((now - decided_at).max(0) as f64) / 86_400.0;
    (0.5f64.powf(age_days / HALF_LIFE_DAYS)).max(DECAY_FLOOR)
}

/// Exact rank-based AUC, ties counted as half.
///
/// Written out rather than approximated by bucketing: the holdout is at most a
/// hundred rows, and an AUC that decides whether a factor may count at all had
/// better be the real one.
pub fn auc(scores: &[(f64, bool)]) -> f64 {
    let positives = scores.iter().filter(|(_, kept)| *kept).count();
    let negatives = scores.len() - positives;
    if positives == 0 || negatives == 0 {
        return 0.5;
    }
    let mut concordant = 0.0;
    for (score, kept) in scores.iter().filter(|(_, kept)| *kept) {
        for (other, _) in scores.iter().filter(|(_, kept)| !*kept) {
            if score > other {
                concordant += 1.0;
            } else if (score - other).abs() < f64::EPSILON {
                concordant += 0.5;
            }
        }
        let _ = kept;
    }
    concordant / (positives as f64 * negatives as f64)
}

/// Fit the weights. Deterministic: zero-initialised, full-batch, fixed step
/// count — the same rows in the same order give byte-identical weights, which is
/// what lets the revision be a cache key.
fn fit(rows: &[TrainingRow], width: usize, now: i64) -> Vec<f64> {
    let mut weights = vec![0.0; width];
    if rows.is_empty() {
        return weights;
    }
    let sample_weights = rows
        .iter()
        .map(|row| decay(row.decided_at, now, row.undated))
        .collect::<Vec<_>>();
    let total_weight = sample_weights.iter().sum::<f64>().max(f64::EPSILON);
    for _ in 0..GRADIENT_STEPS {
        let mut gradient = vec![0.0; width];
        for (row, weight) in rows.iter().zip(&sample_weights) {
            let error = sigmoid(dot(&weights, &row.features)) - f64::from(row.kept);
            for (slot, value) in gradient.iter_mut().zip(&row.features) {
                *slot += weight * error * value;
            }
        }
        for (index, slot) in gradient.iter_mut().enumerate() {
            *slot /= total_weight;
            // L2, and not on the bias: penalising the intercept would pull the
            // base rate towards a half nobody observed.
            if index != 0 {
                *slot += L2_LAMBDA * weights[index] / rows.len() as f64;
            }
        }
        for (weight, step) in weights.iter_mut().zip(&gradient) {
            *weight -= LEARNING_RATE * step;
        }
    }
    weights
}

/// Train, measure, and decide whether the result may count.
///
/// `rows` must be in time order — `store::feedback::training_labels` sorts them,
/// and the split relies on it: a random split would let a decision made after
/// the holdout period leak into the fit and report an AUC nobody will see again.
pub fn train(
    space: &FeatureSpace,
    rows: &[TrainingRow],
    skipped_class: usize,
    now: i64,
    trained_at: &str,
) -> FeedbackModel {
    let kept = rows.iter().filter(|row| row.kept).count();
    let dismissed = rows.len() - kept;
    let samples = SampleCounts {
        kept,
        dismissed,
        total: rows.len(),
        seeded_from_status: rows.iter().filter(|row| row.undated).count(),
        skipped_class,
    };

    let holdout_size = ((rows.len() as f64) * HOLDOUT_SHARE).floor() as usize;
    let split = rows.len().saturating_sub(holdout_size);
    let (fit_rows, holdout_rows) = rows.split_at(split.min(rows.len()));
    let weights = fit(fit_rows, space.len(), now);
    let scored = holdout_rows
        .iter()
        .map(|row| (sigmoid(dot(&weights, &row.features)), row.kept))
        .collect::<Vec<_>>();
    let holdout = Holdout {
        n: holdout_rows.len(),
        auc: auc(&scored),
    };

    let minority = kept.min(dismissed);
    let gate_reason = if samples.total < MIN_LABELS {
        format!("{} of {MIN_LABELS} decisions recorded", samples.total)
    } else if minority < MIN_MINORITY {
        format!(
            "{minority} of {MIN_MINORITY} decisions in the smaller class ({kept} kept, {dismissed} dismissed)"
        )
    } else if holdout.auc < MIN_AUC {
        format!(
            "held-out AUC {:.2} is below {MIN_AUC:.2} on {} rows",
            holdout.auc, holdout.n
        )
    } else {
        format!(
            "{} decisions, {minority} in the smaller class, held-out AUC {:.2}",
            samples.total, holdout.auc
        )
    };
    let active = samples.total >= MIN_LABELS && minority >= MIN_MINORITY && holdout.auc >= MIN_AUC;

    FeedbackModel {
        feature_revision: FEATURE_REVISION.into(),
        feature_names: space.names(),
        weights,
        samples,
        holdout,
        active,
        gate_reason,
        trained_at: trained_at.to_string(),
    }
}

/// A model that has never been trained. Reported rather than absent, so the
/// status endpoint says "not yet learned" instead of saying nothing.
pub fn untrained(space: &FeatureSpace, samples: SampleCounts) -> FeedbackModel {
    FeedbackModel {
        feature_revision: FEATURE_REVISION.into(),
        feature_names: space.names(),
        weights: vec![0.0; space.len()],
        samples,
        holdout: Holdout::default(),
        active: false,
        gate_reason: format!("{} of {MIN_LABELS} decisions recorded", samples.total),
        trained_at: String::new(),
    }
}

/// Whether a stored model may still be used against this vocabulary.
pub fn is_usable(model: &FeedbackModel, space: &FeatureSpace) -> bool {
    model.feature_revision == FEATURE_REVISION
        && model.feature_names == space.names()
        && model.weights.len() == space.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frozen corpus's shape, read from JSON rather than built here.
    #[derive(Debug, Deserialize)]
    struct Corpus {
        acceptance: Acceptance,
        rows: Vec<CorpusRow>,
    }

    #[derive(Debug, Deserialize)]
    struct Acceptance {
        min_holdout_auc: f64,
        deterministic_revision: bool,
    }

    #[derive(Debug, Deserialize)]
    struct CorpusRow {
        kind: String,
        source: String,
        lens: String,
        lens_score: f64,
        content_status: String,
        day: String,
        author: String,
        created_at: String,
        kept: bool,
        decided_at: i64,
    }

    fn corpus() -> Corpus {
        let raw = include_str!("../eval/feedback-corpus.json");
        serde_json::from_str(raw).expect("the frozen corpus parses")
    }

    fn space_for(rows: &[CorpusRow]) -> FeatureSpace {
        let mut sources = rows
            .iter()
            .map(|row| row.source.clone())
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        let mut lenses = rows.iter().map(|row| row.lens.clone()).collect::<Vec<_>>();
        lenses.sort();
        lenses.dedup();
        FeatureSpace { sources, lenses }
    }

    fn rows_from(corpus: &Corpus) -> Vec<TrainingRow> {
        let space = space_for(&corpus.rows);
        let mut rows = corpus
            .rows
            .iter()
            .map(|row| {
                let mut item = FeedItem::new(
                    &format!("https://example.invalid/{}/{}", row.source, row.decided_at),
                    "news",
                    &row.kind,
                );
                item.author = Some(row.author.clone());
                item.content_status = row.content_status.clone();
                item.day = row.day.clone();
                item.created_at = row.created_at.clone();
                let matches = vec![RelevanceMatch {
                    profile_key: row.lens.clone(),
                    profile_label: row.lens.clone(),
                    score: row.lens_score,
                    rationale: String::new(),
                    mode: "semantic".into(),
                    profile_revision: "revision".into(),
                }];
                let vector = features(
                    &space,
                    &FeatureInput {
                        item: &item,
                        matches: &matches,
                        source_id: Some(&row.source),
                        now: Some(1_788_000_000),
                    },
                );
                TrainingRow {
                    features: vector,
                    kept: row.kept,
                    decided_at: row.decided_at,
                    undated: false,
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.decided_at);
        rows
    }

    /// The live shape: 17 keeper, 2 dismissed. The gate must refuse it, and the
    /// reason must name the missing minority class rather than a bare "no".
    #[test]
    fn the_factor_is_inert_below_the_gate() {
        let space = FeatureSpace::new(vec!["source-a".into()], &[]);
        let now = 1_788_000_000;
        let rows = (0..19)
            .map(|index| {
                let mut features = vec![0.0; space.len()];
                features[0] = 1.0;
                TrainingRow {
                    features,
                    kept: index >= 2,
                    decided_at: now - (19 - index) * 86_400,
                    undated: false,
                }
            })
            .collect::<Vec<_>>();
        let model = train(&space, &rows, 0, now, "2026-09-05");
        assert!(!model.active);
        assert!(
            model.gate_reason.contains("of 50 decisions"),
            "the reason must name what is missing: {}",
            model.gate_reason
        );
        let factor = model.factor(None, crate::evaluation::FEEDBACK_WEIGHT);
        assert_eq!(factor.weight, 0.0);
        assert_eq!(factor.key, "feedback");
        assert!(factor.rationale.contains("Not yet learned"));

        // 50 labels with 2 in the smaller class still fails, on the second
        // condition, and says so.
        let mut fifty = rows.clone();
        while fifty.len() < 60 {
            let mut features = vec![0.0; space.len()];
            features[0] = 1.0;
            fifty.push(TrainingRow {
                features,
                kept: true,
                decided_at: now,
                undated: false,
            });
        }
        let model = train(&space, &fifty, 0, now, "2026-09-05");
        assert!(!model.active);
        assert!(
            model.gate_reason.contains("smaller class"),
            "{}",
            model.gate_reason
        );
    }

    #[test]
    fn an_inactive_model_does_not_move_the_context_revision() {
        let space = FeatureSpace::new(vec![], &[]);
        let now = 1_788_000_000;
        let one = train(&space, &[], 0, now, "2026-09-05");
        let mut features = vec![0.0; space.len()];
        features[0] = 1.0;
        let two = train(
            &space,
            &[TrainingRow {
                features,
                kept: true,
                decided_at: now,
                undated: false,
            }],
            0,
            now,
            "2026-09-05",
        );
        assert_eq!(one.revision(), INERT_REVISION);
        assert_eq!(two.revision(), INERT_REVISION);
        assert_eq!(
            one.revision(),
            two.revision(),
            "adding a label while inactive must not restale 372 evaluations"
        );
    }

    #[test]
    fn training_is_deterministic() {
        let corpus = corpus();
        let space = space_for(&corpus.rows);
        let rows = rows_from(&corpus);
        let now = 1_788_000_000;
        let first = train(&space, &rows, 0, now, "2026-09-05");
        let second = train(&space, &rows, 0, now, "2026-09-05");
        assert_eq!(first.weights, second.weights);
        assert_eq!(first.revision(), second.revision());
        assert!(corpus.acceptance.deterministic_revision);

        let mut extra = rows.clone();
        let mut features = extra[0].features.clone();
        features[1] = 1.0 - features[1];
        extra.push(TrainingRow {
            features,
            kept: false,
            decided_at: now,
            undated: false,
        });
        let third = train(&space, &extra, 0, now, "2026-09-05");
        assert_ne!(first.weights, third.weights);
    }

    /// The ranking gate PRD D16 records as missing for digests. The corpus is
    /// frozen and its acceptance value was written before the trainer ran.
    #[test]
    fn the_frozen_corpus_clears_its_own_gate() {
        let corpus = corpus();
        let space = space_for(&corpus.rows);
        let rows = rows_from(&corpus);
        assert!(
            rows.len() >= MIN_LABELS,
            "the corpus must clear the first condition"
        );
        let model = train(&space, &rows, 0, 1_788_000_000, "2026-09-05");
        assert!(
            model.holdout.auc >= corpus.acceptance.min_holdout_auc,
            "held-out AUC {:.3} is below the corpus's own acceptance {:.3}: {}",
            model.holdout.auc,
            corpus.acceptance.min_holdout_auc,
            model.gate_reason
        );
        assert!(model.active, "{}", model.gate_reason);
        assert_ne!(model.revision(), INERT_REVISION);

        // Active, it explains itself by naming its two strongest signals.
        let explanation = model.explain(&rows[0].features);
        assert!(
            explanation.starts_with("Your past decisions:"),
            "{explanation}"
        );
    }

    #[test]
    fn no_feature_slot_carries_text() {
        let space = FeatureSpace::new(vec!["invented-source".into()], &[]);
        let mut item = FeedItem::new("https://example.invalid/x", "news", "github");
        item.author = Some("A Person Who Writes".into());
        item.title = Some("A title nobody should be able to recover".into());
        item.day = "2026-09-01".into();
        item.created_at = "2026-09-01 09:00:00+00:00".into();
        let vector = features(
            &space,
            &FeatureInput {
                item: &item,
                matches: &[],
                source_id: Some("invented-source"),
                now: None,
            },
        );
        assert_eq!(vector.len(), space.len());
        assert!(vector.iter().all(|value| value.is_finite()));
        // The author reaches the vector only as a bucket index.
        let names = space.names();
        let author_slots = names
            .iter()
            .filter(|name| name.starts_with("author:"))
            .count();
        assert_eq!(author_slots, AUTHOR_BUCKETS);
        assert!(names.iter().all(|name| !name.contains("A Person")));
    }

    #[test]
    fn the_vocabulary_is_the_invalidation_rule() {
        let space = FeatureSpace::new(vec!["a".into()], &[]);
        let model = untrained(&space, SampleCounts::default());
        assert!(is_usable(&model, &space));
        let widened = FeatureSpace::new(vec!["a".into(), "b".into()], &[]);
        assert!(
            !is_usable(&model, &widened),
            "a new source changes the vocabulary, so the stored model is stale"
        );
    }

    #[test]
    fn auc_is_the_exact_rank_statistic() {
        assert_eq!(auc(&[(0.9, true), (0.1, false)]), 1.0);
        assert_eq!(auc(&[(0.1, true), (0.9, false)]), 0.0);
        assert_eq!(auc(&[(0.5, true), (0.5, false)]), 0.5);
        // Degenerate holdouts read as a coin rather than as a perfect score.
        assert_eq!(auc(&[(0.9, true), (0.8, true)]), 0.5);
        assert_eq!(auc(&[]), 0.5);
    }

    #[test]
    fn an_undated_decision_counts_at_the_decay_floor() {
        let now = 1_788_000_000;
        assert_eq!(decay(now, now, true), DECAY_FLOOR);
        assert!((decay(now, now, false) - 1.0).abs() < 1e-9);
        // One half-life old is worth half.
        let ninety_days = now - (HALF_LIFE_DAYS as i64) * 86_400;
        assert!((decay(ninety_days, now, false) - 0.5).abs() < 1e-6);
        // And nothing falls below the floor.
        assert_eq!(decay(0, now, false), DECAY_FLOOR);
    }
}
