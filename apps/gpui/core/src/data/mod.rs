//! The data layer both apps share: one slot per read, one press per write, and the adapters
//! behind them.
//!
//! Nothing here knows about GPUI. `apps/gpui/src/data/mod.rs` keeps the half that does — the
//! spawn into a `Context`, the toast a write shows, and `Press`, which names the row a write
//! marks busy. That split is what lets the iOS app drive the same `Repo`, `Remote`, `Busy` and
//! note cache without a window: it replaces the slot from its own event loop.
//!
//! Two adapters read the same records for two different reasons:
//!
//! - [`Repo`] reads the Mac's live database (the file the capability processes write) and falls
//!   back to the shell. The Mac app uses it, and it never queues anything.
//! - [`Board`] reads and writes the app's own replica, with an offline log beside it. The phone
//!   uses it, because a phone has no database to read and no shell to ask.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::shell::{ShellError, ShellResult};

pub mod board;
pub mod notes;
pub mod repo;

pub use board::{Blocked, Board};
pub use notes::Notes;
pub use repo::Repo;

/// Data older than this shows its age beside the page title.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// The last answer for one slot. A failed read keeps the value on screen and adds the error.
pub struct Remote<T> {
    pub value: Option<T>,
    pub error: Option<ShellError>,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
            loaded_at: None,
            loading: false,
        }
    }
}

impl<T> Remote<T> {
    pub fn new_loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }

    pub fn settle(&mut self, result: ShellResult<T>) {
        self.loading = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.loaded_at = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Neither loaded nor failed yet: the page reserves space and shows no error.
    pub fn first_load(&self) -> bool {
        self.value.is_none() && self.error.is_none()
    }

    /// Seconds since this value arrived, for a value that counts up on its own.
    pub fn age_of_value(&self) -> u64 {
        self.loaded_at.map(|at| at.elapsed().as_secs()).unwrap_or(0)
    }

    /// How long ago this value arrived, once it is past [`STALE_AFTER`].
    pub fn age(&self) -> Option<Duration> {
        self.loaded_at
            .map(|at| at.elapsed())
            .filter(|elapsed| *elapsed > STALE_AFTER)
    }

    /// Marks the slot in flight, or reports that a read is already running.
    ///
    /// Public because the spawn that runs the read lives in the app crate: it sets the slot in
    /// flight before it hands the future to GPUI, and a second read for the same slot stops here.
    pub fn begin(&mut self) -> bool {
        if self.loading {
            return false;
        }
        self.loading = true;
        true
    }
}

/// The writes in flight, by key, so one button cannot be pressed twice.
#[derive(Default)]
pub struct Busy(HashSet<String>);

impl Busy {
    /// Marks `key` in flight, or reports that it already is.
    pub fn start(&mut self, key: &str) -> bool {
        self.0.insert(key.to_owned())
    }

    pub fn finish(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_read_does_not_start_while_one_is_in_flight() {
        let mut slot: Remote<u8> = Remote::default();
        assert!(slot.first_load());
        assert!(slot.begin());
        assert!(!slot.begin(), "a read is already running");
        slot.settle(Ok(7));
        assert!(!slot.loading);
        assert_eq!(slot.value, Some(7));
        assert!(slot.begin(), "the slot is free again");
    }

    #[test]
    fn a_failed_read_keeps_the_value_and_reports_the_error() {
        let mut slot: Remote<u8> = Remote::default();
        slot.settle(Ok(1));
        slot.settle(Err(ShellError::Unreachable));
        assert_eq!(slot.value, Some(1), "the page keeps drawing what it had");
        assert_eq!(slot.error, Some(ShellError::Unreachable));
        assert!(!slot.first_load());
    }

    #[test]
    fn a_press_is_in_flight_until_it_finishes() {
        let mut busy = Busy::default();
        assert!(busy.start("Home-Lab/a.md"));
        assert!(!busy.start("Home-Lab/a.md"));
        assert!(busy.contains("Home-Lab/a.md"));
        busy.finish("Home-Lab/a.md");
        assert!(!busy.contains("Home-Lab/a.md"));
        assert!(busy.start("Home-Lab/a.md"));
    }
}
