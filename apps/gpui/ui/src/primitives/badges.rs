//! Status badges, pills, and indicator chips.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId, FontWeight,
    SharedString, Window,
};

use super::tokens::RADIUS_SM;
use crate::ely::{
    buttons::{ToggleButton, ToggleItem},
    data_display::{Badge, Tone},
    feedback::InlineMessage,
    primitives::{Severity, Tooltip},
    theme::ControlSize,
};
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

/// A counted filter toggle; the caller owns exclusive selection or independent toggling.
pub fn filter_pill(
    label: &'static str,
    count: usize,
    active: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    ToggleButton::new(
        SharedString::from(format!("filter-pill-{label}")),
        ToggleItem::new(label).label(format!("{label} ({count})")),
        active,
    )
    .size(ControlSize::Sm)
    .on_toggle(move |on, window, cx| on_change(&on, window, cx))
    .into_any_element()
}

pub fn priority_pill(priority: u8, _palette: Palette) -> AnyElement {
    let (tone, label) = match priority {
        1 => (Tone::Danger, "P1"),
        2 => (Tone::Warning, "P2"),
        _ => (Tone::Neutral, "P3"),
    };
    Badge::new(label).tone(tone).into_any_element()
}

/// A task status as a pill: the label `tasks::status_label` gives it, coloured by how it stands.
pub fn status_pill(status: &str, _palette: Palette) -> AnyElement {
    let label = crate::tasks::status_label(status);
    let badge = match status {
        "doing" => Badge::new(label).tone(Tone::Accent).dot(),
        "waiting" => Badge::new(label).tone(Tone::Warning).dot(),
        "blocked" => Badge::new(label).tone(Tone::Danger).dot(),
        "done" => Badge::new(label).tone(Tone::Success).dot(),
        _ => Badge::new(label).tone(Tone::Neutral),
    };
    if status == "cancelled" {
        div().line_through().child(badge).into_any_element()
    } else {
        badge.into_any_element()
    }
}

pub fn due_badge(due: Option<&str>, done: bool, today: &str, palette: Palette) -> AnyElement {
    match due_status(due, done, today) {
        DueStatus::Today => Badge::new("Today").tone(Tone::Warning).into_any_element(),
        DueStatus::Tomorrow => Badge::new("Tomorrow")
            .tone(Tone::Neutral)
            .into_any_element(),
        DueStatus::Overdue(d) => Badge::new(format!("Overdue · {d}"))
            .tone(Tone::Danger)
            .into_any_element(),
        DueStatus::Upcoming(d) => Badge::new(d).tone(Tone::Neutral).into_any_element(),
        DueStatus::None => div()
            .text_sm()
            .text_color(rgb(palette.muted))
            .child("—")
            .into_any_element(),
    }
}

/// Badge showing sync and offline mutation queue status.
pub fn sync_status_badge(pending: usize, syncing: bool, _palette: Palette) -> AnyElement {
    let (text, severity) = if syncing {
        ("Syncing…".to_owned(), Severity::Info)
    } else if pending > 0 {
        (format!("Offline queue ({pending})"), Severity::Warning)
    } else {
        ("Synced".to_owned(), Severity::Success)
    };
    InlineMessage::new(severity, text).into_any_element()
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

/// Interactive tinted status pill with tooltip and click handler.
pub fn interactive_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: u32,
    tooltip: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap_1p5()
        .px_2p5()
        .py(px(3.))
        .rounded_full()
        .bg(rgba(tint(tone, 0x20)))
        .border_1()
        .border_color(rgba(tint(tone, 0x48)))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(tone))
        .cursor_pointer()
        .tooltip(Tooltip::text(tooltip.into()))
        .hover(move |s| s.opacity(0.85))
        .on_click(on_click)
        .child(label.into())
        .into_any_element()
}
