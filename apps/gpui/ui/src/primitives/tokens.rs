//! Design tokens and atomic micro-primitives.

use gpui_kit::{div, prelude::*, px, rgb, AnyElement, IntoElement, Pixels, SharedString};

/// Standard system font name for tabular/monospace digits.
pub const FONT_MONO_TABULAR: &str = ".AppleSystemUIFontMonospaced";

/// Small corner radius (`4px`): badges, chips, kbd hints, inline pills.
pub const RADIUS_SM: Pixels = px(4.);
/// Medium corner radius (`8px`): inputs, buttons, nested cards, table frames.
pub const RADIUS_MD: Pixels = px(8.);
/// Large corner radius (`12px`): top-level cards, modals, primary surfaces.
pub const RADIUS_LG: Pixels = px(12.);

/// Solid color indicator dot (7px diameter circular element).
pub fn dot(color: u32) -> impl IntoElement {
    div().flex_none().size(px(7.)).rounded_full().bg(rgb(color))
}

/// Small muted note text.
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
