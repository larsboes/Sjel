//! The Journal's six health keys, counted honestly and produced where a source exists.
//!
//! PRD **Q106** (2026-09-09) asked for "a producer, probably" for `exercise`, `social`,
//! `sleep_quality`, `energy`, `mood` and `learning_hours`. **One of the six has a source and
//! five do not.** This module is that answer as an instrument rather than a claim.
//!
//! ## Which of the six can have a producer
//!
//! | Key | Producer | Why |
//! |---|---|---|
//! | `social` | `Journal/` person links | a day that links an `Atlas/People` note names who was in it |
//! | `sleep_quality` | none | Apple Health is not readable from the host — see below |
//! | `exercise` | none | Same store, same refusal |
//! | `learning_hours` | none | Nothing on the host measures hours spent learning |
//! | `energy` | none | A self-report. No machine holds the fact. |
//! | `mood` | none | A self-report. No machine holds the fact. |
//!
//! **Apple Health is not a source on a Mac, and that was measured rather than assumed.**
//! `HealthKit.framework` ships in macOS and its headers carry `API_AVAILABLE(… macos(13.0))`,
//! so the API compiles and a reader could be forgiven for stopping there. A short Swift probe on
//! a current macOS host answers `HKHealthStore.isHealthDataAvailable() == false`, and
//! `~/Library/Health` does not exist. There is no store to read, with or without a phone. Any
//! `sleep_quality` this capability emitted would be invented, so it emits none.
//!
//! **`social` is evidence and it is labelled as evidence.** The rule is the one Q102 has
//! already ratified for `last_contact`: a `Journal/` note that links a person is the record of
//! contact with them. `people.rs` reads that per person; this reads the same links per day. It
//! is a proxy — a day can name someone it did not meet — so the people are served beside the
//! boolean and the stored value is served beside both, exactly as `/api/people` serves `stored`
//! and `disagrees`.
//!
//! ## Nothing here writes
//!
//! Q102 opened the vault to Sjel for two People keys under their existing names. It did not
//! open the Journal, and this module is not the stream that owns the write path. Every number
//! below is recomputed off the notes on request and stored nowhere. What it would take to write
//! `social` later is in the capability README.
//!
//! ## A trailing comment is not a value
//!
//! `markdown_root::parse_fields` reads `mood:                   # 1-5` as the value `# 1-5`,
//! because it splits on the first colon and keeps the rest. That is correct for `parse_fields`
//! and wrong for this census: strip a `#` there and `color: #fff` loses its value, so the fix
//! does not belong in the shared parser. It belongs here, over `Note::raw_frontmatter`, which
//! this crate keeps for precisely this class of question (see `lint.rs`).
//!
//! The difference is not cosmetic. The daily template writes a comment after four of the six
//! keys, so every note stamped from it and left unfilled carries those four lines with a
//! comment and no value. A comment-blind read counts each of them as filled, and on a vault
//! where few days are filled in by hand that can multiply the apparent coverage many times
//! over. No key in `lint::TRACKED` carries a trailing comment, so `lint`'s numbers are
//! unaffected; this is a trap set for the next reader of these six.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::note::Note;
use crate::people;

/// Where the days live. The whole `Journal/` folder also holds weekly, monthly and yearly
/// reviews, and none of those carries a health key — scoping to the daily folder is what makes
/// `present` a denominator rather than a mixture.
///
/// The folder is named the same way `Resources/Bases/Habits.base` names it
/// (`file.inFolder("Journal/01. Daily Notes")`), because two spellings of one folder is how two
/// surfaces start answering differently about the same days.
pub const DAILY: &str = "Journal/01. Daily Notes";

/// The six keys, in the order `Resources/Templates/Daily Journal Template.md` writes them.
///
/// They are not invented here. `Resources/Bases/Habits.base` draws all six as columns and
/// charts five of them, and `Resources/Bases/Journal.base` reads all six in its `habits_done`
/// and `sleep_category` formulas. Under Q102's rule — a producer may write no key a `.base`
/// view does not already read — all six are writable keys and only one has anything to write.
pub const HEALTH_KEYS: [&str; 6] = [
    "exercise",
    "social",
    "sleep_quality",
    "energy",
    "mood",
    "learning_hours",
];

/// What the template puts in a fresh note, per key, where that is not simply nothing.
///
/// Source: `Resources/Templates/Daily Journal Template.md`, which writes `exercise: false` and
/// `social: false`. YAML reads both as values, so a census that counted "has a value" would
/// report every day as filled for two keys nobody may ever have filled in. A day equal to the template's own
/// default was never a statement about that day, and `asserted` is the count that says so.
const TEMPLATE_DEFAULT: [(&str, &str); 2] = [("exercise", "false"), ("social", "false")];

/// Where a key's value could come from, if anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Producer {
    /// Derivable from `Journal/` person links, the same evidence Q102 ratified for
    /// `last_contact`.
    JournalLinks,
    /// A fact only the operator holds. No machine can produce it and none should try.
    SelfReport,
    /// Apple Health would hold it and this machine cannot read Apple Health.
    HealthStoreUnreachable,
    /// Nothing on this machine measures the quantity at all.
    Unmeasured,
}

impl Producer {
    /// The sentence that goes beside the verdict, so a reader gets the reason with the number.
    pub const fn reason(self) -> &'static str {
        match self {
            Producer::JournalLinks => {
                "a Journal day that links an Atlas/People note is the record of contact, \
                 the rule Q102 ratified for last_contact"
            }
            Producer::SelfReport => {
                "a self-report; no machine holds it, so the honest options are the human or \
                 the delete key"
            }
            Producer::HealthStoreUnreachable => {
                "HKHealthStore.isHealthDataAvailable() is false on macOS and ~/Library/Health \
                 does not exist"
            }
            Producer::Unmeasured => "nothing on this machine measures it",
        }
    }
}

/// Which producer answers for a key, decided once so the CLI, the server and the README cannot
/// disagree about it.
fn producer_for(key: &str) -> Producer {
    match key {
        "social" => Producer::JournalLinks,
        "exercise" | "sleep_quality" => Producer::HealthStoreUnreachable,
        "energy" | "mood" => Producer::SelfReport,
        _ => Producer::Unmeasured,
    }
}

/// One key across the daily notes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyCensus {
    pub key: String,
    /// Days whose frontmatter carries the key line at all.
    pub present: usize,
    /// Days where nothing survives comment-stripping.
    pub blank: usize,
    /// Days where the only thing after the colon was a YAML comment. A subset of `blank`, and
    /// the exact population a comment-blind reader counts as filled.
    pub comment_only: usize,
    /// Days carrying a value that is neither blank nor the template's own default. The number
    /// that answers "how many days did a human say something".
    pub asserted: usize,
    /// Every non-blank value and how often it appears. Small by construction, and it is what
    /// makes `asserted` checkable instead of trusted: if the template's default ever changes,
    /// the histogram shows it rather than the count quietly moving.
    pub values: BTreeMap<String, usize>,
    pub producer: Producer,
    pub producer_reason: &'static str,
}

/// One day, and what the Journal itself says about who was in it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DayFacts {
    /// Vault-relative path. The identity that survives a machine.
    pub id: String,
    /// `YYYY-MM-DD`, from the filename and never from a frontmatter key.
    ///
    /// **Not a key.** The date is the first ten characters of the filename, so an iCloud
    /// conflict copy shares it with the note it copied: `2031-03-14 2.md` beside
    /// `2031-03-14.md` is two notes on one date. A consumer that maps date to entry drops one of
    /// the two without saying so.
    pub date: String,
    /// `Atlas/People` notes this day links, by note name. Served beside the boolean because a
    /// derived `true` with no evidence is an assertion.
    pub people: Vec<String>,
    /// The computed value: this day named at least one person in the register.
    pub social: bool,
    /// What the note itself carries, where it carries something.
    pub stated_social: Option<String>,
    /// The stored value is the template's own `false`, so nobody ever said it.
    pub stated_is_default: bool,
    /// A human wrote a value and the links contradict it. Never true for a template default:
    /// see `SocialSummary::unfilled_with_evidence`.
    pub disagrees: bool,
}

/// The one key that has a producer, summarised.
///
/// Four populations, not one accuracy figure. The split matters because
/// `social: false` on an untouched note and `social: false` typed by someone who spent the day
/// alone are the same bytes, and an instrument that called both a disagreement would bury the
/// few real contradictions under every day nobody filled in.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct SocialSummary {
    /// Days linking at least one `Atlas/People` note.
    pub days_with_person_link: usize,
    /// Days whose stored `social` is `true`.
    pub stated_true: usize,
    /// Days where a human wrote a value and the links say the same thing.
    pub agrees: usize,
    /// Days where a human wrote a value and the links say otherwise.
    pub disagrees: usize,
    /// Days still carrying the template's `false` where the Journal names somebody. Nothing is
    /// contradicted here — these are the rows a producer would have something to add to.
    pub unfilled_with_evidence: usize,
    /// Distinct people the daily notes name.
    pub people_named: usize,
}

/// A frontmatter value that is still a template expression.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Unrendered {
    pub id: String,
    pub key: String,
    /// The line's value as it sits on disk. Reported, never rewritten: §5.5 is one-way and this
    /// module has no write path.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct JournalReport {
    /// Notes under `Journal/01. Daily Notes/` whose filename is a date. Notes, not dates: see
    /// `DayFacts::date` for the conflict copy that makes two of these one day.
    pub days: usize,
    /// Notes in the folder that are not days, and therefore not counted anywhere else here.
    pub not_a_day: usize,
    /// `Atlas/People` notes the social rule matches against.
    pub people_notes: usize,
    pub keys: Vec<KeyCensus>,
    pub social: SocialSummary,
    pub unrendered: Vec<Unrendered>,
    pub entries: Vec<DayFacts>,
}

/// One key's running counts while the walk is in progress. Named rather than a tuple, so the
/// four `usize` fields cannot be swapped at the call site — `blank` and `comment_only` differ by
/// one line in `report` and a transposition there would be invisible in every test that only
/// checks a total.
#[derive(Default)]
struct Tally {
    present: usize,
    blank: usize,
    comment_only: usize,
    asserted: usize,
    values: BTreeMap<String, usize>,
}

/// A frontmatter scalar with YAML's comment rule applied.
///
/// `#` opens a comment when it starts the value or follows whitespace, and never inside a
/// quoted scalar. That is the whole rule, and it is the difference between counting the days a
/// human rated their mood and counting every note stamped from the template.
///
/// It is deliberately not in `markdown_root::parse_fields`: that parser feeds a mail adapter
/// and a calendar importer as well as this crate, and a value like `#fff` or `#general` is a
/// value there. The narrow reader lives beside the narrow question.
fn scalar(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut quote: Option<u8> = None;
    let mut cut = raw.len();
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(open) => {
                if b == open {
                    quote = None;
                }
            }
            None => {
                if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
                    cut = i;
                    break;
                }
            }
        }
    }
    raw[..cut].trim().trim_matches('"')
}

/// Every day under `DAILY`, in vault order, with the health census and the social evidence.
///
/// Takes the whole note set the way `people::report` does, so the CLI can hand it one walk and
/// the server can hand it two scoped reads without either growing its own idea of which notes
/// are days.
pub fn report(notes: &[Note]) -> JournalReport {
    let prefix = format!("{DAILY}/");
    let register: BTreeSet<String> = notes
        .iter()
        .filter(|note| note.id.starts_with(&format!("{}/", people::FOLDER)))
        .map(|note| note.basename.to_lowercase())
        .collect();

    let mut out = JournalReport {
        people_notes: register.len(),
        ..JournalReport::default()
    };

    let mut census: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut named: BTreeSet<String> = BTreeSet::new();

    for note in notes.iter().filter(|note| note.id.starts_with(&prefix)) {
        let Some(date) = people::journal_date(&note.id) else {
            out.not_a_day += 1;
            continue;
        };
        out.days += 1;

        let block = note.raw_frontmatter.as_deref().unwrap_or_default();

        for key in HEALTH_KEYS {
            let Some(raw) = crate::lint::raw_value(block, key) else {
                continue;
            };
            let entry = census.entry(key).or_default();
            entry.present += 1;
            let value = scalar(raw);
            if value.is_empty() {
                entry.blank += 1;
                if !raw.is_empty() {
                    entry.comment_only += 1;
                }
                continue;
            }
            *entry.values.entry(value.to_string()).or_insert(0) += 1;
            let is_default = TEMPLATE_DEFAULT
                .iter()
                .any(|(k, default)| *k == key && *default == value);
            if !is_default {
                entry.asserted += 1;
            }
        }

        for line in block.lines() {
            let Some((key, raw)) = line.split_once(':') else {
                continue;
            };
            if raw.contains("<%") {
                out.unrendered.push(Unrendered {
                    id: note.id.clone(),
                    key: key.trim().to_string(),
                    value: raw.trim().to_string(),
                });
            }
        }

        let mut people: Vec<String> = people::linked_basenames(note)
            .into_iter()
            .filter(|name| register.contains(name))
            .collect();
        people.sort();
        people.dedup();
        named.extend(people.iter().cloned());

        let social = !people.is_empty();
        let stated_social = crate::lint::raw_value(block, "social")
            .map(scalar)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        // `social: false` is what the template writes, so it is not a claim anybody made. Two
        // things follow. A day still carrying it cannot contradict the links — most untouched
        // days name somebody, and calling those contradictions would bury the few that are. And a
        // `social:` line holding only a comment says nothing at all, which is why the value is
        // read through `scalar` before any of this.
        let stated_is_default = stated_social.as_deref() == Some("false");
        // Only `true` is a claim: `false` is the template's, anything else is not a boolean and
        // is comparable to nothing.
        let claimed = matches!(stated_social.as_deref(), Some("true"));
        let disagrees = claimed && !social;

        if social {
            out.social.days_with_person_link += 1;
        }
        if claimed {
            out.social.stated_true += 1;
            if social {
                out.social.agrees += 1;
            } else {
                out.social.disagrees += 1;
            }
        } else if stated_is_default && social {
            out.social.unfilled_with_evidence += 1;
        }

        out.entries.push(DayFacts {
            id: note.id.clone(),
            date,
            people,
            social,
            stated_is_default,
            stated_social,
            disagrees,
        });
    }

    out.social.people_named = named.len();
    out.entries.sort_by(|a, b| a.date.cmp(&b.date));
    out.unrendered
        .sort_by(|a, b| a.id.cmp(&b.id).then(a.key.cmp(&b.key)));

    out.keys = HEALTH_KEYS
        .iter()
        .map(|key| {
            let tally = census.remove(*key).unwrap_or_default();
            let producer = producer_for(key);
            KeyCensus {
                key: (*key).to_string(),
                present: tally.present,
                blank: tally.blank,
                comment_only: tally.comment_only,
                asserted: tally.asserted,
                values: tally.values,
                producer,
                producer_reason: producer.reason(),
            }
        })
        .collect();

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn note(id: &str, text: &str) -> Note {
        let (fields, body_start, raw) = match markdown_root::frontmatter_spanned(text) {
            Ok(fm) => {
                let raw = fm.block.map(|(s, e)| text[s..e].to_string());
                (fm.fields, fm.body_start, raw)
            }
            Err(_) => (HashMap::new(), 0, None),
        };
        let basename = id
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .trim_end_matches(".md")
            .to_string();
        Note {
            id: id.to_string(),
            path: PathBuf::from(id),
            basename,
            folder: id.split('/').next().unwrap_or("").to_string(),
            fields,
            body_start,
            raw_frontmatter: raw,
            text: text.to_string(),
        }
    }

    fn day(name: &str, text: &str) -> Note {
        note(&format!("{DAILY}/{name}.md"), text)
    }

    fn census_of<'a>(report: &'a JournalReport, key: &str) -> &'a KeyCensus {
        report
            .keys
            .iter()
            .find(|c| c.key == key)
            .expect("every key is in the census")
    }

    /// The planted bad input this module exists for: a key whose only content is the template's
    /// own comment. The crate's parsed map calls that filled; this census calls it empty. Both
    /// halves are asserted, because a test that only checks the new answer cannot show that the
    /// old one was wrong.
    #[test]
    fn a_trailing_comment_is_not_a_value() {
        let planted = day(
            "2031-03-09",
            "---\ntype: journal\nmood:                   # 1-5\n---\nbody\n",
        );
        assert!(
            planted.has("mood"),
            "the comment-blind reader is supposed to be fooled here; if it is not, \
             this test has stopped measuring the gap it was written for"
        );

        let report = report(&[planted]);
        let mood = census_of(&report, "mood");
        assert_eq!(mood.present, 1);
        assert_eq!(mood.blank, 1);
        assert_eq!(mood.comment_only, 1);
        assert_eq!(mood.asserted, 0, "a comment was counted as a mood");
        assert!(mood.values.is_empty());
    }

    /// The other side of the same guard. Over-stripping is the failure that would make this
    /// instrument answer zero for a vault that is fully filled in, which looks identical to
    /// success.
    #[test]
    fn a_hash_inside_a_value_is_still_a_value() {
        let report = report(&[
            day("2031-03-01", "---\nmood: \"#5\"\n---\n"),
            day("2031-03-02", "---\nmood: 5 # felt good\n---\n"),
        ]);
        let mood = census_of(&report, "mood");
        assert_eq!(mood.asserted, 2);
        assert_eq!(mood.values.get("#5"), Some(&1));
        assert_eq!(mood.values.get("5"), Some(&1));
    }

    /// `exercise: false` is what the template writes, not what a day says.
    #[test]
    fn the_template_default_is_not_an_assertion() {
        let report = report(&[
            day("2031-03-01", "---\nexercise: false\nsocial: false\n---\n"),
            day("2031-03-02", "---\nexercise: true\nsocial: false\n---\n"),
        ]);
        let exercise = census_of(&report, "exercise");
        assert_eq!(exercise.present, 2);
        assert_eq!(exercise.blank, 0);
        assert_eq!(
            exercise.asserted, 1,
            "the template's own default was read as a statement about the day"
        );
        assert_eq!(exercise.values.get("false"), Some(&1));
        assert_eq!(census_of(&report, "social").asserted, 0);
    }

    /// The producer itself: the day that names a person, and the day that names a note which is
    /// not a person.
    #[test]
    fn a_day_that_names_a_person_is_social_and_a_day_that_names_a_book_is_not() {
        let notes = vec![
            note("Atlas/People/Ida Muster.md", "---\n---\n"),
            day("2031-03-01", "---\n---\nCoffee with [[Ida Muster]].\n"),
            day("2031-03-02", "---\n---\nRead [[Thinking Fast and Slow]].\n"),
        ];
        let report = report(&notes);
        assert_eq!(report.days, 2);
        assert_eq!(report.people_notes, 1);
        assert_eq!(report.social.days_with_person_link, 1);
        assert_eq!(report.social.people_named, 1);
        assert!(report.entries[0].social);
        assert_eq!(report.entries[0].people, vec!["ida muster".to_string()]);
        assert!(!report.entries[1].social);
        assert!(report.entries[1].people.is_empty());
    }

    /// The drift, served the way `/api/people` serves it — and split three ways, because the
    /// template's `false` is not a claim anybody made. Collapsing these into one "disagrees"
    /// reports every untouched day that names somebody as a contradiction.
    #[test]
    fn a_template_false_is_not_a_contradiction_and_a_typed_true_is() {
        let notes = vec![
            note("Atlas/People/Jan Schmidt.md", "---\n---\n"),
            // Template default, and the day names somebody: a row the producer would fill, not
            // a row that argues with anyone.
            day("2031-03-01", "---\nsocial: false\n---\n[[Jan Schmidt]]\n"),
            // A typed `true` against a day that names nobody: the real contradiction.
            day("2031-03-02", "---\nsocial: true\n---\nAlone all day.\n"),
            // The comment-only line, which says nothing in either direction.
            day(
                "2031-03-03",
                "---\nsocial:                 # unfilled\n---\n[[Jan Schmidt]]\n",
            ),
            // A typed `true` the links back up.
            day("2031-03-04", "---\nsocial: true\n---\n[[Jan Schmidt]]\n"),
        ];
        let report = report(&notes);
        assert_eq!(report.social.disagrees, 1);
        assert_eq!(report.social.agrees, 1);
        assert_eq!(report.social.unfilled_with_evidence, 1);
        assert_eq!(report.social.stated_true, 2);

        assert!(
            !report.entries[0].disagrees,
            "the template's own default was read as a claim that the day was not social"
        );
        assert!(report.entries[0].stated_is_default);
        assert!(report.entries[1].disagrees);
        assert!(
            !report.entries[2].disagrees,
            "a comment-only line was read as a claim"
        );
        assert_eq!(report.entries[2].stated_social, None);
        assert!(!report.entries[3].disagrees);
    }

    /// The `week:` defect, as a class rather than as two note names. A template expression that
    /// reached the vault means the template never ran, and the value it holds is not a link,
    /// a date, or anything else a reader can use.
    #[test]
    fn a_frontmatter_value_that_is_still_a_template_expression_is_reported() {
        let report = report(&[day(
            "2031-03-15",
            "---\nweek: \"[[<% tp.date.now('yyyy-[W]ww') %>]]\"\nmood:\n---\n",
        )]);
        assert_eq!(report.unrendered.len(), 1);
        assert_eq!(report.unrendered[0].key, "week");
        assert!(report.unrendered[0].value.contains("tp.date.now"));
        assert_eq!(report.unrendered[0].id, format!("{DAILY}/2031-03-15.md"));
    }

    /// The same refusal `people.rs` makes: a note in the daily folder whose name is not a date
    /// is not a day, and it is counted as refused rather than dropped.
    #[test]
    fn a_note_in_the_daily_folder_that_is_not_a_date_is_not_a_day() {
        let report = report(&[
            day("2031-03-01", "---\nmood: 5\n---\n"),
            day("Untitled", "---\nmood: 1\n---\n"),
        ]);
        assert_eq!(report.days, 1);
        assert_eq!(report.not_a_day, 1);
        assert_eq!(
            census_of(&report, "mood").asserted,
            1,
            "a note that is not a day contributed to the census"
        );
    }

    /// An iCloud conflict copy is a second note on one date, and this pins what the census
    /// does with it rather than leaving it to be discovered by a writer.
    ///
    /// iCloud writes a copy such as `2031-03-14 2.md` beside `2031-03-14.md`, so `days` counts
    /// both and two entries answer to one date. Deduplicating is a defensible change; making it
    /// without noticing is not, and this test is what makes the difference visible.
    #[test]
    fn a_conflict_copy_is_a_second_note_on_one_date() {
        let report = report(&[
            day("2031-03-14", "---\nmood: 5\n---\n"),
            day("2031-03-14 2", "---\nmood:                   # 1-5\n---\n"),
        ]);
        assert_eq!(report.days, 2);
        assert_eq!(report.not_a_day, 0, "a conflict copy is not refused");
        let dates: Vec<&str> = report.entries.iter().map(|e| e.date.as_str()).collect();
        assert_eq!(dates, vec!["2031-03-14", "2031-03-14"]);
        let mood = census_of(&report, "mood");
        assert_eq!(mood.present, 2, "the copy is in the denominator");
        assert_eq!(mood.asserted, 1);
        assert_eq!(mood.comment_only, 1);
    }

    /// Every key carries its verdict and the reason for it, so no consumer has to hold a second
    /// copy of which of the six can be produced.
    #[test]
    fn each_key_states_its_producer() {
        let report = report(&[day("2031-03-01", "---\n---\n")]);
        assert_eq!(
            census_of(&report, "social").producer,
            Producer::JournalLinks
        );
        assert_eq!(census_of(&report, "mood").producer, Producer::SelfReport);
        assert_eq!(
            census_of(&report, "sleep_quality").producer,
            Producer::HealthStoreUnreachable
        );
        assert_eq!(
            census_of(&report, "learning_hours").producer,
            Producer::Unmeasured
        );
        assert!(census_of(&report, "energy")
            .producer_reason
            .contains("self-report"));
    }
}
