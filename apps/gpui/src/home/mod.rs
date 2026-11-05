//! Today answers "what needs me now": what is waiting on me, what is urgent, and the day itself.
//!
//! Home and Today were two pages and are one (`core::area`), so the day's timeline is a block
//! here rather than a window of its own. What needs you costs no height when nothing does.

pub mod cards;
pub mod focus_tasks;
pub mod needs_you;

use gpui_kit::{div, prelude::*, px, Context, IntoElement, Window};

use crate::workspace::Workspace;
use sjel_gpui_ui::theme::Palette;

pub use cards::{prompt_outbox, today, travel_card};
pub use focus_tasks::focus_tasks;
pub use needs_you::needs_you;

pub fn render(
    view: &Workspace,
    palette: Palette,
    window: &Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let window_width = f32::from(window.viewport_size().width);
    let is_wide = window_width >= 1080.;

    let content = if is_wide {
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_6()
            .w_full()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(today(view, palette, cx))
                    .children(focus_tasks(view, palette, cx)),
            )
            .child(
                div()
                    .w(px(400.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .children(prompt_outbox(view, palette, cx))
                    .child(travel_card(view, palette, cx)),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .w_full()
            .child(today(view, palette, cx))
            .children(focus_tasks(view, palette, cx))
            .children(prompt_outbox(view, palette, cx))
            .child(travel_card(view, palette, cx))
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .children(needs_you(view, palette, cx))
        .child(content)
}
