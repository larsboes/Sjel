//! Today answers "what needs me now": what is waiting on me, what is urgent, and the day itself.
//!
//! Home and Today were two pages and are one (`core::area`), so the day's timeline is a block
//! here rather than a window of its own. What needs you costs no height when nothing does.

pub mod cards;
pub mod focus_tasks;
pub mod needs_you;

use chrono::Local;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, Context, FontWeight, IntoElement, Window};

use crate::route::Page;
use crate::workspace::Workspace;
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::theme::{tint, Palette};

pub use cards::{prompt_outbox, today, travel_card};
pub use focus_tasks::focus_tasks;
pub use needs_you::needs_you;

fn home_toolbar(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let now = Local::now();
    let date_str = now.format("%A, %B %-d").to_string();
    let urgent_count = view.urgent_task_count();
    let approvals_count = view
        .agent
        .value
        .as_ref()
        .map(|a| a.pending.len())
        .unwrap_or(0);
    let down_count = view
        .health
        .value
        .as_ref()
        .map(|h| h.down().len())
        .unwrap_or(0);

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .py_1()
        .px_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(date_str),
                )
                .when(urgent_count > 0, |el| {
                    el.child(
                        div()
                            .id("home-toolbar-urgent")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(2.))
                            .rounded_full()
                            .bg(rgba(tint(palette.accent, 0x22)))
                            .border_1()
                            .border_color(rgba(tint(palette.accent, 0x44)))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.accent))
                            .cursor_pointer()
                            .hover(move |s| s.opacity(0.85))
                            .on_click(cx.listener(|view, _, _, cx| view.go(Page::Projects, cx)))
                            .child(format!("{urgent_count} urgent")),
                    )
                })
                .when(approvals_count > 0, |el| {
                    el.child(
                        div()
                            .id("home-toolbar-approvals")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(2.))
                            .rounded_full()
                            .bg(rgba(tint(palette.amber, 0x22)))
                            .border_1()
                            .border_color(rgba(tint(palette.amber, 0x44)))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.amber))
                            .cursor_pointer()
                            .hover(move |s| s.opacity(0.85))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.right_panel.open_on(Seat::Agent);
                                cx.notify();
                            }))
                            .child(format!("{approvals_count} approvals")),
                    )
                })
                .when(down_count > 0, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(2.))
                            .rounded_full()
                            .bg(rgba(tint(palette.red, 0x22)))
                            .border_1()
                            .border_color(rgba(tint(palette.red, 0x44)))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.red))
                            .child(format!("{down_count} degraded")),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("home-nav-calendar")
                        .xsmall()
                        .ghost()
                        .label("Calendar")
                        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Calendar, cx))),
                )
                .child(
                    Button::new("home-nav-projects")
                        .xsmall()
                        .ghost()
                        .label("Projects")
                        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Projects, cx))),
                )
                .child(
                    Button::new("home-nav-travel")
                        .xsmall()
                        .ghost()
                        .label("Travel")
                        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Travel, cx))),
                )
                .child(
                    Button::new("home-nav-feed")
                        .xsmall()
                        .ghost()
                        .label("Feed")
                        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Feed, cx))),
                ),
        )
}

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
        .child(home_toolbar(view, palette, cx))
        .children(needs_you(view, palette, cx))
        .child(content)
}
