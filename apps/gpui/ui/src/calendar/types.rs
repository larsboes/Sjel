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
