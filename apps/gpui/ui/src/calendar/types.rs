//! Common types, enums, and component states for Calendar views.
use gpui::{AnyElement, FontWeight};
use gpui_kit::{div, prelude::*, px, rgb, rgba};
pub use sjel_gpui_core::calendar::CalendarSyncStatus;
use sjel_gpui_core::shell::CalendarEntry;

use crate::theme::tint;
use crate::Palette;

/// Badge showing sync status for the calendar (Google, iCloud, replica).
pub fn sync_status_mark(status: &CalendarSyncStatus, palette: Palette) -> AnyElement {
    let (label, color, bg) = if status.has_error() {
        (status.label(), palette.red, tint(palette.red, 0x20))
    } else if status.replica_synced_at.is_some()
        || status.google_synced_at.is_some()
        || status.icloud_synced_at.is_some()
    {
        (status.label(), palette.green, tint(palette.green, 0x20))
    } else {
        (
            "Sync status unavailable".to_string(),
            palette.muted,
            tint(palette.muted, 0x20),
        )
    };

    div()
        .id("calendar-sync-status")
        .debug_selector(|| "calendar-sync-status".into())
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgba(bg))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(color)))
        .child(label)
        .into_any_element()
}

/// A calendar action handled by the application that owns navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarIntent {
    PlanTravel { day: chrono::NaiveDate },
    OpenTrip { plan_id: String },
    OpenMail { item_id: Option<String> },
    OpenInbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Month,
    Week,
}

/// The application-owned part of an Ely editor session.
pub(crate) struct Form {
    pub(crate) editing: Option<CalendarEntry>,
    pub(crate) draft: crate::ely::calendar::EventDraft,
    pub(crate) kind: String,
    pub(crate) commitment: String,
    pub(crate) session: u64,
}

pub(crate) fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}
