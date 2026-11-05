//! Common types, tab enums, and formatting utilities for Backup views.
#[cfg(test)]
use sjel_gpui_core::backup::format_age_seconds;
use sjel_gpui_core::backup::format_bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackupTab {
    #[default]
    Contracts,
    Targets,
    History,
}

#[cfg(test)]
pub fn age(seconds: Option<u64>) -> String {
    format_age_seconds(seconds)
}

pub fn size(bytes: u64) -> String {
    format_bytes(Some(bytes))
}
