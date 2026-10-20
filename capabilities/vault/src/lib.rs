//! The vault, read as data.
//!
//! A library because this capability has two binaries over one reader: the `vault` CLI
//! (`links`, `lint`, `class`, `bases`) and `vault-server`, which answers readiness. Both load
//! notes the same way or they would eventually disagree about what a note is, which is the whole
//! reason `note::load_all` is one pass in one place.
//!
//! Read-only throughout. Until 2026-10-20 `vault fields --apply` wrote `last_contact` and
//! `met_at` on `Atlas/People/**` through the Obsidian Local REST API. That folder and `Journal/`
//! left the vault that day, and the code went with them.

pub mod bases;
pub mod class;
pub mod graph;
pub mod lint;
pub mod note;
