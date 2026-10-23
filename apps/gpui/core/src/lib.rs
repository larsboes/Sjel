//! What the Mac app (`apps/gpui`) and the iOS app (`ios/`) share, with no GPUI type in it, so
//! either platform can build it:
//!
//! - [`calendar`]: month and week grids, entry coverage, and the new-entry draft.
//! - [`capture`]: the capture inbox's rows and the rules that say where each one goes.
//! - [`blocks`] and [`inline`]: a note as markdown blocks, and the formatting a block shows.
//! - [`data`]: one slot per read, one press per write, and the two adapters behind them — [`data::Repo`]
//!   for the Mac's live database, [`data::Board`] for a phone's own replica and its offline log.
//! - [`mail`]: mail triage proposals and the rules for showing and acting on them.
//! - [`route`]: the destinations of the web dashboard.
//! - [`systems`]: the Systems page's payloads — macmon, storage, updates, host watch and the
//!   runtime profile — and the rules that decide how each one reads.
//! - [`shell`]: the client for the Sjel shell, over loopback with the inbound token.
//! - [`device`]: a paired device's client, over TLS pinned to the Mac's LAN listener.
//! - [`store`]: the local LibSQL replica and the queue of writes waiting for the Mac.
//! - [`sync`]: replaying that queue at the Mac and taking back what it changed.

pub mod accounts;
pub mod backup;
pub mod blocks;
pub mod calendar;
pub mod capture;
pub mod context;
pub mod cv;
pub mod data;
pub mod device;
pub mod finance;
pub mod inline;
pub mod inventory;
pub mod mail;
pub mod media;
pub mod packs;
pub mod people;
pub mod research;
pub mod route;
pub mod self_model;
pub mod shell;
pub mod soundscape;
pub mod store;
pub mod sync;
pub mod systems;
pub mod travel;
