//! What the Mac app (`apps/gpui`) and the iOS app (`ios/`) share, with no GPUI type in it, so
//! either platform can build it:
//!
//! - [`blocks`] and [`inline`]: a note as markdown blocks, and the formatting a block shows.
//! - [`data`]: one slot per read, one press per write, and the two adapters behind them — [`data::Repo`]
//!   for the Mac's live database, [`data::Board`] for a phone's own replica and its offline log.
//! - [`route`]: the destinations of the web dashboard.
//! - [`shell`]: the client for the Sjel shell, over loopback with the inbound token.
//! - [`device`]: a paired device's client, over TLS pinned to the Mac's LAN listener.
//! - [`store`]: the local LibSQL replica and the queue of writes waiting for the Mac.
//! - [`sync`]: replaying that queue at the Mac and taking back what it changed.

pub mod blocks;
pub mod data;
pub mod device;
pub mod inline;
pub mod route;
pub mod shell;
pub mod store;
pub mod sync;
