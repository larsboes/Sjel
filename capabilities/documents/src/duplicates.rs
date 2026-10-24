//! Duplicate report over the library. Read-only: nothing here files, archives
//! or deletes.
//!
//! Exact SHA-256 equality is the only authoritative rule, as in
//! `capabilities/media/src/perceptual.rs`. Rescans of the same paper and parts
//! split out of a longer scan have different bytes, so they are found by overlap
//! of extracted text and are proposals for a human to review.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::review::{CandidateKind, PartRelation, ReviewCandidate, SourceLocation};
use crate::Document;

/// Two texts sharing this fraction of their word triples are the same paper.
/// My estimate, not measured.
pub const RESCAN_MIN_JACCARD: f64 = 0.6;
/// A text with this fraction of its word triples inside a longer text is a part
/// of it, such as one letter split out of a 24-page scan. My estimate, not measured.
pub const PART_MIN_CONTAINMENT: f64 = 0.8;
/// Texts shorter than this many word triples say too little to compare.
const MIN_SHINGLES: usize = 20;

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Byte-identical originals: one SHA-256 with several records or source paths.
    pub copies: Vec<Copies>,
    /// Different bytes, nearly the same text.
    pub rescans: Vec<Pair>,
    /// `a` is contained in the longer `b`.
    pub parts: Vec<Pair>,
}

#[derive(Debug, PartialEq)]
pub struct Copies {
    pub sha256: String,
    pub ids: Vec<String>,
    pub paths: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub score: f64,
}

/// Build the report from every record and the `(id, path)` source receipts.
pub fn report(documents: &[Document], sources: &[(String, String)]) -> Report {
    let mut by_hash = BTreeMap::<&str, Vec<&Document>>::new();
    for document in documents {
        by_hash.entry(&document.sha256).or_default().push(document);
    }
    let mut report = Report::default();
    for (sha256, group) in &by_hash {
        let ids: Vec<String> = group.iter().map(|d| d.id.clone()).collect();
        let paths: Vec<String> = sources
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(_, path)| path.clone())
            .collect();
        if ids.len() > 1 || paths.len() > 1 {
            report.copies.push(Copies {
                sha256: (*sha256).to_string(),
                ids,
                paths,
            });
        }
    }

    // One text per distinct original; copies are already reported above.
    // Plain text has no scan to redo or split, and edited note versions would
    // flood the queue: 5,888 of 7,154 records produced most of 49,015 pairs
    // (measured 2026-10-23).
    let texts: Vec<(&str, HashSet<u64>)> = by_hash
        .values()
        .filter_map(|group| {
            let document = group[0];
            if document.media_type == "text/plain" {
                return None;
            }
            let set = shingles(document.extracted_text.as_deref()?);
            (set.len() >= MIN_SHINGLES).then_some((document.id.as_str(), set))
        })
        .collect();
    for (i, j) in candidate_pairs(&texts) {
        let ((left_id, left), (right_id, right)) = (&texts[i], &texts[j]);
        let shared = left.intersection(right).count();
        if shared == 0 {
            continue;
        }
        let jaccard = shared as f64 / (left.len() + right.len() - shared) as f64;
        let (small, large) = if left.len() <= right.len() {
            ((left_id, left), (right_id, right))
        } else {
            ((right_id, right), (left_id, left))
        };
        let containment = shared as f64 / small.1.len() as f64;
        if jaccard >= RESCAN_MIN_JACCARD {
            report.rescans.push(Pair {
                a: left_id.to_string(),
                b: right_id.to_string(),
                score: jaccard,
            });
        } else if containment >= PART_MIN_CONTAINMENT {
            report.parts.push(Pair {
                a: small.0.to_string(),
                b: large.0.to_string(),
                score: containment,
            });
        }
    }
    report
}

/// Index pairs `(i, j)`, `i < j`, that can reach either threshold. Exact prefix
/// filter (Chaudhuri et al., "A Primitive Operator for Similarity Joins", ICDE
/// 2006): both thresholds need the smaller set to share at least 60% of its
/// shingles, because Jaccard ≥ 0.6 implies that too. So the smaller set's
/// rarest `len - ceil(0.6 len) + 1` shingles must include a shared one. All
/// pairs took over ten minutes on 7,154 documents (measured 2026-10-22).
fn candidate_pairs(texts: &[(&str, HashSet<u64>)]) -> BTreeSet<(usize, usize)> {
    let min_overlap = RESCAN_MIN_JACCARD.min(PART_MIN_CONTAINMENT);
    let mut postings = HashMap::<u64, Vec<usize>>::new();
    for (index, (_, set)) in texts.iter().enumerate() {
        for &shingle in set {
            postings.entry(shingle).or_default().push(index);
        }
    }
    let mut pairs = BTreeSet::new();
    for (i, (_, set)) in texts.iter().enumerate() {
        let mut rarest: Vec<u64> = set.iter().copied().collect();
        rarest.sort_unstable_by_key(|shingle| (postings[shingle].len(), *shingle));
        // One extra shingle absorbs float rounding in ceil(); a longer prefix
        // only adds candidates, never loses one.
        let prefix =
            (set.len() - (set.len() as f64 * min_overlap).ceil() as usize + 2).min(set.len());
        for shingle in &rarest[..prefix] {
            for &j in &postings[shingle] {
                if j != i && texts[j].1.len() >= set.len() {
                    pairs.insert((i.min(j), i.max(j)));
                }
            }
        }
    }
    pairs
}

/// Build stable, metadata-only candidates for the operator review queue.
pub fn candidates(documents: &[Document], sources: &[(String, String)]) -> Vec<ReviewCandidate> {
    let report = report(documents, sources);
    let locations = |ids: &[String]| {
        sources
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(document_id, path)| SourceLocation {
                document_id: document_id.clone(),
                path: path.clone(),
            })
            .collect::<Vec<_>>()
    };
    let mut candidates =
        Vec::with_capacity(report.copies.len() + report.rescans.len() + report.parts.len());

    // One record found in several folders stores one blob; only several
    // records for the same bytes leave something to decide.
    for group in report
        .copies
        .into_iter()
        .filter(|group| group.ids.len() > 1)
    {
        let mut member_ids = group.ids;
        member_ids.sort();
        candidates.push(ReviewCandidate {
            candidate_key: format!("exact_copy:{}", group.sha256),
            kind: CandidateKind::ExactCopy,
            locations: locations(&member_ids),
            member_ids,
            score: None,
            sha256: Some(group.sha256),
            part: None,
        });
    }
    for pair in report.rescans {
        let mut member_ids = vec![pair.a, pair.b];
        member_ids.sort();
        candidates.push(ReviewCandidate {
            candidate_key: format!("rescan:{}:{}", member_ids[0], member_ids[1]),
            kind: CandidateKind::Rescan,
            locations: locations(&member_ids),
            member_ids,
            score: Some(pair.score),
            sha256: None,
            part: None,
        });
    }
    for pair in report.parts {
        let mut member_ids = vec![pair.a.clone(), pair.b.clone()];
        member_ids.sort();
        candidates.push(ReviewCandidate {
            candidate_key: format!("part:{}:{}", pair.a, pair.b),
            kind: CandidateKind::Part,
            locations: locations(&member_ids),
            member_ids,
            score: Some(pair.score),
            sha256: None,
            part: Some(PartRelation {
                part_id: pair.a,
                parent_id: pair.b,
            }),
        });
    }
    candidates.sort_by(|left, right| left.candidate_key.cmp(&right.candidate_key));
    candidates
}

/// Hashed lowercase word triples. Words shorter than two characters are OCR
/// noise more often than content, so they are dropped.
fn shingles(text: &str) -> HashSet<u64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
        .collect();
    words
        .windows(3)
        .map(|triple| {
            let mut hasher = DefaultHasher::new();
            triple.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: &str = "Sehr geehrte Damen und Herren, hiermit erhalten Sie einen Gutschein \
        im Wert von einhundert Euro fuer Ihre naechste Buchung. Der Gutschein ist bis zum \
        Ende des Jahres gueltig und kann online eingeloest werden. Mit freundlichen Gruessen \
        Ihr Kundenservice Team";
    const OTHER: &str = "Die Jahresabrechnung fuer Strom und Gas liegt bei. Bitte pruefen Sie \
        den Zaehlerstand und melden Sie Abweichungen innerhalb von vier Wochen an unseren \
        Kundenservice. Der neue Abschlag gilt ab dem naechsten Monat und wird automatisch \
        eingezogen wie bisher vereinbart";

    fn doc(id: &str, sha: &str, text: &str) -> Document {
        Document {
            id: id.repeat(64),
            sha256: sha.repeat(64),
            filename: format!("{id}.pdf"),
            media_type: "application/pdf".into(),
            byte_len: 1,
            imported_at: String::new(),
            status: "inbox".into(),
            category: None,
            project: None,
            extracted_title: None,
            extracted_text: Some(text.into()),
            extractor: None,
            suggested_category: None,
            suggestion_reason: None,
        }
    }

    #[test]
    fn same_bytes_in_two_folders_is_one_copy_group() {
        let documents = [doc("a", "1", LETTER)];
        let sources = [
            ("a".repeat(64), "/x/Scan.pdf".into()),
            ("a".repeat(64), "/y/Scan.pdf".into()),
        ];
        let report = report(&documents, &sources);
        assert_eq!(report.copies.len(), 1);
        assert_eq!(report.copies[0].paths.len(), 2);
        assert!(report.rescans.is_empty() && report.parts.is_empty());
    }

    #[test]
    fn a_rescan_with_ocr_noise_is_proposed_and_an_unrelated_letter_is_not() {
        let noisy = LETTER
            .replace("Gutschein", "Gutsch3in")
            .replace("Wert", "Wcrt");
        let documents = [
            doc("a", "1", LETTER),
            doc("b", "2", &noisy),
            doc("c", "3", OTHER),
        ];
        let report = report(&documents, &[]);
        assert!(report.copies.is_empty());
        assert_eq!(report.rescans.len(), 1, "{report:?}");
        assert_eq!(report.rescans[0].a, "a".repeat(64));
        assert_eq!(report.rescans[0].b, "b".repeat(64));
        assert!(report.parts.is_empty());
    }

    #[test]
    fn a_letter_split_out_of_a_longer_scan_is_a_part_of_it() {
        let bundle = format!("{OTHER} {LETTER} {OTHER} {OTHER}");
        let documents = [doc("p", "1", &bundle), doc("c", "2", LETTER)];
        let report = report(&documents, &[]);
        assert!(report.rescans.is_empty(), "{report:?}");
        assert_eq!(report.parts.len(), 1);
        assert_eq!(report.parts[0].a, "c".repeat(64));
        assert_eq!(report.parts[0].b, "p".repeat(64));
    }

    #[test]
    fn the_prefix_filter_finds_every_pair_that_all_pairs_would() {
        // Overlapping windows of one word stream, so overlaps fall on both
        // sides of both thresholds.
        let words: Vec<String> = (0..400).map(|n| format!("w{}", n * 7 % 113)).collect();
        let texts: Vec<String> = (0..40)
            .map(|n| words[n * 5..n * 5 + 60 + n % 7 * 15].join(" "))
            .collect();
        let sets: Vec<(&str, HashSet<u64>)> =
            texts.iter().map(|text| ("", shingles(text))).collect();
        let reachable: BTreeSet<_> = (0..sets.len())
            .flat_map(|i| (i + 1..sets.len()).map(move |j| (i, j)))
            .filter(|&(i, j)| {
                let shared = sets[i].1.intersection(&sets[j].1).count() as f64;
                shared / sets[i].1.len().min(sets[j].1.len()) as f64 >= RESCAN_MIN_JACCARD
            })
            .collect();
        assert!(
            reachable.len() > 10,
            "fixture too sparse: {}",
            reachable.len()
        );
        assert!(candidate_pairs(&sets).is_superset(&reachable));
    }

    #[test]
    fn plain_text_versions_and_one_record_in_two_folders_are_not_queued() {
        let mut note = doc("a", "1", LETTER);
        note.media_type = "text/plain".into();
        let mut edited = doc("b", "2", &LETTER.replace("Wert", "Wcrt"));
        edited.media_type = "text/plain".into();
        let sources = [
            ("a".repeat(64), "/x/a.md".into()),
            ("a".repeat(64), "/y/a.md".into()),
        ];
        assert_eq!(
            report(&[note.clone(), edited.clone()], &sources)
                .copies
                .len(),
            1
        );
        assert!(candidates(&[note, edited], &sources).is_empty());
    }

    #[test]
    fn candidates_have_stable_keys_and_only_metadata() {
        let documents = [doc("a", "1", LETTER), doc("b", "1", LETTER)];
        let sources = [("a".repeat(64), "/x/a.pdf".into())];
        let candidates = candidates(&documents, &sources);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].candidate_key,
            format!("exact_copy:{}", "1".repeat(64))
        );
        assert_eq!(candidates[0].kind, CandidateKind::ExactCopy);
        assert_eq!(
            candidates[0].member_ids,
            vec!["a".repeat(64), "b".repeat(64)]
        );
        assert_eq!(candidates[0].locations.len(), 1);
        assert_eq!(candidates[0].score, None);
    }
}
