//! The notes opened most recently, held in memory so going back to one costs no read.
//!
//! A note is read in full: one record with its content, read from the local database or the
//! shell. Reading it again to see the text that was on screen a click ago is the one read this
//! cache exists to remove. Every write the app makes drops the note it wrote, so what is kept is
//! never older than the last write of this app; [`NOTE_TTL`] bounds how old it can be.
//!
//! It lives beside [`Repo`] because it is the same read path on both platforms: the editor keeps
//! one whether a `Context` exists or not.
//!
//! [`Repo`]: super::Repo

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::shell::ProjectRecord;

/// How long a kept note opens without a read. An edit made elsewhere in that window is caught by
/// the next save, which the capability refuses when the note changed since it was read.
const NOTE_TTL: Duration = Duration::from_secs(300);

/// How many notes stay in memory. A note is a few kilobytes to a few tens of kilobytes, so this
/// is under a megabyte while covering every note someone tabs between.
const NOTE_CAPACITY: usize = 16;

/// The notes opened most recently, most recent first, shared by every [`Repo`] clone.
///
/// [`Repo`]: super::Repo
#[derive(Clone, Default)]
pub struct Notes {
    inner: Arc<Mutex<VecDeque<Kept>>>,
}

/// One kept note and when it was read.
struct Kept {
    record: ProjectRecord,
    read_at: Instant,
}

impl Notes {
    /// The note at `path`, if it was opened within [`NOTE_TTL`].
    ///
    /// A hit moves the note to the front, so the note someone keeps tabbing back to is the one an
    /// eviction spares.
    pub fn get(&self, path: &str) -> Option<ProjectRecord> {
        let mut kept = self.lock();
        let at = kept.iter().position(|one| one.record.path == path)?;
        if kept[at].read_at.elapsed() > NOTE_TTL {
            kept.remove(at);
            return None;
        }
        let one = kept.remove(at)?;
        let record = one.record.clone();
        kept.push_front(one);
        Some(record)
    }

    /// Keeps `record` for the next open, replacing what the same path kept before.
    pub fn put(&self, record: &ProjectRecord) {
        let mut kept = self.lock();
        kept.retain(|one| one.record.path != record.path);
        kept.push_front(Kept {
            record: record.clone(),
            read_at: Instant::now(),
        });
        kept.truncate(NOTE_CAPACITY);
    }

    /// Records the body this app just saved on the note at `path`, if it is kept.
    ///
    /// The editor writes the body only (`apps/gpui/src/blocks.rs` holds the frontmatter verbatim
    /// and does not edit it), so the rest of the kept record still holds.
    pub fn set_content(&self, path: &str, content: &str) {
        let mut kept = self.lock();
        if let Some(one) = kept.iter_mut().find(|one| one.record.path == path) {
            content.clone_into(&mut one.record.content);
        }
    }

    /// Drops the note at `path`. Every write to a record calls this, so a kept note is never
    /// older than the last write.
    pub fn forget(&self, path: &str) {
        self.lock().retain(|one| one.record.path != path);
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<Kept>> {
        // A cache is an optimisation: a panic elsewhere must not turn a read into a panic here.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn note(path: &str) -> ProjectRecord {
        ProjectRecord {
            path: path.into(),
            kind: "note".into(),
            title: path.trim_end_matches(".md").into(),
            content: format!("# {path}\n"),
            done: false,
            due: None,
            priority: 2,
        }
    }

    fn paths(notes: &Notes) -> Vec<String> {
        notes
            .lock()
            .iter()
            .map(|one| one.record.path.clone())
            .collect()
    }

    #[test]
    fn a_note_put_once_is_read_from_memory_the_next_time() {
        let notes = Notes::default();
        notes.put(&note("P/Plan.md"));
        let kept = notes.get("P/Plan.md").expect("kept");
        assert_eq!(kept.content, "# P/Plan.md\n");
        assert_eq!(kept.kind, "note");
    }

    #[test]
    fn a_hit_moves_to_the_front_so_the_next_eviction_takes_the_least_recent() {
        let notes = Notes::default();
        for at in 0..6 {
            notes.put(&note(&format!("P/{at}.md")));
        }
        // Read the oldest one; the one put before it is now the least recent.
        assert!(notes.get("P/0.md").is_some());
        assert_eq!(paths(&notes)[0], "P/0.md");
        assert_eq!(paths(&notes)[5], "P/1.md");
    }

    #[test]
    fn only_the_capacity_is_kept() {
        let notes = Notes::default();
        for at in 0..(NOTE_CAPACITY + 4) {
            notes.put(&note(&format!("P/{at}.md")));
        }
        assert_eq!(paths(&notes).len(), NOTE_CAPACITY);
        assert!(notes.get("P/0.md").is_none());
        assert!(notes
            .get(&format!("P/{}.md", NOTE_CAPACITY + 4 - 1))
            .is_some());
    }

    #[test]
    fn putting_the_same_note_twice_keeps_one() {
        let notes = Notes::default();
        notes.put(&note("P/Plan.md"));
        notes.put(&note("P/Plan.md"));
        assert_eq!(paths(&notes), ["P/Plan.md"]);
    }

    #[test]
    fn a_note_older_than_the_ttl_is_read_again() {
        let notes = Notes::default();
        notes.put(&note("P/Plan.md"));
        notes.inner.lock().unwrap()[0].read_at = Instant::now() - NOTE_TTL - Duration::from_secs(1);
        assert!(notes.get("P/Plan.md").is_none());
        // Reading it dropped it, so the next put starts clean.
        assert!(paths(&notes).is_empty());
    }

    #[test]
    fn a_forgotten_note_is_read_again() {
        let notes = Notes::default();
        notes.put(&note("P/Plan.md"));
        notes.forget("P/Plan.md");
        assert!(notes.get("P/Plan.md").is_none());
        // Forgetting a note that is not kept is nothing.
        notes.forget("P/Plan.md");
    }

    #[test]
    fn a_saved_body_replaces_what_was_kept() {
        let notes = Notes::default();
        notes.put(&note("P/Plan.md"));
        notes.set_content("P/Plan.md", "# Plan\n\nEdited\n");
        let kept = notes.get("P/Plan.md").expect("kept");
        assert_eq!(kept.content, "# Plan\n\nEdited\n");
        assert_eq!(kept.title, "P/Plan");
    }

    #[test]
    fn saving_a_note_that_is_not_kept_keeps_nothing() {
        let notes = Notes::default();
        notes.set_content("P/Plan.md", "# Plan\n");
        assert!(paths(&notes).is_empty());
    }
}
