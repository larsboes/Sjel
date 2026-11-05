//! Event and mode definitions for the YouTube music importer.

pub enum MusicEvent {
    Close,
}

/// What one run of the importer produces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Song,
    Album,
}
