//! Shared UI primitives: sections, framed lists, cards, badges, pills, progress bars, and density-aware rows.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, list as gpui_list, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId,
    FontWeight, ListState, MouseButton, Pixels, ScrollHandle, SharedString, Window,
};

use crate::data::Remote;
use crate::theme::{tint, Density, Palette};

/// Small corner radius (`4px`): badges, chips, kbd hints, inline pills.
pub const RADIUS_SM: Pixels = px(4.);
/// Medium corner radius (`8px`): inputs, buttons, nested cards, table frames.
pub const RADIUS_MD: Pixels = px(8.);
/// Large corner radius (`12px`): top-level cards, modals, primary surfaces.
pub const RADIUS_LG: Pixels = px(12.);

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
        .rounded(px(16.))
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

/// Virtualized vertical list with a bound scrollbar and flex height contract (`flex_1` + `min_h_0`).
pub fn virtual_list(
    id: impl Into<ElementId>,
    state: &ListState,
    render_item: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .vertical_scrollbar(state)
        .child(gpui_list(state.clone(), render_item).size_full())
        .into_any_element()
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
        .rounded(px(16.))
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

/// Compact horizontal metric cell (`label`, `value`, optional `detail`, semantic `tone`).
pub fn stat_card(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Option<SharedString>,
    tone: u32,
    palette: Palette,
) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(120.))
        .p_3()
        .rounded(RADIUS_MD)
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(label.into()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone))
                .child(value.into()),
        )
        .when_some(detail, |el, det| {
            el.child(div().text_xs().text_color(rgb(palette.muted)).child(det))
        })
        .into_any_element()
}

/// Horizontal strip of [`stat_card`] metrics.
pub fn stat_strip(cards: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_stretch()
        .gap_3()
        .children(cards)
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

/// Standardised sub-tab strip with optional counts.
pub fn sub_tabs(
    id_prefix: &'static str,
    tabs: Vec<(&'static str, Option<usize>, bool)>,
    palette: Palette,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    let mut strip = div().flex().flex_wrap().items_center().gap_1p5();
    for (idx, (label, count, active)) in tabs.into_iter().enumerate() {
        let on_select = on_select.clone();
        let text = match count {
            Some(c) => format!("{label} ({c})"),
            None => label.to_owned(),
        };
        strip = strip.child(
            div()
                .id(SharedString::from(format!("{id_prefix}-tab-{idx}")))
                .px_3()
                .py_1()
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .text_xs()
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .bg(if active {
                    rgba(tint(palette.accent, 0x20))
                } else {
                    rgba(palette.raised)
                })
                .text_color(if active {
                    rgb(palette.accent)
                } else {
                    rgb(palette.muted)
                })
                .hover(move |s| {
                    s.bg(rgba(if active {
                        tint(palette.accent, 0x28)
                    } else {
                        tint(palette.border, 0x35)
                    }))
                    .text_color(rgb(if active {
                        palette.accent
                    } else {
                        palette.text
                    }))
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_select(idx, window, cx)
                })
                .child(text),
        );
    }
    strip.into_any_element()
}

/// Column header row for dense data tables.
pub fn table_header(
    columns: Vec<(&'static str, Option<Pixels>, bool)>,
    density: Density,
    palette: Palette,
) -> AnyElement {
    let mut row = row_for_density(density)
        .min_h(px(30.))
        .py_1()
        .bg(rgba(tint(palette.raised, 0x30)))
        .border_b_1()
        .border_color(rgba(palette.border))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted));
    for (label, width, right_align) in columns {
        let cell = match width {
            Some(w) => div().w(w).flex_none(),
            None => div().flex_1().min_w_0(),
        };
        row = row.child(
            cell.when(right_align, |c| c.text_right())
                .truncate()
                .child(label),
        );
    }
    row.into_any_element()
}

/// Density-aware interactive data row with hover and selection states.
pub fn data_row(
    id: impl Into<ElementId>,
    density: Density,
    selected: bool,
    palette: Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    row_for_density(density)
        .id(id.into())
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .when(selected, |r| {
            r.bg(rgba(tint(palette.accent, 0x18)))
                .border_l_2()
                .border_color(rgb(palette.accent))
        })
        .when(!selected, |r| r.hover(move |s| s.bg(rgba(palette.raised))))
}

/// Single keyboard shortcut hint (`[key] action`) for footer bars.
pub fn shortcut_hint(key: &'static str, action: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1p5()
                .py(px(1.))
                .rounded(RADIUS_SM)
                .bg(rgba(palette.raised))
                .border_1()
                .border_color(rgba(palette.border))
                .font_family(".AppleSystemUIFontMonospaced")
                .text_xs()
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().text_xs().text_color(rgb(palette.muted)).child(action))
        .into_any_element()
}

/// Footer bar of keyboard shortcut hints.
pub fn key_bar(hints: &[(&'static str, &'static str)], palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .pt_2()
        .text_xs()
        .text_color(rgb(palette.muted))
        .children(hints.iter().map(|(k, a)| shortcut_hint(k, a, palette)))
        .into_any_element()
}

/// Scrollable page body container that always attaches `.track_scroll` and `.vertical_scrollbar`.
pub fn page_scroll(
    id: impl Into<ElementId>,
    scroll: &ScrollHandle,
    body: impl IntoElement,
) -> AnyElement {
    div()
        .id(id.into())
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .vertical_scrollbar(scroll)
        .child(body)
        .into_any_element()
}

/// Uniform state presenter for [`Remote<T>`]: renders a reserved-height loading placeholder,
/// an inline error with a Retry button, or the loaded content.
pub fn remote_state<T>(
    remote: &Remote<T>,
    height: f32,
    palette: Palette,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    render_value: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    if let Some(value) = remote.value.as_ref() {
        return render_value(value);
    }
    if let Some(error) = remote.error.as_ref() {
        return div()
            .p_4()
            .rounded(RADIUS_MD)
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(tint(palette.red, 0x40)))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            )
            .child(
                Button::new("remote-retry")
                    .small()
                    .ghost()
                    .label("Retry")
                    .on_click(on_retry),
            )
            .into_any_element();
    }
    placeholder(height)
}
