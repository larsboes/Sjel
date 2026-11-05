//! Status badges, pills, and indicator chips.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, FontWeight, SharedString, Window,
};

use super::tokens::{dot, RADIUS_SM};
use crate::theme::{tint, Palette};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DueStatus {
    Overdue(String),
    Today,
    Tomorrow,
    Upcoming(String),
    None,
}

pub fn due_status(due: Option<&str>, done: bool, today: &str) -> DueStatus {
    let Some(due) = due.filter(|d| !d.is_empty()) else {
        return DueStatus::None;
    };
    if done {
        return DueStatus::Upcoming(due.to_string());
    }
    if due < today {
        DueStatus::Overdue(due.to_string())
    } else if due == today {
        DueStatus::Today
    } else {
        if let (Ok(d), Ok(t)) = (
            chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d"),
        ) {
            if d == t + chrono::Duration::days(1) {
                return DueStatus::Tomorrow;
            }
        }
        DueStatus::Upcoming(due.to_string())
    }
}

pub fn filter_pill(
    label: &'static str,
    count: usize,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("filter-pill-{label}")))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(8.))
        .text_xs()
        .cursor_pointer()
        .when(active, |el| {
            el.bg(rgba(tint(palette.accent, 0x22)))
                .text_color(rgb(palette.accent))
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!active, |el| {
            el.bg(rgba(palette.raised))
                .text_color(rgb(palette.muted))
                .hover(|s| s.text_color(rgb(palette.text)))
        })
        .on_click(on_click)
        .child(label)
        .child(
            div()
                .opacity(if active { 1.0 } else { 0.7 })
                .child(format!("({count})")),
        )
        .into_any_element()
}

pub fn priority_pill(priority: u8, palette: Palette) -> AnyElement {
    let (bg_color, border_color, text_color, label) = match priority {
        1 => (
            tint(palette.red, 0x20),
            tint(palette.red, 0x40),
            palette.red,
            "P1",
        ),
        2 => (
            tint(palette.amber, 0x18),
            tint(palette.amber, 0x35),
            palette.amber,
            "P2",
        ),
        _ => (palette.raised, palette.border, palette.muted, "P3"),
    };
    div()
        .px_2()
        .py(px(1.5))
        .rounded(px(8.))
        .bg(rgba(bg_color))
        .border_1()
        .border_color(rgba(border_color))
        .text_color(rgb(text_color))
        .text_xs()
        .font_weight(if priority == 1 {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .child(label)
        .into_any_element()
}

/// A task status as a pill: the label `tasks::status_label` gives it, coloured by how it stands.
pub fn status_pill(status: &str, palette: Palette) -> AnyElement {
    let color = match status {
        "doing" => palette.accent,
        "waiting" => palette.amber,
        "blocked" => palette.red,
        "done" => palette.green,
        _ => palette.muted,
    };
    div()
        .px_2()
        .py(px(1.5))
        .rounded(px(8.))
        .bg(rgba(tint(color, 0x18)))
        .border_1()
        .border_color(rgba(tint(color, 0x35)))
        .text_color(rgb(color))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .when(status == "cancelled", |pill| pill.line_through())
        .child(crate::tasks::status_label(status))
        .into_any_element()
}

pub fn due_badge(due: Option<&str>, done: bool, today: &str, palette: Palette) -> AnyElement {
    match due_status(due, done, today) {
        DueStatus::Today => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(8.))
            .bg(rgba(tint(palette.amber, 0x22)))
            .border_1()
            .border_color(rgba(tint(palette.amber, 0x38)))
            .text_color(rgb(palette.amber))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Today")
            .into_any_element(),
        DueStatus::Tomorrow => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(8.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .text_color(rgb(palette.text))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Tomorrow")
            .into_any_element(),
        DueStatus::Overdue(d) => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(8.))
            .bg(rgba(tint(palette.red, 0x22)))
            .border_1()
            .border_color(rgba(tint(palette.red, 0x38)))
            .text_color(rgb(palette.red))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(format!("Overdue · {d}"))
            .into_any_element(),
        DueStatus::Upcoming(d) => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(8.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .text_color(rgb(palette.muted))
            .text_xs()
            .child(d)
            .into_any_element(),
        DueStatus::None => div()
            .text_sm()
            .text_color(rgb(palette.muted))
            .child("—")
            .into_any_element(),
    }
}

/// Badge showing sync and offline mutation queue status.
pub fn sync_status_badge(pending: usize, syncing: bool, palette: Palette) -> AnyElement {
    let (text, color, bg) = if syncing {
        ("Syncing…", palette.accent, tint(palette.accent, 0x20))
    } else if pending > 0 {
        ("Offline queue", palette.amber, tint(palette.amber, 0x20))
    } else {
        ("Synced", palette.green, tint(palette.green, 0x20))
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(RADIUS_SM)
        .bg(rgba(bg))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(dot(color))
        .child(if pending > 0 && !syncing {
            format!("{text} ({pending})")
        } else {
            text.to_string()
        })
        .into_any_element()
}

/// Semantic tone chip (`text` on `tint(tone, 0x20)` background with `tint(tone, 0x38)` border).
pub fn tone_chip(text: impl Into<SharedString>, tone: u32) -> AnyElement {
    div()
        .px_2()
        .py(px(1.5))
        .rounded(RADIUS_SM)
        .bg(rgba(tint(tone, 0x20)))
        .border_1()
        .border_color(rgba(tint(tone, 0x38)))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(tone))
        .child(text.into())
        .into_any_element()
}

/// Right-aligned monospace text for tabular numerical alignment (amounts, dates, counts, sizes).
pub fn mono_num(text: impl Into<SharedString>, color: u32) -> gpui_kit::Div {
    div()
        .font_family(".AppleSystemUIFontMonospaced")
        .text_right()
        .text_color(rgb(color))
        .child(text.into())
}
