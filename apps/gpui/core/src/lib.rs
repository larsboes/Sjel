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
//! - [`search`]: whether a typed text is a search or a prompt, and how candidates rank.
//! - [`area`]: the nine areas the app groups its pages into, and the tab each one shows.
//! - [`systems`]: the Systems page's payloads — macmon, storage, updates, host watch and the
//!   runtime profile — and the rules that decide how each one reads.
//! - [`shell`]: the client for the Sjel shell, over loopback with the inbound token.
//! - [`device`]: a paired device's client, over TLS pinned to the Mac's LAN listener.
//! - [`store`]: the local LibSQL replica and the queue of writes waiting for the Mac.
//! - [`sync`]: replaying that queue at the Mac and taking back what it changed.
//! - [`mirror`]: the Mac's read-only copy of its data, fetched over the paired connection for
//!   the pages the phone reads but does not write.
//! - [`people`] and [`places`]: the contact registry and the places register, and the name the two
//!   are joined by ([`places::name_key`], the same one `capabilities/places` keys its decisions by).
//! - [`notify`]: what the phone puts on its lock screen, and when — the lead a person sets, the
//!   entries and tasks that earn a reminder, and what a rebuild cancels.
//! - [`today`]: the phone's landing page — the day the Mac's copy answered, the tasks the replica
//!   holds, and the projects those tasks belong to.

pub mod accounts;
pub mod agent;
pub mod area;
pub mod backup;
pub mod blocks;
pub mod calendar;
pub mod capture;
pub mod clipper;
pub mod context;
pub mod cv;
pub mod data;
pub mod device;
pub mod finance;
pub mod inline;
pub mod interior;
pub mod inventory;
pub mod mail;
pub mod media;
pub mod mirror;
pub mod notify;
pub mod packs;
pub mod people;
pub mod places;
pub mod record;
pub mod research;
pub mod route;
pub mod scouting;
pub mod search;
pub mod self_model;
pub mod setup;
pub mod shell;
pub mod soundscape;
pub mod store;
pub mod sync;
pub mod systems;
pub mod terminal;
pub mod today;
pub mod travel;
