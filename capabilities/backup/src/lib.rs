//! backup: what a run did, what a target proved, and when the next one is due.
//!
//! A library, not a process. `capabilities/sjel-status` is the surface — it already lists
//! backup age and already triggers a backup, and it already links a capability's store
//! module (`capabilities/devices`) for its own gate. A second process serving the same
//! job would be a second trigger and a second truth for one thing.
//!
//! Mechanism stays in `tools/`: `tools/backup.sh` moves and checks the bytes, and this
//! crate drives it with argv arrays and records what happened. The one thing this owns
//! that the tools cannot is memory: a failed run leaves no receipt behind, and a run that
//! leaves no trace is the failure a backup tool must not have.
//!
//! Claims and their falsifiers: `ISA.md` here.

pub mod policy;
pub mod runner;
pub mod store;
pub mod targets;

pub use store::{ArchiveIdentity, BackupStore, RunRow, TargetRow, VerificationRow};
