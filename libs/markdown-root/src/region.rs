//! A marked region a machine may regenerate, inside a file a human owns.
//!
//! `capabilities/trips/README.md` specified this in prose a month before anything
//! implemented it: regenerate only a marked Sjel-owned section, preserve
//! everything outside it, and record a conflict rather than choosing between two
//! changed revisions. Every capability that wants to write back to a knowledge
//! store needs the same primitive, so it lives here rather than inside `trips`.
//!
//! ## The shape on disk
//!
//! ```text
//! <!-- sjel:begin owner=finance v=1 sha=1a2b3c4d5e6f7890 -->
//! anything the machine generated
//! <!-- sjel:end owner=finance -->
//! ```
//!
//! HTML comments because Obsidian renders them as nothing, so the note stays
//! readable. The owner is on both markers so two capabilities can each hold a
//! region in one note without either having to know about the other.
//!
//! ## Why the hash is in the marker
//!
//! Conflict detection needs to know what the machine last wrote. Storing that
//! anywhere else means a second source of truth that can go missing, get restored
//! from an older backup, or disagree with the file. Putting it in the marker makes
//! the file self-describing: the only input the check needs is the file itself.
//!
//! It is FNV-1a rather than a cryptographic digest, and that is deliberate. The
//! question being asked is "did a human touch this since we wrote it", not "is
//! this a forgery". FNV-1a is nine lines and no dependency, which is what this
//! crate is built to hold to. An adversary who wants to hide an edit from their
//! own note-taking system has already won.
//!
//! ## What this deliberately is not
//!
//! Not a filesystem API. Everything here is a pure function from string to
//! string, so the caller keeps the read, the write, and the choice of whether a
//! conflict should stop the run. It also makes every case below testable without
//! a temp directory.
//!
//! Not a merge. Two changed revisions produce [`RegionOutcome::Conflict`] carrying
//! both, and the caller decides. Silently picking one is the failure mode this
//! whole thing exists to prevent.

use std::fmt;

const BEGIN: &str = "<!-- axon:begin";
const END: &str = "<!-- axon:end";

/// Which region, and what version of the generator wrote it. The version travels
/// into the marker so a later generator can recognise output it no longer knows
/// how to produce, instead of silently regenerating it in the new shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSpec {
    pub owner: String,
    pub version: u32,
}

impl RegionSpec {
    pub fn new(owner: impl Into<String>, version: u32) -> Self {
        Self {
            owner: owner.into(),
            version,
        }
    }
}

/// Why a document could not be parsed. Every variant is a structural problem with
/// the file rather than a runtime blip, and none of them is repaired: a
/// best-effort fix to a marker is how a machine writes over prose it does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionError {
    /// An opener with no matching closer. Everything after it would be swallowed
    /// by the next write, so the write refuses.
    Unterminated { owner: String, line: usize },
    /// A second opener for the same owner before the first one closed.
    Nested { owner: String, line: usize },
    /// A closer with no opener above it.
    CloserWithoutOpener { owner: String, line: usize },
    /// Two complete regions for one owner. Which one is authoritative is not a
    /// question this crate gets to answer.
    Duplicate { owner: String, line: usize },
    /// An `sjel:begin` line missing `owner=`. Without it the region belongs to
    /// nobody and no caller can claim it.
    MalformedOpener { line: usize },
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionError::Unterminated { owner, line } => write!(
                f,
                "region '{owner}' opened at line {line} is never closed; refusing to write"
            ),
            RegionError::Nested { owner, line } => write!(
                f,
                "region '{owner}' reopened at line {line} before it closed"
            ),
            RegionError::CloserWithoutOpener { owner, line } => write!(
                f,
                "region '{owner}' closed at line {line} without being opened"
            ),
            RegionError::Duplicate { owner, line } => write!(
                f,
                "a second complete region '{owner}' begins at line {line}; which one is authoritative is not decidable here"
            ),
            RegionError::MalformedOpener { line } => {
                write!(f, "axon:begin at line {line} names no owner")
            }
        }
    }
}

impl std::error::Error for RegionError {}

/// What a write did, or refused to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionOutcome {
    /// No region existed, so one was appended.
    Created,
    /// The region held what we last wrote, and the new body differs.
    Updated,
    /// The region held what we last wrote, and the new body is identical. The
    /// document is returned untouched, so a caller can skip the write entirely
    /// and avoid a no-op commit in the vault's git history.
    Unchanged,
    /// A human changed the region since we last wrote it. Nothing was written.
    Conflict {
        /// What the file holds now.
        theirs: String,
        /// What this call would have written.
        ours: String,
    },
}

/// A region located in a document, with the byte range its body occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRegion {
    pub owner: String,
    pub version: u32,
    /// The hash recorded in the marker, or `None` on a marker written by hand.
    pub recorded_hash: Option<u64>,
    pub body: String,
    /// Byte offset of the first character of the opener line.
    pub start: usize,
    /// Byte offset one past the newline that ends the closer line.
    pub end: usize,
}

impl FoundRegion {
    /// Whether the body on disk still matches what was recorded when it was
    /// written. A marker with no recorded hash is treated as touched, because the
    /// honest answer to "did a human write this" is yes: somebody typed the
    /// marker by hand.
    pub fn is_intact(&self) -> bool {
        match self.recorded_hash {
            Some(h) => h == fnv1a64(self.body.as_bytes()),
            None => false,
        }
    }
}

/// FNV-1a, 64-bit. Change detection, not a security boundary — see the module
/// docs for why that is the right trade here.
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Pull `key=value` out of a marker line. Tolerant of ordering and extra
/// whitespace, because the marker is a thing humans will see and occasionally
/// retype.
fn marker_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|token| {
        let rest = token.strip_prefix(key)?;
        rest.strip_prefix('=')
    })
}

/// The line ending this document already uses. A file written on one platform
/// should not silently change ending when a machine appends to it.
fn line_ending(doc: &str) -> &'static str {
    if doc.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Iterate lines with their byte offsets, keeping the terminator attached so
/// offsets stay exact and nothing has to be reconstructed by guessing.
fn lines_with_offsets(doc: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < doc.len() {
        match doc[start..].find('\n') {
            Some(rel) => {
                let end = start + rel + 1;
                out.push((start, end, doc[start..end].trim_end_matches(['\n', '\r'])));
                start = end;
            }
            None => {
                out.push((start, doc.len(), &doc[start..]));
                break;
            }
        }
    }
    out
}

/// Locate the region belonging to `owner`, if the document has one.
///
/// Returns `Ok(None)` for a document with no such region, which is a normal
/// first-write state rather than a problem. Structural damage is an error: see
/// [`RegionError`].
pub fn find(doc: &str, owner: &str) -> Result<Option<FoundRegion>, RegionError> {
    let lines = lines_with_offsets(doc);
    let mut open: Option<(usize, usize, u32, Option<u64>)> = None;
    let mut found: Option<FoundRegion> = None;

    for (index, (start, end, text)) in lines.iter().enumerate() {
        let trimmed = text.trim_start();
        let human_line = index + 1;

        if trimmed.starts_with(BEGIN) {
            let Some(this_owner) = marker_value(trimmed, "owner") else {
                return Err(RegionError::MalformedOpener { line: human_line });
            };
            if this_owner != owner {
                continue;
            }
            if open.is_some() {
                return Err(RegionError::Nested {
                    owner: owner.to_string(),
                    line: human_line,
                });
            }
            if found.is_some() {
                return Err(RegionError::Duplicate {
                    owner: owner.to_string(),
                    line: human_line,
                });
            }
            let version = marker_value(trimmed, "v")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let recorded =
                marker_value(trimmed, "sha").and_then(|s| u64::from_str_radix(s, 16).ok());
            open = Some((*start, *end, version, recorded));
            continue;
        }

        if trimmed.starts_with(END) {
            // A closer with no owner belongs to whichever region is open, which
            // keeps hand-written markers usable without making them ambiguous.
            match marker_value(trimmed, "owner") {
                Some(o) if o != owner => continue,
                _ => {}
            }
            let Some((region_start, body_start, version, recorded)) = open.take() else {
                return Err(RegionError::CloserWithoutOpener {
                    owner: owner.to_string(),
                    line: human_line,
                });
            };
            found = Some(FoundRegion {
                owner: owner.to_string(),
                version,
                recorded_hash: recorded,
                body: doc[body_start..*start].to_string(),
                start: region_start,
                end: *end,
            });
        }
    }

    if let Some((start, _, _, _)) = open {
        let line = doc[..start].matches('\n').count() + 1;
        return Err(RegionError::Unterminated {
            owner: owner.to_string(),
            line,
        });
    }

    Ok(found)
}

/// The body exactly as it will sit on disk: terminated, so the closer starts its
/// own line.
///
/// Hashing has to happen on this form rather than on what the caller passed. The
/// first version hashed the raw argument and appended the terminator afterwards,
/// so every read-back saw one more byte than the recorded hash covered and
/// `is_intact` was false for every region ever written. Every write looked like a
/// human had edited it.
fn normalize_body(body: &str, eol: &str) -> String {
    if body.is_empty() || body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}{eol}")
    }
}

/// Render the marker pair around a body that is already normalized.
fn render(spec: &RegionSpec, normalized: &str, eol: &str) -> String {
    let hash = fnv1a64(normalized.as_bytes());
    let mut out = String::new();
    out.push_str(&format!(
        "{BEGIN} owner={} v={} sha={hash:016x} -->{eol}",
        spec.owner, spec.version
    ));
    out.push_str(normalized);
    out.push_str(&format!("{END} owner={} -->{eol}", spec.owner));
    out
}

/// Regenerate `spec`'s region with `new_body`, returning the new document.
///
/// The three things this guarantees, which are the reasons it exists:
///
/// - Every byte outside the region is preserved exactly. The document is rebuilt
///   as `prefix + region + suffix` from the original slices, never re-serialised.
/// - A region a human edited since the last machine write is never overwritten.
///   The call returns [`RegionOutcome::Conflict`] with both revisions and leaves
///   the document as it found it.
/// - A document with no region gets one appended rather than one guessed into
///   the middle.
pub fn apply(
    doc: &str,
    spec: &RegionSpec,
    new_body: &str,
) -> Result<(String, RegionOutcome), RegionError> {
    let eol = line_ending(doc);
    let normalized = normalize_body(new_body, eol);

    let Some(region) = find(doc, &spec.owner)? else {
        let mut out = String::with_capacity(doc.len() + normalized.len() + 128);
        out.push_str(doc);
        if !doc.is_empty() && !doc.ends_with('\n') {
            out.push_str(eol);
        }
        if !doc.is_empty() {
            out.push_str(eol);
        }
        out.push_str(&render(spec, &normalized, eol));
        return Ok((out, RegionOutcome::Created));
    };

    if !region.is_intact() {
        return Ok((
            doc.to_string(),
            RegionOutcome::Conflict {
                theirs: region.body,
                ours: normalized,
            },
        ));
    }

    if region.body == normalized {
        return Ok((doc.to_string(), RegionOutcome::Unchanged));
    }

    let mut out = String::with_capacity(doc.len() + normalized.len());
    out.push_str(&doc[..region.start]);
    out.push_str(&render(spec, &normalized, eol));
    out.push_str(&doc[region.end..]);
    Ok((out, RegionOutcome::Updated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> RegionSpec {
        RegionSpec::new("finance", 1)
    }

    // Every byte outside the markers is preserved.
    #[test]
    fn creating_a_region_preserves_the_original_document_byte_for_byte() {
        let doc = "# Claude Max\n\nWhy I pay for this.\n";
        let (out, outcome) = apply(doc, &spec(), "monthly: 100 EUR").unwrap();
        assert_eq!(outcome, RegionOutcome::Created);
        assert!(out.starts_with(doc), "original prose must survive verbatim");
        assert!(out.contains("monthly: 100 EUR"));
    }

    #[test]
    fn a_region_round_trips_through_find() {
        let (doc, _) = apply("# Note\n", &spec(), "burn: 12").unwrap();
        let found = find(&doc, "finance").unwrap().unwrap();
        assert_eq!(found.body, "burn: 12\n");
        assert_eq!(found.version, 1);
        assert!(found.is_intact());
    }

    // A human edit outside the region survives regeneration, proven by
    // mutating the prose and the region in the same file.
    #[test]
    fn a_human_edit_outside_the_region_survives_regeneration() {
        let (first, _) = apply("# Note\n\nOriginal prose.\n", &spec(), "burn: 12").unwrap();
        let edited = first.replace("Original prose.", "Rewritten prose, and a new line.");

        let (second, outcome) = apply(&edited, &spec(), "burn: 19").unwrap();
        assert_eq!(outcome, RegionOutcome::Updated);
        assert!(second.contains("Rewritten prose, and a new line."));
        assert!(second.contains("burn: 19"));
        assert!(!second.contains("burn: 12"));
        assert!(!second.contains("Original prose."));
    }

    #[test]
    fn trailing_content_after_the_region_is_preserved() {
        let doc = "# Note\n";
        let (first, _) = apply(doc, &spec(), "a").unwrap();
        let with_tail = format!("{first}\nA footer a human wrote.\n");
        let (second, _) = apply(&with_tail, &spec(), "b").unwrap();
        assert!(second.ends_with("A footer a human wrote.\n"));
    }

    // An edit inside the region is a conflict, and nothing is written.
    #[test]
    fn a_human_edit_inside_the_region_conflicts_and_writes_nothing() {
        let (doc, _) = apply("# Note\n", &spec(), "burn: 12").unwrap();
        let tampered = doc.replace("burn: 12", "burn: 12  # I fixed this by hand");

        let (out, outcome) = apply(&tampered, &spec(), "burn: 19").unwrap();
        assert_eq!(
            out, tampered,
            "a conflict must leave the document untouched"
        );
        match outcome {
            RegionOutcome::Conflict { theirs, ours } => {
                assert!(theirs.contains("I fixed this by hand"));
                assert_eq!(ours, "burn: 19\n");
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    #[test]
    fn a_hand_written_marker_with_no_hash_is_treated_as_touched() {
        let doc = "# Note\n<!-- axon:begin owner=finance v=1 -->\nhand written\n<!-- axon:end owner=finance -->\n";
        let (out, outcome) = apply(doc, &spec(), "generated").unwrap();
        assert_eq!(out, doc);
        assert!(matches!(outcome, RegionOutcome::Conflict { .. }));
    }

    #[test]
    fn an_identical_body_is_unchanged_rather_than_rewritten() {
        let (doc, _) = apply("# Note\n", &spec(), "burn: 12").unwrap();
        let (out, outcome) = apply(&doc, &spec(), "burn: 12").unwrap();
        assert_eq!(outcome, RegionOutcome::Unchanged);
        assert_eq!(out, doc, "a no-op must not churn the vault's git history");
    }

    // Each malformed marker case returns its own named error.
    #[test]
    fn an_unterminated_region_is_an_error() {
        let doc = "# Note\n<!-- axon:begin owner=finance v=1 sha=0 -->\nbody\n";
        assert!(matches!(
            find(doc, "finance"),
            Err(RegionError::Unterminated { .. })
        ));
    }

    #[test]
    fn a_nested_opener_is_an_error() {
        let doc = "<!-- axon:begin owner=finance v=1 sha=0 -->\n<!-- axon:begin owner=finance v=1 sha=0 -->\nbody\n<!-- axon:end owner=finance -->\n";
        assert!(matches!(
            find(doc, "finance"),
            Err(RegionError::Nested { .. })
        ));
    }

    #[test]
    fn a_closer_without_an_opener_is_an_error() {
        let doc = "# Note\n<!-- axon:end owner=finance -->\n";
        assert!(matches!(
            find(doc, "finance"),
            Err(RegionError::CloserWithoutOpener { .. })
        ));
    }

    #[test]
    fn a_second_complete_region_is_an_error() {
        let doc = "<!-- axon:begin owner=finance v=1 sha=0 -->\na\n<!-- axon:end owner=finance -->\n<!-- axon:begin owner=finance v=1 sha=0 -->\nb\n<!-- axon:end owner=finance -->\n";
        assert!(matches!(
            find(doc, "finance"),
            Err(RegionError::Duplicate { .. })
        ));
    }

    #[test]
    fn an_opener_naming_no_owner_is_an_error() {
        let doc = "# Note\n<!-- axon:begin v=1 sha=0 -->\nbody\n<!-- axon:end -->\n";
        assert!(matches!(
            find(doc, "finance"),
            Err(RegionError::MalformedOpener { .. })
        ));
    }

    // Two capabilities, one note. Neither may disturb the other.
    #[test]
    fn two_owners_coexist_and_each_regenerates_only_its_own() {
        let (one, _) = apply("# Note\n", &RegionSpec::new("finance", 1), "money").unwrap();
        let (two, _) = apply(&one, &RegionSpec::new("trips", 1), "journeys").unwrap();

        let (three, outcome) = apply(&two, &RegionSpec::new("finance", 1), "more money").unwrap();
        assert_eq!(outcome, RegionOutcome::Updated);
        assert!(three.contains("more money"));
        assert!(three.contains("journeys"), "the other owner is untouched");
        assert!(
            find(&three, "trips").unwrap().unwrap().is_intact(),
            "the other owner's hash must still validate"
        );
    }

    #[test]
    fn crlf_documents_keep_crlf() {
        let doc = "# Note\r\n\r\nProse.\r\n";
        let (out, _) = apply(doc, &spec(), "body").unwrap();
        assert!(
            out.contains("-->\r\n"),
            "markers must match the file's endings"
        );
        assert!(!out.contains("-->\n\r"), "no mixed endings");
    }

    #[test]
    fn an_empty_document_gets_a_region_without_leading_blank_lines() {
        let (out, outcome) = apply("", &spec(), "body").unwrap();
        assert_eq!(outcome, RegionOutcome::Created);
        assert!(out.starts_with(BEGIN));
    }

    #[test]
    fn a_multi_line_body_round_trips_exactly() {
        let body = "line one\nline two\n\nline four\n";
        let (doc, _) = apply("# Note\n", &spec(), body).unwrap();
        let found = find(&doc, "finance").unwrap().unwrap();
        assert_eq!(found.body, body);
        assert!(found.is_intact());
    }
}
