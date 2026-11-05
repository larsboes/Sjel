use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context};

use crate::route::Page;
use crate::selection::Item;
use crate::workspace::Workspace;
use sjel_gpui_core::context::TimelineKind;
use sjel_gpui_ui::primitives::{due_badge, list, note, placeholder, priority_pill, row, section};
use sjel_gpui_ui::theme::{tint, Palette};

pub fn focus_tasks(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if view.context_page.read(cx).filter_kind == Some(TimelineKind::Task) {
        return None;
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let selected = view.selected();
    let body = match (&view.focus_tasks.value, &view.focus_tasks.error) {
        (_, Some(error)) => note(format!("Could not load tasks. {error}"), palette.muted),
        (None, None) => placeholder(56.),
        (Some(_), None) => {
            let urgent = view.urgent_tasks();
            let total = view.urgent_task_count();

            if urgent.is_empty() {
                return None;
            }
            let shown = urgent.len();
            let mut rows: Vec<AnyElement> = urgent
                .into_iter()
                .enumerate()
                .map(|(index, task)| {
                    let task_id = task.id.clone();
                    let task_toggle_id = task.id.clone();
                    let task_priority_id = task.id.clone();
                    let proj_name = task.project.clone();
                    let proj_name_title = task.project.clone();
                    let due = task.due.clone().filter(|d| !d.is_empty());
                    let priority = task.priority.clamp(1, 3);
                    let marked = selected == Some(Item::FocusTask(task_id.clone()));
                    view.selection
                        .mark(row(), marked, palette)
                        .child(
                            div().w(px(20.)).child(
                                Checkbox::new(("home-task-done", index))
                                    .checked(task.done)
                                    .on_click(cx.listener(move |view, done: &bool, window, cx| {
                                        view.toggle_task(task_toggle_id.clone(), *done, window, cx);
                                    })),
                            ),
                        )
                        .child(
                            div()
                                .id(("home-task-title", index))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |s| s.text_color(rgb(palette.accent)))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.go_to_project(proj_name_title.clone(), cx);
                                }))
                                .child(task.title.clone()),
                        )
                        .child(
                            div()
                                .id(("home-proj-tag", index))
                                .px_2()
                                .py(px(2.))
                                .rounded(px(6.))
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
                                        view.cycle_task_priority(id.clone(), priority, window, cx);
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
                                    .rounded(px(6.))
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
    };
    Some(section("Urgent & overdue", body, palette).into_any_element())
}

/// `+N more` for the rows the list left out. Not selectable: selection covers only drawn rows.
pub fn more_row(hidden: usize, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
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
