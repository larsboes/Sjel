//! Markdown event notes, read once and imported on purpose.
//!
//! Calendar already accepted entries four ways: typed by hand, materialized
//! from a rhythm, reviewed out of a Google draft, promoted from a scouting
//! opportunity. What it had no path for was a directory of structured markdown
//! event notes — so an operator holding one either kept two editable stores of
//! the same events forever, or wrote into calendar's tables behind the
//! capability's back. Both are worse than an importer.
//!
//! ## The shape of the contract
//!
//! **Scanning never writes.** [`scan`] reads a declared root, parses what it
//! finds, and hands back both the entries it *would* write and every record it
//! refused, each with the reason. Nothing reaches the store until [`plan`]'s
//! output is passed to an explicit import call with the ids to write.
//!
//! **Identity is stable and derived, never invented.** An imported entry is
//! keyed `(source, external_id)` where `source` is the declared source id and
//! `external_id` is the note's path relative to the declared root. Re-importing
//! updates in place through the unique index calendar already carries, so the
//! second run of a scan produces no duplicates.
//!
//! **Uncertain means [`Commitment::Possible`].** The status vocabulary of a
//! notes system is not calendar's commitment model, and only two values map
//! cleanly onto "this is happening". Everything else — including a status this
//! code has never seen — lands on `possible`, which shows as evidence and
//! blocks no day.
//!
//! **Malformed times fail closed.** A note with no start, an end before its
//! start, or one date-only side and one timed side is reported and skipped. It
//! is never guessed into a plausible entry: an event silently placed on the
//! wrong day is worse than an event that visibly did not import.
//!
//! ## What this module will never do
//!
//! Delete, move or rewrite a source note. The import is one-directional by
//! construction and there is no API here that could be pointed at the vault as
//! a writer. Deciding a note has been superseded is the operator's call, made
//! after they have seen the entries land.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::date;
use crate::model::{Commitment, NewEntry};
use markdown_root::{frontmatter, MarkdownRoot};

/// A declared source of markdown event notes.
///
/// The root and glob come from the operator's private config; nothing here
/// hardcodes a vault path, and the public example config carries placeholders.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarkdownSource {
    /// Stable id. Becomes the entry `source`, so it must satisfy calendar's
    /// source-token rule (1-40 chars of `[a-z0-9_]`) — checked by
    /// `NewEntry::validate`, not silently corrected here.
    pub id: String,
    /// Root directory of the note store. `~/` is expanded at config load.
    pub path: String,
    /// Bounded glob relative to `path`. A directory (`Atlas/Events/*.md`) or
    /// one exact file.
    #[serde(default = "default_glob")]
    pub events_glob: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_glob() -> String {
    "*.md".into()
}

fn default_enabled() -> bool {
    true
}

/// One note that would become one entry.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Candidate {
    /// Note path relative to the declared root — the `external_id` half of the
    /// identity, and the only thing tying an entry back to its note.
    pub external_id: String,
    /// Ready to write. Carries its own provenance in `payload`.
    pub entry: NewEntry,
}

/// One note that would not, and why.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Refusal {
    pub external_id: String,
    pub reason: String,
}

/// The whole read-only result of looking at a source.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub source: String,
    /// The resolved root, so an operator reviewing a preview can see which
    /// store it came from rather than trusting the id.
    pub root: String,
    pub candidates: Vec<Candidate>,
    /// Notes that parsed but are not events, or are events this importer
    /// refuses to guess at. Never silently dropped.
    pub refused: Vec<Refusal>,
}

impl Preview {
    /// Every candidate id, for the caller that has reviewed a preview and wants
    /// to import all of it.
    pub fn all_ids(&self) -> Vec<String> {
        self.candidates
            .iter()
            .map(|c| c.external_id.clone())
            .collect()
    }
}

/// Read a declared source. Never writes, never touches the store.
///
/// The `Err` case is the source being unusable at all — a root that is not
/// there, a glob that tries to climb out of it. A note that cannot be read is
/// a [`Refusal`] inside an otherwise successful preview, because one broken
/// note is not a reason to refuse the other hundred.
pub fn scan(source: &MarkdownSource) -> Result<Preview, String> {
    let root =
        MarkdownRoot::declare(&source.path).map_err(|e| format!("source '{}': {e}", source.id))?;
    let files = root
        .markdown_files(&source.events_glob)
        .map_err(|e| format!("source '{}': {e}", source.id))?;

    let mut candidates = Vec::new();
    let mut refused = Vec::new();

    for file in files {
        let external_id = match root.relative_id(&file) {
            Some(id) => id,
            // Unreachable through markdown_files, which only returns contained
            // paths — but "unreachable" is a claim, and a wrong one here would
            // import a note under an identity derived from nothing.
            None => continue,
        };
        let body = match std::fs::read_to_string(&file) {
            Ok(body) => body,
            Err(e) => {
                refused.push(Refusal {
                    external_id,
                    reason: format!("cannot read: {e}"),
                });
                continue;
            }
        };
        match candidate_from_note(&source.id, &external_id, &body) {
            Ok(Some(candidate)) => candidates.push(candidate),
            Ok(None) => {}
            Err(reason) => refused.push(Refusal {
                external_id,
                reason,
            }),
        }
    }

    candidates.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    refused.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    Ok(Preview {
        source: source.id.clone(),
        root: root.path().to_string_lossy().into_owned(),
        candidates,
        refused,
    })
}

/// Narrow a preview to an explicit selection.
///
/// `Ok(None)` for a note is "not an event note" and `Err` is "an event note
/// this importer will not guess at" — the distinction the preview shows and
/// this preserves. An id that is not in the preview is an error rather than a
/// no-op: a caller importing something the review never showed them has lost
/// track of what it is writing.
pub fn plan<'a>(preview: &'a Preview, selection: &[String]) -> Result<Vec<&'a Candidate>, String> {
    let by_id: HashMap<&str, &Candidate> = preview
        .candidates
        .iter()
        .map(|c| (c.external_id.as_str(), c))
        .collect();
    selection
        .iter()
        .map(|id| {
            by_id.get(id.as_str()).copied().ok_or_else(|| {
                format!(
                    "'{id}' is not a candidate in this scan of source '{}'",
                    preview.source
                )
            })
        })
        .collect()
}

/// Turn one note into the entry it would become.
///
/// `Ok(None)` means the file is not an event note at all — a MOC, a template,
/// a piece of prose that happens to live in the same directory. That is
/// ordinary and silent. `Err` means it *is* an event note and something about
/// it cannot be honoured, which the operator has to see.
fn candidate_from_note(
    source_id: &str,
    external_id: &str,
    body: &str,
) -> Result<Option<Candidate>, String> {
    let fm = frontmatter(body)?;

    // A note store holds more than events. Only a declared event is a
    // candidate; anything else is not this importer's business, and treating an
    // untyped note as an event is how an arbitrary page becomes a calendar
    // entry.
    match fm.get("type").map(String::as_str) {
        Some("event") => {}
        _ => return Ok(None),
    }

    let title = pick(&fm, &["summary", "title"])
        .or_else(|| note_stem(external_id))
        .ok_or("no title: neither a summary field nor a usable filename")?;

    let start_raw = pick(&fm, &["start", "date"]).ok_or("no start date")?;
    let (starts_at, start_timed) = instant(&start_raw)?;

    // An absent end is a one-day event, which is the vault convention and also
    // the only reading that cannot be wrong in a way nobody notices.
    let end_raw = pick(&fm, &["end"]);
    let (ends_at, all_day) = match end_raw {
        None => (exclusive_end(&starts_at, start_timed)?, !start_timed),
        Some(raw) => {
            let (end, end_timed) = instant(&raw)?;
            if end_timed != start_timed {
                return Err("start and end disagree about whether this is an all-day entry".into());
            }
            if start_timed {
                (end, false)
            } else {
                // Note stores write an *inclusive* end date: a single-day event
                // has start == end. Calendar's ends_at is exclusive. Converting
                // is the one place this importer changes a value rather than
                // carrying it, so it is stated here and tested.
                (exclusive_end(&end, false)?, true)
            }
        }
    };

    let entry = NewEntry {
        kind: "event".into(),
        commitment: commitment_from_status(fm.get("status").map(String::as_str)),
        title,
        starts_at,
        ends_at,
        all_day,
        location: pick(&fm, &["location"]),
        notes: None,
        attendees: Vec::new(),
        source: source_id.to_string(),
        external_id: Some(external_id.to_string()),
        rhythm_id: None,
        payload: provenance(source_id, external_id, &fm),
    };
    // The importer's own shape checks are above; this is calendar's, and it is
    // the one that decides. Failing here is a refusal, not a panic later at the
    // store: everything reaching an operator's review is already writable.
    entry.validate()?;

    Ok(Some(Candidate {
        external_id: external_id.to_string(),
        entry,
    }))
}

/// A vault status vocabulary is not calendar's commitment model, and pretending
/// otherwise is how an idea becomes a blocked day. Only the two values that
/// actually claim "this is happening" map upward; everything else, including a
/// status this code has never seen and a note with none, is `possible`.
///
/// `cancelled` needs no special case: it is not `confirmed` or `planned`, so it
/// lands on `possible`, which is exactly right — visible as evidence, blocking
/// nothing.
fn commitment_from_status(status: Option<&str>) -> Commitment {
    match status
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("confirmed") | Some("completed") => Commitment::Committed,
        Some("planned") => Commitment::Planned,
        _ => Commitment::Possible,
    }
}

/// Inert evidence: enough to audit the mapping later without copying the note.
///
/// Deliberately not the note's body. The point of the import is that calendar
/// owns the structured event and the note keeps the prose — duplicating the
/// prose into a database column would recreate the two-stores problem this is
/// meant to end.
fn provenance(
    source_id: &str,
    external_id: &str,
    fm: &HashMap<String, String>,
) -> serde_json::Value {
    json!({
        "schema_version": "calendar-markdown-import-v1",
        "source": source_id,
        "note": external_id,
        // The fields the mapping actually consumed, so a wrong entry can be
        // explained without reopening the vault. Absent keys stay absent.
        "observed": {
            "status": fm.get("status"),
            "category": fm.get("category"),
            "start": pick(fm, &["start", "date"]),
            "end": fm.get("end").cloned(),
        }
    })
}

/// First present, non-empty value among `keys`.
fn pick(fm: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| fm.get(*key))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// `Atlas/Events/Some Event.md` → `Some Event`.
fn note_stem(external_id: &str) -> Option<String> {
    let name = external_id.rsplit('/').next()?;
    let stem = name.strip_suffix(".md").unwrap_or(name).trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Parse an instant and say whether it carried a time.
///
/// Seconds are dropped rather than refused: a note store writing `T18:00:00` is
/// saying six in the evening, and calendar stores minutes.
fn instant(raw: &str) -> Result<(String, bool), String> {
    let (days, time) = date::parse_instant(raw.trim())
        .ok_or_else(|| format!("'{raw}' is not a date or a local time"))?;
    Ok(match time {
        None => (date::format_date(days), false),
        Some((h, m)) => (format!("{}T{h:02}:{m:02}", date::format_date(days)), true),
    })
}

/// The exclusive end for a date-only instant: the day after.
fn exclusive_end(instant: &str, timed: bool) -> Result<String, String> {
    if timed {
        return Err("a timed entry needs its own end time".into());
    }
    let days = date::parse_date(instant).ok_or_else(|| format!("'{instant}' is not a date"))?;
    Ok(date::format_date(days + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(body: &str) -> Result<Option<Candidate>, String> {
        candidate_from_note("vault_events", "Atlas/Events/A Note.md", body)
    }

    #[test]
    fn a_dated_event_note_becomes_an_entry_keyed_by_its_path() {
        let candidate = note(
            "---\ntype: event\nsummary: A talk\nstart: 2026-02-03\nend: 2026-02-03\n\
             location: Bonn\nstatus: confirmed\n---\nbody",
        )
        .expect("parses")
        .expect("is an event");

        assert_eq!(candidate.external_id, "Atlas/Events/A Note.md");
        assert_eq!(candidate.entry.source, "vault_events");
        assert_eq!(
            candidate.entry.external_id.as_deref(),
            Some("Atlas/Events/A Note.md"),
            "identity is the note's path under the declared root, nothing invented"
        );
        assert_eq!(candidate.entry.title, "A talk");
        assert_eq!(candidate.entry.location.as_deref(), Some("Bonn"));
    }

    /// The one value this importer converts rather than carries.
    #[test]
    fn an_inclusive_end_date_becomes_an_exclusive_one() {
        let single =
            note("---\ntype: event\nsummary: One day\nstart: 2026-02-03\nend: 2026-02-03\n---\n")
                .unwrap()
                .unwrap();
        assert_eq!(single.entry.starts_at, "2026-02-03");
        assert_eq!(
            single.entry.ends_at, "2026-02-04",
            "a note store's end is the last day; calendar's is the day after"
        );
        assert!(single.entry.all_day);

        let span = note(
            "---\ntype: event\nsummary: Three days\nstart: 2026-02-03\nend: 2026-02-05\n---\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(span.entry.ends_at, "2026-02-06");
    }

    #[test]
    fn a_note_with_no_end_is_a_single_day() {
        let candidate = note("---\ntype: event\nsummary: Just a day\nstart: 2026-02-03\n---\n")
            .unwrap()
            .unwrap();
        assert_eq!(candidate.entry.starts_at, "2026-02-03");
        assert_eq!(candidate.entry.ends_at, "2026-02-04");
    }

    #[test]
    fn a_timed_note_keeps_its_times_and_is_not_all_day() {
        let candidate = note(
            "---\ntype: event\nsummary: Evening\nstart: 2026-02-03T18:00\nend: 2026-02-03T21:30\n---\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.entry.starts_at, "2026-02-03T18:00");
        assert_eq!(candidate.entry.ends_at, "2026-02-03T21:30");
        assert!(!candidate.entry.all_day);
    }

    #[test]
    fn seconds_are_dropped_rather_than_refused() {
        let candidate = note(
            "---\ntype: event\nsummary: Precise\nstart: 2026-02-03T18:00:00\nend: 2026-02-03T21:30:00\n---\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.entry.starts_at, "2026-02-03T18:00");
    }

    // -----------------------------------------------------------------------
    // Fail closed. Each of these is a note that would otherwise have become a
    // plausible entry on the wrong day.
    // -----------------------------------------------------------------------

    /// Nine notes in the corpus this was built against declare `start:` with
    /// nothing after it. An empty value is no value, not today's date.
    #[test]
    fn a_note_with_no_start_is_refused_not_guessed() {
        let reason = note("---\ntype: event\nsummary: When?\nstart:\nend:\n---\n")
            .expect_err("no usable start");
        assert_eq!(reason, "no start date");

        let garbage = note("---\ntype: event\nsummary: When?\nstart: sometime in spring\n---\n")
            .expect_err("not a date");
        assert!(garbage.contains("not a date"), "got: {garbage}");
    }

    #[test]
    fn a_half_timed_note_is_refused_rather_than_flattened() {
        let reason = note(
            "---\ntype: event\nsummary: Mixed\nstart: 2026-02-03\nend: 2026-02-04T12:00\n---\n",
        )
        .expect_err("one side timed, one not");
        assert!(reason.contains("all-day"), "got: {reason}");
    }

    #[test]
    fn an_end_before_its_start_is_refused_by_calendars_own_rule() {
        assert!(note(
            "---\ntype: event\nsummary: Backwards\nstart: 2026-02-05\nend: 2026-02-03\n---\n"
        )
        .is_err());
    }

    #[test]
    fn a_zero_length_timed_note_is_refused() {
        assert!(note(
            "---\ntype: event\nsummary: Instant\nstart: 2026-02-03T18:00\nend: 2026-02-03T18:00\n---\n"
        )
        .is_err());
    }

    #[test]
    fn a_note_that_is_not_an_event_is_skipped_quietly() {
        assert_eq!(note("---\ntype: moc\nsummary: Hub\n---\n").unwrap(), None);
        assert_eq!(note("---\ntype: note\n---\n").unwrap(), None);
        assert_eq!(
            note("# just prose\n").unwrap(),
            None,
            "no frontmatter at all"
        );
    }

    // -----------------------------------------------------------------------
    // Commitment. The rule is that uncertainty never blocks a day.
    // -----------------------------------------------------------------------

    #[test]
    fn only_a_status_claiming_it_happens_reaches_committed() {
        assert_eq!(
            commitment_from_status(Some("confirmed")),
            Commitment::Committed
        );
        assert_eq!(
            commitment_from_status(Some("completed")),
            Commitment::Committed
        );
        assert_eq!(commitment_from_status(Some("planned")), Commitment::Planned);
    }

    #[test]
    fn every_other_status_including_one_nobody_defined_is_possible() {
        for status in [
            Some("discovered"),
            Some("applied"),
            Some("cancelled"),
            Some("a status this code has never seen"),
            None,
        ] {
            assert_eq!(
                commitment_from_status(status),
                Commitment::Possible,
                "status {status:?} must not block a day"
            );
        }
    }

    #[test]
    fn a_cancelled_event_still_imports_as_evidence_that_blocks_nothing() {
        let candidate = note(
            "---\ntype: event\nsummary: Called off\nstart: 2026-02-03\nend: 2026-02-03\nstatus: cancelled\n---\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.entry.commitment, Commitment::Possible);
    }

    // -----------------------------------------------------------------------
    // Titles, provenance, selection.
    // -----------------------------------------------------------------------

    #[test]
    fn a_note_with_no_summary_falls_back_to_its_filename() {
        let candidate = candidate_from_note(
            "vault_events",
            "Atlas/Events/Cloud Forum.md",
            "---\ntype: event\nstart: 2026-02-03\n---\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.entry.title, "Cloud Forum");
    }

    #[test]
    fn provenance_records_what_the_mapping_read_and_not_the_note_body() {
        let candidate = note(
            "---\ntype: event\nsummary: A talk\nstart: 2026-02-03\nstatus: discovered\ncategory: conference\n---\n\
             A long private paragraph nobody wants copied into a database.",
        )
        .unwrap()
        .unwrap();
        let payload = &candidate.entry.payload;
        assert_eq!(payload["schema_version"], "calendar-markdown-import-v1");
        assert_eq!(payload["note"], "Atlas/Events/A Note.md");
        assert_eq!(payload["observed"]["status"], "discovered");
        assert!(
            !payload.to_string().contains("long private paragraph"),
            "the prose stays in the note; only the structured event moves"
        );
    }

    #[test]
    fn a_selection_naming_something_the_review_never_showed_is_an_error() {
        let preview = Preview {
            source: "vault_events".into(),
            root: "/somewhere".into(),
            candidates: vec![
                note("---\ntype: event\nsummary: A\nstart: 2026-02-03\n---\n")
                    .unwrap()
                    .unwrap(),
            ],
            refused: vec![],
        };
        assert!(plan(&preview, &["Atlas/Events/A Note.md".into()]).is_ok());
        assert!(
            plan(&preview, &["Atlas/Events/Never Seen.md".into()]).is_err(),
            "importing outside the reviewed set is how a review stops meaning anything"
        );
    }

    // -----------------------------------------------------------------------
    // The whole read path, against a real directory. Fixtures are written here
    // and thrown away: no vault path and no real note ever enters this file.
    // -----------------------------------------------------------------------

    struct Tree(std::path::PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("axon-calendar-md-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp tree");
            Tree(dir)
        }

        fn note(&self, relative: &str, body: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(path, body).expect("write");
        }

        fn source(&self, glob: &str) -> MarkdownSource {
            MarkdownSource {
                id: "vault_events".into(),
                path: self.0.to_string_lossy().into_owned(),
                events_glob: glob.into(),
                enabled: true,
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_scan_separates_events_from_everything_else_sharing_the_directory() {
        let tree = Tree::new("mixed");
        tree.note(
            "Events/Talk.md",
            "---\ntype: event\nsummary: A talk\nstart: 2026-02-03\nend: 2026-02-03\n---\n",
        );
        tree.note("Events/Hub.md", "---\ntype: moc\n---\n# index");
        tree.note("Events/Template.md", "# a template with no frontmatter");
        tree.note(
            "Events/Undated.md",
            "---\ntype: event\nsummary: When?\nstart:\n---\n",
        );

        let preview = scan(&tree.source("Events/*.md")).expect("scans");

        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].external_id, "Events/Talk.md");
        assert_eq!(
            preview.refused,
            vec![Refusal {
                external_id: "Events/Undated.md".into(),
                reason: "no start date".into(),
            }],
            "an event note that cannot be honoured is reported; a non-event is not"
        );
    }

    #[test]
    fn a_scan_writes_nothing_and_can_be_run_again_for_the_same_answer() {
        let tree = Tree::new("idempotent");
        tree.note(
            "Events/Talk.md",
            "---\ntype: event\nsummary: A talk\nstart: 2026-02-03\n---\n",
        );
        let source = tree.source("Events/*.md");

        let first = scan(&source).expect("scans");
        let second = scan(&source).expect("scans again");

        assert_eq!(first.candidates, second.candidates);
        assert_eq!(
            first.all_ids(),
            vec!["Events/Talk.md".to_string()],
            "the identity a re-import updates through is the note's own path"
        );
    }

    #[test]
    fn a_source_root_that_is_not_there_names_itself_rather_than_scanning_nothing() {
        let source = MarkdownSource {
            id: "vault_events".into(),
            path: std::env::temp_dir()
                .join("axon-calendar-md-gone-4c3b2a")
                .to_string_lossy()
                .into_owned(),
            events_glob: "*.md".into(),
            enabled: true,
        };
        let error = scan(&source).expect_err("no such root");
        assert!(error.contains("vault_events"), "got: {error}");
    }

    #[test]
    fn a_glob_that_climbs_out_of_the_root_is_refused_before_anything_is_read() {
        let tree = Tree::new("traversal");
        tree.note("Events/Talk.md", "---\ntype: event\n---\n");
        let error = scan(&tree.source("../*.md")).expect_err("escapes");
        assert!(error.contains("escapes"), "got: {error}");
    }

    #[test]
    fn a_second_scan_of_an_unchanged_note_produces_the_same_identity() {
        let body = "---\ntype: event\nsummary: A talk\nstart: 2026-02-03\n---\n";
        let first = note(body).unwrap().unwrap();
        let second = note(body).unwrap().unwrap();
        assert_eq!(
            first.external_id, second.external_id,
            "a stable identity is what makes re-import an update instead of a duplicate"
        );
        assert_eq!(first.entry, second.entry);
    }
}
