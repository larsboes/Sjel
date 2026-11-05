//! Card frames, lists, density rows, progress bars, and metrics.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, list as gpui_list, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId,
    FontWeight, ListState, SharedString, Window,
};

use super::tokens::RADIUS_MD;
use crate::theme::{tint, Density, Palette};

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

/// Interactive horizontal metric cell (`label`, `value`, optional `detail`, semantic `tone`, `active` state, and click handler).
pub fn stat_card_interactive(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Option<SharedString>,
    tone: u32,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .flex_1()
        .min_w(px(120.))
        .p_3()
        .rounded(RADIUS_MD)
        .bg(rgba(if active {
            tint(palette.accent, 0x18)
        } else {
            palette.surface
        }))
        .border_1()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(palette.border)
        })
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgba(if active {
                tint(palette.accent, 0x24)
            } else {
                palette.raised
            }))
        })
        .on_click(on_click)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if active {
                            palette.accent
                        } else {
                            palette.muted
                        }))
                        .child(label.into()),
                )
                .when(active, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .child("Filtered"),
                    )
                }),
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
