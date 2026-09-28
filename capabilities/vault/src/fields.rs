//! Which frontmatter keys Sjel writes, and what it is allowed to do to them.
//!
//! `libs/markdown-root/src/fields.rs` holds the *mechanism*: rewrite one scalar,
//! leave every other byte alone, refuse anything ambiguous. This file holds the
//! *policy*: which keys, on which folder, in which direction.
//!
//! ## Why a key on the note, when `/api/people` already serves the value
//!
//! `Resources/Bases/People.base` computes
//! `days_since_contact = (today() - last_contact) / 1 day` and
//! `lost_touch = days_since_contact > 90`. A `.base` cannot call HTTP, so
//! `GET /api/people` cannot reach the embedded view. Only a key on the note can.
//! The keys therefore keep their existing names; a prefixed twin would be a
//! second column the Base does not read.
//!
//! ## The rule that keeps the list short
//!
//! **A producer writes no key that a `.base` view does not already read.**
//! `mention_count` is deliberately absent from [`OWNED`] for that reason: no
//! `.base` names it, so writing it would create a machine-maintained value with
//! no reader, a second source of truth whose drift nothing would surface. It
//! stays computed and reported by `vault people`.
//!
//! ## The conflict rule, and why it is not `region.rs`'s
//!
//! Two changed revisions produce a recorded conflict, never a silent choice.
//! `region.rs` implements that with an FNV-1a hash in the marker, so it can ask
//! "did a human touch this since we wrote it". A frontmatter key has nowhere to
//! carry a hash, so **this writer can never tell its own value from a human's.**
//!
//! What it can do instead is make every write *monotone in a declared
//! direction*, so a write can only add information and never destroy it:
//!
//! - `last_contact` is a **maximum**. A Journal entry naming a person proves
//!   contact on that day; contact that happened off the Journal can still be
//!   later. So the Journal is a lower bound, and moving the value *forward* is
//!   safe whoever typed it.
//! - `met_at` is a **minimum**, the mirror image: the earliest Journal entry is
//!   an upper bound on when you met, so moving the value *backward* is safe.
//!
//! When the stored value is already ahead of the computed one, the note knows
//! something the Journal does not, and the writer stops: [`Verdict::Behind`],
//! recorded and printed, never written. **That is the recorded conflict.** Its
//! resolution is fixed in the human's favour, which is why there is no
//! `--force`. An operator who wants the Journal's value deletes the key and
//! runs again.
//!
//! The property this buys, checked in `never_lengthens_a_gap` below: **a write
//! can never move a note into `lost_touch`, only out of it.** Creating a key on
//! a note that has none can, and that is correct: a person last named in 2024
//! *is* out of touch.
//!
//! ## What the written value claims
//!
//! "The newest dated `Journal/` note that links this person." That is not the
//! same claim as "the last time we spoke", and `people.rs` already says so for
//! `met_at`: "first appears in the record", not "the day we met". The monotone
//! rule is what makes writing it survivable: a hand-typed date that is more
//! accurate than the Journal is never overwritten, only reported.
//!
//! ## Relationship to `capabilities/entities`
//!
//! The entities import reads `last_contact` from `/api/people`, which is the
//! value computed from Journal links, never the key stored on the note. Writing
//! the key therefore cannot feed back into the computation or into entities.
//! On a recorded conflict the note keeps its later hand-typed date while
//! entities holds the Journal's earlier one; `/api/people` reports that pair in
//! `stored` and `disagrees`.

use std::collections::HashMap;
use std::fmt;

use markdown_root::{find_field, FieldError};
use serde::Serialize;

use crate::note::Note;
use crate::people::{self, PeopleReport};

/// Which way a value is allowed to move, which is what stands in for the hash
/// `region.rs` puts in its marker. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Later is more true; the stored value is a maximum. `last_contact`.
    Latest,
    /// Earlier is more true; the stored value is a minimum. `met_at`.
    Earliest,
}

impl Direction {
    fn advances(self, from: &str, to: &str) -> bool {
        match self {
            Direction::Latest => to > from,
            Direction::Earliest => to < from,
        }
    }
}

/// One key Sjel writes, and the evidence that it is allowed to.
///
/// `read_by` is the licence: a producer writes only what a view reads, so a key
/// with no `.base` behind it does not belong in this table.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct OwnedKey {
    pub key: &'static str,
    /// The folder the ownership is bounded to, as a vault-relative prefix.
    pub folder: &'static str,
    pub direction: Direction,
    /// The `.base` whose view reads this key.
    pub read_by: &'static str,
    /// What the written value actually claims, as opposed to what the key's
    /// name suggests. See the module docs.
    pub means: &'static str,
}

/// **The whole ownership contract.** Sjel writes these keys, on these folders,
/// in these directions, and nothing else anywhere in the vault.
///
/// `mention_count` is deliberately not here; see the module docs. Neither is
/// `contact_frequency`: how often you want to see someone is a judgement no
/// backlink count derives.
pub const OWNED: &[OwnedKey] = &[
    OwnedKey {
        key: "last_contact",
        folder: people::FOLDER,
        direction: Direction::Latest,
        read_by: "Resources/Bases/People.base",
        means: "the newest dated Journal note linking this person",
    },
    OwnedKey {
        key: "met_at",
        folder: people::FOLDER,
        direction: Direction::Earliest,
        read_by: "Resources/Bases/People.base",
        means: "the oldest dated Journal note linking this person",
    },
];

/// The `lost_touch` threshold, copied from `Resources/Bases/People.base`:
/// `lost_touch: formula.days_since_contact != null && formula.days_since_contact > 90`.
pub const LOST_TOUCH_DAYS: i64 = 90;

/// What the writer would do, or refuses to do, to one key on one note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// The note has no such key and the Journal has a value. Written.
    Create { value: String },
    /// The stored value moves along the declared direction. Written.
    Advance { from: String, to: String },
    /// The stored value already says exactly this. Nothing written, so the run
    /// leaves no commit in the vault's git history.
    Unchanged { value: String },
    /// The note is ahead of the Journal in the declared direction: it knows
    /// something the Journal does not. **The recorded conflict.** Never written.
    Behind { stored: String, computed: String },
    /// The note's shape, or the stored value, is something this writer will not
    /// touch. Never written.
    Refused { reason: String },
    /// The Journal names this person on no dated note, so there is nothing to
    /// write. Distinct from `Refused`: nothing is wrong.
    NotComputed,
}

impl Verdict {
    /// Whether this verdict causes a write. The one place the question is
    /// answered, so the dry run and the apply cannot disagree about it.
    pub fn writes(&self) -> bool {
        matches!(self, Verdict::Create { .. } | Verdict::Advance { .. })
    }

    /// The value that would land on the note, when one would.
    pub fn value(&self) -> Option<&str> {
        match self {
            Verdict::Create { value } => Some(value),
            Verdict::Advance { to, .. } => Some(to),
            _ => None,
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Create { value } => write!(f, "create {value}"),
            Verdict::Advance { from, to } => write!(f, "advance {from} -> {to}"),
            Verdict::Unchanged { value } => write!(f, "unchanged {value}"),
            Verdict::Behind { stored, computed } => {
                write!(f, "CONFLICT note has {stored}, Journal has {computed}")
            }
            Verdict::Refused { reason } => write!(f, "REFUSED {reason}"),
            Verdict::NotComputed => write!(f, "the Journal names no dated note"),
        }
    }
}

/// One planned decision: a note, a key, and what would happen to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Planned {
    pub id: String,
    pub key: &'static str,
    #[serde(flatten)]
    pub verdict: Verdict,
}

/// The `lost_touch` view of the vault, which is the number this writer exists
/// to correct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LostTouch {
    /// Notes carrying a readable `last_contact`.
    pub carrying: usize,
    /// Of those, the ones `People.base` draws in its "Lost Touch" view.
    pub lost: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FieldsPlan {
    /// The date the `lost_touch` counts were taken against.
    pub today: String,
    pub notes: usize,
    pub create: usize,
    pub advance: usize,
    pub unchanged: usize,
    pub behind: usize,
    pub refused: usize,
    pub not_computed: usize,
    /// `lost_touch` as the vault stands now.
    pub before: LostTouch,
    /// `lost_touch` as it would stand once every writing verdict landed.
    pub after: LostTouch,
    /// Every decision except `Unchanged` and `NotComputed`, which are the two
    /// that say nothing happened and nothing is wrong.
    pub items: Vec<Planned>,
}

/// A stored or computed scalar read as a plain `YYYY-MM-DD` day, or `None`.
///
/// Stricter than `people::report`'s comparison on purpose. That one folds
/// brackets and a trailing time away so it can *report* a disagreement over a
/// wikilinked date; this one decides whether to *write*, and a value it cannot
/// read exactly is a value it must not replace.
fn day(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    raw.chars()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
        .then_some(raw)
}

/// The verdict for one key on one note, from the note's own bytes.
///
/// Reads the file text rather than `Note::fields`, because the parsed map has
/// already dropped the quoting and flattened lists, and the shapes it flattens
/// are exactly the ones this must refuse.
pub fn verdict_for(text: &str, owned: &OwnedKey, computed: Option<&str>) -> Verdict {
    let Some(computed) = computed else {
        return Verdict::NotComputed;
    };
    let Some(computed) = day(computed) else {
        return Verdict::Refused {
            reason: format!("the computed value `{computed}` is not a YYYY-MM-DD day"),
        };
    };

    let stored = match find_field(text, owned.key) {
        Ok(found) => found,
        Err(FieldError::NoFrontmatter) => {
            return Verdict::Refused {
                reason: "the note has no frontmatter block".to_string(),
            }
        }
        Err(e) => {
            return Verdict::Refused {
                reason: e.to_string(),
            }
        }
    };

    let Some(stored) = stored else {
        return Verdict::Create {
            value: computed.to_string(),
        };
    };

    let Some(stored_day) = day(&stored.value) else {
        return Verdict::Refused {
            reason: format!(
                "line {}: the stored value `{}` is not a YYYY-MM-DD day",
                stored.line, stored.value
            ),
        };
    };

    if stored_day == computed {
        return Verdict::Unchanged {
            value: computed.to_string(),
        };
    }
    if owned.direction.advances(stored_day, computed) {
        return Verdict::Advance {
            from: stored_day.to_string(),
            to: computed.to_string(),
        };
    }
    Verdict::Behind {
        stored: stored_day.to_string(),
        computed: computed.to_string(),
    }
}

/// What `people.rs` computed for this key on this person.
fn computed_for<'a>(facts: &'a people::PersonFacts, key: &str) -> Option<&'a str> {
    match key {
        "last_contact" => facts.last_contact.as_deref(),
        "met_at" => facts.met_at.as_deref(),
        // Unreachable while OWNED holds only the two keys above. A third key
        // added without a computed source plans `NotComputed` and writes nothing.
        _ => None,
    }
}

/// Plan every owned key over every note, writing nothing.
///
/// `today` is passed in rather than read, so the `lost_touch` counts are
/// reproducible and a test can pin the date.
pub fn plan(notes: &[Note], report: &PeopleReport, today: &str) -> FieldsPlan {
    let by_id: HashMap<&str, &Note> = notes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut out = FieldsPlan {
        today: today.to_string(),
        notes: report.people,
        ..FieldsPlan::default()
    };

    // `last_contact` before and after the plan lands, per note, so the
    // `lost_touch` count is taken over the same set both times.
    let mut projected: Vec<(Option<String>, Option<String>)> = Vec::new();

    for facts in &report.facts {
        let Some(note) = by_id.get(facts.id.as_str()) else {
            continue;
        };
        let mut stored_contact = find_field(&note.text, "last_contact")
            .ok()
            .flatten()
            .and_then(|f| day(&f.value).map(str::to_string));
        let before_contact = stored_contact.clone();

        for owned in OWNED {
            if !facts.id.starts_with(&format!("{}/", owned.folder)) {
                continue;
            }
            let verdict = verdict_for(&note.text, owned, computed_for(facts, owned.key));

            match &verdict {
                Verdict::Create { .. } => out.create += 1,
                Verdict::Advance { .. } => out.advance += 1,
                Verdict::Unchanged { .. } => out.unchanged += 1,
                Verdict::Behind { .. } => out.behind += 1,
                Verdict::Refused { .. } => out.refused += 1,
                Verdict::NotComputed => out.not_computed += 1,
            }

            if owned.key == "last_contact" {
                if let Some(value) = verdict.value() {
                    stored_contact = Some(value.to_string());
                }
            }

            if !matches!(verdict, Verdict::Unchanged { .. } | Verdict::NotComputed) {
                out.items.push(Planned {
                    id: facts.id.clone(),
                    key: owned.key,
                    verdict,
                });
            }
        }

        projected.push((before_contact, stored_contact));
    }

    out.before = lost_touch(projected.iter().filter_map(|(b, _)| b.as_deref()), today);
    out.after = lost_touch(projected.iter().filter_map(|(_, a)| a.as_deref()), today);
    out
}

/// What an apply actually did, as opposed to what the plan said it would.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Applied {
    /// The base URL the writes went to, so the report can say where.
    pub through: String,
    /// Notes stored with a `PUT`.
    pub notes_written: usize,
    /// Keys changed across those notes. One `PUT` carries every key planned
    /// for its note.
    pub keys_written: usize,
    /// Of the notes written, the ones whose bytes read back afterwards are
    /// exactly the bytes sent: the original note with only the planned scalars
    /// changed.
    pub verified: usize,
    /// Notes not written, each with the note and the reason.
    pub refused: Vec<String>,
    /// Notes written whose bytes read back differ from the bytes sent, or that
    /// could not be read back. Any entry here makes the run fail.
    pub diverged: Vec<String>,
}

/// Carry out the writing verdicts in `plan`, one note at a time.
///
/// Per note, every planned key is spliced into one buffer and stored with one
/// `PUT`, bracketed by two reads:
///
/// 1. **Before.** Read the note through the API and require it to be
///    byte-identical to what the walk read. That one comparison catches a note
///    edited since the plan, and a `--root` naming a *different vault* from the
///    one Obsidian has open. Identical bytes give an identical verdict, so the
///    plan still holds.
/// 2. **Splice.** `markdown_root::set_field` per key. Every byte outside the
///    rewritten scalars is copied from the read, so the note differs from its
///    previous self only on the target lines.
/// 3. **After.** Read again and require exactly the bytes sent. Anything else
///    is `diverged` and fails the run.
///
/// The Local REST API has no precondition on `PUT` (`obsidian.rs`), so an edit
/// that lands between step 1 and the `PUT` on the same note is not detected.
pub fn apply(api: &crate::obsidian::Obsidian, notes: &[Note], plan: &FieldsPlan) -> Applied {
    let mut out = Applied {
        through: api.base().to_string(),
        ..Applied::default()
    };
    let by_id: HashMap<&str, &Note> = notes.iter().map(|n| (n.id.as_str(), n)).collect();

    // The writing items grouped per note, in plan order.
    let mut order: Vec<&str> = Vec::new();
    let mut per_note: HashMap<&str, Vec<(&'static str, &str)>> = HashMap::new();
    for item in plan.items.iter().filter(|i| i.verdict.writes()) {
        let Some(value) = item.verdict.value() else {
            continue;
        };
        let entry = per_note.entry(item.id.as_str()).or_default();
        if entry.is_empty() {
            order.push(item.id.as_str());
        }
        entry.push((item.key, value));
    }

    for id in order {
        let keys = &per_note[id];
        let names = keys.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ");
        let Some(walked) = by_id.get(id).map(|n| n.text.as_str()) else {
            out.refused
                .push(format!("{id}: not among the notes that were walked"));
            continue;
        };

        let before = match api.read(id) {
            Ok(text) => text,
            Err(e) => {
                out.refused.push(e);
                continue;
            }
        };
        if before != walked {
            out.refused.push(format!(
                "{id} ({names}): the note Obsidian serves is not the one that was read \
                 ({} bytes against {}). Either it changed since the plan, or the Local \
                 REST API has a different vault open than --root names. Nothing written.",
                before.len(),
                walked.len()
            ));
            continue;
        }

        let planned = match splice(&before, keys) {
            Ok(text) => text,
            Err(e) => {
                out.refused.push(format!("{id}: {e}"));
                continue;
            }
        };
        if planned == before {
            // Every key already held its value; the plan said otherwise only if
            // the note changed, which the comparison above rules out.
            continue;
        }

        if let Err(e) = api.write(id, &planned) {
            out.refused.push(e);
            continue;
        }
        out.notes_written += 1;
        out.keys_written += keys.len();

        match api.read(id) {
            Ok(after) if after == planned => out.verified += 1,
            Ok(after) => out.diverged.push(format!(
                "{id} ({names}): the note read back is not the note sent \
                 ({} bytes sent, {} read back); first difference at line {}",
                planned.len(),
                after.len(),
                first_different_line(&planned, &after)
            )),
            Err(e) => out
                .diverged
                .push(format!("{id} ({names}): written, but not re-readable: {e}")),
        }
    }

    out
}

/// Apply `set_field` for every key in turn. Pure, so the byte-stability claim
/// can be tested without a server.
pub fn splice(text: &str, keys: &[(&str, &str)]) -> Result<String, String> {
    let mut current = text.to_string();
    for (key, value) in keys {
        current = markdown_root::set_field(&current, key, value)
            .map(|(next, _)| next)
            .map_err(|e| format!("`{key}`: {e}"))?;
    }
    Ok(current)
}

/// The 1-based number of the first line where two documents differ.
fn first_different_line(a: &str, b: &str) -> usize {
    let mut a_lines = a.split('\n');
    let mut b_lines = b.split('\n');
    let mut n = 1;
    loop {
        match (a_lines.next(), b_lines.next()) {
            (Some(x), Some(y)) if x == y => n += 1,
            _ => return n,
        }
    }
}

/// `People.base`'s "Lost Touch" view, counted over a set of `last_contact` days.
pub fn lost_touch<'a>(days: impl Iterator<Item = &'a str>, today: &str) -> LostTouch {
    let Some(now) = civil_date::unix_day_of_iso(today) else {
        return LostTouch::default();
    };
    let mut out = LostTouch::default();
    for value in days {
        let Some(then) = civil_date::unix_day_of_iso(value) else {
            continue;
        };
        out.carrying += 1;
        if now - then > LOST_TOUCH_DAYS {
            out.lost += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsidian::{mock, Obsidian};

    const LAST_CONTACT: &OwnedKey = &OWNED[0];
    const MET_AT: &OwnedKey = &OWNED[1];

    fn note(fields: &str) -> String {
        format!("---\ntype: person\n{fields}---\n\n# A person\n")
    }

    #[test]
    fn the_contract_only_covers_keys_a_base_reads() {
        assert!(!OWNED.iter().any(|o| o.key == "mention_count"));
        assert_eq!(OWNED.len(), 2);
        for owned in OWNED {
            assert_eq!(owned.folder, "Atlas/People");
            assert_eq!(owned.read_by, "Resources/Bases/People.base");
        }
    }

    #[test]
    fn a_journal_date_newer_than_the_note_advances_last_contact() {
        let text = note("last_contact: \"2026-02-23\"\n");
        assert_eq!(
            verdict_for(&text, LAST_CONTACT, Some("2026-06-17")),
            Verdict::Advance {
                from: "2026-02-23".into(),
                to: "2026-06-17".into()
            }
        );
    }

    /// The note is ahead of the Journal, which is what contact recorded off the
    /// Journal looks like from here. The writer must refuse, not "fix" it.
    #[test]
    fn a_note_ahead_of_the_journal_is_a_recorded_conflict_and_never_written() {
        let text = note("last_contact: \"2026-07-01\"\n");
        let verdict = verdict_for(&text, LAST_CONTACT, Some("2026-06-17"));
        assert_eq!(
            verdict,
            Verdict::Behind {
                stored: "2026-07-01".into(),
                computed: "2026-06-17".into()
            }
        );
        assert!(!verdict.writes(), "a conflict must not produce a write");
        assert_eq!(verdict.value(), None);
    }

    /// `met_at` is the mirror: earlier is the advance, later is the conflict.
    #[test]
    fn met_at_moves_the_other_way() {
        let text = note("met_at: \"2024-08-10\"\n");
        assert!(matches!(
            verdict_for(&text, MET_AT, Some("2023-01-05")),
            Verdict::Advance { .. }
        ));
        assert!(matches!(
            verdict_for(&text, MET_AT, Some("2025-01-05")),
            Verdict::Behind { .. }
        ));
    }

    #[test]
    fn an_absent_key_is_created_and_an_equal_one_is_left_alone() {
        assert_eq!(
            verdict_for(&note(""), LAST_CONTACT, Some("2026-06-17")),
            Verdict::Create {
                value: "2026-06-17".into()
            }
        );
        let text = note("last_contact: \"2026-06-17\"\n");
        let verdict = verdict_for(&text, LAST_CONTACT, Some("2026-06-17"));
        assert!(!verdict.writes());
        assert!(matches!(verdict, Verdict::Unchanged { .. }));
    }

    #[test]
    fn nothing_computed_is_not_the_same_answer_as_something_refused() {
        assert_eq!(
            verdict_for(&note(""), LAST_CONTACT, None),
            Verdict::NotComputed
        );
    }

    /// Every shape `markdown_root::fields` refuses arrives here as a refusal
    /// with the reason on it, and none of them writes.
    #[test]
    fn a_shape_the_writer_cannot_read_is_refused_by_name() {
        for fields in [
            "last_contact:\n  - \"2026-01-01\"\n",
            "last_contact: [2026-01-01]\n",
            "last_contact: 2026-01-01 # from memory\n",
            "last_contact: \"2026-01-01\"\nlast_contact: \"2025-01-01\"\n",
            "last_contact: yesterday\n",
            "last_contact: \"[[2026-01-01]]\"\n",
        ] {
            let verdict = verdict_for(&note(fields), LAST_CONTACT, Some("2026-06-17"));
            assert!(
                matches!(verdict, Verdict::Refused { .. }),
                "{fields:?} gave {verdict:?}"
            );
            assert!(!verdict.writes());
        }
        assert!(matches!(
            verdict_for("no frontmatter here\n", LAST_CONTACT, Some("2026-06-17")),
            Verdict::Refused { .. }
        ));
    }

    /// The safety property the monotone rule buys: no verdict that writes can
    /// push a note further from today, so no note that is in touch can be made
    /// lost.
    #[test]
    fn never_lengthens_a_gap() {
        let today = "2026-09-09";
        for stored in ["2026-09-01", "2026-01-01", "2020-05-05"] {
            for computed in ["2026-09-08", "2026-06-17", "2019-01-01"] {
                let text = note(&format!("last_contact: \"{stored}\"\n"));
                let verdict = verdict_for(&text, LAST_CONTACT, Some(computed));
                let Some(landed) = verdict.value() else {
                    continue;
                };
                let before = lost_touch(std::iter::once(stored), today);
                let after = lost_touch(std::iter::once(landed), today);
                assert!(
                    after.lost <= before.lost,
                    "{stored} -> {landed} moved a note INTO lost_touch"
                );
            }
        }
    }

    #[test]
    fn lost_touch_counts_the_view_the_base_draws() {
        // People.base: `days_since_contact > 90`, strictly greater.
        let today = "2026-09-09";
        // 2026-06-11 is exactly 90 days back, so it is not lost.
        assert_eq!(
            lost_touch(
                ["2026-06-11", "2026-06-10", "not-a-date"].into_iter(),
                today
            ),
            LostTouch {
                carrying: 2,
                lost: 1
            }
        );
    }

    /// A synthetic People note with the shapes a YAML serialiser rewrites:
    /// quoted and single-quoted neighbours, a comment line, a trailing comment
    /// on another key, hand-aligned values, and a `---` in the body.
    fn awkward_note() -> String {
        [
            "---",
            "# kept by hand",
            "type: person",
            "aliases: ['Ida', \"I. M.\"]",
            "met_at:       \"2024-01-05\"",
            "last_contact: \"2026-02-23\"",
            "company: 'Example GmbH'   # since spring",
            "---",
            "",
            "# Ida Muster",
            "",
            "---",
            "",
            "Prose a human wrote, with a \"quote\".",
            "",
        ]
        .join("\n")
    }

    /// The byte-stability claim for two keys in one `PUT`: the note differs
    /// from its previous self only on the two target lines.
    #[test]
    fn splicing_two_keys_changes_exactly_their_two_lines() {
        let before = awkward_note();
        let after = splice(
            &before,
            &[("last_contact", "2026-06-17"), ("met_at", "2023-12-24")],
        )
        .expect("splice");
        let changed: Vec<(&str, &str)> = before
            .lines()
            .zip(after.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(
            changed,
            vec![
                (
                    "met_at:       \"2024-01-05\"",
                    "met_at:       \"2023-12-24\""
                ),
                (
                    "last_contact: \"2026-02-23\"",
                    "last_contact: \"2026-06-17\""
                ),
            ]
        );
        assert_eq!(before.len(), after.len());
    }

    /// A throwaway vault under the OS temp dir, removed on drop.
    struct Vault(std::path::PathBuf);

    impl Vault {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("sjel-vault-fields-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("Atlas/People")).expect("people");
            std::fs::create_dir_all(dir.join("Journal")).expect("journal");
            Vault(dir)
        }

        fn write(&self, relative: &str, text: &str) {
            std::fs::write(self.0.join(relative), text).expect("write");
        }

        fn person(&self, name: &str, fields: &str) {
            self.write(
                &format!("Atlas/People/{name}.md"),
                &format!("---\ntype: person\n{fields}---\n\n# {name}\n\nProse a human wrote.\n"),
            );
        }

        fn load(&self) -> Vec<Note> {
            let root = markdown_root::MarkdownRoot::declare(&self.0).expect("root");
            let (notes, problems) = crate::note::load_all(&root).expect("load");
            assert!(problems.is_empty(), "{problems:?}");
            notes
        }
    }

    impl Drop for Vault {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The gate, watched over planted inputs. Every "bad" row is a shape a
    /// naive writer would have overwritten.
    #[test]
    fn every_planted_shape_gets_the_verdict_it_earns() {
        let vault = Vault::new("planted");
        let seen = [
            "Stale Note",
            "Behind Human",
            "Blank Note",
            "Agrees Already",
            "List Shaped",
            "Commented",
            "Twice Over",
            "Not A Date",
            "No Frontmatter",
        ];
        let links: Vec<String> = seen.iter().map(|n| format!("[[{n}]]")).collect();
        for date in ["2024-01-05", "2026-06-17"] {
            vault.write(
                &format!("Journal/{date}.md"),
                &format!("---\ntype: journal\n---\n\nSaw {}.\n", links.join(", ")),
            );
        }

        vault.person(
            "Stale Note",
            "last_contact: \"2026-02-23\"\nmet_at: \"2024-01-05\"\n",
        );
        vault.person(
            "Behind Human",
            "last_contact: \"2026-08-30\"\nmet_at: \"2024-01-05\"\n",
        );
        vault.person("Blank Note", "summary: \"no dates yet\"\n");
        vault.person(
            "Agrees Already",
            "last_contact: \"2026-06-17\"\nmet_at: \"2024-01-05\"\n",
        );
        vault.person(
            "List Shaped",
            "last_contact:\n  - \"2026-02-23\"\nmet_at: \"2024-01-05\"\n",
        );
        vault.person(
            "Commented",
            "last_contact: 2026-02-23 # from memory\nmet_at: \"2024-01-05\"\n",
        );
        vault.person(
            "Twice Over",
            "last_contact: \"2026-02-23\"\nmet_at: \"2024-01-05\"\nlast_contact: \"2025-01-01\"\n",
        );
        vault.person(
            "Not A Date",
            "last_contact: sometime last spring\nmet_at: \"2024-01-05\"\n",
        );
        vault.write(
            "Atlas/People/No Frontmatter.md",
            "Just prose about a person, with no properties at all.\n",
        );

        let notes = vault.load();
        let plan = plan(&notes, &people::report(&notes), "2026-09-09");

        let verdict = |id: &str, key: &str| {
            plan.items
                .iter()
                .find(|i| i.id == id && i.key == key)
                .map(|i| i.verdict.to_string())
        };

        assert_eq!(
            verdict("Atlas/People/Stale Note.md", "last_contact").as_deref(),
            Some("advance 2026-02-23 -> 2026-06-17")
        );
        assert_eq!(
            verdict("Atlas/People/Behind Human.md", "last_contact").as_deref(),
            Some("CONFLICT note has 2026-08-30, Journal has 2026-06-17")
        );
        assert_eq!(
            verdict("Atlas/People/Blank Note.md", "last_contact").as_deref(),
            Some("create 2026-06-17")
        );
        // Equal on both keys, so it never reaches the item list at all.
        assert_eq!(
            verdict("Atlas/People/Agrees Already.md", "last_contact"),
            None
        );
        for (name, fragment) in [
            ("List Shaped", "may open a list"),
            ("Commented", "followed by a comment"),
            ("Twice Over", "appears twice"),
            ("Not A Date", "is not a YYYY-MM-DD day"),
            ("No Frontmatter", "no frontmatter block"),
        ] {
            let said = verdict(&format!("Atlas/People/{name}.md"), "last_contact")
                .unwrap_or_else(|| panic!("{name} produced no verdict"));
            assert!(said.starts_with("REFUSED"), "{name}: {said}");
            assert!(said.contains(fragment), "{name}: {said}");
        }

        assert_eq!(
            (plan.create, plan.advance, plan.behind, plan.refused),
            (2, 1, 1, 6),
            "one create per key on Blank Note, one advance, one conflict, six refusals"
        );
        // The only note the plan moves out of the Lost Touch view is the one it
        // wrote.
        assert_eq!(
            plan.before,
            LostTouch {
                carrying: 3,
                lost: 1
            }
        );
        assert_eq!(
            plan.after,
            LostTouch {
                carrying: 4,
                lost: 0
            }
        );
    }

    /// A vault with one awkward note that needs both keys moved, and one note
    /// that is ahead of the Journal. Served from the loopback mock with the
    /// walked bytes, so `apply` sees the same vault the plan read.
    fn served_vault(name: &str) -> (Vault, Vec<Note>, FieldsPlan, mock::Server) {
        let vault = Vault::new(name);
        for date in ["2023-12-24", "2026-06-17"] {
            vault.write(
                &format!("Journal/{date}.md"),
                "---\ntype: journal\n---\n\nSaw [[Ida Muster]] and [[Ben Beispiel]].\n",
            );
        }
        vault.write("Atlas/People/Ida Muster.md", &awkward_note());
        vault.person(
            "Ben Beispiel",
            "last_contact: \"2026-08-30\"\nmet_at: \"2023-12-24\"\n",
        );
        let notes = vault.load();
        let plan = plan(&notes, &people::report(&notes), "2026-09-09");

        let server = mock::Server::start();
        for n in &notes {
            server.put_note(&n.id, &n.text);
        }
        (vault, notes, plan, server)
    }

    /// The end-to-end byte-stability proof: after `apply` against the mock
    /// Local REST API, the stored note differs from the original only on the
    /// two target lines. The conflict note is untouched, and the only write
    /// call is a raw `PUT`.
    #[test]
    fn apply_writes_only_the_target_lines_and_leaves_a_conflict_alone() {
        let (_vault, notes, plan, server) = served_vault("apply");
        let api = Obsidian::connect_to(&server.base, mock::KEY.into()).expect("connect");

        let applied = apply(&api, &notes, &plan);
        assert!(applied.refused.is_empty(), "{:?}", applied.refused);
        assert!(applied.diverged.is_empty(), "{:?}", applied.diverged);
        assert_eq!(
            (
                applied.notes_written,
                applied.keys_written,
                applied.verified
            ),
            (1, 2, 1)
        );

        let original = awkward_note();
        let stored = server.note("Atlas/People/Ida Muster.md").expect("stored");
        let changed: Vec<(&str, &str)> = original
            .lines()
            .zip(stored.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(
            changed,
            vec![
                (
                    "met_at:       \"2024-01-05\"",
                    "met_at:       \"2023-12-24\""
                ),
                (
                    "last_contact: \"2026-02-23\"",
                    "last_contact: \"2026-06-17\""
                ),
            ]
        );
        assert_eq!(original.len(), stored.len());

        let ben = notes
            .iter()
            .find(|n| n.id == "Atlas/People/Ben Beispiel.md")
            .expect("ben");
        assert_eq!(
            server.note("Atlas/People/Ben Beispiel.md").as_deref(),
            Some(ben.text.as_str()),
            "a recorded conflict is never written"
        );

        let log = server.log();
        let writes: Vec<&String> = log.iter().filter(|l| !l.starts_with("GET")).collect();
        assert_eq!(
            writes,
            vec!["PUT /vault/Atlas/People/Ida%20Muster.md application/octet-stream"],
            "{log:?}"
        );
    }

    /// A note edited between the walk and the write is refused, and nothing
    /// is stored over the edit.
    #[test]
    fn a_note_that_moved_since_the_plan_is_refused_and_not_written() {
        let (_vault, notes, plan, server) = served_vault("moved");
        let edited = awkward_note().replace("Prose a human", "Prose the human");
        server.put_note("Atlas/People/Ida Muster.md", &edited);
        let api = Obsidian::connect_to(&server.base, mock::KEY.into()).expect("connect");

        let applied = apply(&api, &notes, &plan);
        assert_eq!(applied.notes_written, 0);
        assert_eq!(applied.refused.len(), 1, "{:?}", applied.refused);
        assert!(applied.refused[0].contains("Nothing written"));
        assert_eq!(
            server.note("Atlas/People/Ida Muster.md").as_deref(),
            Some(edited.as_str())
        );
        assert!(!server.log().iter().any(|l| l.starts_with("PUT")));
    }

    /// If the server stored anything but the bytes sent (here, a serialiser
    /// that drops double quotes), the read-back names it as a divergence.
    #[test]
    fn a_server_that_respells_the_note_is_reported_as_diverged() {
        let (_vault, notes, plan, server) = served_vault("respelt");
        server.strip_quotes_on_put();
        let api = Obsidian::connect_to(&server.base, mock::KEY.into()).expect("connect");

        let applied = apply(&api, &notes, &plan);
        assert_eq!(applied.notes_written, 1);
        assert_eq!(applied.verified, 0);
        assert_eq!(applied.diverged.len(), 1, "{:?}", applied.diverged);
        assert!(
            applied.diverged[0].contains("not the note sent"),
            "{}",
            applied.diverged[0]
        );
    }
}
