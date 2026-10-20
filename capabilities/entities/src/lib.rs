//! The entity core: who and what Sjel knows about, as typed data (PRD Q117).
//!
//! `model.rs` carries the kinds, the field registry and the rules. `store.rs` carries the
//! tables. `places.rs` asks `capabilities/places` for a coordinate. `server.rs` is the
//! binary and its HTTP surface.

pub mod config;
pub mod duplicates;
pub mod google;
pub mod journal;
pub mod model;
pub mod places;
pub mod resolve;
pub mod store;
pub mod sync;
