//! Home answers "what needs me now": waiting agent writes and broken capabilities first, then
//! what is still ahead today and tomorrow, then what the agents changed.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};

use crate::calendar;
use crate::selection::Item;
use crate::shell::{Approval, ShellError};
use crate::theme::Palette;
use crate::ui::{dot, due_badge, list, note, placeholder, priority_pill, row, section};
use crate::workspace::Workspace;

const RECENT_WRITES: usize = 6;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_8()
        .child(needs_you(view, palette, cx))
        .child(ahead(view, palette, cx))
        .child(focus_tasks(view, palette, cx))
        .child(agent_writes(view, palette, cx))
}

fn needs_you(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let selected = view.selected();
    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(agent) = &view.agent.value {
        for approval in &agent.pending {
            let marked = selected == Some(Item::Approval(approval.id.clone()));
            rows.push(approval_row(view, marked, approval, palette, cx));
        }
    }
    if let Some(health) = &view.health.value {
        for name in health.down() {
            let marked = selected == Some(Item::Down(name.clone()));
            rows.push(down_row(view, marked, name, palette, cx));
        }
    }

    let body = match (&view.health.error, rows.is_empty()) {
        (
            Some(
                error @ (ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback),
            ),
            _,
        ) => note(error.to_string(), palette.red),
        (_, false) => list(rows, palette),
        (_, true) if view.health.first_load() => placeholder(56.),
        (_, true) => note("Nothing needs you.", palette.muted),
    };
    section("Needs you", body, palette)
}

fn approval_row(
    view: &Workspace,
    selected: bool,
    approval: &Approval,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&approval.id);
    let mono = cx.theme().mono_font_family.clone();
    let (allow_id, deny_id) = (approval.id.clone(), approval.id.clone());
    view.selection
        .mark(row(), selected, palette)
        .child(Tag::warning().small().child("agent write"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().font_family(mono).text_sm().child(format!(
                    "{} {} · {}",
                    approval.method, approval.path, approval.capability
                )))
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(approval.preview.clone()),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("deny-{}", approval.id)))
                .ghost()
                .small()
                .label("Deny")
                .tooltip("Deny (⌘⌫)")
                .disabled(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.decide(deny_id.clone(), false, window, cx)
                })),
        )
        .child(
            Button::new(SharedString::from(format!("allow-{}", approval.id)))
                .primary()
                .small()
                .label("Allow")
                .tooltip("Allow (↩)")
                .loading(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.decide(allow_id.clone(), true, window, cx)
                })),
        )
        .into_any_element()
}

fn down_row(
    view: &Workspace,
    selected: bool,
    name: String,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&name);
    let start_name = name.clone();
    view.selection
        .mark(row(), selected, palette)
        .child(dot(palette.red))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_sm().child(name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Kept running, but not answering"),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("start-{name}")))
                .small()
                .label("Start")
                .tooltip("Start (↩)")
                .loading(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.set_running(start_name.clone(), true, window, cx)
                })),
        )
        .into_any_element()
}

fn ahead(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let now = chrono::Local::now().naive_local();
    let mono = cx.theme().mono_font_family.clone();
    let selected = view.selected();
    let body = match (&view.calendar.value, &view.calendar.error) {
        (_, Some(error)) => note(
            format!("The calendar did not answer. {error}"),
            palette.muted,
        ),
        (None, None) => placeholder(96.),
        (Some(_), None) => {
            let entries = view.ahead();
            if entries.is_empty() {
                note("Nothing else today or tomorrow.", palette.muted)
            } else {
                list(
                    entries
                        .iter()
                        .enumerate()
                        .map(|(offset, entry)| {
                            view.selection
                                .mark(row(), selected == Some(Item::Entry(offset)), palette)
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_none()
                                        .font_family(mono.clone())
                                        .text_sm()
                                        .text_color(rgb(palette.muted))
                                        .child(calendar::when(entry, now)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(entry.title.clone()),
                                )
                                .when(entry.commitment == "possible", |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child("possible"),
                                    )
                                })
                                .into_any_element()
                        })
                        .collect(),
                    palette,
                )
            }
        }
    };
    section("Ahead", body, palette)
}

fn focus_tasks(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let selected = view.selected();
    let body = match (&view.focus_tasks.value, &view.focus_tasks.error) {
        (_, Some(error)) => note(format!("Could not load tasks. {error}"), palette.muted),
        (None, None) => placeholder(56.),
        (Some(_), None) => {
            let urgent = view.urgent_tasks();

            if urgent.is_empty() {
                note("No urgent or overdue tasks today.", palette.muted)
            } else {
                list(
                    urgent
                        .into_iter()
                        .enumerate()
                        .map(|(index, task)| {
                            let task_id = task.id.clone();
                            let task_toggle_id = task.id.clone();
                            let task_priority_id = task.id.clone();
                            let proj_name = task.project.clone();
                            let due = task.due.clone().filter(|d| !d.is_empty());
                            let priority = task.priority.clamp(1, 3);
                            let marked = selected == Some(Item::FocusTask(task_id.clone()));
                            view.selection
                                .mark(row(), marked, palette)
                                .child(
                                    div().w(px(20.)).child(
                                        Checkbox::new(("home-task-done", index))
                                            .checked(task.done)
                                            .on_click(cx.listener(
                                                move |view, done: &bool, window, cx| {
                                                    view.toggle_task(
                                                        task_toggle_id.clone(),
                                                        *done,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                    ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(task.title.clone()),
                                )
                                .child(
                                    div()
                                        .id(("home-proj-tag", index))
                                        .px_2()
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(rgba(palette.raised))
                                        .text_color(rgb(palette.accent))
                                        .text_xs()
                                        .truncate()
                                        .cursor_pointer()
                                        .hover(move |s| s.text_color(rgb(palette.text)))
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.go_to_project(proj_name.clone(), cx);
                                        }))
                                        .child(task.project.clone()),
                                )
                                .child(div().w(px(96.)).child(due_badge(
                                    due.as_deref(),
                                    task.done,
                                    &today,
                                    palette,
                                )))
                                .child(
                                    div()
                                        .id(("home-priority-btn", index))
                                        .w(px(48.))
                                        .cursor_pointer()
                                        .hover(move |s| s.opacity(0.8))
                                        .on_click(cx.listener({
                                            let id = task_priority_id.clone();
                                            move |view, _, window, cx| {
                                                view.cycle_task_priority(
                                                    id.clone(),
                                                    priority,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }))
                                        .child(priority_pill(priority, palette)),
                                )
                                .when(marked, |row| {
                                    row.child(
                                        div()
                                            .flex_none()
                                            .px(px(6.))
                                            .py(px(1.5))
                                            .rounded(px(3.))
                                            .bg(rgba(palette.accent << 8 | 0x24))
                                            .text_color(rgb(palette.accent))
                                            .text_xs()
                                            .child("Space"),
                                    )
                                })
                                .into_any_element()
                        })
                        .collect(),
                    palette,
                )
            }
        }
    };
    section("Today's Focus & Urgent Tasks", body, palette)
}

fn agent_writes(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let mono = cx.theme().mono_font_family.clone();
    let body = match (&view.agent.value, &view.agent.error) {
        (_, Some(error)) => note(error.to_string(), palette.muted),
        (None, None) => placeholder(56.),
        (Some(agent), None) => {
            let writes: Vec<_> = agent
                .calls
                .iter()
                .filter(|call| call.decision != "read")
                .take(RECENT_WRITES)
                .collect();
            if writes.is_empty() {
                note(
                    format!("Only reads in the last {} agent calls.", agent.calls.len()),
                    palette.muted,
                )
            } else {
                list(
                    writes
                        .into_iter()
                        .map(|call| {
                            let at = chrono::DateTime::from_timestamp(call.at, 0)
                                .map(|t| {
                                    t.with_timezone(&chrono::Local).format("%H:%M").to_string()
                                })
                                .unwrap_or_default();
                            let tone = if call.status >= 400 {
                                palette.red
                            } else {
                                palette.muted
                            };
                            row()
                                .font_family(mono.clone())
                                .text_sm()
                                .child(
                                    div()
                                        .w(px(56.))
                                        .flex_none()
                                        .text_color(rgb(palette.muted))
                                        .child(at),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(format!(
                                    "{} {} · {}",
                                    call.method, call.path, call.capability
                                )))
                                .child(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child(call.decision.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(36.))
                                        .flex_none()
                                        .text_right()
                                        .text_color(rgb(tone))
                                        .child(call.status.to_string()),
                                )
                                .into_any_element()
                        })
                        .collect(),
                    palette,
                )
            }
        }
    };
    section("Agent writes", body, palette)
}
