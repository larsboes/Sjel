//! Today event definitions, context re-exports, and row badge formatting.
use chrono::NaiveDate;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, FontWeight};

use crate::theme::{tint, Palette};

pub use crate::context::{
    bind_keys as bind_context_keys, ContextPage, NextDay, PreviousDay, Refresh as RefreshDay,
    Today as TodayAction,
};

/// What the page asks its host to do. The host owns the network, the copy on disk and the sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodayEvent {
    /// The person tapped the day's header or an entry: show that day in the Calendar tab.
    OpenDay(NaiveDate),
    /// The person tapped a project: open it in the Projects page.
    OpenProject(String),
    /// The person tapped the gear, or the banner that says this phone is not paired yet.
    Settings,
    /// The person tapped the note: the soundscape seat of the right edge, which the host slides in.
    Soundscape,
    /// The person tapped the list: the activity seat of the same edge — what this phone is doing.
    Activity,
    /// The person asked for newer data.
    Refresh,
    /// A task was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
    /// The person tapped quick capture on Today to record a note, link, or task.
    Capture,
    /// The person tapped the People & Places shortcut.
    People,
    /// The person tapped the Documents shortcut.
    Documents,
}

/// The due date in the width a phone row can spare: `Overdue` when it has passed, else `30 Sep`.
pub fn due_mark(due: &str, overdue: bool, palette: Palette) -> AnyElement {
    if overdue {
        return div()
            .flex_none()
            .px_2()
            .py(px(2.))
            .rounded(px(6.))
            .bg(rgba(tint(palette.red, 0x22)))
            .text_color(rgb(palette.red))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Overdue")
            .into_any_element();
    }
    let label = sjel_gpui_core::calendar::parse_day(due)
        .map(|day| day.format("%-d %b").to_string())
        .unwrap_or_else(|| due.to_owned());
    div()
        .flex_none()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(label)
        .into_any_element()
}
