//! Today answers "what needs me now": what is waiting on me, what is urgent, and the day itself.
//!
//! Home and Today were two pages and are one (`core::area`), so the day's timeline is a block
//! here rather than a window of its own. What needs you costs no height when nothing does.

pub mod cards;
pub mod focus_tasks;
pub mod needs_you;
pub mod prefs;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::IconName;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, IntoElement, SharedString,
    Window,
};

use crate::route::Page;
use crate::workspace::{OpenPalette, Workspace};
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::feedback::Toast;
use sjel_gpui_ui::ely::primitives::{Severity, Tooltip};
use sjel_gpui_ui::ely::theme::ControlSize;
use sjel_gpui_ui::feedback::WindowToastExt;
use sjel_gpui_ui::primitives::{
    interactive_pill, quick_action_button, sub_tabs, surface_card, QuickAction,
};
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::theme::Palette;

pub use cards::{
    inbox_triage_card, projects_momentum, prompt_outbox, scratchpad_card, today, travel_card,
};
pub use focus_tasks::focus_tasks;
pub use needs_you::needs_you;
pub use prefs::{HomeMode, HomePreferences};

fn save_preferences(view: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Err(error) = view.home_prefs.save() {
        window.push_toast(Toast::new(error).severity(Severity::Danger), cx);
    }
}

fn hero_strip(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
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
    let soundscape = view.soundscape.value.as_ref();
    let playing = soundscape.is_some_and(|s| s.playing);
    let preset = soundscape
        .map(|s| s.preset.clone())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "Soundscape".to_string());
    let focus_active = view.home_prefs.mode == HomeMode::Focus;

    let mut chips = div().flex().flex_wrap().items_center().gap_2();
    if urgent_count > 0 {
        chips = chips.child(interactive_pill(
            "home-toolbar-urgent",
            format!("{urgent_count} urgent"),
            palette.accent,
            "Open urgent tasks in Projects",
            cx.listener(|view, _, _, cx| view.go(Page::Projects, cx)),
        ));
    }
    if approvals_count > 0 {
        chips = chips.child(interactive_pill(
            "home-toolbar-approvals",
            format!("{approvals_count} approvals"),
            palette.amber,
            "Review pending agent writes",
            cx.listener(|view, _, _, cx| {
                view.right_panel.open_on(Seat::Agent);
                cx.notify();
            }),
        ));
    }
    if down_count > 0 {
        chips = chips.child(interactive_pill(
            "home-toolbar-degraded",
            format!("{down_count} degraded"),
            palette.red,
            "Inspect degraded capabilities",
            cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx)),
        ));
    }
    chips = chips.child(interactive_pill(
        "home-toolbar-soundscape",
        if playing {
            format!("Playing · {preset}")
        } else {
            format!("{preset} · Paused")
        },
        if playing {
            palette.green
        } else {
            palette.muted
        },
        "Open Soundscape controls",
        cx.listener(|view, _, window, cx| view.open_soundscape(window, cx)),
    ));
    if view.home_prefs.mode != HomeMode::Auto {
        chips = chips.child(interactive_pill(
            "home-toolbar-mode",
            format!("{} mode", view.home_prefs.mode.label()),
            palette.accent,
            "Switch back to Auto mode",
            cx.listener(|view, _, window, cx| {
                view.home_prefs.mode = HomeMode::Auto;
                save_preferences(view, window, cx);
                cx.notify();
            }),
        ));
    }

    let actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(quick_action_button(
            QuickAction {
                id: "home-qa-capture".into(),
                icon: IconName::Plus,
                label: "Scratchpad".into(),
                tooltip: "Open 24-hour quick Scratchpad (⌘\\)".into(),
                active: view.right_panel.is_open() && view.right_panel.seat() == Seat::Scratchpad,
                tone: None,
                palette,
            },
            cx.listener(|view, _, window, cx| view.toggle_scratchpad(window, cx)),
        ))
        .child(quick_action_button(
            QuickAction {
                id: "home-qa-task".into(),
                icon: IconName::CircleCheck,
                label: "+ Task".into(),
                tooltip: "Open Projects & Tasks (⌘4)".into(),
                active: false,
                tone: None,
                palette,
            },
            cx.listener(|view, _, _, cx| view.go(Page::Projects, cx)),
        ))
        .child(quick_action_button(
            QuickAction {
                id: "home-qa-ask".into(),
                icon: IconName::Search,
                label: "Ask".into(),
                tooltip: "Search or ask Sjel (⌘K)".into(),
                active: false,
                tone: None,
                palette,
            },
            cx.listener(|view, _, window, cx| view.open_palette(&OpenPalette, window, cx)),
        ))
        .child(quick_action_button(
            QuickAction {
                id: "home-qa-sound".into(),
                icon: if playing {
                    IconName::Pause
                } else {
                    IconName::Play
                },
                label: if playing { "Pause" } else { "Sound" }.into(),
                tooltip: "Toggle ambient soundscape (⌘0)".into(),
                active: playing,
                tone: Some(palette.green),
                palette,
            },
            cx.listener(move |view, _, window, cx| view.toggle_soundscape(!playing, window, cx)),
        ))
        .child(quick_action_button(
            QuickAction {
                id: "home-qa-focus".into(),
                icon: IconName::Eye,
                label: "Focus".into(),
                tooltip: if focus_active {
                    "Exit Focus mode"
                } else {
                    "Enter distraction-free Focus mode"
                }
                .into(),
                active: focus_active,
                tone: None,
                palette,
            },
            cx.listener(|view, _, window, cx| {
                view.home_prefs.mode = if view.home_prefs.mode == HomeMode::Focus {
                    HomeMode::Auto
                } else {
                    HomeMode::Focus
                };
                save_preferences(view, window, cx);
                cx.notify();
            }),
        ));

    surface_card(palette)
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .px_4()
        .py_3p5()
        .child(chips)
        .child(actions)
        .into_any_element()
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
                    ))
                    .child(toggle(
                        "home-sec-inbox",
                        "Inbox",
                        prefs.show_inbox,
                        |p, v| {
                            p.show_inbox = v;
                        },
                    ))
                    .child(toggle(
                        "home-sec-projects",
                        "Projects",
                        prefs.show_projects,
                        |p, v| {
                            p.show_projects = v;
                        },
                    )),
            )
            .into_any_element(),
    )
}

pub fn render(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let window_width = f32::from(window.viewport_size().width);
    let is_wide = window_width >= 1080.;
    let scratchpad = scratchpad_card(view, palette, window, cx);
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
    let inbox_card = inbox_triage_card(view, palette, cx);
    let projects_card = projects_momentum(view, palette, cx);
    let travel = travel_card(view, palette, cx);
    let travel_imminent = view.home_prefs.effective_show_travel()
        && (view.home_prefs.force_expanded_map()
            || view
                .travel
                .read(cx)
                .home_trip_days_away()
                .is_some_and(|days| days <= 2));
    let use_right_rail = is_wide;

    let rail_width = if travel_imminent && view.home_prefs.large_map {
        520.
    } else {
        400.
    };

    let mut rail_sections: Vec<(&'static str, &'static str, AnyElement)> =
        vec![("scratchpad", "Scratchpad", scratchpad)];
    if let Some(el) = prompts {
        rail_sections.push(("prompts", "Prompts", el));
    }
    if let Some(el) = inbox_card {
        rail_sections.push(("inbox", "Inbox", el));
    }
    if let Some(el) = projects_card {
        rail_sections.push(("projects", "Projects", el));
    }
    if let Some(el) = travel {
        rail_sections.push(("travel", "Trip & Map", el));
    }

    let build_rail = |mut sections: Vec<(&'static str, &'static str, AnyElement)>,
                      selected_idx: usize,
                      cx: &mut Context<Workspace>|
     -> Option<AnyElement> {
        if sections.is_empty() {
            return None;
        }
        if sections.len() == 1 {
            return Some(sections.remove(0).2);
        }
        let active_idx = selected_idx.min(sections.len() - 1);
        let tabs_spec = sections
            .iter()
            .enumerate()
            .map(|(ix, (_, label, _))| (*label, None, ix == active_idx))
            .collect::<Vec<_>>();
        let panel = sections.remove(active_idx).2;
        let owner = cx.entity();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(sub_tabs("home-rail", tabs_spec, move |idx, _, cx| {
                    owner.update(cx, |view, cx| {
                        view.home_prefs.rail_tab = idx;
                        cx.notify();
                    });
                }))
                .child(panel)
                .into_any_element(),
        )
    };

    let rail_el = build_rail(rail_sections, view.home_prefs.rail_tab, cx);

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
            .children(rail_el.map(|rail| {
                div()
                    .w(px(rail_width))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(rail)
            }))
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .w_full()
            .child(today(view, palette, cx))
            .children(tasks)
            .children(rail_el)
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap_5()
        .w_full()
        .child(hero_strip(view, palette, cx))
        .children(customize_panel(view, palette, cx))
        .children(needs_you(view, palette, cx))
        .child(content)
}
