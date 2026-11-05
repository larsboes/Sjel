//! Event types, key bindings, and actions for ContextPage.

use chrono::NaiveDate;
use gpui_kit::{actions, App, KeyBinding};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextEvent {
    OpenCalendar {
        day: NaiveDate,
        entry_id: Option<String>,
    },
    OpenProject(String),
    OpenFinance {
        day: NaiveDate,
    },
}

actions!(
    context_page,
    [
        /// Select the previous day.
        PreviousDay,
        /// Select the next day.
        NextDay,
        /// Select today.
        Today,
        /// Reload data for the day on screen.
        Refresh,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("h", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("[", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("right", NextDay, Some("ContextTimeline")),
        KeyBinding::new("l", NextDay, Some("ContextTimeline")),
        KeyBinding::new("]", NextDay, Some("ContextTimeline")),
        KeyBinding::new("t", Today, Some("ContextTimeline")),
    ]);
}
