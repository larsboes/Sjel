//! Task and cross-project task/note tables for Projects.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
};

use crate::primitives::{
    due_badge, filter_pill, list, priority_pill, row_for_density, status_pill,
};
use crate::tasks::{self, StatusFilter, TaskFilter, TaskSort, TASK_STATUSES};
use crate::theme::{tint, Palette};
use sjel_gpui_core::shell::{GlobalNote, GlobalTask, ProjectRecord};

use super::{column, search_box, NoteKindFilter, Projects, ProjectsTab};

impl Projects {
    pub(super) fn task_table(
        &self,
        records: &[ProjectRecord],
        new_task: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let open_count = records
            .iter()
            .filter(|r| r.kind == "task" && !r.done)
            .count();
        let all_count = records.iter().filter(|r| r.kind == "task").count();
        let done_count = all_count.saturating_sub(open_count);

        let filter_bar = div()
            .flex()
            .items_center()
            .gap_2()
            .mb_2()
            .child(filter_pill(
                "Open",
                open_count,
                self.task_filter == TaskFilter::Open,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.task_filter = TaskFilter::Open;
                    cx.notify();
                }),
            ))
            .child(filter_pill(
                "All",
                all_count,
                self.task_filter == TaskFilter::All,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.task_filter = TaskFilter::All;
                    cx.notify();
                }),
            ))
            .child(filter_pill(
                "Done",
                done_count,
                self.task_filter == TaskFilter::Done,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.task_filter = TaskFilter::Done;
                    cx.notify();
                }),
            ));

        let header = row_for_density(cols.density)
            .min_h(px(34.))
            .py_1()
            .bg(rgba(tint(palette.raised, 0x30)))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child(div().w(px(24.)))
            .child(div().flex_1().child("TASK"))
            .child(div().w(px(cols.due)).child("DUE"))
            .child(div().w(px(cols.priority)).child("PRIORITY"))
            .child(div().w(px(28.)))
            .into_any_element();
        let mut rows = vec![header];

        let filtered: Vec<_> = tasks::ordered(records)
            .into_iter()
            .filter(|t| self.task_filter.matches(t.done))
            .collect();

        if filtered.is_empty() && !records.is_empty() {
            rows.push(
                row_for_density(cols.density)
                    .min_h(px(40.))
                    .py_2()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(match self.task_filter {
                        TaskFilter::Open => "No open tasks in this project.",
                        TaskFilter::Done => "No completed tasks yet.",
                        TaskFilter::All => "No tasks in this project.",
                    })
                    .into_any_element(),
            );
        }

        for (index, task) in filtered.into_iter().enumerate() {
            let (toggle, open, del) = (task.clone(), task.clone(), task.clone());
            let due = task.due.clone().filter(|d| !d.is_empty());
            let editing = self
                .due_edit
                .as_ref()
                .filter(|(path, _, _)| *path == task.path)
                .map(|(_, input, _)| input.clone());
            let due_cell = match editing {
                Some(input) => {
                    self.due_editor(task.path.clone(), &input, index, cols.due, palette, cx)
                }
                None => {
                    let edit = task.clone();
                    div()
                        .id(("due", index))
                        .w(px(cols.due))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit_due(edit.path.clone(), edit.due.clone(), window, cx)
                        }))
                        .child(due_badge(due.as_deref(), task.done, &today, palette))
                        .into_any_element()
                }
            };
            let subtask_badge =
                tasks::parse_subtask_progress(&task.content).map(|(done, total)| {
                    div()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded(px(4.))
                        .bg(rgba(if done == total {
                            tint(palette.green, 0x22)
                        } else {
                            palette.raised
                        }))
                        .text_xs()
                        .text_color(rgb(if done == total {
                            palette.green
                        } else {
                            palette.muted
                        }))
                        .child(format!("{done}/{total}"))
                });
            let cycle = task.clone();
            let priority = task.priority.clamp(1, 3);
            rows.push(
                row_for_density(cols.density)
                    .id(("task-row", index))
                    .group("task-row")
                    .min_h(px(40.))
                    .py_1()
                    .hover(move |s| s.bg(rgba(tint(palette.raised, 0x55))))
                    .when(task.done, |row| row.opacity(0.55))
                    .child(
                        div().w(px(24.)).flex().items_center().child(
                            Checkbox::new(("task-done", index))
                                .checked(task.done)
                                .on_click(cx.listener(move |view, done: &bool, window, cx| {
                                    view.toggle_task(toggle.path.clone(), *done, window, cx)
                                })),
                        ),
                    )
                    .child(
                        div()
                            .id(("task", index))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.open_note(open.path.clone(), true, window, cx)
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .when(task.done, |title| title.line_through())
                                    .hover(move |style| style.text_color(rgb(palette.accent)))
                                    .child(task.title.clone()),
                            )
                            .children(subtask_badge),
                    )
                    .child(due_cell)
                    .child(
                        div()
                            .id(("priority", index))
                            .w(px(cols.priority))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let next = cycle.priority.clamp(1, 3) % 3 + 1;
                                view.set_task_field(
                                    &cycle,
                                    "priority",
                                    next.to_string(),
                                    window,
                                    cx,
                                )
                            }))
                            .child(priority_pill(priority, palette)),
                    )
                    .child(
                        div().w(px(28.)).flex().justify_center().child(
                            div()
                                .id(("del-task", index))
                                .cursor_pointer()
                                .text_color(rgb(palette.muted))
                                .opacity(0.)
                                .group_hover("task-row", |s| s.opacity(1.))
                                .hover(move |s| s.text_color(rgb(palette.red)))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.remove_record(del.path.clone(), window, cx)
                                }))
                                .child("×"),
                        ),
                    )
                    .into_any_element(),
            );
        }
        rows.push(
            row_for_density(cols.density)
                .min_h(px(40.))
                .py_1()
                .child(
                    div()
                        .w(px(24.))
                        .flex()
                        .items_center()
                        .text_color(rgb(palette.muted))
                        .child("+"),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(new_task).appearance(false).px_0()),
                )
                .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(filter_bar)
            .child(list(rows, palette))
            .into_any_element()
    }

    /// The due-date field and its preset chips, for the task at `path`.
    pub(super) fn due_editor(
        &self,
        path: String,
        input: &Entity<InputState>,
        index: usize,
        width: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = chrono::Local::now().date_naive();
        let day = |offset: i64| {
            (today + chrono::Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        };
        let presets = [
            ("Today", day(0)),
            ("Tmrw", day(1)),
            ("+7d", day(7)),
            ("Clear", String::new()),
        ];
        let chips = presets.into_iter().map(|(label, value)| {
            let path = path.clone();
            div()
                .id(SharedString::from(format!("due-chip-{label}-{index}")))
                .px_1()
                .py(px(0.5))
                .rounded(px(3.))
                .bg(rgba(palette.raised))
                .text_xs()
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.accent)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.due_edit = None;
                    view.set_task_property(path.clone(), "due", value.clone(), window, cx);
                    cx.notify();
                }))
                .child(label)
        });
        div()
            .w(px(width.max(160.)))
            .flex()
            .flex_col()
            .gap_1()
            .child(Input::new(input).small())
            .child(div().flex().gap_1().children(chips))
            .into_any_element()
    }

    /// The six status chips, the current one marked. A click writes the status.
    pub(super) fn status_chips(
        &self,
        path: &str,
        current: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = TASK_STATUSES.into_iter().map(|status| {
            let path = path.to_owned();
            let active = status == current;
            div()
                .id(SharedString::from(format!("status-{status}-{path}")))
                .cursor_pointer()
                .when(!active, |chip| chip.opacity(0.6).hover(|s| s.opacity(1.)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.status_edit = None;
                    if !active {
                        view.set_task_property(
                            path.clone(),
                            "status",
                            status.to_owned(),
                            window,
                            cx,
                        );
                    }
                    cx.notify();
                }))
                .child(status_pill(status, palette))
        });
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(chips)
            .into_any_element()
    }

    pub(super) fn all_tasks_table(
        &self,
        tasks: &[GlobalTask],
        search_input: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let count =
            |filter: StatusFilter| tasks.iter().filter(|t| filter.matches(t.status())).count();

        let pill = |label: &'static str, filter: StatusFilter, cx: &mut Context<Self>| {
            filter_pill(
                label,
                count(filter),
                self.global_status == filter,
                palette,
                cx.listener(move |view, _, _, cx| {
                    view.global_status = filter;
                    cx.notify();
                }),
            )
        };
        let mut pills = vec![
            pill("Active", StatusFilter::Active, cx),
            pill("All", StatusFilter::All, cx),
        ];
        for status in TASK_STATUSES {
            pills.push(pill(
                tasks::status_label(status),
                StatusFilter::Is(status),
                cx,
            ));
        }
        let group_toggle = filter_pill(
            "Group by project",
            tasks
                .iter()
                .map(|t| t.project.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            self.group_by_project,
            palette,
            cx.listener(|view, _, _, cx| {
                view.group_by_project = !view.group_by_project;
                cx.notify();
            }),
        );
        let filter_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .mb_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .children(pills)
                    .child(group_toggle),
            )
            .child(search_box(cols.search).child(Input::new(search_input).small()));

        let (sort, ascending) = self.task_sort;
        let heading = |label: &'static str, column: TaskSort, cx: &mut Context<Self>| {
            let arrow = match (sort == column, ascending) {
                (true, true) => " ↑",
                (true, false) => " ↓",
                _ => "",
            };
            div()
                .id(SharedString::from(format!("sort-{label}")))
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    // Ascending, then descending, then back to the default order.
                    view.task_sort = match view.task_sort {
                        (current, true) if current == column => (column, false),
                        (current, false) if current == column => (TaskSort::Default, true),
                        _ => (column, true),
                    };
                    cx.notify();
                }))
                .child(format!("{label}{arrow}"))
        };
        let header = row_for_density(cols.density)
            .min_h(px(34.))
            .py_1()
            .bg(rgba(tint(palette.raised, 0x30)))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child(div().w(px(24.)))
            .child(div().flex_1().child(heading("TASK", TaskSort::Title, cx)))
            .child(column(cols.project).child(heading("PROJECT", TaskSort::Project, cx)))
            .child(column(cols.status).child(heading("STATUS", TaskSort::Status, cx)))
            .child(
                div()
                    .w(px(cols.due))
                    .child(heading("DUE", TaskSort::Due, cx)),
            )
            .child(
                div()
                    .w(px(cols.priority))
                    .child(heading("PRIO", TaskSort::Priority, cx)),
            )
            .into_any_element();
        let mut rows = vec![header];

        let query = self.search_query.as_str();
        let mut filtered: Vec<&GlobalTask> = tasks
            .iter()
            .filter(|t| self.global_status.matches(t.status()))
            .filter(|t| {
                query.is_empty()
                    || t.title.to_lowercase().contains(query)
                    || t.project.to_lowercase().contains(query)
            })
            .collect();
        tasks::sort_tasks(&mut filtered, sort, ascending);

        if filtered.is_empty() && !tasks.is_empty() {
            rows.push(
                row_for_density(cols.density)
                    .min_h(px(40.))
                    .py_2()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(if query.is_empty() {
                        "No tasks in this filter view.".to_string()
                    } else {
                        format!("No tasks matching \"{query}\".")
                    })
                    .into_any_element(),
            );
        }

        let mut index = 0;
        if self.group_by_project {
            for (project, group) in tasks::group_by_project(&filtered) {
                let folded = self.collapsed_groups.contains(&project);
                let name = project.clone();
                rows.push(
                    row_for_density(cols.density)
                        .id(SharedString::from(format!("group-{project}")))
                        .min_h(px(32.))
                        .py_1()
                        .gap_2()
                        .bg(rgba(tint(palette.raised, 0x18)))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if !view.collapsed_groups.remove(&name) {
                                view.collapsed_groups.insert(name.clone());
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(24.))
                                .text_color(rgb(palette.muted))
                                .child(if folded { "▸" } else { "▾" }),
                        )
                        .child(project)
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{}", group.len())),
                        )
                        .into_any_element(),
                );
                if !folded {
                    for task in group {
                        rows.push(self.global_task_row(task, index, &today, palette, cx));
                        index += 1;
                    }
                }
            }
        } else {
            for task in filtered {
                rows.push(self.global_task_row(task, index, &today, palette, cx));
                index += 1;
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(filter_bar)
            .child(list(rows, palette))
            .into_any_element()
    }

    fn global_task_row(
        &self,
        task: &GlobalTask,
        index: usize,
        today: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let status = task.status().to_owned();
        let closed = tasks::status_closes(&status);
        let selected = self.task_panel.as_ref().is_some_and(|p| p.path == task.id);
        let (toggle, select, cycle, edit, pick) = (
            task.id.clone(),
            task.clone(),
            task.id.clone(),
            task.id.clone(),
            task.id.clone(),
        );
        let proj_name = task.project.clone();
        let priority = task.priority.clamp(1, 3);
        let subtask_badge = task
            .summary
            .as_deref()
            .and_then(tasks::parse_subtask_progress)
            .map(|(done, total)| {
                div()
                    .px_1p5()
                    .py(px(0.5))
                    .rounded(px(4.))
                    .bg(rgba(if done == total {
                        tint(palette.green, 0x22)
                    } else {
                        palette.raised
                    }))
                    .text_xs()
                    .text_color(rgb(if done == total {
                        palette.green
                    } else {
                        palette.muted
                    }))
                    .child(format!("{done}/{total}"))
            });

        let status_cell = if self.status_edit.as_deref() == Some(task.id.as_str()) {
            div()
                .w(px(cols.status.unwrap_or(0.).max(220.)))
                .child(self.status_chips(&task.id, &status, palette, cx))
                .into_any_element()
        } else {
            column(cols.status)
                .child(
                    div()
                        .id(("all-task-status", index))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.status_edit = Some(pick.clone());
                            cx.notify();
                        }))
                        .child(status_pill(&status, palette)),
                )
                .into_any_element()
        };
        let editing = self
            .due_edit
            .as_ref()
            .filter(|(path, _, _)| *path == task.id)
            .map(|(_, input, _)| input.clone());
        let due_cell = match editing {
            Some(input) => self.due_editor(task.id.clone(), &input, index, cols.due, palette, cx),
            None => {
                let due = task.due.clone().filter(|d| !d.is_empty());
                let current = due.clone();
                div()
                    .id(("all-task-due", index))
                    .w(px(cols.due))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.edit_due(edit.clone(), current.clone(), window, cx)
                    }))
                    .child(due_badge(due.as_deref(), closed, today, palette))
                    .into_any_element()
            }
        };

        row_for_density(cols.density)
            .id(("all-task-row", index))
            .group("all-task-row")
            .min_h(px(40.))
            .py_1()
            .when(selected, |row| row.bg(rgba(tint(palette.accent, 0x14))))
            .hover(move |s| s.bg(rgba(tint(palette.raised, 0x55))))
            .when(closed, |row| row.opacity(0.55))
            .child(
                div().w(px(24.)).flex().items_center().child(
                    Checkbox::new(("all-task-done", index))
                        .checked(task.done)
                        .on_click(cx.listener(move |view, done: &bool, window, cx| {
                            view.toggle_global_task(toggle.clone(), *done, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .id(("all-task", index))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |view, _, window, cx| {
                            view.select_task(&select, window, cx)
                        }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .when(closed, |title| title.line_through())
                            .hover(move |style| style.text_color(rgb(palette.accent)))
                            .child(task.title.clone()),
                    )
                    .children(subtask_badge),
            )
            .child(
                column(cols.project).child(
                    div()
                        .id(("proj-tag", index))
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(rgba(tint(palette.accent, 0x12)))
                        .border_1()
                        .border_color(rgba(tint(palette.accent, 0x24)))
                        .text_color(rgb(palette.accent))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .cursor_pointer()
                        .hover(move |s| {
                            s.bg(rgba(tint(palette.accent, 0x22)))
                                .text_color(rgb(palette.text))
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.open_project(proj_name.clone(), cx);
                            view.tab = ProjectsTab::Hubs;
                        }))
                        .child(task.project.clone()),
                ),
            )
            .child(status_cell)
            .child(due_cell)
            .child(
                div()
                    .id(("all-task-priority", index))
                    .w(px(cols.priority))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let next = priority % 3 + 1;
                        view.set_task_property(
                            cycle.clone(),
                            "priority",
                            next.to_string(),
                            window,
                            cx,
                        )
                    }))
                    .child(priority_pill(priority, palette)),
            )
            .into_any_element()
    }

    pub(super) fn all_notes_table(
        &self,
        notes: &[GlobalNote],
        search_input: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let hubs_count = notes.iter().filter(|n| n.kind == "project").count();
        let regular_count = notes.len().saturating_sub(hubs_count);

        let filter_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .mb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(filter_pill(
                        "All",
                        notes.len(),
                        self.note_kind_filter == NoteKindFilter::All,
                        palette,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::All;
                            cx.notify();
                        }),
                    ))
                    .child(filter_pill(
                        "Hubs ★",
                        hubs_count,
                        self.note_kind_filter == NoteKindFilter::Hubs,
                        palette,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::Hubs;
                            cx.notify();
                        }),
                    ))
                    .child(filter_pill(
                        "Notes ≡",
                        regular_count,
                        self.note_kind_filter == NoteKindFilter::Notes,
                        palette,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::Notes;
                            cx.notify();
                        }),
                    )),
            )
            .child(search_box(cols.search).child(Input::new(search_input).small()));

        let header = row_for_density(cols.density)
            .min_h(px(34.))
            .py_1()
            .bg(rgba(tint(palette.raised, 0x30)))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child(div().w(px(24.)))
            .child(div().flex_1().child("NOTE"))
            .child(column(cols.project).child("PROJECT"))
            .child(column(cols.kind).child("TYPE"))
            .into_any_element();
        let mut rows = vec![header];

        let query = self.notes_search_query.as_str();
        let filtered: Vec<_> = notes
            .iter()
            .filter(|n| {
                let matches_kind = match self.note_kind_filter {
                    NoteKindFilter::All => true,
                    NoteKindFilter::Hubs => n.kind == "project",
                    NoteKindFilter::Notes => n.kind != "project",
                };
                matches_kind
                    && (query.is_empty()
                        || n.title.to_lowercase().contains(query)
                        || n.project.to_lowercase().contains(query)
                        || n.path.to_lowercase().contains(query))
            })
            .collect();

        if filtered.is_empty() && !notes.is_empty() {
            rows.push(
                row_for_density(cols.density)
                    .min_h(px(40.))
                    .py_2()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(format!("No notes matching \"{query}\"."))
                    .into_any_element(),
            );
        }

        for (index, note) in filtered.into_iter().enumerate() {
            let open_path = note.path.clone();
            let proj_name = note.project.clone();
            let is_hub = note.kind == "project";
            rows.push(
                row_for_density(cols.density)
                    .id(("all-note-row", index))
                    .group("all-note-row")
                    .min_h(px(40.))
                    .py_1()
                    .hover(move |s| s.bg(rgba(tint(palette.raised, 0x55))))
                    .child(
                        div().w(px(24.)).flex().items_center().child(
                            div()
                                .size(px(20.))
                                .rounded(px(4.))
                                .bg(rgba(palette.raised))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgb(if is_hub {
                                    palette.accent
                                } else {
                                    palette.muted
                                }))
                                .child(if is_hub { "★" } else { "≡" }),
                        ),
                    )
                    .child(
                        div()
                            .id(("all-note", index))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .cursor_pointer()
                            .hover(move |s| s.text_color(rgb(palette.accent)))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.open_note(open_path.clone(), true, window, cx);
                            }))
                            .child(note.title.clone()),
                    )
                    .child(
                        column(cols.project).child(
                            div()
                                .id(("all-note-proj", index))
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(tint(palette.accent, 0x12)))
                                .border_1()
                                .border_color(rgba(tint(palette.accent, 0x24)))
                                .text_color(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .cursor_pointer()
                                .hover(move |s| {
                                    s.bg(rgba(tint(palette.accent, 0x22)))
                                        .text_color(rgb(palette.text))
                                })
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_project(proj_name.clone(), cx);
                                    view.tab = ProjectsTab::Hubs;
                                }))
                                .child(note.project.clone()),
                        ),
                    )
                    .child(
                        column(cols.kind).child(
                            div()
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(if is_hub {
                                    tint(palette.accent, 0x18)
                                } else {
                                    palette.raised
                                }))
                                .border_1()
                                .border_color(rgba(if is_hub {
                                    tint(palette.accent, 0x30)
                                } else {
                                    palette.border
                                }))
                                .text_xs()
                                .font_weight(if is_hub {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(rgb(if is_hub {
                                    palette.accent
                                } else {
                                    palette.muted
                                }))
                                .child(note.kind.clone()),
                        ),
                    )
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(filter_bar)
            .child(list(rows, palette))
            .into_any_element()
    }
}
