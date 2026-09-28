//! The vault, read as data.
//!
//! A library because this capability now has two binaries over one reader: the
//! `vault` CLI (`links`, `lint`, `names`, `class`, `people`, `journal`,
//! `bases`, `fields`) and `vault-server`, the read-only HTTP surface the
//! dashboard's decision ladder reads the Action kind through (PRD Q48,
//! 2026-08-27). Both load notes the same way or they would eventually
//! disagree about what a note is, which is the whole reason `note::load_all` is
//! one pass in one place.
//!
//! Read-only except in one place: `vault fields --apply` writes `last_contact`
//! and `met_at` on `Atlas/People/**`, because `Resources/Bases/People.base`
//! computes `lost_touch` from `last_contact` and a `.base` cannot call HTTP.
//! `fields` holds what may be written and under what rule; `obsidian` is the
//! only thing here that writes, through the Obsidian Local REST API with no file
//! fallback. `vault-server` stays read-only.

pub mod bases;
pub mod class;
pub mod fields;
pub mod graph;
pub mod journal;
pub mod lint;
pub mod names;
pub mod note;
pub mod obsidian;
pub mod people;
pub mod tasks;
