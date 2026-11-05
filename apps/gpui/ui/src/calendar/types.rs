//! Common types, enums, and component states for Calendar views.
use gpui::{Entity, FontWeight, Subscription};
use gpui_kit::component::input::InputState;
use gpui_kit::{div, prelude::*, px, rgb, rgba};
use sjel_gpui_core::calendar::kind_color;
use sjel_gpui_core::shell::CalendarEntry;

use crate::theme::tint;
use crate::Palette;

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

/// The desktop new-entry form's fields. The text fields are inputs; the rest are buttons.
pub(crate) struct Form {
    pub(crate) title: Entity<InputState>,
    pub(crate) start: Entity<InputState>,
    pub(crate) end: Entity<InputState>,
    pub(crate) all_day: bool,
    pub(crate) kind: &'static str,
    pub(crate) commitment: &'static str,
    pub(crate) error: Option<&'static str>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

/// Parses "HH:MM" into hour (0-23) and minute (0-59).
pub(crate) fn parse_time(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split(':');
    let h: u32 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next()?.trim().parse().ok()?;
    if h < 24 && m < 60 {
        Some((h, m))
    } else {
        None
    }
}

/// One entry as a coloured badge pill and its title.
pub(crate) fn chip(entry: &CalendarEntry, palette: Palette) -> gpui_kit::Div {
    let color = kind_color(&entry.kind);
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .px_1()
        .py(px(1.5))
        .rounded(px(4.))
        .bg(rgba(tint(color, 0x22)))
        .child(
            div()
                .w(px(5.))
                .h(px(5.))
                .flex_none()
                .rounded_full()
                .bg(rgb(color)),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if entry.commitment == "possible" {
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
