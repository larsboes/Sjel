//! inventory: what the household owns, wants and gave away.
//!
//! One item table for furniture, gear and clothes. Placement in a flat is spatial and belongs to
//! `interior`; the test for the boundary is whether a room is needed to answer the question.
//! This is its own capability because `interior` is on-demand, and trips' packing lists need the
//! rows after every restart.

mod aliases;
pub mod api;
pub mod budget;
pub mod client;
pub mod identity;
pub mod import;
pub mod intake;
mod mail_proposals;
mod resolution;
pub mod store;

/// Resolve the owner file for an isolated deployment or rehearsal.
pub fn database_path() -> std::path::PathBuf {
    sjel_config::env_var("SJEL_INVENTORY_DB_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map_or_else(sjel_config::database_path, |path| {
            sjel_config::expand_tilde(&path)
        })
}
