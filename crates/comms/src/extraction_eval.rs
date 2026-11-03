//! Offline quality gate for extraction and normalization snapshots.
//!
//! The corpus stores the raw text handed from each extractor class to the
//! normalizer. That keeps the test deterministic while making adapter changes
//! explicit: refresh or add the corresponding snapshot before changing the
//! adapter, then keep the judgement terms fixed while comparing the result.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::extraction::{self, Extractor};
use crate::normalize;
use crate::{CommsError, Result};

const INPUT_CLASSES: [&str; 7] = [
    "article",
    "html",
    "repository",
    "paper",
    "client-rendered-page",
    "captured-page",
    "pdf",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    acceptance: Acceptance,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acceptance {
    minimum_useful_retention_percent: f64,
    maximum_boilerplate_leakage_percent: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    input_class: String,
    source: PathBuf,
    must_survive: Vec<String>,
    must_not_survive: Vec<String>,
    expected_rules: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FixtureResult {
    pub id: String,
    pub input_class: String,
    pub raw_chars: usize,
    pub normalized_chars: usize,
    pub retained_percent: f64,
    pub useful_retention_percent: f64,
    pub boilerplate_leakage_percent: f64,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug)]
pub struct EvaluationReport {
    pub fixtures: Vec<FixtureResult>,
    pub passed: bool,
    pub minimum_useful_retention_percent: f64,
    pub maximum_boilerplate_leakage_percent: f64,
}

pub fn evaluate_file(path: &Path) -> Result<EvaluationReport> {
    let body = fs::read_to_string(path)?;
    let corpus: Corpus = serde_json::from_str(&body)?;
    evaluate(corpus, path.parent().unwrap_or_else(|| Path::new(".")))
}

fn evaluate(corpus: Corpus, root: &Path) -> Result<EvaluationReport> {
    validate_corpus(&corpus)?;
    let mut exercised_rules = BTreeSet::new();
    let mut fixtures = Vec::with_capacity(corpus.fixtures.len());

    for fixture in corpus.fixtures {
        let source_text = fs::read_to_string(root.join(&fixture.source))?;

        // The `html` class starts one stage earlier than the rest: its fixture
        // is a page, so extraction runs first and the normalizer is scored on
        // what the extractor actually hands it.
        //
        // That gap is why this gate could not see the defect it exists to
        // catch. Every other fixture stores text already shaped like an
        // extractor's output — multi-line — while the real HTML path emitted
        // one long line, and the normalizer's rules are all guarded by a max
        // line length. The corpus passed at 0% leakage while production stored
        // consent walls verbatim. It is also what lets a replacement extractor
        // (#77) be scored against the built-in one on the same pages.
        let raw = if fixture.input_class == "html" {
            extraction::Builtin
                .extract(&extraction::Document::html(source_text.as_bytes()))?
                .text
        } else {
            source_text.clone()
        };

        let normalized = normalize::normalize(&raw);
        let raw_chars = raw.chars().count();
        let normalized_chars = normalized.text.chars().count();
        let retained_percent = percent(normalized_chars, raw_chars);
        let mut failures = Vec::new();

        // Checked against the file, not against `raw`: for an html fixture the
        // terms are judgements about the page, and an extractor that dropped a
        // boilerplate term itself is doing its job rather than failing.
        for term in fixture
            .must_survive
            .iter()
            .chain(fixture.must_not_survive.iter())
        {
            if !source_text.contains(term) {
                failures.push(format!(
                    "judgement term `{term}` is absent from the raw fixture"
                ));
            }
        }

        let useful_total: usize = fixture.must_survive.iter().map(|s| s.chars().count()).sum();
        let useful_kept: usize = fixture
            .must_survive
            .iter()
            .filter(|term| normalized.text.contains(term.as_str()))
            .map(|s| s.chars().count())
            .sum();
        let useful_retention_percent = percent(useful_kept, useful_total);

        let boilerplate_total: usize = fixture
            .must_not_survive
            .iter()
            .map(|s| s.chars().count())
            .sum();
        let boilerplate_leaked: usize = fixture
            .must_not_survive
            .iter()
            .filter(|term| normalized.text.contains(term.as_str()))
            .map(|s| s.chars().count())
            .sum();
        let boilerplate_leakage_percent = percent(boilerplate_leaked, boilerplate_total);

        if useful_retention_percent < corpus.acceptance.minimum_useful_retention_percent {
            failures.push(format!(
                "useful retention {useful_retention_percent:.1}% is below {:.1}%",
                corpus.acceptance.minimum_useful_retention_percent
            ));
        }
        if boilerplate_leakage_percent > corpus.acceptance.maximum_boilerplate_leakage_percent {
            failures.push(format!(
                "boilerplate leakage {boilerplate_leakage_percent:.1}% exceeds {:.1}%",
                corpus.acceptance.maximum_boilerplate_leakage_percent
            ));
        }

        let actual_rules: BTreeSet<_> = normalized.dropped.iter().map(|d| d.rule).collect();
        for rule in &fixture.expected_rules {
            if !actual_rules.contains(rule.as_str()) {
                failures.push(format!("expected normalization rule `{rule}` did not fire"));
            }
            exercised_rules.insert(rule.clone());
        }

        fixtures.push(FixtureResult {
            id: fixture.id,
            input_class: fixture.input_class,
            raw_chars,
            normalized_chars,
            retained_percent,
            useful_retention_percent,
            boilerplate_leakage_percent,
            passed: failures.is_empty(),
            failures,
        });
    }

    let required_rules: BTreeSet<String> = normalize::RULES
        .iter()
        .map(|rule| rule.name.to_string())
        .chain(normalize::structural_rules().map(|(name, _)| name.to_string()))
        .collect();
    if exercised_rules != required_rules {
        return Err(CommsError::Other(format!(
            "corpus rule coverage differs: expected {required_rules:?}, got {exercised_rules:?}"
        )));
    }

    Ok(EvaluationReport {
        passed: fixtures.iter().all(|fixture| fixture.passed),
        fixtures,
        minimum_useful_retention_percent: corpus.acceptance.minimum_useful_retention_percent,
        maximum_boilerplate_leakage_percent: corpus.acceptance.maximum_boilerplate_leakage_percent,
    })
}

fn validate_corpus(corpus: &Corpus) -> Result<()> {
    let mut by_class = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for fixture in &corpus.fixtures {
        if !ids.insert(&fixture.id) {
            return Err(CommsError::Other(format!(
                "duplicate fixture id `{}`",
                fixture.id
            )));
        }
        if fixture.must_survive.is_empty() || fixture.must_not_survive.is_empty() {
            return Err(CommsError::Other(format!(
                "fixture `{}` needs both positive and negative judgements",
                fixture.id
            )));
        }
        *by_class
            .entry(fixture.input_class.as_str())
            .or_insert(0usize) += 1;
    }

    let actual: BTreeSet<_> = by_class.keys().copied().collect();
    let required: BTreeSet<_> = INPUT_CLASSES.into_iter().collect();
    if actual != required {
        return Err(CommsError::Other(format!(
            "input class coverage differs: expected {required:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        100.0
    } else {
        numerator as f64 * 100.0 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_corpus_passes_fixed_gate() {
        let runfiles_path = Path::new("capabilities/comms/eval/extraction-corpus.json");
        let path = if runfiles_path.exists() {
            runfiles_path.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/extraction-corpus.json")
        };
        let report = evaluate_file(&path).expect("evaluate committed extraction corpus");
        assert!(report.passed, "{:#?}", report.fixtures);
    }
}
