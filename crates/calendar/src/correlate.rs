//! Phase C: does a candidate date survive the operator's real calendar?
//!
//! Pure over a slice of entries — the caller hands in the window, this module
//! never queries and never reads another capability's store. Verdicts are
//! soft (README's "No hard filter on conflicts"): the layer explains, the
//! operator decides. Nothing here filters a candidate out.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::date;
use crate::model::{Commitment, Entry};

mod events;
mod feasibility;
mod trips;
mod windows;

pub use events::{
    is_same_event, verdict_for, verdicts_for, without_already_adopted, Candidate, Evidence,
    FeasibleWindow, Verdict,
};
pub use feasibility::{impact, Feasibility};
pub use trips::{cluster_trips, TripDraft, TripDrafts, Unclustered};
pub use windows::{feasible_windows, query_window};

#[cfg(test)]
mod test_suite;
