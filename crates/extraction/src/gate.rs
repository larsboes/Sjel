//! The frozen DE/EN corpus gate: what an OCR engine has to clear to enter.
//!
//! This is the same instrument that decided the embedding and reranking
//! adoptions, pointed at a different job. `capabilities/comms/eval/README.md`
//! states the rule it runs on, and the rule is the valuable part:
//!
//! > Add or revise a judgement from its meaning and rationale before looking at
//! > a model's score. Never tune a label merely to turn a failing model green.
//!
//! `multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` entered Sjel by
//! clearing a corpus like this one; `multilingual-e5-small-mlx` and both Apple
//! native embedding variants did not, and their failing runs are still on disk
//! as evidence. No engine enters the extraction ladder any other way
//! (PRD Q63 -> B30).
//!
//! ## Hermetic on purpose
//!
//! [`evaluate`] scores an engine's TEXT. It opens nothing, spawns nothing and
//! needs no macOS, so `cargo test` runs the whole scoring rule against a
//! recorded engine output on every host. Running a live engine is the binary's
//! job: `cargo run --bin extraction-gate`, by hand, exactly the way
//! `comms-extraction-eval` and `bun run-relevance.ts` are run by hand today.
//!
//! ## Two verdict lines, never one number
//!
//! Prose recall and notation fidelity are reported apart, because the whole
//! ladder is built on the fact that they diverge: `upstreams.toml [auge]`
//! records one engine that is excellent at the first and useless at the second.
//! A single aggregate would hide exactly the split that decides which rung an
//! engine is fit for. An engine may take **rung 2** on the prose line alone;
//! only the notation line earns **rung 3**.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::math;

/// What the corpus asks of any candidate. Fixed between candidates: an
/// acceptance threshold that moves to admit an engine has stopped being one.
#[derive(Debug, Clone, Deserialize)]
pub struct Acceptance {
    pub minimum_must_survive_percent: f64,
    pub maximum_must_not_survive: usize,
    pub maximum_forbidden_confusions: usize,
    pub require_detector_agreement: bool,
}

/// A relation an engine may read as something else. Ordered: `expected` is what
/// the page carries, `read_as` is the corruption.
#[derive(Debug, Clone, Deserialize)]
pub struct Confusion {
    pub expected: String,
    pub read_as: String,
    /// The corpus's own reason this substitution is forbidden rather than
    /// merely wrong. Named for the JSON key it binds — the sibling `_note` and
    /// `_judgement` fields carry their underscore in the file too, and a
    /// mismatch here would drop the rationale silently, because serde ignores
    /// an unknown field by default.
    #[serde(default)]
    pub why: String,
}

/// What the detector is required to do on this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorRule {
    /// Prose, a table, a mixed page: rung 3 must never be reached from here. A
    /// false positive on Vision's excellent case is the regression that matters.
    MustNotFire,
    /// Notation: the detector must fire exactly when the engine got the notation
    /// wrong. Stated as a coupling rather than a fixed expectation because a
    /// perfect engine needs no rung 3 and a detector that fired anyway would be
    /// wrong about it.
    MustFireWhenNotationFailed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub language: String,
    pub page_kind: String,
    pub source: String,
    pub must_survive: Vec<String>,
    pub must_not_survive: Vec<String>,
    pub forbidden_confusions: Vec<Confusion>,
    pub detector_rule: DetectorRule,
    #[serde(default)]
    _judgement: String,
    #[serde(default)]
    _judgement_revision: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub acceptance: Acceptance,
    pub fixtures: Vec<Fixture>,
    #[serde(default)]
    _note: String,
    #[serde(default)]
    _matching: String,
}

impl Corpus {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("ocr-corpus.json: {error}"))
    }
}

#[derive(Debug, Clone)]
pub struct FixtureScore {
    pub id: String,
    pub page_kind: String,
    pub language: String,
    pub judged: usize,
    pub survived: usize,
    pub leaked: Vec<String>,
    pub confusions: Vec<String>,
    pub detector_fired: bool,
    pub detector_agrees: bool,
    pub failures: Vec<String>,
}

impl FixtureScore {
    pub fn recall_percent(&self) -> f64 {
        if self.judged == 0 {
            return 100.0;
        }
        self.survived as f64 / self.judged as f64 * 100.0
    }

    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    fn is_notation(&self) -> bool {
        self.page_kind == "math"
    }
}

/// The two verdict lines plus the detector's own line.
#[derive(Debug, Clone)]
pub struct Scorecard {
    pub engine: String,
    pub fixtures: Vec<FixtureScore>,
    pub prose_recall_percent: f64,
    pub notation_recall_percent: f64,
    pub notation_confusions: usize,
    pub detector_agreement_percent: f64,
}

impl Scorecard {
    /// Clears the bar for rung 2: every judged string on every non-notation page.
    pub fn prose_line_passed(&self, acceptance: &Acceptance) -> bool {
        self.prose_recall_percent + f64::EPSILON >= acceptance.minimum_must_survive_percent
            && self
                .fixtures
                .iter()
                .filter(|f| !f.is_notation())
                .all(|f| f.leaked.len() <= acceptance.maximum_must_not_survive)
    }

    /// Clears the bar for rung 3: the notation pages, with no forbidden
    /// confusion anywhere on them.
    pub fn notation_line_passed(&self, acceptance: &Acceptance) -> bool {
        self.notation_recall_percent + f64::EPSILON >= acceptance.minimum_must_survive_percent
            && self.notation_confusions <= acceptance.maximum_forbidden_confusions
    }

    pub fn detector_line_passed(&self, acceptance: &Acceptance) -> bool {
        !acceptance.require_detector_agreement || self.detector_agreement_percent >= 100.0
    }
}

/// Score one engine's output for every fixture in the corpus.
///
/// `outputs` maps a fixture id to the text the engine returned for that page. A
/// fixture with no entry scores as an empty read, which is the honest reading:
/// an engine that returned nothing for a page did not pass it.
pub fn evaluate(corpus: &Corpus, engine: &str, outputs: &BTreeMap<String, String>) -> Scorecard {
    let scores: Vec<FixtureScore> = corpus
        .fixtures
        .iter()
        .map(|fixture| {
            score_fixture(
                fixture,
                outputs.get(&fixture.id).map(String::as_str).unwrap_or(""),
                &corpus.acceptance,
            )
        })
        .collect();

    let prose: Vec<&FixtureScore> = scores.iter().filter(|f| !f.is_notation()).collect();
    let notation: Vec<&FixtureScore> = scores.iter().filter(|f| f.is_notation()).collect();

    Scorecard {
        engine: engine.to_string(),
        prose_recall_percent: recall(&prose),
        notation_recall_percent: recall(&notation),
        notation_confusions: notation.iter().map(|f| f.confusions.len()).sum(),
        detector_agreement_percent: if scores.is_empty() {
            100.0
        } else {
            scores.iter().filter(|f| f.detector_agrees).count() as f64 / scores.len() as f64 * 100.0
        },
        fixtures: scores,
    }
}

fn recall(group: &[&FixtureScore]) -> f64 {
    let judged: usize = group.iter().map(|f| f.judged).sum();
    let survived: usize = group.iter().map(|f| f.survived).sum();
    if judged == 0 {
        return 100.0;
    }
    survived as f64 / judged as f64 * 100.0
}

fn score_fixture(fixture: &Fixture, output: &str, acceptance: &Acceptance) -> FixtureScore {
    let flat = collapse(output);
    let tight = tighten(output);

    let missing: Vec<&String> = fixture
        .must_survive
        .iter()
        .filter(|judged| !flat.contains(&collapse(judged)))
        .collect();
    let leaked: Vec<String> = fixture
        .must_not_survive
        .iter()
        .filter(|forbidden| flat.contains(&collapse(forbidden)))
        .cloned()
        .collect();

    // A forbidden confusion is only reported where the corpus can prove it: the
    // judged string is ABSENT and the same string with the substitution applied
    // is PRESENT. Comparison ignores whitespace entirely, because that is how
    // the recorded failure looked -- `q = 10 nC` came back as `q- 10nC`, with
    // the spacing rearranged as well as the character replaced.
    let mut confusions = Vec::new();
    for confusion in &fixture.forbidden_confusions {
        for judged in &fixture.must_survive {
            if !judged.contains(&confusion.expected) || flat.contains(&collapse(judged)) {
                continue;
            }
            let corrupted = judged.replace(&confusion.expected, &confusion.read_as);
            if tight.contains(&tighten(&corrupted)) {
                confusions.push(format!(
                    "{judged:?} came back as {corrupted:?} ({:?} read as {:?})",
                    confusion.expected, confusion.read_as
                ));
            }
        }
    }

    let detector_fired = math::inspect(output).fires();
    let notation_failed = !missing.is_empty() || !confusions.is_empty();
    let detector_agrees = match fixture.detector_rule {
        DetectorRule::MustNotFire => !detector_fired,
        DetectorRule::MustFireWhenNotationFailed => detector_fired == notation_failed,
    };

    let mut failures = Vec::new();
    if !missing.is_empty() {
        failures.push(format!(
            "did not survive: {}",
            missing
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if leaked.len() > acceptance.maximum_must_not_survive {
        failures.push(format!("must not have survived: {leaked:?}"));
    }
    if confusions.len() > acceptance.maximum_forbidden_confusions {
        failures.extend(confusions.iter().cloned());
    }
    if acceptance.require_detector_agreement && !detector_agrees {
        failures.push(match fixture.detector_rule {
            DetectorRule::MustNotFire => {
                "the math detector fired on a page that is not notation".to_string()
            }
            DetectorRule::MustFireWhenNotationFailed if notation_failed => {
                "the math detector did not fire on notation this engine got wrong".to_string()
            }
            DetectorRule::MustFireWhenNotationFailed => {
                "the math detector fired on notation this engine got right".to_string()
            }
        });
    }

    FixtureScore {
        id: fixture.id.clone(),
        page_kind: fixture.page_kind.clone(),
        language: fixture.language.clone(),
        judged: fixture.must_survive.len(),
        survived: fixture.must_survive.len() - missing.len(),
        leaked,
        confusions,
        detector_fired,
        detector_agrees,
        failures,
    }
}

/// Every run of whitespace becomes one space. An engine's line breaks are a fact
/// about the renderer's line width, not about the characters it read.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// All whitespace removed. Used only for the confusion check, where the spacing
/// is part of the corruption rather than part of the evidence.
fn tighten(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frozen corpus itself, embedded at compile time so this test needs no
    /// working directory and no files on disk.
    const CORPUS: &str = include_str!("../eval/ocr-corpus.json");

    /// One engine's actual output over that corpus, recorded on 2026-09-02 and
    /// never edited. This is what makes the scoring rule testable on a host with
    /// no macOS, no Vision and no binary.
    const RECORDED: &str = include_str!("../eval/recorded/apple-vision-2026-09-02.json");

    fn recorded() -> (Corpus, BTreeMap<String, String>) {
        let corpus = Corpus::parse(CORPUS).expect("the tracked corpus must parse");
        let recording: serde_json::Value =
            serde_json::from_str(RECORDED).expect("the recording must parse");
        let outputs = recording["outputs"]
            .as_object()
            .expect("outputs object")
            .iter()
            .map(|(id, text)| (id.clone(), text.as_str().unwrap_or_default().to_string()))
            .collect();
        (corpus, outputs)
    }

    #[test]
    fn the_tracked_corpus_parses_and_covers_both_languages_and_both_page_kinds() {
        let corpus = Corpus::parse(CORPUS).expect("parses");
        assert!(corpus.fixtures.iter().any(|f| f.language.contains("de")));
        assert!(corpus.fixtures.iter().any(|f| f.language.contains("en")));
        assert!(corpus.fixtures.iter().any(|f| f.page_kind == "math"));
        assert!(corpus.fixtures.iter().any(|f| f.page_kind != "math"));
        // A notation page with no forbidden confusion declared would score the
        // recorded failure as a pass.
        assert!(corpus
            .fixtures
            .iter()
            .filter(|f| f.page_kind == "math")
            .all(|f| !f.forbidden_confusions.is_empty()));
    }

    #[test]
    fn every_forbidden_confusion_carries_its_reason_out_of_the_file() {
        // serde ignores an unknown field, so a Rust name that does not match
        // the JSON key drops the corpus's reasoning silently and leaves the
        // scorer's Debug output — where a reviewer reads it — saying nothing.
        let corpus = Corpus::parse(CORPUS).expect("parses");
        let confusions: Vec<&Confusion> = corpus
            .fixtures
            .iter()
            .flat_map(|f| f.forbidden_confusions.iter())
            .collect();
        assert!(!confusions.is_empty(), "the corpus declares confusions");
        assert!(
            confusions.iter().all(|c| !c.why.is_empty()),
            "{confusions:?}"
        );
    }

    #[test]
    fn a_judged_string_broken_across_lines_still_counts_as_surviving() {
        // The renderer chose the line width. Judging an engine on it would fail
        // every candidate for the wrong reason.
        let output = "Köln Hbf und\nMünchen Hbf über eine\ngeänderte Streckenführung";
        assert!(collapse(output).contains(&collapse("München Hbf über eine geänderte")));
    }

    #[test]
    fn a_relation_read_as_a_hyphen_is_reported_as_a_confusion_and_not_as_a_bare_miss() {
        // The point of the forbidden_confusions field: an engine that turns
        // `q = 10 nC` into `q- 10nC` fails on the signature that decided the
        // ladder, named, rather than on an aggregate that could hide it.
        let fixture = Fixture {
            id: "probe".into(),
            language: "de".into(),
            page_kind: "math".into(),
            source: String::new(),
            must_survive: vec!["q = 10 nC".into()],
            must_not_survive: vec![],
            forbidden_confusions: vec![Confusion {
                expected: "=".into(),
                read_as: "-".into(),
                why: String::new(),
            }],
            detector_rule: DetectorRule::MustFireWhenNotationFailed,
            _judgement: String::new(),
            _judgement_revision: String::new(),
        };
        let score = score_fixture(&fixture, "Gegeben\nq- 10nC\nd-2am", &acceptance());
        assert_eq!(score.confusions.len(), 1, "{score:?}");
        assert!(score.confusions[0].contains("read as"), "{score:?}");
        assert!(!score.passed());
    }

    #[test]
    fn an_engine_that_read_the_notation_correctly_clears_the_notation_line() {
        // The gate has to be passable, or it is a rejection dressed as a
        // measurement. This is the same fixture, read right.
        let fixture = Fixture {
            id: "probe".into(),
            language: "de".into(),
            page_kind: "math".into(),
            source: String::new(),
            must_survive: vec!["q = 10 nC".into()],
            must_not_survive: vec![],
            forbidden_confusions: vec![Confusion {
                expected: "=".into(),
                read_as: "-".into(),
                why: String::new(),
            }],
            detector_rule: DetectorRule::MustFireWhenNotationFailed,
            _judgement: String::new(),
            _judgement_revision: String::new(),
        };
        let score = score_fixture(
            &fixture,
            "Gegeben\nq = 10 nC\nd = 2 cm\nE = F / q",
            &acceptance(),
        );
        assert!(score.passed(), "{score:?}");
        assert!(!score.detector_fired, "a correct read needs no rung three");
    }

    #[test]
    fn the_recorded_apple_vision_run_passes_the_prose_line_and_fails_the_notation_line() {
        // The result that put rung 3 in the design. Asserted here so a change
        // to the detector, the scorer or the corpus that quietly reverses it
        // fails the build instead of a review.
        let (corpus, outputs) = recorded();
        let card = evaluate(&corpus, "apple-vision", &outputs);
        assert!(
            card.prose_line_passed(&corpus.acceptance),
            "prose recall {:.1}%: {:#?}",
            card.prose_recall_percent,
            card.fixtures
                .iter()
                .filter(|f| f.page_kind != "math")
                .collect::<Vec<_>>()
        );
        assert!(
            !card.notation_line_passed(&corpus.acceptance),
            "the recorded run must still fail on notation: {:#?}",
            card.fixtures
                .iter()
                .filter(|f| f.page_kind == "math")
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_detector_agreed_with_the_recorded_run_on_every_page() {
        // Both halves at once: it stayed quiet on four prose pages and fired on
        // the notation this engine got wrong. A regression on either side is a
        // rung-3 call that should not happen or a wrong page stored as right.
        let (corpus, outputs) = recorded();
        let card = evaluate(&corpus, "apple-vision", &outputs);
        assert!(
            card.detector_line_passed(&corpus.acceptance),
            "{:#?}",
            card.fixtures
                .iter()
                .filter(|f| !f.detector_agrees)
                .collect::<Vec<_>>()
        );
    }

    fn acceptance() -> Acceptance {
        Acceptance {
            minimum_must_survive_percent: 100.0,
            maximum_must_not_survive: 0,
            maximum_forbidden_confusions: 0,
            require_detector_agreement: true,
        }
    }
}
