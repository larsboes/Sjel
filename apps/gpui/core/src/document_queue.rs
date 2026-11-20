//! Local resume state for the operator's Documents split queue.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct SplitQueue {
    pub resume: Option<String>,
    pub deferred: BTreeSet<String>,
    pub drafts: BTreeMap<String, SavedSplit>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct SavedSplit {
    pub cuts: BTreeSet<u32>,
    pub names: BTreeMap<u32, String>,
    pub offset: usize,
    pub collection_id: Option<String>,
}

impl SplitQueue {
    pub fn load() -> io::Result<Self> {
        let path = crate::scratchpad::local_app_state_path("document-split-queue.json")
            .ok_or_else(|| io::Error::other("No local Documents state directory"))?;
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = crate::scratchpad::local_app_state_path("document-split-queue.json")
            .ok_or_else(|| io::Error::other("No local Documents state directory"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)?;
        let temp = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp)?;
        let result = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, path));
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
