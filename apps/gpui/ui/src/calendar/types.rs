//! Common types, enums, and component states for Calendar views.
use gpui::{AnyElement, Entity, FontWeight, Subscription};
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::time_field::TimeFieldState;
use gpui_kit::{div, prelude::*, px, rgb, rgba};
use sjel_gpui_core::calendar::kind_color;
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

/// What the mobile day page asks its host to do. The host owns the network, the copy on disk, and the sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarEvent {
    /// The person asked for newer data: a sync into the store, or a newer copy when there is none.
    Refresh,
    /// An entry was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
    /// The new-entry form opened (`true`) or closed, so the host can show or hide the keyboard.
    Editing(bool),
}

/// A calendar action handled by the application that owns navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarIntent {
    PlanTravel { day: chrono::NaiveDate },
    OpenTrip { plan_id: String },
    OpenMail { item_id: Option<String> },
    OpenInbox,
}

/// Where the week on screen was read from, which is also what the page may do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// This device's own store: the phone's replica, or the Mac's database. Writes land here.
    Store,
    /// The Mac's read-only copy, because the store holds no calendar yet.
    Copy,
    /// Neither answered.
    Unread,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DayViewMode {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Day,
    Month,
    Week,
}

/// The new-entry form for mobile day view: a title, times, and commitment chips.
pub(crate) struct Composer {
    pub(crate) input: Entity<InputState>,
    pub(crate) all_day: bool,
    pub(crate) start: String,
    pub(crate) end: String,
    pub(crate) commitment: &'static str,
    /// The listener that saves on Enter, held for as long as the form is open.
    pub(crate) _subscription: Subscription,
}

/// The desktop new-entry form's fields.
pub(crate) struct Form {
    pub(crate) editing: Option<CalendarEntry>,
    pub(crate) title: Entity<InputState>,
    pub(crate) start_date: Entity<DatePickerState>,
    pub(crate) end_date: Entity<DatePickerState>,
    pub(crate) start: Entity<TimeFieldState>,
    pub(crate) end: Entity<TimeFieldState>,
    pub(crate) all_day: bool,
    pub(crate) kind: String,
    pub(crate) commitment: String,
    pub(crate) error: Option<&'static str>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

/// One entry as a coloured badge pill and its title.
pub(crate) fn chip(entry: &CalendarEntry, palette: Palette) -> gpui_kit::Div {
    let color = kind_color(&entry.kind);
    let is_possible = entry.commitment == "possible" || entry.commitment == "tentative";
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w_0()
        .px_1p5()
        .py(px(1.5))
        .rounded(px(4.))
        .bg(rgba(tint(color, if is_possible { 0x18 } else { 0x30 })))
        .border_1()
        .border_color(if is_possible {
            rgba(tint(color, 0x40))
        } else {
            rgba(tint(color, 0x60))
        })
        .hover(|el| el.bg(rgba(tint(color, if is_possible { 0x2c } else { 0x48 }))))
        .child(
            div()
                .w(px(6.))
                .h(px(6.))
                .flex_none()
                .rounded_full()
                .when(is_possible, |dot| dot.border_1().border_color(rgb(color)))
                .when(!is_possible, |dot| dot.bg(rgb(color))),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if is_possible {
                    palette.muted
                } else {
                    palette.text
                }))
                .child(entry.title.clone()),
        )
}

pub(crate) fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}
