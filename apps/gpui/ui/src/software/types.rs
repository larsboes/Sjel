//! Filtering, labels, formatters, and decision constants for software inventory.

use crate::theme::Palette;
use sjel_gpui_core::shell::SoftwareApp;

pub const SCAN: &str = "software:scan";

/// Which rows the page lists. Review holds every kind: a stale decision is one list to work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Apps,
    Leftovers,
    Tools,
    Review,
}

pub const FILTERS: [(Filter, &str); 4] = [
    (Filter::Apps, "Apps"),
    (Filter::Leftovers, "Leftovers"),
    (Filter::Tools, "Tools"),
    (Filter::Review, "Review"),
];

/// Present rows only: an uninstalled one keeps its history in the store, not on this page.
pub fn shows(filter: Filter, app: &SoftwareApp) -> bool {
    app.present
        && match filter {
            Filter::Apps => app.kind == "app" || app.kind.is_empty(),
            Filter::Leftovers => app.kind == "leftover",
            Filter::Tools => app.kind == "tool",
            Filter::Review => app.review.is_some(),
        }
}

pub const DECISIONS: [(&str, &str); 4] = [
    ("use", "Use"),
    ("remove", "Remove"),
    ("replace", "Replace"),
    ("rebuild", "Rebuild"),
];

/// Says what the button will do: Trash for an app or leftover, a manager's uninstall otherwise.
pub fn uninstall_label(app: &SoftwareApp) -> String {
    match (app.kind.as_str(), app.source.as_str()) {
        ("tool", manager) => format!("Uninstall ({manager})"),
        (_, source) if source.starts_with("cask:") => "Uninstall (brew cask)".to_owned(),
        _ => "Move to Trash".to_owned(),
    }
}

pub fn decision_look(decision: Option<&str>, palette: Palette) -> (u32, &'static str) {
    match decision {
        Some("use") => (palette.green, "use"),
        Some("remove") => (palette.red, "remove"),
        Some("replace") => (palette.amber, "replace"),
        Some("rebuild") => (palette.amber, "rebuild"),
        _ => (palette.muted, ""),
    }
}

/// The date part of Spotlight's stamp, or `never` when it holds none.
pub fn last_used(stamp: Option<&str>) -> String {
    match stamp.and_then(|s| s.get(..10)) {
        Some(date) => date.to_owned(),
        None => "never".to_owned(),
    }
}

pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.;
    if mb >= 1_024. {
        format!("{:.1} GB", mb / 1_024.)
    } else {
        format!("{mb:.0} MB")
    }
}
