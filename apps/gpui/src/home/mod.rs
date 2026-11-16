//! Today answers "what needs me now": what is waiting on me, what is urgent, and the day itself.
//!
//! Home and Today were two pages and are one (`core::area`), so the day's timeline is a block
//! here rather than a window of its own. What needs you costs no height when nothing does.

pub mod cards;
pub mod focus_tasks;
pub mod needs_you;
pub mod prefs;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, IntoElement, SharedString,
    Window,
};

use crate::route::Page;
use crate::workspace::Workspace;
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::feedback::Toast;
use sjel_gpui_ui::ely::primitives::{Severity, Tooltip};
use sjel_gpui_ui::ely::theme::ControlSize;
use sjel_gpui_ui::feedback::WindowToastExt;
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::theme::{tint, Palette};

pub use cards::{prompt_outbox, today, travel_card};
pub use focus_tasks::focus_tasks;
pub use needs_you::needs_you;
pub use prefs::{HomeMode, HomePreferences};

fn save_preferences(view: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Err(error) = view.home_prefs.save() {
        window.push_toast(Toast::new(error).severity(Severity::Danger), cx);
    }
}

fn home_toolbar(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
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

    if urgent_count == 0 && approvals_count == 0 && down_count == 0 {
        return None;
    }

    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .py_1()
            .px_1()
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
            })
            .into_any_element(),
    )
}

fn customize_panel(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if !view.home_customize_open {
        return None;
    }
    let prefs = view.home_prefs;

    let mut mode_row = div().flex().flex_wrap().items_center().gap_2();
    for mode in HomeMode::ALL {
        let active = prefs.mode == mode;
        let label = mode.label();
        let btn = Button::new(SharedString::from(format!("home-mode-{label}")), label)
            .size(ControlSize::Sm)
            .variant(if active {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Ghost
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.home_prefs.mode = mode;
                save_preferences(view, window, cx);
                cx.notify();
            }));
        mode_row = mode_row.child(
            div()
                .id(SharedString::from(format!("home-mode-tip-{label}")))
                .tooltip(Tooltip::text(mode.description()))
                .child(btn),
        );
    }

    let toggle = |id: &'static str,
                  label: &'static str,
                  checked: bool,
                  on_change: fn(&mut HomePreferences, bool)| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(Checkbox::new(id).checked(checked).on_click(cx.listener(
                move |view, value: &bool, window, cx| {
                    on_change(&mut view.home_prefs, *value);
                    save_preferences(view, window, cx);
                    cx.notify();
                },
            )))
            .child(div().text_xs().text_color(rgb(palette.text)).child(label))
    };

    Some(
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("MODE"),
                            )
                            .child(mode_row)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(prefs.mode.description()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("home-customize-reset", "Reset")
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.home_prefs = HomePreferences::default();
                                        save_preferences(view, window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("home-customize-done", "Done")
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.home_customize_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("SECTIONS"),
                    )
                    .child(toggle(
                        "home-sec-timeline",
                        "Timeline",
                        prefs.show_timeline,
                        |p, v| {
                            p.show_timeline = v;
                        },
                    ))
                    .child(toggle(
                        "home-sec-tasks",
                        "Urgent tasks",
                        prefs.show_tasks,
                        |p, v| {
                            p.show_tasks = v;
                        },
                    ))
                    .child(toggle(
                        "home-sec-travel",
                        "Trip & Map",
                        prefs.show_travel,
                        |p, v| {
                            p.show_travel = v;
                        },
                    ))
                    .child(toggle(
                        "home-sec-prompts",
                        "Prompts",
                        prefs.show_prompts,
                        |p, v| {
                            p.show_prompts = v;
                        },
                    ))
                    .child(toggle(
                        "home-sec-spending",
                        "Spending",
                        prefs.show_spending,
                        |p, v| {
                            p.show_spending = v;
                        },
                    )),
            )
            .into_any_element(),
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
    let prompts = view
        .home_prefs
        .show_prompts
        .then(|| prompt_outbox(view, palette, cx))
        .flatten();
    let tasks = view
        .home_prefs
        .show_tasks
        .then(|| focus_tasks(view, palette, cx))
        .flatten();
    let travel = travel_card(view, palette, cx);
    let travel_imminent = view.home_prefs.effective_show_travel()
        && (view.home_prefs.force_expanded_map()
            || view
                .travel
                .read(cx)
                .home_trip_days_away()
                .is_some_and(|days| days <= 2));
    let use_right_rail = is_wide && (prompts.is_some() || travel_imminent);

    let rail_width = if travel_imminent && view.home_prefs.large_map {
        520.
    } else {
        400.
    };

    let content = if use_right_rail {
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
                    .children(tasks),
            )
            .child(
                div()
                    .w(px(rail_width))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .children(prompts)
                    .children(travel),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .w_full()
            .child(today(view, palette, cx))
            .children(tasks)
            .children(prompts)
            .children(travel)
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .children(home_toolbar(view, palette, cx))
        .children(customize_panel(view, palette, cx))
        .children(needs_you(view, palette, cx))
        .child(content)
}
