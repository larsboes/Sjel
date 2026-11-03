//! The frozen digest-quality corpus, and the runner that scores it.
//!
//! PRD D16: `eval/relevance-corpus.json` gates ranking with written judgements and an
//! acceptance threshold, and nothing equivalent existed for `libs/summarize`. That gap stopped
//! two decisions cold on 2026-08-30 — the strong local rung became a 4B where it had been a 9B,
//! and Cohere entered the roster as a third public-tier provider, and **neither quality change
//! could be measured**. Both were taken on availability and cost alone.
//!
//! This is the redaction shadow's shape, applied to digests:
//!
//! - The corpus is **frozen** and lives in the private overlay. It holds real mail and real
//!   articles, so it is never in this repository.
//! - The judgements are **hand-written before any score is computed**, and the exporter reads
//!   no verdict while it writes the skeleton.
//! - The runner makes **zero model calls**. It joins labels to digests already in the store,
//!   keyed by `(source, item_id)`, so re-scoring is free and the number is the same every run.
//!
//! **One metric decides: the unfaithful rate.** A digest that asserts something its source does
//! not support is the failure that costs a decision — it is read instead of the article, and it
//! is wrong in a way nothing downstream can catch. Usefulness is measured and reported per
//! producer, and does not gate, because "thin but true" is a preference and "confident and
//! false" is a defect.
//!
//! A fixture whose stored digest has changed since it was labelled is **skipped with a named
//! reason** rather than scored against a judgement written about different words. An
//! all-skipped corpus is a FAIL, never a perfect score.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::store::{Store, StoredDigest};
use crate::{CommsError, Result};

/// What a corpus is allowed to answer and still pass.
#[derive(Debug, Clone, Deserialize)]
pub struct Acceptance {
    /// The one gate. A policy judgement rather than a measurement, and carried from the first
    /// run for that reason — the same distinction `max_false_eviction_percent` carries in the
    /// mail corpus beside it.
    pub max_unfaithful_percent: f64,
    /// Null until the first run has been read. A threshold invented before the measurement is
    /// a number chosen to be met.
    #[serde(default)]
    pub minimum_useful_percent: Option<f64>,
}

/// One judged digest.
#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub source: String,
    pub item_id: String,
    /// The producer that wrote the digest when the fixture was exported. Kept so the report
    /// can split by rung — the whole point of the corpus is comparing them.
    #[serde(default)]
    pub producer: String,
    /// The digest text as it stood when the judgement was written. A digest regenerated since
    /// makes the judgement stale, and the fixture is skipped rather than scored.
    #[serde(default)]
    pub digest_text: String,
    /// The hand-written answer: does every claim in the digest follow from the source?
    pub faithful: Option<bool>,
    /// 0 = says nothing the title did not, 3 = I did not need the source.
    #[serde(default)]
    pub useful_band: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub acceptance: Acceptance,
    pub fixtures: Vec<Fixture>,
}

/// Why one fixture was not scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    /// The store holds no digest for this item any more.
    NoDigest,
    /// The stored text differs from the text that was judged.
    Regenerated,
    /// The stored digest came from a different rung than the one judged.
    DifferentProducer,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::NoDigest => "no digest in the store",
            SkipReason::Regenerated => "regenerated since it was judged",
            SkipReason::DifferentProducer => "a different rung wrote it since",
        }
    }
}

/// What one producer's digests scored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProducerScore {
    pub scored: usize,
    pub unfaithful: usize,
    /// Fixtures whose `useful_band` was written. Absent bands are not counted as zero.
    pub banded: usize,
    pub band_total: i64,
}

impl ProducerScore {
    pub fn unfaithful_percent(&self) -> f64 {
        if self.scored == 0 {
            return 0.0;
        }
        (self.unfaithful as f64) * 100.0 / (self.scored as f64)
    }

    pub fn mean_useful_band(&self) -> Option<f64> {
        if self.banded == 0 {
            return None;
        }
        Some((self.band_total as f64) / (self.banded as f64))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub declared: usize,
    pub scored: usize,
    pub unfaithful: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub by_producer: BTreeMap<String, ProducerScore>,
    pub max_unfaithful_percent: f64,
    pub minimum_useful_percent: Option<f64>,
}

impl Report {
    pub fn unfaithful_percent(&self) -> f64 {
        if self.scored == 0 {
            return 0.0;
        }
        (self.unfaithful as f64) * 100.0 / (self.scored as f64)
    }

    /// An all-skipped corpus FAILS. Nothing was measured, and a runner that answers "0% wrong"
    /// to that question is the failure mode every eval beside this one is written to avoid.
    pub fn passed(&self) -> bool {
        self.scored > 0 && self.unfaithful_percent() <= self.max_unfaithful_percent
    }
}

pub fn evaluate_file(path: &Path, store: &Store) -> Result<Report> {
    let body = fs::read_to_string(path)
        .map_err(|error| CommsError::Config(format!("{}: {error}", path.display())))?;
    let corpus: Corpus = serde_json::from_str(&body)
        .map_err(|error| CommsError::Config(format!("{}: {error}", path.display())))?;

    // Unjudged rows are refused, not scored. `comms digest corpus` writes every `faithful` as
    // null, and a null read as "faithful" would report a corpus nobody judged as a pass.
    let unjudged = corpus
        .fixtures
        .iter()
        .filter(|fixture| fixture.faithful.is_none())
        .count();
    if unjudged > 0 {
        return Err(CommsError::Config(format!(
            "{}: {unjudged} of {} fixture(s) carry no judgement. Write `faithful` for every one \
             before scoring — an unjudged digest is not a good digest.",
            path.display(),
            corpus.fixtures.len()
        )));
    }

    let stored = store
        .generated_digests()
        .map_err(|error| CommsError::Other(error.to_string()))?;
    let by_key: BTreeMap<(String, String), StoredDigest> = stored
        .into_iter()
        .map(|digest| ((digest.source.clone(), digest.item_id.clone()), digest))
        .collect();
    Ok(evaluate(&corpus, &by_key))
}

pub fn evaluate(corpus: &Corpus, stored: &BTreeMap<(String, String), StoredDigest>) -> Report {
    let mut report = Report {
        declared: corpus.fixtures.len(),
        max_unfaithful_percent: corpus.acceptance.max_unfaithful_percent,
        minimum_useful_percent: corpus.acceptance.minimum_useful_percent,
        ..Report::default()
    };

    for fixture in &corpus.fixtures {
        let key = (fixture.source.clone(), fixture.item_id.clone());
        let Some(digest) = stored.get(&key) else {
            *report.skipped.entry(SkipReason::NoDigest).or_default() += 1;
            continue;
        };
        if digest.producer != fixture.producer {
            *report
                .skipped
                .entry(SkipReason::DifferentProducer)
                .or_default() += 1;
            continue;
        }
        if digest.text.as_deref().unwrap_or_default().trim() != fixture.digest_text.trim() {
            *report.skipped.entry(SkipReason::Regenerated).or_default() += 1;
            continue;
        }

        report.scored += 1;
        let entry = report
            .by_producer
            .entry(digest.producer.clone())
            .or_default();
        entry.scored += 1;
        if fixture.faithful == Some(false) {
            report.unfaithful += 1;
            entry.unfaithful += 1;
        }
        if let Some(band) = fixture.useful_band {
            entry.banded += 1;
            entry.band_total += band;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(source: &str, item: &str, producer: &str, text: &str) -> StoredDigest {
        StoredDigest {
            source: source.into(),
            item_id: item.into(),
            text: Some(text.into()),
            state: "generated".into(),
            shape: "standard".into(),
            depth: "standard".into(),
            focus: String::new(),
            producer: producer.into(),
            source_chars: 4_000,
            redactions: 0,
            attempts: 1,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: "2026-09-07 00:00:00.000+00:00".into(),
        }
    }

    fn fixture(item: &str, producer: &str, text: &str, faithful: bool, band: i64) -> Fixture {
        Fixture {
            source: "feed".into(),
            item_id: item.into(),
            producer: producer.into(),
            digest_text: text.into(),
            faithful: Some(faithful),
            useful_band: Some(band),
        }
    }

    fn stored(digests: Vec<StoredDigest>) -> BTreeMap<(String, String), StoredDigest> {
        digests
            .into_iter()
            .map(|digest| ((digest.source.clone(), digest.item_id.clone()), digest))
            .collect()
    }

    #[test]
    fn the_gate_is_the_unfaithful_rate_and_it_is_per_corpus() {
        let corpus = Corpus {
            acceptance: Acceptance {
                max_unfaithful_percent: 2.0,
                minimum_useful_percent: None,
            },
            fixtures: vec![
                fixture("a", "rung-a", "one", true, 2),
                fixture("b", "rung-a", "two", false, 1),
            ],
        };
        let report = evaluate(
            &corpus,
            &stored(vec![
                digest("feed", "a", "rung-a", "one"),
                digest("feed", "b", "rung-a", "two"),
            ]),
        );
        assert_eq!((report.scored, report.unfaithful), (2, 1));
        assert_eq!(report.unfaithful_percent(), 50.0);
        assert!(!report.passed());
    }

    #[test]
    fn a_regenerated_digest_is_skipped_rather_than_scored() {
        // The judgement was written about words that are no longer there. Scoring it would
        // credit or blame a rung for a digest nobody read.
        let corpus = Corpus {
            acceptance: Acceptance {
                max_unfaithful_percent: 2.0,
                minimum_useful_percent: None,
            },
            fixtures: vec![fixture("a", "rung-a", "the judged text", true, 3)],
        };
        let report = evaluate(
            &corpus,
            &stored(vec![digest("feed", "a", "rung-a", "new text")]),
        );
        assert_eq!(report.scored, 0);
        assert_eq!(report.skipped.get(&SkipReason::Regenerated), Some(&1));
        assert!(!report.passed(), "an all-skipped corpus is a FAIL");
    }

    #[test]
    fn a_rung_swap_is_skipped_by_name_because_that_is_the_question() {
        let corpus = Corpus {
            acceptance: Acceptance {
                max_unfaithful_percent: 2.0,
                minimum_useful_percent: None,
            },
            fixtures: vec![fixture("a", "the-9b", "one", true, 3)],
        };
        let report = evaluate(&corpus, &stored(vec![digest("feed", "a", "the-4b", "one")]));
        assert_eq!(report.skipped.get(&SkipReason::DifferentProducer), Some(&1));
    }

    #[test]
    fn usefulness_is_reported_per_producer_and_never_gates() {
        let corpus = Corpus {
            acceptance: Acceptance {
                max_unfaithful_percent: 2.0,
                minimum_useful_percent: None,
            },
            fixtures: vec![
                fixture("a", "rung-a", "one", true, 3),
                fixture("b", "rung-b", "two", true, 1),
            ],
        };
        let report = evaluate(
            &corpus,
            &stored(vec![
                digest("feed", "a", "rung-a", "one"),
                digest("feed", "b", "rung-b", "two"),
            ]),
        );
        assert_eq!(report.by_producer["rung-a"].mean_useful_band(), Some(3.0));
        assert_eq!(report.by_producer["rung-b"].mean_useful_band(), Some(1.0));
        // Thin but true passes. "Confident and false" is the defect; "says little" is a
        // preference, and a preference must not fail a build.
        assert!(report.passed());
    }

    #[test]
    fn a_band_nobody_wrote_is_absent_and_not_a_zero() {
        let mut unbanded = fixture("a", "rung-a", "one", true, 0);
        unbanded.useful_band = None;
        let corpus = Corpus {
            acceptance: Acceptance {
                max_unfaithful_percent: 2.0,
                minimum_useful_percent: None,
            },
            fixtures: vec![unbanded],
        };
        let report = evaluate(&corpus, &stored(vec![digest("feed", "a", "rung-a", "one")]));
        assert_eq!(report.by_producer["rung-a"].mean_useful_band(), None);
        assert_eq!(report.by_producer["rung-a"].scored, 1);
    }
}
