//! Shared UI primitives: sections, framed lists, cards, badges, pills, progress bars, and density-aware rows.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, FontWeight, SharedString, Window,
};

use crate::theme::{Density, Palette};

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

pub fn section(title: &'static str, body: AnyElement, palette: Palette) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(title),
        )
        .child(body)
}

pub fn list(rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    let count = rows.len();
    let mut list = div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden();
    for (index, row) in rows.into_iter().enumerate() {
        list = list.child(row);
        if index + 1 < count {
            list = list.child(div().h(px(1.)).mx_4().bg(rgba(palette.border)));
        }
    }
    list.into_any_element()
}

/// Standard desktop row.
pub fn row() -> gpui_kit::Div {
    row_for_density(Density::Desktop)
}

/// Density-aware row respecting Apple HIG touch targets on mobile (44pt+).
pub fn row_for_density(density: Density) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px(density.pad_x())
        .py(density.pad_y())
        .min_h(density.row_height())
}

pub fn card(body: AnyElement, palette: Palette) -> AnyElement {
    div()
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .p_4()
        .child(body)
        .into_any_element()
}

pub fn dot(color: u32) -> impl IntoElement {
    div().flex_none().size(px(7.)).rounded_full().bg(rgb(color))
}

pub fn note(text: impl Into<SharedString>, color: u32) -> AnyElement {
    div()
        .text_sm()
        .text_color(rgb(color))
        .child(text.into())
        .into_any_element()
}

/// Reserves the final height while the first answer is on its way, so nothing jumps.
pub fn placeholder(height: f32) -> AnyElement {
    div().h(px(height)).into_any_element()
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
        .rounded(px(6.))
        .text_xs()
        .cursor_pointer()
        .when(active, |el| {
            el.bg(rgba(palette.accent & 0xffffff | 0x22000000))
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
            palette.red & 0xffffff | 0x20000000,
            palette.red & 0xffffff | 0x40000000,
            palette.red,
            "P1",
        ),
        2 => (
            palette.amber & 0xffffff | 0x18000000,
            palette.amber & 0xffffff | 0x35000000,
            palette.amber,
            "P2",
        ),
        _ => (palette.raised, palette.border, palette.muted, "P3"),
    };
    div()
        .px_2()
        .py(px(1.5))
        .rounded(px(4.))
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

pub fn due_badge(due: Option<&str>, done: bool, today: &str, palette: Palette) -> AnyElement {
    match due_status(due, done, today) {
        DueStatus::Today => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
            .bg(rgba(palette.amber & 0xffffff | 0x22000000))
            .border_1()
            .border_color(rgba(palette.amber & 0xffffff | 0x38000000))
            .text_color(rgb(palette.amber))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Today")
            .into_any_element(),
        DueStatus::Tomorrow => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
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
            .rounded(px(4.))
            .bg(rgba(palette.red & 0xffffff | 0x22000000))
            .border_1()
            .border_color(rgba(palette.red & 0xffffff | 0x38000000))
            .text_color(rgb(palette.red))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(format!("Overdue · {d}"))
            .into_any_element(),
        DueStatus::Upcoming(d) => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
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

pub fn progress_bar(progress: f32, color: u32, palette: Palette) -> AnyElement {
    let width = 48.0;
    let clamped = progress.clamp(0.0, 1.0);
    div()
        .w(px(width))
        .h(px(4.))
        .rounded_full()
        .bg(rgba(palette.border))
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(color))
                .w(px(width * clamped)),
        )
        .into_any_element()
}

pub fn back_button(
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    palette: Palette,
) -> AnyElement {
    div()
        .id("back")
        .text_sm()
        .text_color(rgb(palette.accent))
        .cursor_pointer()
        .hover(move |s| s.text_color(rgb(palette.text)))
        .on_click(on_click)
        .child(label.into())
        .into_any_element()
}

/// Badge showing sync and offline mutation queue status.
pub fn sync_status_badge(pending: usize, syncing: bool, palette: Palette) -> AnyElement {
    let (text, color, bg) = if syncing {
        (
            "Syncing…",
            palette.accent,
            palette.accent & 0xffffff | 0x20000000,
        )
    } else if pending > 0 {
        (
            "Offline queue",
            palette.amber,
            palette.amber & 0xffffff | 0x20000000,
        )
    } else {
        (
            "Synced",
            palette.green,
            palette.green & 0xffffff | 0x20000000,
        )
    };
    div()
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
        .child(dot(color))
        .child(if pending > 0 && !syncing {
            format!("{text} ({pending})")
        } else {
            text.to_string()
        })
        .into_any_element()
}
