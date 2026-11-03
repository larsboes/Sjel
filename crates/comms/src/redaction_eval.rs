//! Offline recall gate for the cloud redaction path.
//!
//! The corpus freezes real mail with a hand-written list of what a correct redaction must
//! remove, assigned from the text before any redactor was run. Mail rather than feed, because
//! mail is where redaction is load-bearing: after the 2026-08-30 re-derivation the feed's
//! pending backlog is 127 `public` and 1 `personal`, so almost nothing there takes this path.
//!
//! What it measures is **recall**, not similarity. A redaction is wrong in exactly one direction
//! that matters — a value that had to go and is still in the document — and `prepare` already
//! reports what it removed. What it cannot report is what it missed, which is the whole reason a
//! labelled corpus exists.
//!
//! Two modes, one gate. [`Mode::Destructive`] measures `prepare` (`[person]` markers);
//! [`Mode::Pseudonymized`] measures `prepare_pseudonymized` (typed tokens, PRD §6.2c) with the
//! operator's people registry as rung 0. Same corpus, same containment test, same floor: a
//! reversible token is judged by what it leaves in the document, exactly like a marker.
//!
//! `false_positive_markers` is counted but not gated. Over-redaction costs summary quality and
//! is worth watching; it is not a privacy failure, and a gate that fails on it would push the
//! next person to loosen the recognizers.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::cloud_derivative::{self, CloudDocumentInput, LOCAL_ONLY_REFUSAL};
use crate::{CommsError, Result};

#[derive(Debug, Deserialize)]
struct Corpus {
    #[serde(default)]
    acceptance: Acceptance,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Default, Deserialize)]
struct Acceptance {
    /// Recall the incumbent must hold, per entity type. Absent in the first pass on purpose:
    /// the first run's job is to find out what recall IS, and a threshold invented before the
    /// measurement is a number chosen to be met.
    #[serde(default)]
    minimum_recall_percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    language: String,
    data_class: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    must_remove: Vec<Label>,
}

#[derive(Debug, Deserialize)]
struct Label {
    #[serde(rename = "type")]
    entity_type: String,
    value: String,
}

#[derive(Debug)]
pub struct Leak {
    pub fixture: String,
    pub language: String,
    pub entity_type: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct EvaluationReport {
    pub fixtures: usize,
    /// `(type, language)` -> (caught, total). Split by language because the question the trial
    /// exists to answer is whether German prose is the gap.
    pub by_type_language: BTreeMap<(String, String), (usize, usize)>,
    pub leaks: Vec<Leak>,
    pub markers_written: usize,
    pub minimum_recall_percent: Option<f64>,
    /// Fixtures `prepare` refused, by the class it refused them for. Counted rather than
    /// silently skipped: the refusal is stated as the complement of `c0`/`c1`
    /// (`cloud_derivative::prepare`), so a corpus written against an older class vocabulary is
    /// refused whole — and a skip nobody counts is a measurement of nothing that reads like a
    /// measurement of everything.
    pub skipped: BTreeMap<String, usize>,
}

impl EvaluationReport {
    pub fn caught(&self) -> usize {
        self.by_type_language.values().map(|(c, _)| c).sum()
    }
    pub fn total(&self) -> usize {
        self.by_type_language.values().map(|(_, t)| t).sum()
    }
    pub fn skipped_fixtures(&self) -> usize {
        self.skipped.values().sum()
    }
    /// Zero when nothing was measured. A run with no label to check has no recall, and the
    /// vacuous 100% this used to answer is exactly what let an all-skipped corpus print PASS.
    pub fn recall_percent(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.caught() as f64 * 100.0 / total as f64
    }
    /// Whether a corpus was measured at all. A gate cannot judge what it did not read, and this
    /// is the state to say so in rather than to score.
    pub fn empty_measurement(&self) -> bool {
        self.total() == 0
    }
    pub fn passed(&self) -> bool {
        if self.empty_measurement() {
            return false;
        }
        match self.minimum_recall_percent {
            Some(minimum) => self.recall_percent() >= minimum,
            None => true,
        }
    }
    /// Why the run measured nothing, in the words a reader can act on: which classes were
    /// refused and how often. Empty when there is nothing to explain.
    pub fn skip_reasons(&self) -> Vec<String> {
        self.skipped
            .iter()
            .map(|(data_class, count)| {
                format!("{count} fixture(s) refused for data_class \"{data_class}\": {LOCAL_ONLY_REFUSAL}")
            })
            .collect()
    }
}

/// Which transformation the corpus is run through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// `cloud_derivative::prepare`, `REDACTION_VERSION`.
    #[default]
    Destructive,
    /// `cloud_derivative::prepare_pseudonymized` with `people_registry::entity_registry()`,
    /// `PSEUDONYMIZE_VERSION`.
    Pseudonymized,
}

pub fn evaluate_file(path: &Path) -> Result<EvaluationReport> {
    evaluate_file_with(path, Mode::Destructive)
}

pub fn evaluate_file_with(path: &Path, mode: Mode) -> Result<EvaluationReport> {
    let body = fs::read_to_string(path)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    let corpus: Corpus = serde_json::from_str(&body)
        .map_err(|e| CommsError::Config(format!("{}: {e}", path.display())))?;
    evaluate(corpus, mode, crate::people_registry::entity_registry())
}

fn evaluate(
    corpus: Corpus,
    mode: Mode,
    registry: &sjel_pseudonymize::EntityRegistry,
) -> Result<EvaluationReport> {
    let mut report = EvaluationReport {
        minimum_recall_percent: corpus.acceptance.minimum_recall_percent,
        ..Default::default()
    };

    for fixture in &corpus.fixtures {
        let input = CloudDocumentInput {
            source: "redaction-eval".into(),
            id: fixture.id.clone(),
            title: Some(fixture.title.clone()),
            author: Some(fixture.author.clone()),
            summary: None,
            content: Some(fixture.content.clone()),
            data_class: fixture.data_class.clone(),
        };
        // `prepare` admits c0 and c1 and refuses everything else, so a c2, a c3 or an unknown
        // class produces no document at all. That refusal is the correct answer and not a recall
        // result: counting it either way would be a claim about a document that was never built.
        // Corpora therefore carry c0/c1 fixtures — a stricter class measures the gate above, not
        // the redaction below it. The refusal is counted by class all the same, because the one
        // failure this gate cannot survive is measuring nothing and reporting it as clean.
        let prepared = match mode {
            Mode::Destructive => cloud_derivative::prepare(&input),
            Mode::Pseudonymized => {
                cloud_derivative::prepare_pseudonymized(&input, registry).map(|p| p.preview)
            }
        };
        let Ok(preview) = prepared else {
            *report
                .skipped
                .entry(fixture.data_class.clone())
                .or_default() += 1;
            continue;
        };
        report.fixtures += 1;
        report.markers_written += preview.redaction_count;

        let haystack = preview.document.to_lowercase();
        for label in &fixture.must_remove {
            let entry = report
                .by_type_language
                .entry((label.entity_type.clone(), fixture.language.clone()))
                .or_insert((0, 0));
            entry.1 += 1;
            // Case-insensitive containment. A redactor that lowercases a name has still leaked
            // it, and one that leaves `LARS BOES` where the label says `Lars Boes` has too.
            if haystack.contains(&label.value.to_lowercase()) {
                report.leaks.push(Leak {
                    fixture: fixture.id.clone(),
                    language: fixture.language.clone(),
                    entity_type: label.entity_type.clone(),
                    value: label.value.clone(),
                });
            } else {
                entry.0 += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A corpus this test owns, on a path this test owns.
    fn evaluate_json(name: &str, json: &str) -> EvaluationReport {
        let path = std::env::temp_dir().join(format!("comms-redaction-eval-{name}.json"));
        fs::write(&path, json).expect("a writable temp directory");
        let report = evaluate_file(&path).expect("the corpus parses");
        let _ = fs::remove_file(&path);
        report
    }

    /// The failure this gate is for: a corpus whose every fixture `prepare` refuses used to
    /// print recall 0/0 = 100% and exit 0 — a green gate over a corpus that was never read.
    /// Point it at pre-migration fixtures (or a backup of them) and it must say so.
    #[test]
    fn a_corpus_nothing_could_be_measured_on_fails_and_names_the_reason() {
        let report = evaluate_json(
            "all-refused",
            r#"{
              "acceptance": { "minimum_recall_percent": 90.0 },
              "fixtures": [
                { "id": "old-1", "language": "de", "data_class": "personal",
                  "title": "Rechnung", "content": "IBAN DE02120300000000202051",
                  "must_remove": [ { "type": "financial_identifier",
                                     "value": "DE02120300000000202051" } ] },
                { "id": "old-2", "language": "de", "data_class": "vault",
                  "title": "Code", "content": "Code 448215",
                  "must_remove": [ { "type": "long_number", "value": "448215" } ] }
              ]
            }"#,
        );

        assert_eq!(report.fixtures, 0, "nothing was measured");
        assert_eq!(report.skipped_fixtures(), 2);
        assert!(report.empty_measurement());
        assert!(!report.passed(), "an empty measurement is not a pass");
        assert_eq!(report.recall_percent(), 0.0, "0/0 is not 100%");
        let reasons = report.skip_reasons().join("\n");
        assert!(reasons.contains("personal"), "names the class: {reasons}");
        assert!(reasons.contains("vault"), "names the class: {reasons}");
    }

    /// An empty fixture list is the same failure by a different route, and the message says
    /// which of the two it was.
    #[test]
    fn a_corpus_with_no_fixtures_fails_too() {
        let report = evaluate_json("no-fixtures", r#"{ "fixtures": [] }"#);
        assert!(report.empty_measurement());
        assert!(
            !report.passed(),
            "a run with no gate declared still measured nothing"
        );
        assert!(report.skip_reasons().is_empty(), "nothing was refused");
    }

    /// And the gate still passes what it can actually read, or the change above would be a
    /// gate that always fails.
    #[test]
    fn an_admitted_corpus_is_measured_as_before() {
        let report = evaluate_json(
            "admitted",
            r#"{
              "acceptance": { "minimum_recall_percent": 90.0 },
              "fixtures": [
                { "id": "new-1", "language": "de", "data_class": "c1",
                  "title": "Rechnung", "content": "IBAN DE02120300000000202051 bitte pruefen",
                  "must_remove": [ { "type": "financial_identifier",
                                     "value": "DE02120300000000202051" } ] }
              ]
            }"#,
        );

        assert_eq!(report.fixtures, 1);
        assert_eq!(report.skipped_fixtures(), 0);
        assert_eq!(report.total(), 1);
        assert_eq!(report.caught(), 1, "the IBAN is removed");
        assert!(report.passed());
    }

    /// The reversible mode reads the same corpus through the same containment test, and the
    /// author field no longer leaks its display name there.
    #[test]
    fn the_pseudonymized_mode_measures_the_same_labels() {
        let corpus: Corpus = serde_json::from_str(
            r#"{
              "acceptance": { "minimum_recall_percent": 90.0 },
              "fixtures": [
                { "id": "p-1", "language": "en", "data_class": "c1",
                  "author": "Alice Example <alice@example.com>",
                  "title": "Hello Bob", "content": "Ref #1234567, write to bob@example.com",
                  "must_remove": [
                    { "type": "person", "value": "Alice Example" },
                    { "type": "email", "value": "alice@example.com" },
                    { "type": "person", "value": "Bob" },
                    { "type": "identifier", "value": "1234567" },
                    { "type": "email", "value": "bob@example.com" } ] }
              ]
            }"#,
        )
        .unwrap();
        let empty = sjel_pseudonymize::EntityRegistry::builder().build();
        let report = evaluate(corpus, Mode::Pseudonymized, &empty).unwrap();
        assert_eq!(report.total(), 5);
        assert_eq!(report.caught(), 5, "{:?}", report.leaks);
        assert!(report.passed());
    }

    #[test]
    fn pseudonymized_measurement_catches_decomposed_registry_names() {
        let corpus: Corpus = serde_json::from_str(
            r#"{
              "acceptance": { "minimum_recall_percent": 100.0 },
              "fixtures": [
                { "id": "nfd-name", "language": "de", "data_class": "c1",
                  "content": "Jörg schreibt an Jo\u0308rg Mu\u0308ller",
                  "must_remove": [ { "type": "person", "value": "Jo\u0308rg Mu\u0308ller" } ] }
              ]
            }"#,
        )
        .unwrap();
        let registry = sjel_pseudonymize::EntityRegistry::builder()
            .add_person("Jörg Müller")
            .build();
        let report = evaluate(corpus, Mode::Pseudonymized, &registry).unwrap();

        assert_eq!(report.total(), 1);
        assert_eq!(report.caught(), 1, "{:?}", report.leaks);
        assert!(report.passed());
    }
}
