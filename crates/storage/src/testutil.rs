//! Planted fixtures for the unit tests.
//!
//! `std::env::temp_dir()` plus the process id, which is the idiom the workspace already
//! uses (capabilities/calendar/src/store.rs, capabilities/punctuality/src/store.rs). No
//! `tempfile` dependency: it is in `Cargo.lock` transitively, but no workspace member
//! declares it, and one function is cheaper than becoming the crate that starts.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty directory. Cargo runs a crate's tests in threads of one process, so the
/// process id alone is not unique — the counter is what keeps two tests apart.
pub fn tempdir(name: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sjel-storage-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture directory");
    dir
}
