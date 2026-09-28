//! The vault, read as data.
//!
//! A library because this capability now has two binaries over one reader: the
//! `vault` CLI (`links`, `lint`, `names`, `class`, `people`, `bases`) and
//! `vault-server`, the read-only HTTP surface the dashboard's decision ladder
//! reads the Action kind through (PRD Q48, 2026-08-27). Both load notes the
//! same way or they would eventually
//! disagree about what a note is, which is the whole reason `note::load_all` is
//! one pass in one place.
//!
//! Still read-only, in both binaries. `note::Note` carries a `path` and a
//! `body_start` so a future writer has somewhere to splice; nothing here uses
//! them to write.

pub mod bases;
pub mod class;
pub mod graph;
pub mod journal;
pub mod lint;
pub mod names;
pub mod note;
pub mod people;
pub mod tasks;
