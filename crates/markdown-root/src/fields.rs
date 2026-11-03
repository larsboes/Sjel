//! One frontmatter key, rewritten in place, with every other byte left alone.
//!
//! `region.rs` above solves the same problem for the *body*: a marked span a
//! machine regenerates inside a file a human owns. This module does it for one
//! top-level frontmatter key. Its first caller is `capabilities/vault`, which
//! writes `last_contact` and `met_at` on People notes because an Obsidian Base
//! computes its "lost touch" view from `last_contact`, and a `.base` cannot call
//! HTTP (`capabilities/vault/src/fields.rs`).
//!
//! ## Why this is not `region.rs` with a different delimiter
//!
//! A region carries an FNV-1a hash of what the machine last wrote, so
//! `is_intact()` can answer "did a human touch this since". **A frontmatter key
//! has nowhere to carry that hash.** `last_contact: "2026-06-17"` is a key a
//! human reads, a Base draws as a column, and Obsidian's own property editor
//! writes; a `sha=` rider on it would be visible junk in every one of those.
//!
//! So this module deliberately promises **less** than `region.rs`, and the whole
//! of what it promises is mechanical:
//!
//! - Everything outside the one scalar it rewrites is byte-identical. The
//!   document is rebuilt as `prefix + new scalar + suffix` from the original
//!   slices, never re-serialised. Key order, quoting, indentation, comments,
//!   blank lines and the entire body survive untouched.
//! - Anything it cannot rewrite unambiguously is refused by name, never
//!   best-effort repaired. A duplicate key, a list, a block scalar, a trailing
//!   comment and an unbalanced quote each get their own [`FieldError`].
//! - Writing the value that is already there returns [`FieldWrite::Unchanged`]
//!   and the document untouched, so a run that changes nothing leaves no commit
//!   in the vault's git history.
//!
//! **It has no opinion about whether the write is a good idea.** Whether a
//! stored value may be replaced at all is a policy question with a real answer
//! and no mechanical one; `capabilities/vault/src/fields.rs` holds it, together
//! with the list of keys Sjel writes.
//!
//! ## What it deliberately is not
//!
//! Not a YAML writer. It rewrites a plain scalar on a top-level key line, and
//! refuses every other shape. A frontmatter block is a thing a person edits by
//! hand, and a serialiser that round-trips it will eventually reflow something
//! nobody asked it to touch. Obsidian's own frontmatter `PATCH` is such a
//! serialiser: it rewrites the whole block, and drops the quotes from keys the
//! call never named.
//!
//! Not a file API, for the same reason `region.rs` is not: pure string to
//! string, so the caller keeps the read, the write and the choice, and every
//! case below is testable without a temp directory.

use std::fmt;

use crate::frontmatter_spanned;

/// Why a key could not be read or written. Every variant names a shape this
/// module refuses to guess at, and the line it sits on, because each one is
/// something a human typed and only a human can settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    /// The note has no frontmatter block at all. Refused rather than created: a
    /// note with no properties is a different kind of note, and opening a block
    /// on one is a larger edit than setting a key.
    NoFrontmatter,
    /// `frontmatter_spanned` could not find the closing fence.
    Unparsable(String),
    /// Two top-level lines carry the same key. Which one a Base reads is not
    /// decidable here — the same refusal `region.rs` makes for a duplicate
    /// region.
    Duplicate { key: String, line: usize },
    /// The key is there but its value is not a plain scalar this can rewrite.
    NotAScalar {
        key: String,
        line: usize,
        reason: &'static str,
    },
    /// The block contains an odd number of double quotes on one line, which is
    /// how a value continued over several lines looks from here. A top-level
    /// key line inside such a value is not a key, and telling the two apart
    /// needs a YAML parser this crate does not have.
    AmbiguousBlock { line: usize },
    /// The value the caller asked to write cannot sit on one scalar line.
    ValueNotWritable { reason: &'static str },
    /// The key the caller asked for cannot appear as a top-level key line.
    KeyNotWritable { reason: &'static str },
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldError::NoFrontmatter => {
                write!(f, "the note has no frontmatter block; refusing to open one")
            }
            FieldError::Unparsable(detail) => write!(f, "frontmatter unreadable: {detail}"),
            FieldError::Duplicate { key, line } => write!(
                f,
                "`{key}` appears twice in the frontmatter, again at line {line}; which one is authoritative is not decidable here"
            ),
            FieldError::NotAScalar { key, line, reason } => {
                write!(f, "`{key}` at line {line} is {reason}; refusing to rewrite it")
            }
            FieldError::AmbiguousBlock { line } => write!(
                f,
                "frontmatter line {line} leaves a double quote open, so a key below it may be inside a value; refusing to guess"
            ),
            FieldError::ValueNotWritable { reason } => {
                write!(f, "the value {reason}")
            }
            FieldError::KeyNotWritable { reason } => write!(f, "the key {reason}"),
        }
    }
}

impl std::error::Error for FieldError {}

/// What a write did. There is no `Conflict` here on purpose: whether the stored
/// value may be replaced is the caller's ruling, and this returns what the
/// stored value *was* so the caller can make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldWrite {
    /// The key was absent and a line for it was appended to the block.
    Created,
    /// The key was there carrying a different scalar, now replaced.
    Replaced { from: String },
    /// The key was there carrying exactly this scalar. The document is returned
    /// untouched.
    Unchanged,
}

/// A key found on a top-level frontmatter line, with the byte span of its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundField {
    pub key: String,
    /// The scalar with its surrounding quotes removed, which is what a Base sees.
    pub value: String,
    /// The quote character the value was written with, so a rewrite keeps the
    /// note's own convention. A note that writes `last_contact: "2026-06-17"`
    /// keeps its quotes; dropping them on every write would be a diff on every
    /// such note that nobody asked for.
    pub quote: Option<char>,
    /// 1-based line number within the whole document.
    pub line: usize,
    /// Byte range of the scalar *including* its quotes, absolute in the document.
    pub span: (usize, usize),
}

/// Byte offsets of every top-level frontmatter line, as `(line_start, line_end,
/// line_number)`. `line_end` excludes the newline.
///
/// Top level means column zero. An indented line belongs to the key above it —
/// a block list item, a nested mapping, the continuation of a block scalar —
/// and a key nested inside another key is not a key this module owns.
fn top_level_lines(
    doc: &str,
    block: (usize, usize),
) -> Result<Vec<(usize, usize, usize)>, FieldError> {
    let (block_start, block_end) = block;
    let mut out = Vec::new();
    let mut cursor = block_start;
    // Line 1 is the opening fence, so the first block line is line 2.
    let mut number = doc[..block_start].matches('\n').count() + 1;

    while cursor < block_end {
        let line_end = doc[cursor..block_end]
            .find('\n')
            .map(|i| cursor + i)
            .unwrap_or(block_end);
        let line = &doc[cursor..line_end];

        // An odd quote count means the value continues on the next line, and
        // from here a top-level-looking key below it is indistinguishable from
        // text inside that value. Refuse the whole block rather than match the
        // wrong line.
        if line.matches('"').count() % 2 == 1 {
            return Err(FieldError::AmbiguousBlock { line: number });
        }

        if !line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            out.push((cursor, line_end, number));
        }

        cursor = line_end + 1;
        number += 1;
    }

    Ok(out)
}

/// Split `key: value` on a top-level line, returning the key and the byte span
/// of the value part. `None` when the line is not a `key:` line at all.
fn split_key_line(doc: &str, start: usize, end: usize) -> Option<(&str, usize, usize)> {
    let line = &doc[start..end];
    let colon = line.find(':')?;
    let key = line[..colon].trim_end();
    if key.is_empty() || key.starts_with('#') || key.starts_with('-') {
        return None;
    }
    let after = colon + 1;
    // Skip the whitespace between the colon and the value; it belongs to the
    // note's formatting and stays where it is.
    let value_offset = line[after..]
        .find(|c: char| c != ' ' && c != '\t')
        .map(|i| after + i)
        .unwrap_or(line.len());
    Some((key, start + value_offset, end))
}

/// Locate `key` on a top-level frontmatter line.
///
/// `Ok(None)` means the note has the block but not the key, which is a normal
/// first-write state. Every other refusal is a [`FieldError`] naming the shape.
pub fn find_field(doc: &str, key: &str) -> Result<Option<FoundField>, FieldError> {
    check_key(key)?;
    let fm = frontmatter_spanned(doc).map_err(FieldError::Unparsable)?;
    let Some(block) = fm.block else {
        return Err(FieldError::NoFrontmatter);
    };

    let mut found: Option<FoundField> = None;
    for (start, end, number) in top_level_lines(doc, block)? {
        let Some((this_key, value_start, value_end)) = split_key_line(doc, start, end) else {
            continue;
        };
        if this_key != key {
            continue;
        }
        if found.is_some() {
            return Err(FieldError::Duplicate {
                key: key.to_string(),
                line: number,
            });
        }

        let raw = doc[value_start..value_end].trim_end();
        let value_end = value_start + raw.len();
        if let Some(reason) = unrewritable_scalar(raw) {
            return Err(FieldError::NotAScalar {
                key: key.to_string(),
                line: number,
                reason,
            });
        }

        let (value, quote) = match raw.chars().next() {
            Some(q @ ('"' | '\'')) if raw.len() >= 2 && raw.ends_with(q) => {
                (raw[1..raw.len() - 1].to_string(), Some(q))
            }
            _ => (raw.to_string(), None),
        };

        found = Some(FoundField {
            key: key.to_string(),
            value,
            quote,
            line: number,
            span: (value_start, value_end),
        });
    }

    Ok(found)
}

/// Why a stored value cannot be rewritten, or `None` when it can.
///
/// The list is short and every entry is a shape where replacing the rest of the
/// line would drop something the note's author put there.
fn unrewritable_scalar(raw: &str) -> Option<&'static str> {
    if raw.is_empty() {
        // `key:` alone opens a block list or holds a null. Both are shapes a
        // scalar write would silently flatten.
        return Some("empty, so it may open a list on the lines below");
    }
    match raw.chars().next() {
        Some('-') => return Some("a list"),
        Some('[') => return Some("an inline list"),
        Some('{') => return Some("an inline mapping"),
        Some('|') | Some('>') => return Some("a block scalar"),
        Some('&') | Some('*') => return Some("a YAML anchor or alias"),
        _ => {}
    }
    // A trailing comment is the author's note to themselves. Replacing the rest
    // of the line would take it with the value. A quoted value that only
    // contains ` #` is refused as well: telling the two apart needs a YAML
    // parser, and a false refusal costs one printed line where a false match
    // costs the comment.
    if raw.contains(" #") {
        return Some("followed by a comment");
    }
    None
}

fn check_key(key: &str) -> Result<(), FieldError> {
    if key.is_empty() {
        return Err(FieldError::KeyNotWritable { reason: "is empty" });
    }
    if key
        .chars()
        .any(|c| c == ':' || c == '#' || c.is_whitespace())
    {
        return Err(FieldError::KeyNotWritable {
            reason: "holds a colon, a hash or whitespace",
        });
    }
    Ok(())
}

fn check_value(value: &str) -> Result<(), FieldError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(FieldError::ValueNotWritable {
            reason: "holds a line break, and this writes one scalar line",
        });
    }
    if value.contains('"') || value.contains('\'') {
        return Err(FieldError::ValueNotWritable {
            reason: "holds a quote, which would need escaping this does not do",
        });
    }
    if value.contains(" #") {
        return Err(FieldError::ValueNotWritable {
            reason: "holds ` #`, which YAML would read as a comment",
        });
    }
    if value.trim() != value {
        return Err(FieldError::ValueNotWritable {
            reason: "has leading or trailing whitespace",
        });
    }
    if value.is_empty() {
        return Err(FieldError::ValueNotWritable {
            reason:
                "is empty, and an empty key is indistinguishable from a producer that never ran",
        });
    }
    Ok(())
}

/// The line ending this document already uses, so a note written on one
/// platform does not silently change ending when a machine appends to it.
/// `region.rs` makes the same check for the same reason.
fn line_ending(doc: &str) -> &'static str {
    if doc.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Write `value` to `key`, returning the new document and what happened.
///
/// The guarantee, and the only one: **every byte of the document except the one
/// scalar is preserved exactly.** On a create, the only insertion is one new
/// line immediately above the closing fence. There is no re-serialisation
/// anywhere in this function, which is why a Bases embed, a Mermaid fence and a
/// hand-aligned property block all survive it.
pub fn set_field(doc: &str, key: &str, value: &str) -> Result<(String, FieldWrite), FieldError> {
    check_key(key)?;
    check_value(value)?;

    let existing = find_field(doc, key)?;

    let Some(field) = existing else {
        // Create. `frontmatter_spanned` put `block.1` at the first byte of the
        // closing fence line, so inserting there appends to the block and
        // touches nothing above it.
        let fm = frontmatter_spanned(doc).map_err(FieldError::Unparsable)?;
        let Some((_, fence_start)) = fm.block else {
            return Err(FieldError::NoFrontmatter);
        };
        let eol = line_ending(doc);
        let mut out = String::with_capacity(doc.len() + key.len() + value.len() + 4);
        out.push_str(&doc[..fence_start]);
        out.push_str(&format!("{key}: {value}{eol}"));
        out.push_str(&doc[fence_start..]);
        return Ok((out, FieldWrite::Created));
    };

    if field.value == value {
        return Ok((doc.to_string(), FieldWrite::Unchanged));
    }

    let rendered = match field.quote {
        Some(q) => format!("{q}{value}{q}"),
        None => value.to_string(),
    };
    let (start, end) = field.span;
    let mut out = String::with_capacity(doc.len() + rendered.len());
    out.push_str(&doc[..start]);
    out.push_str(&rendered);
    out.push_str(&doc[end..]);
    Ok((
        out,
        FieldWrite::Replaced {
            from: field.value.clone(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic People note in the shape Obsidian's property editor writes:
    /// quoted dates, a list, a wikilink, a body with a Bases embed.
    fn person() -> String {
        [
            "---",
            "type: person",
            "relation:",
            "  - friend",
            "  - colleague",
            "summary: \"met at a conference\"",
            "last_contact: \"2026-02-23\"",
            "met_at: \"2024-08-10\"",
            "birthday: 1995-04-02",
            "---",
            "",
            "# Ida Muster",
            "",
            "```base",
            "filters:",
            "  and:",
            "    - file.inFolder(\"Atlas/People\")",
            "```",
            "",
            "---",
            "",
            "Some prose with a [[wikilink]].",
            "",
        ]
        .join("\n")
    }

    fn body_of(doc: &str) -> String {
        let fm = frontmatter_spanned(doc).expect("parse");
        doc[fm.body_start..].to_string()
    }

    #[test]
    fn replacing_a_value_leaves_every_other_byte_alone() {
        let before = person();
        let (after, write) = set_field(&before, "last_contact", "2026-06-17").expect("write");
        assert_eq!(
            write,
            FieldWrite::Replaced {
                from: "2026-02-23".into()
            }
        );
        // The whole document, minus the ten bytes of the date, is identical.
        assert_eq!(
            before.replace("2026-02-23", "2026-06-17"),
            after,
            "only the scalar may move"
        );
        assert_eq!(body_of(&before), body_of(&after));
    }

    /// The byte-stability claim over the shapes a YAML serialiser rewrites:
    /// double- and single-quoted neighbours, a comment line, a trailing comment
    /// on another key, hand-aligned values, and a key that sorts before the
    /// target. Obsidian's frontmatter `PATCH` drops the quotes on such
    /// neighbours; this must not. The output differs from the input only on the
    /// target line, and only inside the value's quotes.
    #[test]
    fn only_the_target_line_changes_when_neighbours_are_quoted_and_commented() {
        let before = [
            "---",
            "# people fields, kept by hand",
            "aliases: ['Ida', \"I. M.\"]",
            "met_at:       \"2023-11-02\"",
            "last_contact: \"2026-02-23\"",
            "company: 'Example GmbH'   # since spring",
            "birthday: \"1990-05-17\"",
            "---",
            "",
            "Prose with a \"quote\" and a [[Link]].",
            "",
        ]
        .join("\n");
        let (after, write) = set_field(&before, "last_contact", "2026-06-17").expect("write");
        assert_eq!(
            write,
            FieldWrite::Replaced {
                from: "2026-02-23".into()
            }
        );
        assert_eq!(
            before.len(),
            after.len(),
            "same length: one date for another"
        );

        let changed: Vec<(usize, &str, &str)> = before
            .lines()
            .zip(after.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i + 1, a, b))
            .collect();
        assert_eq!(
            changed,
            vec![(
                5,
                "last_contact: \"2026-02-23\"",
                "last_contact: \"2026-06-17\""
            )],
            "exactly one line may change"
        );
    }

    #[test]
    fn the_notes_own_quoting_survives_a_write() {
        let quoted = "---\nlast_contact: \"2026-01-01\"\n---\nx\n";
        let (after, _) = set_field(quoted, "last_contact", "2026-06-17").expect("write");
        assert!(after.contains("last_contact: \"2026-06-17\""));

        let bare = "---\nlast_contact: 2026-01-01\n---\nx\n";
        let (after, _) = set_field(bare, "last_contact", "2026-06-17").expect("write");
        assert!(after.contains("last_contact: 2026-06-17"));
        assert!(!after.contains('"'));
    }

    #[test]
    fn writing_the_value_that_is_already_there_changes_nothing() {
        let before = person();
        let (after, write) = set_field(&before, "last_contact", "2026-02-23").expect("write");
        assert_eq!(write, FieldWrite::Unchanged);
        assert_eq!(before, after, "an unchanged write must not touch the bytes");
    }

    #[test]
    fn a_created_key_lands_last_in_the_block_and_the_body_is_untouched() {
        let before = person();
        let (after, write) = set_field(&before, "follow_up", "2026-10-01").expect("write");
        assert_eq!(write, FieldWrite::Created);
        assert_eq!(body_of(&before), body_of(&after));
        let fm = frontmatter_spanned(&after).expect("parse");
        let (start, end) = fm.block.expect("block");
        assert!(after[start..end].ends_with("follow_up: 2026-10-01\n"));
        // Every key that was there is still there, in order.
        assert!(after.contains("last_contact: \"2026-02-23\"\nmet_at: \"2024-08-10\""));
    }

    #[test]
    fn a_crlf_note_keeps_its_line_endings() {
        let doc = "---\r\ntype: person\r\n---\r\nbody\r\n";
        let (after, write) = set_field(doc, "last_contact", "2026-06-17").expect("write");
        assert_eq!(write, FieldWrite::Created);
        assert_eq!(
            after,
            "---\r\ntype: person\r\nlast_contact: 2026-06-17\r\n---\r\nbody\r\n"
        );
    }

    // --- the refusals. Each one is a shape a rewrite would damage. -----------

    #[test]
    fn a_note_with_no_frontmatter_is_refused_rather_than_given_one() {
        assert_eq!(
            set_field("Just prose.\n", "last_contact", "2026-06-17"),
            Err(FieldError::NoFrontmatter)
        );
    }

    #[test]
    fn a_duplicated_key_is_refused_because_which_one_wins_is_not_decidable() {
        let doc = "---\nlast_contact: \"2026-01-01\"\ntype: person\nlast_contact: \"2025-01-01\"\n---\nx\n";
        assert_eq!(
            set_field(doc, "last_contact", "2026-06-17"),
            Err(FieldError::Duplicate {
                key: "last_contact".into(),
                line: 4
            })
        );
    }

    #[test]
    fn a_list_a_block_scalar_and_a_comment_are_each_refused_by_name() {
        for (doc, reason) in [
            (
                "---\nlast_contact:\n  - 2026-01-01\n---\nx\n",
                "empty, so it may open a list on the lines below",
            ),
            (
                "---\nlast_contact: [2026-01-01]\n---\nx\n",
                "an inline list",
            ),
            (
                "---\nlast_contact: |\n  2026-01-01\n---\nx\n",
                "a block scalar",
            ),
            (
                "---\nlast_contact: 2026-01-01 # from memory\n---\nx\n",
                "followed by a comment",
            ),
        ] {
            assert_eq!(
                set_field(doc, "last_contact", "2026-06-17"),
                Err(FieldError::NotAScalar {
                    key: "last_contact".into(),
                    line: 2,
                    reason
                }),
                "{doc:?}"
            );
        }
    }

    #[test]
    fn a_key_nested_under_another_key_is_not_this_key() {
        // The indented `last_contact` belongs to `history`, not to the note.
        let doc = "---\nhistory:\n  last_contact: 2020-01-01\n---\nx\n";
        let (after, write) = set_field(doc, "last_contact", "2026-06-17").expect("write");
        assert_eq!(write, FieldWrite::Created);
        assert!(
            after.contains("  last_contact: 2020-01-01\n"),
            "the nested one is untouched"
        );
        assert!(after.contains("\nlast_contact: 2026-06-17\n"));
    }

    #[test]
    fn a_value_continued_over_two_lines_makes_the_whole_block_ambiguous() {
        let doc = "---\nsummary: \"line one\n  line two\"\nlast_contact: 2020-01-01\n---\nx\n";
        assert_eq!(
            set_field(doc, "last_contact", "2026-06-17"),
            Err(FieldError::AmbiguousBlock { line: 2 })
        );
    }

    #[test]
    fn a_value_that_cannot_sit_on_one_line_is_refused_before_anything_is_read() {
        for value in ["", "a\nb", "has \"quotes\"", " padded ", "x # y"] {
            assert!(
                matches!(
                    set_field(&person(), "last_contact", value),
                    Err(FieldError::ValueNotWritable { .. })
                ),
                "{value:?} should be refused"
            );
        }
    }

    #[test]
    fn a_key_that_is_not_a_key_is_refused() {
        for key in ["", "two words", "a:b", "#comment"] {
            assert!(
                matches!(
                    set_field(&person(), key, "2026-06-17"),
                    Err(FieldError::KeyNotWritable { .. })
                ),
                "{key:?} should be refused"
            );
        }
    }

    #[test]
    fn a_horizontal_rule_in_the_body_is_not_the_closing_fence() {
        // `person()` has a `---` in its body. If the block ended there, the
        // create below would splice into the prose.
        let before = person();
        let (after, _) = set_field(&before, "follow_up", "2026-10-01").expect("write");
        assert_eq!(body_of(&before), body_of(&after));
        assert_eq!(after.matches("follow_up").count(), 1);
    }

    #[test]
    fn find_reports_the_stored_scalar_without_its_quotes() {
        let field = find_field(&person(), "last_contact")
            .expect("read")
            .expect("present");
        assert_eq!(field.value, "2026-02-23");
        assert_eq!(field.quote, Some('"'));
        assert_eq!(field.line, 7);
    }

    #[test]
    fn find_answers_none_for_a_key_the_note_does_not_carry() {
        assert_eq!(find_field(&person(), "follow_up").expect("read"), None);
    }

    /// The property the whole module rests on, checked over every key in a
    /// realistic note rather than argued: writing any one key touches one line
    /// and leaves an untouched prefix and suffix on either side of it.
    ///
    /// Stated as a common prefix and suffix rather than a zip, because a create
    /// inserts a line and shifts every line below it — under a zip that reads
    /// as thirteen changes and hides the property being checked.
    #[test]
    fn writing_any_one_key_touches_one_line_and_nothing_else() {
        let before = person();
        for key in [
            "type",
            "summary",
            "last_contact",
            "met_at",
            "birthday",
            "brand_new",
        ] {
            let (after, _) = set_field(&before, key, "2026-06-17").expect(key);
            let a: Vec<&str> = before.lines().collect();
            let b: Vec<&str> = after.lines().collect();

            let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
            let suffix = a[prefix..]
                .iter()
                .rev()
                .zip(b[prefix..].iter().rev())
                .take_while(|(x, y)| x == y)
                .count();

            assert!(
                a.len() - prefix - suffix <= 1 && b.len() - prefix - suffix <= 1,
                "{key}: {} old and {} new lines outside the untouched prefix/suffix",
                a.len() - prefix - suffix,
                b.len() - prefix - suffix
            );
        }
    }
}
