//! State types, derived storage/updates views, and beat constants for systems.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use sjel_gpui_core::systems::{StorageTrend, StorageView, UpdatesView};

/// The live sample's beat. The page exists for temperature and power, and macmon samples the
/// SMC every five seconds upstream, so a slower beat would show a stale reading.
pub(super) const MACMON_BEAT: Duration = Duration::from_secs(3);

/// Pending agent writes are what a person is waiting on.
pub(super) const AGENT_BEAT: Duration = Duration::from_secs(5);

/// The runtime profile changes when its owner changes it; a slower beat is enough.
pub(super) const RUNTIME_BEAT: Duration = Duration::from_secs(30);

/// While an apply runs, the report is the progress bar: the route answers before the work starts.
pub(super) const APPLY_BEAT: Duration = Duration::from_secs(4);

/// An apply compiles for minutes, so the watch is bounded: an hour of beats, then it stops
/// rather than polling a dead shell forever.
pub(super) const APPLY_BEATS: usize = 900;

pub(super) const MACMON_START: &str = "macmon:start";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemsTab {
    Overview,
    Storage,
    Agent,
}

#[derive(Default)]
pub(super) struct SystemsDerived {
    pub(super) storage_at: Option<Instant>,
    pub(super) storage_view: Option<StorageView>,
    pub(super) history_at: Option<Instant>,
    pub(super) storage_trend: Option<StorageTrend>,
    pub(super) class_deltas: BTreeMap<String, String>,
    pub(super) updates_at: Option<Instant>,
    pub(super) updates_view: Option<UpdatesView>,
}
