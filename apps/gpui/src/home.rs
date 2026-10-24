//! Today answers "what needs me now": what is waiting on me, what is urgent, and the day itself.
//!
//! Home and Today were two pages and are one (`core::area`), so the day's timeline is a block
//! here rather than a window of its own. What needs you costs no height when nothing does.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};

use crate::route::Page;
use crate::selection::Item;
use crate::shell::{Approval, ShellError};
use crate::theme::{tint, Palette};
use crate::ui::{dot, due_badge, list, note, placeholder, priority_pill, row, section};
use crate::workspace::Workspace;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_8()
        .children(needs_you(view, palette, cx))
        .child(focus_tasks(view, palette, cx))
        .child(today(view, palette, cx))
}

/// The day on screen: the date, its three counts, and the timeline, from the Today view.
fn today(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    view.context_page
        .update(cx, |page, cx| page.set_palette(palette, cx));
    view.context_page.clone().into_any_element()
}

/// Nothing that needs you draws nothing: a line saying so pushes the day down for no reason.
/// The first answer still gets its height, so the page does not jump when it arrives.
fn needs_you(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
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

    let error = view.health.error.as_ref().filter(|error| {
        matches!(
            error,
            ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback
        )
    });
    let body = match error {
        Some(error) => note(error.to_string(), palette.red),
        // Nothing waiting and no shell to ask: the row would be the truth, not the answer.
        None if rows.is_empty() => {
            if !view.health.first_load() {
                return None;
            }
            placeholder(56.)
        }
        None => list(rows, palette),
    };
    Some(section("Needs you", body, palette).into_any_element())
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
            let total = view.urgent_task_count();

            if urgent.is_empty() {
                note("No urgent or overdue tasks today.", palette.muted)
            } else {
                let shown = urgent.len();
                let mut rows: Vec<AnyElement> = urgent
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
                                        .bg(rgba(tint(palette.accent, 0x24)))
                                        .text_color(rgb(palette.accent))
                                        .text_xs()
                                        .child("Space"),
                                )
                            })
                            .into_any_element()
                    })
                    .collect();
                if total > shown {
                    rows.push(more_row(total - shown, palette, cx));
                }
                list(rows, palette)
            }
        }
    };
    section("Urgent & overdue", body, palette)
}

/// `+N more` for the rows the list left out. Not selectable: selection covers only drawn rows.
fn more_row(hidden: usize, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    row()
        .id("home-more-urgent")
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Projects, cx)))
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{hidden} more urgent, open Projects")),
        )
        .into_any_element()
}
