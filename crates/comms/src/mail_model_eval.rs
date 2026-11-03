//! Offline agreement gate for the mail model rung.
//!
//! **Zero model calls.** It joins a frozen corpus of hand-written labels to the
//! verdicts a shadow pass already stored, keyed by triage id. That split is
//! what makes the number reproducible and is why the redaction gate it copies
//! is a gate at all: a runner that re-prompted would need a live model server,
//! about a hundred sequential calls per run, and would produce a different
//! number every time. All the model time lives in the pass, which already has a
//! receipt, a state ledger and a bounded retry; the scoring stays pure. Drift
//! between the two is impossible because this file builds no prompt.
//!
//! What it measures, and why each one:
//!
//! - **the control**, which needs no model either. Every fixture the rung sees
//!   is a fallback row, so the deterministic classifier answered `aktiv` for
//!   all of them, and its agreement is simply the share of hand-written labels
//!   reading `aktiv` — computable from the corpus alone, before anything runs.
//! - **model agreement**, against the same labels.
//! - **false eviction**: a mail whose label IS `aktiv` that the model proposes
//!   moving out of it. Every applied write is an eviction from `aktiv`, and a
//!   correctly-`aktiv` mail moved out of it vanishes from the operator's
//!   ladder. That is the one failure here that costs a decision, so it has its
//!   own ceiling and the ceiling is a stated policy judgement rather than a
//!   measurement.
//! - **the urgency band error**, mapping the model's continuous 0-10000
//!   self-report onto the corpus's four ordinal bands. Urgency ranks nothing
//!   until this number exists.
//!
//! A fixture whose stored verdict is missing, or was reached against a
//! different item revision or a different prompt, is **skipped with a named
//! reason** rather than scored. An all-skipped corpus is a FAIL, never a
//! perfect score.
//!
//! The corpus itself lives in the private overlay beside
//! `comms-redaction-shadow.json`, with a companion `.md` holding the write-up.
//! The path in this repository (`eval/mail-stream-corpus.json`) deliberately
//! does not exist; the operator passes the overlay path as `argv[1]`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::mail_model::{item_revision, MAIL_MODEL_PROMPT_REVISION};
use crate::store::{ModelVerdict, Store};
use crate::{rules, CommsError, Result};

/// The four ordinal urgency bands a human can write, and their cut points on
/// the model's continuous scale.
///
/// A human cannot reliably choose between 6,500 and 7,000, so the label is an
/// ordinal and the comparison maps the self-report through fixed cuts. If the
/// first measurement shows the error is dominated by rows sitting on a cut
/// point, the cuts move before the gate does.
pub const URGENCY_CUTS: [i64; 3] = [2_500, 5_000, 7_500];

/// Which band a model's basis points fall in: 0 nothing is asked, 1 this week,
/// 2 the next day or two, 3 today.
pub fn band(urgency_bp: i64) -> i64 {
    URGENCY_CUTS
        .iter()
        .filter(|cut| urgency_bp >= **cut)
        .count() as i64
}

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    acceptance: Acceptance,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Default, Deserialize)]
struct Acceptance {
    /// Null until the first run has measured it. The rule the redaction gate
    /// states and the one this follows: a threshold invented before the
    /// measurement is a number chosen to be met.
    #[serde(default)]
    minimum_agreement_percent: Option<f64>,
    /// A stated policy judgement rather than a measurement, which is why it
    /// carries a value from the start.
    #[serde(default = "default_max_false_eviction_percent")]
    max_false_eviction_percent: f64,
    #[serde(default)]
    max_urgency_band_error: Option<f64>,
    #[serde(default)]
    max_urgency_overstatement_percent: Option<f64>,
}

fn default_max_false_eviction_percent() -> f64 {
    2.0
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    #[serde(default)]
    language: String,
    data_class: String,
    #[serde(default)]
    sender_domain: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    snippet: String,
    /// What the deterministic rules answered. `aktiv` for every fallback row by
    /// construction, and the control is computed from the labels against it.
    rule_stream: String,
    /// The hand-written answer, from `rules::STREAMS`, written before any model
    /// output was read.
    label: String,
    /// The hand-written urgency band, 0-3. Absent means this fixture does not
    /// score urgency.
    #[serde(default)]
    urgency_band: Option<i64>,
}

/// Why one fixture was not scored.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    NoVerdict,
    StaleItemRevision,
    StalePromptRevision,
    /// The verdict exists but the model never answered — a refusal, an
    /// over-window verdict, an error. Counted by the state it carries.
    NoProposal(String),
    /// The corpus itself names a stream outside the vocabulary.
    LabelOutsideVocabulary(String),
}

impl SkipReason {
    pub fn describe(&self) -> String {
        match self {
            Self::NoVerdict => "no stored verdict — run the shadow pass first".into(),
            Self::StaleItemRevision => {
                "the stored verdict was reached against a different subject, snippet, sender \
                 domain or class"
                    .into()
            }
            Self::StalePromptRevision => {
                "the stored verdict was reached with a different prompt".into()
            }
            Self::NoProposal(state) => format!("the model produced no proposal (state {state})"),
            Self::LabelOutsideVocabulary(label) => {
                format!("the corpus label \"{label}\" is not one of the seven streams")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    /// Fixtures actually scored.
    pub fixtures: usize,
    /// Fixtures the corpus declared, scored or not.
    pub declared: usize,
    /// `(label, proposal)` -> count, for the confusion table.
    pub confusion: BTreeMap<(String, String), usize>,
    /// `language` -> `(scored, agreed)`. The first measurement's real question:
    /// the redaction corpus found German to be the stronger half, and one
    /// English prompt over a mixed mailbox is exactly the assumption that
    /// deserves a per-language number rather than a guess.
    pub by_language: BTreeMap<String, (usize, usize)>,
    /// How often the deterministic answer equals the label. Needs no model.
    pub control_agreement: usize,
    /// How often the model's proposal equals the label.
    pub model_agreement: usize,
    /// Correctly-`aktiv` mail the model proposes moving out of `aktiv`.
    pub false_evictions: usize,
    /// Fixtures whose label IS `aktiv` — the denominator of the rate above.
    pub aktiv_labels: usize,
    /// `(scored, summed absolute band error, times the model overstated)`.
    pub urgency_scored: usize,
    pub urgency_band_error_total: i64,
    pub urgency_overstated: usize,
    pub skipped: BTreeMap<SkipReason, Vec<String>>,
    pub minimum_agreement_percent: Option<f64>,
    pub max_false_eviction_percent: f64,
    pub max_urgency_band_error: Option<f64>,
    pub max_urgency_overstatement_percent: Option<f64>,
    pub producer: Option<String>,
}

impl Report {
    pub fn skipped_fixtures(&self) -> usize {
        self.skipped.values().map(Vec::len).sum()
    }

    /// Zero when nothing was scored. A vacuous 100% over an all-skipped corpus
    /// is exactly the failure the redaction gate had to be fixed for.
    pub fn control_agreement_percent(&self) -> f64 {
        percent(self.control_agreement, self.fixtures)
    }

    pub fn model_agreement_percent(&self) -> f64 {
        percent(self.model_agreement, self.fixtures)
    }

    pub fn false_eviction_percent(&self) -> f64 {
        percent(self.false_evictions, self.aktiv_labels)
    }

    pub fn urgency_band_error(&self) -> f64 {
        if self.urgency_scored == 0 {
            return 0.0;
        }
        self.urgency_band_error_total as f64 / self.urgency_scored as f64
    }

    pub fn urgency_overstatement_percent(&self) -> f64 {
        percent(self.urgency_overstated, self.urgency_scored)
    }

    /// Whether a corpus was measured at all. A gate cannot judge what it did
    /// not read, and this is the state to say so in rather than to score.
    pub fn empty_measurement(&self) -> bool {
        self.fixtures == 0
    }

    /// Why the run measured what it did, in words a reader can act on.
    pub fn skip_reasons(&self) -> Vec<String> {
        self.skipped
            .iter()
            .map(|(reason, ids)| format!("{} fixture(s): {}", ids.len(), reason.describe()))
            .collect()
    }

    /// Every gate this run can judge, and its verdict. An undeclared threshold
    /// is reported as measured-not-judged rather than silently passed.
    pub fn verdicts(&self) -> Vec<(String, bool)> {
        let mut verdicts = Vec::new();
        if let Some(minimum) = self.minimum_agreement_percent {
            verdicts.push((
                format!(
                    "model agreement {:.1}% >= {minimum:.1}%",
                    self.model_agreement_percent()
                ),
                self.model_agreement_percent() >= minimum,
            ));
        }
        verdicts.push((
            format!(
                "false eviction {:.1}% <= {:.1}%",
                self.false_eviction_percent(),
                self.max_false_eviction_percent
            ),
            self.false_eviction_percent() <= self.max_false_eviction_percent,
        ));
        if let Some(maximum) = self.max_urgency_band_error {
            verdicts.push((
                format!(
                    "urgency band error {:.2} <= {maximum:.2}",
                    self.urgency_band_error()
                ),
                self.urgency_band_error() <= maximum,
            ));
        }
        if let Some(maximum) = self.max_urgency_overstatement_percent {
            verdicts.push((
                format!(
                    "urgency overstated {:.1}% <= {maximum:.1}%",
                    self.urgency_overstatement_percent()
                ),
                self.urgency_overstatement_percent() <= maximum,
            ));
        }
        verdicts
    }

    pub fn passed(&self) -> bool {
        !self.empty_measurement() && self.verdicts().iter().all(|(_, passed)| *passed)
    }
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 * 100.0 / whole as f64
}

/// Score a corpus against the verdicts a store already holds.
pub fn evaluate_file(path: &Path, store: &Store) -> Result<Report> {
    let body = fs::read_to_string(path)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    let corpus: Corpus = serde_json::from_str(&body)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    // An unlabelled row is refused, not scored. `comms mail corpus` writes the skeleton with
    // every `label` empty, and an empty label scored as a stream name disagrees with every
    // verdict — a corpus half filled in would report a low agreement that looks like a
    // measurement of the model. The all-skipped corpus is already a FAIL for the same reason.
    let unlabelled = corpus
        .fixtures
        .iter()
        .filter(|fixture| fixture.label.trim().is_empty())
        .count();
    if unlabelled > 0 {
        return Err(CommsError::Config(format!(
            "{}: {unlabelled} of {} fixture(s) carry no label. Fill them in before scoring —              an empty label is not a disagreement, it is an unwritten judgement.",
            path.display(),
            corpus.fixtures.len()
        )));
    }
    let verdicts = store
        .model_verdicts()
        .map_err(|e| CommsError::Other(e.to_string()))?;
    Ok(evaluate(&corpus, &verdicts))
}

fn evaluate(corpus: &Corpus, verdicts: &std::collections::HashMap<String, ModelVerdict>) -> Report {
    let mut report = Report {
        declared: corpus.fixtures.len(),
        minimum_agreement_percent: corpus.acceptance.minimum_agreement_percent,
        max_false_eviction_percent: corpus.acceptance.max_false_eviction_percent,
        max_urgency_band_error: corpus.acceptance.max_urgency_band_error,
        max_urgency_overstatement_percent: corpus.acceptance.max_urgency_overstatement_percent,
        ..Report::default()
    };

    for fixture in &corpus.fixtures {
        let mut skip = |reason: SkipReason| {
            report
                .skipped
                .entry(reason)
                .or_default()
                .push(fixture.id.clone());
        };
        if !rules::STREAMS.contains(&fixture.label.as_str()) {
            skip(SkipReason::LabelOutsideVocabulary(fixture.label.clone()));
            continue;
        }
        let Some(verdict) = verdicts.get(&fixture.id) else {
            skip(SkipReason::NoVerdict);
            continue;
        };
        // The guard that makes the number mean something: a fixture whose live
        // row has since been re-swept is a different question, and scoring the
        // old answer against the new text would be a measurement of neither.
        let domain = (!fixture.sender_domain.is_empty()).then_some(fixture.sender_domain.as_str());
        if verdict.item_revision
            != item_revision(
                domain,
                &fixture.subject,
                &fixture.snippet,
                &fixture.data_class,
            )
        {
            skip(SkipReason::StaleItemRevision);
            continue;
        }
        if verdict.prompt_revision != MAIL_MODEL_PROMPT_REVISION {
            skip(SkipReason::StalePromptRevision);
            continue;
        }
        let Some(proposal) = verdict.model_stream.clone() else {
            skip(SkipReason::NoProposal(verdict.state.clone()));
            continue;
        };

        report
            .producer
            .get_or_insert_with(|| verdict.producer.clone());
        report.fixtures += 1;
        let language = report
            .by_language
            .entry(if fixture.language.is_empty() {
                "unstated".into()
            } else {
                fixture.language.clone()
            })
            .or_default();
        language.0 += 1;
        if proposal == fixture.label {
            language.1 += 1;
        }
        *report
            .confusion
            .entry((fixture.label.clone(), proposal.clone()))
            .or_default() += 1;
        if fixture.rule_stream == fixture.label {
            report.control_agreement += 1;
        }
        if proposal == fixture.label {
            report.model_agreement += 1;
        }
        if fixture.label == "aktiv" {
            report.aktiv_labels += 1;
            if proposal != "aktiv" {
                report.false_evictions += 1;
            }
        }
        if let (Some(labelled), Some(urgency_bp)) = (fixture.urgency_band, verdict.urgency_bp) {
            let scored = band(urgency_bp);
            report.urgency_scored += 1;
            report.urgency_band_error_total += (scored - labelled).abs();
            if scored > labelled {
                report.urgency_overstated += 1;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture(id: &str, label: &str, urgency_band: Option<i64>) -> Fixture {
        Fixture {
            id: id.into(),
            language: "de".into(),
            data_class: "c1".into(),
            sender_domain: "example.com".into(),
            subject: "A subject".into(),
            snippet: "A preview.".into(),
            rule_stream: "aktiv".into(),
            label: label.into(),
            urgency_band,
        }
    }

    fn verdict(fixture: &Fixture, proposal: Option<&str>, urgency_bp: Option<i64>) -> ModelVerdict {
        ModelVerdict {
            triage_id: fixture.id.clone(),
            mode: "shadow".into(),
            state: proposal.map_or("local_refused", |_| "generated").into(),
            rule_decided_by: "fallback".into(),
            rule_stream: fixture.rule_stream.clone(),
            model_stream: proposal.map(str::to_string),
            confidence_bp: Some(9_000),
            urgency_bp,
            rationale: None,
            urgency_rationale: None,
            redactions: 0,
            data_class: fixture.data_class.clone(),
            redaction_class: fixture.data_class.clone(),
            producer: "foundation-models:apple:mail-stream-v1-english".into(),
            item_revision: item_revision(
                Some(&fixture.sender_domain),
                &fixture.subject,
                &fixture.snippet,
                &fixture.data_class,
            ),
            prompt_revision: MAIL_MODEL_PROMPT_REVISION.into(),
            classification_version: "mail-model-v1".into(),
            attempts: 0,
            last_error: None,
            next_attempt: None,
            held_reason: None,
            applied_at: None,
        }
    }

    fn corpus(fixtures: Vec<Fixture>) -> Corpus {
        Corpus {
            acceptance: Acceptance {
                minimum_agreement_percent: None,
                max_false_eviction_percent: 2.0,
                max_urgency_band_error: None,
                max_urgency_overstatement_percent: None,
            },
            fixtures,
        }
    }

    /// The property that makes this a gate rather than a report: nothing here
    /// builds a prompt or a target, so re-scoring is free and the number is the
    /// same every run.
    #[test]
    fn the_control_is_scored_without_a_model() {
        let source = include_str!("mail_model_eval.rs");
        for forbidden in ["Target", "summarize::ask", "reqwest", "classify_one"] {
            assert!(
                !source
                    .split_once("#[cfg(test)]")
                    .map_or(source, |(body, _)| body)
                    .contains(forbidden),
                "the evaluator must not reach {forbidden}"
            );
        }

        // Ten fixtures, six labelled aktiv: the control is the share of labels
        // that agree with what the rules already answered.
        let fixtures: Vec<Fixture> = (0..10)
            .map(|n| {
                fixture(
                    &format!("thread:{n}"),
                    if n < 6 { "aktiv" } else { "feed" },
                    None,
                )
            })
            .collect();
        let verdicts: HashMap<String, ModelVerdict> = fixtures
            .iter()
            .map(|f| (f.id.clone(), verdict(f, Some("feed"), None)))
            .collect();

        let report = evaluate(&corpus(fixtures), &verdicts);
        assert_eq!(report.fixtures, 10);
        assert_eq!(report.control_agreement_percent(), 60.0);
        // The model answered `feed` for everything, so it agrees with the four
        // feed labels and evicts all six correctly-aktiv mails.
        assert_eq!(report.model_agreement_percent(), 40.0);
        assert_eq!(report.false_evictions, 6);
        assert_eq!(report.aktiv_labels, 6);
        assert_eq!(report.false_eviction_percent(), 100.0);
        assert!(!report.passed(), "100% false eviction cannot pass");
    }

    #[test]
    fn an_empty_measurement_is_a_failure() {
        let fixtures = vec![fixture("thread:0", "aktiv", None)];
        let report = evaluate(&corpus(fixtures), &HashMap::new());
        assert!(report.empty_measurement());
        assert!(
            !report.passed(),
            "a corpus nothing was read from cannot pass"
        );
        assert_eq!(
            report.control_agreement_percent(),
            0.0,
            "not a vacuous 100%"
        );
        assert_eq!(report.skipped_fixtures(), 1);
        assert!(report.skip_reasons()[0].contains("run the shadow pass first"));
    }

    #[test]
    fn a_drifted_row_is_skipped_not_scored() {
        let fixtures = vec![
            fixture("thread:stale-item", "aktiv", None),
            fixture("thread:stale-prompt", "aktiv", None),
            fixture("thread:refused", "aktiv", None),
        ];
        let mut verdicts = HashMap::new();
        let mut stale_item = verdict(&fixtures[0], Some("feed"), None);
        stale_item.item_revision = "the subject changed".into();
        verdicts.insert(fixtures[0].id.clone(), stale_item);
        let mut stale_prompt = verdict(&fixtures[1], Some("feed"), None);
        stale_prompt.prompt_revision = "mail-stream-v0".into();
        verdicts.insert(fixtures[1].id.clone(), stale_prompt);
        verdicts.insert(fixtures[2].id.clone(), verdict(&fixtures[2], None, None));

        let report = evaluate(&corpus(fixtures), &verdicts);
        assert!(report.empty_measurement());
        assert_eq!(report.skipped_fixtures(), 3);
        assert_eq!(report.skipped[&SkipReason::StaleItemRevision].len(), 1);
        assert_eq!(report.skipped[&SkipReason::StalePromptRevision].len(), 1);
        assert_eq!(
            report.skipped[&SkipReason::NoProposal("local_refused".into())].len(),
            1
        );
    }

    /// A label the corpus made up is named rather than counted as a
    /// disagreement, because it is a defect in the corpus and not in the model.
    #[test]
    fn a_label_outside_the_vocabulary_is_named() {
        let fixtures = vec![fixture("thread:bad", "urgent", None)];
        let verdicts: HashMap<String, ModelVerdict> = fixtures
            .iter()
            .map(|f| (f.id.clone(), verdict(f, Some("aktiv"), None)))
            .collect();
        let report = evaluate(&corpus(fixtures), &verdicts);
        assert_eq!(
            report.skipped[&SkipReason::LabelOutsideVocabulary("urgent".into())].len(),
            1
        );
    }

    #[test]
    fn the_urgency_bands_map_a_self_report_onto_an_ordinal() {
        assert_eq!(band(0), 0);
        assert_eq!(band(2_499), 0);
        assert_eq!(band(2_500), 1);
        assert_eq!(band(4_999), 1);
        assert_eq!(band(5_000), 2);
        assert_eq!(band(7_499), 2);
        assert_eq!(band(7_500), 3);
        assert_eq!(band(10_000), 3);

        let fixtures = vec![
            fixture("thread:0", "aktiv", Some(0)),
            fixture("thread:1", "aktiv", Some(3)),
        ];
        let mut verdicts = HashMap::new();
        // Band 0 labelled, band 2 reported: an overstatement of two.
        verdicts.insert(
            fixtures[0].id.clone(),
            verdict(&fixtures[0], Some("aktiv"), Some(5_000)),
        );
        // Band 3 labelled, band 3 reported: exact.
        verdicts.insert(
            fixtures[1].id.clone(),
            verdict(&fixtures[1], Some("aktiv"), Some(9_000)),
        );
        let report = evaluate(&corpus(fixtures), &verdicts);
        assert_eq!(report.urgency_scored, 2);
        assert_eq!(report.urgency_band_error(), 1.0);
        assert_eq!(report.urgency_overstated, 1);
        assert_eq!(report.urgency_overstatement_percent(), 50.0);
        // Undeclared thresholds are measured, not judged — but the false
        // eviction ceiling is a policy number and is always judged.
        assert_eq!(report.verdicts().len(), 1);
        assert!(report.passed());
    }
}
