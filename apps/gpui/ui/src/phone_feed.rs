//! Shared `Mutex` + `watch::Sender` state wrapper for phone device-source feeds.

use std::sync::{Mutex, MutexGuard};

use sjel_gpui_core::device::Source;
use tokio::sync::watch;

/// Paired Mac/shell source plus the latest published snapshot.
pub(crate) struct SourceState<Snap> {
    pub(crate) source: Option<Source>,
    pub(crate) snap: Snap,
}

/// Thread-safe snapshot feed backed by a `tokio::sync::watch` channel.
pub(crate) struct SourceFeed<Snap: Clone> {
    state: Mutex<SourceState<Snap>>,
    tx: watch::Sender<Snap>,
}

impl<Snap: Default + Clone> SourceFeed<Snap> {
    pub(crate) fn new() -> Self {
        let snap = Snap::default();
        let (tx, _) = watch::channel(snap.clone());
        Self {
            state: Mutex::new(SourceState { source: None, snap }),
            tx,
        }
    }
}

impl<Snap: Clone> SourceFeed<Snap> {
    pub(crate) fn state(&self) -> MutexGuard<'_, SourceState<Snap>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn publish(&self, s: &SourceState<Snap>) {
        self.tx.send_replace(s.snap.clone());
    }

    pub(crate) fn snapshots(&self) -> &watch::Sender<Snap> {
        &self.tx
    }
}
