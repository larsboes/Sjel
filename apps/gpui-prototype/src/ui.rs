//! The list primitives every page shares: a titled section, a framed list of rows, a status
//! dot, a one-line note and a size-reserving placeholder.

use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, FontWeight, SharedString};

use crate::theme::Palette;

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

pub fn row() -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py(px(10.))
        .min_h(px(48.))
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
