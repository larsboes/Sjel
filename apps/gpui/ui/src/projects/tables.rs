//! Task and cross-project task/note tables for Projects.

use std::rc::Rc;

use gpui_kit::{div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, Entity, SharedString};

use crate::ely::data_display::Tone;
use crate::ely::forms::{Input, TextInput};
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction, SortKey};
use crate::ely::theme::{ControlSize, Density as ElyDensity};
use crate::primitives::{filter_pill, key_bar, row_for_density, status_pill};
use crate::tasks::{self, NoteSort, StatusFilter, TaskFilter, TaskSort, TASK_STATUSES};
use crate::theme::{Density, Palette};
use sjel_gpui_core::shell::{GlobalNote, GlobalTask, ProjectRecord};

use super::{
    search_box, NoteKindFilter, ProjectSection, ProjectSort, Projects, ProjectsTab, TaskFocusFilter,
};

fn ely_density(density: Density) -> ElyDensity {
    match density {
        Density::Compact | Density::Mobile => ElyDensity::Compact,
        Density::Desktop => ElyDensity::Standard,
        Density::Comfortable => ElyDensity::Comfortable,
    }
}

fn status_tone(status: &str) -> Tone {
    match status {
        "done" => Tone::Success,
        "doing" => Tone::Accent,
        "waiting" | "blocked" => Tone::Warning,
        "cancelled" => Tone::Danger,
        _ => Tone::Neutral,
    }
}

fn priority_label(priority: u8) -> &'static str {
    match priority.clamp(1, 3) {
        1 => "P1",
        2 => "P2",
        _ => "P3",
    }
}

fn priority_value(choice: &str) -> &'static str {
    match choice {
        "P1" | "1" => "1",
        "P2" | "2" => "2",
        _ => "3",
    }
}

fn priority_tone(priority: u8) -> Tone {
    match priority.clamp(1, 3) {
        1 => Tone::Danger,
        2 => Tone::Warning,
        _ => Tone::Neutral,
    }
}

fn due_cell(due: Option<&str>, done: bool, today: &str) -> Cell {
    let Some(date) = due.filter(|d| !d.is_empty()) else {
        return Cell::Empty;
    };
    let tone = if done {
        Tone::Neutral
    } else if date < today {
        Tone::Danger
    } else if date == today {
        Tone::Warning
    } else {
        Tone::Neutral
    };
    Cell::Tag(date.to_owned().into(), tone)
}

fn title_cell(title: &str, content: Option<&str>) -> Cell {
    match content.and_then(tasks::parse_subtask_progress) {
        Some((done, total)) => Cell::Lines(
            title.to_owned().into(),
            format!("{done}/{total} subtasks").into(),
        ),
        None => Cell::Text(title.to_owned().into()),
    }
}

pub(super) fn parse_frontmatter_status(content: &str, done: bool) -> String {
    let mut in_fm = false;
    for line in content.lines() {
        let t = line.trim();
        if t == "---" {
            if in_fm {
                break;
            }
            in_fm = true;
            continue;
        }
        if in_fm {
            if let Some((k, v)) = t.split_once(':') {
                if k.trim() == "status" {
                    let s = v.trim().trim_matches('"').trim_matches('\'');
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
        }
    }
    if done {
        "done".into()
    } else {
        "open".into()
    }
}

pub(super) fn project_record_to_global_task(rec: &ProjectRecord) -> GlobalTask {
    let mut fields = std::collections::BTreeMap::new();
    let mut in_fm = false;
    for line in rec.content.lines() {
        let t = line.trim();
        if t == "---" {
            if in_fm {
                break;
            }
            in_fm = true;
            continue;
        }
        if in_fm {
            if let Some((k, v)) = t.split_once(':') {
                let key = k.trim().to_string();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() {
                    fields.insert(key, val);
                }
            }
        }
    }
    let status = parse_frontmatter_status(&rec.content, rec.done);
    fields.insert("status".to_string(), status);
    let project = if rec.project.is_empty() {
        rec.path.split('/').next().unwrap_or("").to_string()
    } else {
        rec.project.clone()
    };
    GlobalTask {
        id: rec.path.clone(),
        kind: rec.kind.clone(),
        title: rec.title.clone(),
        done: rec.done,
        due: rec.due.clone(),
        priority: rec.priority,
        summary: None,
        project,
        fields,
    }
}

impl Projects {
    fn inline_edit_bar(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some((_, _, input, _)) = &self.task_title_edit {
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .mb_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.raised))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Rename task:"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(input).size(ControlSize::Sm)),
                    )
                    .into_any_element(),
            );
        }
        if let Some((path, input, _)) = &self.due_edit {
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .mb_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.raised))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Due date:"),
                    )
                    .child(self.due_editor(path.clone(), input, 0, 220., palette, cx))
                    .into_any_element(),
            );
        }
        None
    }

    pub(super) fn task_table(
        &self,
        records: &[ProjectRecord],
        new_task: &Entity<TextInput>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let filtered: Vec<_> = tasks::ordered(records)
            .into_iter()
            .filter(|t| self.task_filter.matches(t.done))
            .collect();

        let columns = vec![
            Column::new("done", "").width(rems(32. / 16.)).editable(),
            Column::new("title", "TASK").min_width(rems(15.)).editable(),
            Column::new("status", "STATUS")
                .width(rems(cols.status.unwrap_or(100. / 16.)))
                .choices(TASK_STATUSES),
            Column::new("due", "DUE")
                .width(rems(cols.due / 16.))
                .editable(),
            Column::new("priority", "PRIORITY")
                .width(rems(cols.priority / 16.))
                .choices(["P1", "P2", "P3"]),
            Column::new("actions", "").width(rems(110. / 16.)).actions([
                RowAction::new("due", "Due"),
                RowAction::new("delete", "Delete"),
            ]),
        ];

        let mut hidden: Vec<&'static str> = Vec::new();
        if cols.status.is_none() {
            hidden.push("status");
        }

        let rows: Rc<Vec<Row>> = Rc::new(
            filtered
                .iter()
                .map(|task| {
                    let status = parse_frontmatter_status(&task.content, task.done);
                    let mut action_keys = vec!["due"];
                    if !sjel_gpui_core::shell::is_external_task(&task.path) {
                        action_keys.push("delete");
                    }
                    Row::new(
                        task.path.clone(),
                        [
                            Cell::Check(task.done),
                            title_cell(&task.title, Some(&task.content)),
                            Cell::Tag(status.clone().into(), status_tone(&status)),
                            due_cell(task.due.as_deref(), task.done, &today),
                            Cell::Tag(
                                priority_label(task.priority).into(),
                                priority_tone(task.priority),
                            ),
                            Cell::Actions(action_keys.into_iter().map(Into::into).collect()),
                        ],
                    )
                })
                .collect(),
        );

        let empty_msg = match self.task_filter {
            TaskFilter::Open => "No open tasks in this project.",
            TaskFilter::Done => "No completed tasks yet.",
            TaskFilter::All => "No tasks in this project.",
        };

        let due_map: std::collections::HashMap<String, Option<String>> = filtered
            .iter()
            .map(|t| (t.path.clone(), t.due.clone()))
            .collect();

        let (on_curr, on_open, on_edit, on_act) =
            (cx.entity(), cx.entity(), cx.entity(), cx.entity());
        let table = DataTable::new("project-tasks", columns)
            .label("Project tasks")
            .rows(rows)
            .hidden(hidden)
            .density(ely_density(cols.density))
            .current(self.selected_task_path.clone())
            .on_current(move |key, _, cx| {
                on_curr.update(cx, |view, cx| {
                    view.selected_task_path = Some(key.to_string());
                    cx.notify();
                });
            })
            .on_open(move |key, window, cx| {
                on_open.update(cx, |view, cx| {
                    view.selected_task_path = Some(key.to_string());
                    view.open_task_by_path(key, window, cx);
                });
            })
            .on_edit(move |row_key, col_key, value, window, cx| {
                let (row_key, value) = (row_key.to_string(), value.trim().to_string());
                on_edit.update(cx, |view, cx| match col_key.as_ref() {
                    "done" => {
                        if !sjel_gpui_core::shell::is_external_task(&row_key) {
                            view.toggle_task(row_key, value == "true", window, cx);
                        }
                    }
                    "title" => {
                        if !value.is_empty() {
                            view.rename_task(row_key, value, window, cx);
                        }
                    }
                    "status" => {
                        view.set_task_property(row_key, "status", value, window, cx);
                    }
                    "due" => {
                        if tasks::valid_due(&value) {
                            view.set_task_property(row_key, "due", value, window, cx);
                        }
                    }
                    "priority" => {
                        view.set_task_property(
                            row_key,
                            "priority",
                            priority_value(&value).to_owned(),
                            window,
                            cx,
                        );
                    }
                    _ => {}
                });
            })
            .on_action(move |row_key, action, window, cx| {
                let row_key = row_key.to_string();
                let due = due_map.get(&row_key).cloned().flatten();
                on_act.update(cx, |view, cx| match action.as_ref() {
                    "due" => view.edit_due(row_key, due, window, cx),
                    "delete" => view.remove_record(row_key, window, cx),
                    _ => {}
                });
            })
            .empty(empty_msg);

        let add_row = row_for_density(cols.density)
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
                    .child(Input::new(new_task).size(ControlSize::Sm)),
            );

        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.inline_edit_bar(palette, cx))
            .child(table)
            .child(add_row)
            .into_any_element()
    }

    /// The due-date field and its preset chips, for the task at `path`.
    pub(super) fn due_editor(
        &self,
        path: String,
        input: &Entity<TextInput>,
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
            .child(Input::new(input).size(ControlSize::Sm))
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
        search_input: &Entity<TextInput>,
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
            cx.listener(|view, _, _, cx| {
                view.group_by_project = !view.group_by_project;
                cx.notify();
            }),
        );
        let is_mobile = cols.density == Density::Mobile;
        let filter_bar = div()
            .flex()
            .when(is_mobile, |el| el.flex_col().items_stretch().gap_2())
            .when(!is_mobile, |el| el.items_center().justify_between().gap_2())
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
            .child(
                search_box(cols.search)
                    .when(is_mobile, |el| el.w_full())
                    .child(Input::new(search_input).size(ControlSize::Sm)),
            );

        let focus_pills: Vec<AnyElement> = TaskFocusFilter::ALL
            .into_iter()
            .map(|focus| {
                let n = tasks
                    .iter()
                    .filter(|t| focus.matches(t, tasks, &today))
                    .count();
                filter_pill(
                    focus.label(),
                    n,
                    self.task_focus == focus,
                    cx.listener(move |view, _, _, cx| {
                        view.task_focus = focus;
                        cx.notify();
                    }),
                )
            })
            .collect();
        let due_chip = self.due_date_filter.as_ref().map(|date| {
            let label = format!("Due: {date} ×");
            crate::ely::buttons::Button::new("clear-due-date-filter", label)
                .size(crate::ely::theme::ControlSize::Sm)
                .variant(crate::ely::buttons::ButtonVariant::Primary)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.due_date_filter = None;
                    cx.notify();
                }))
                .into_any_element()
        });

        let focus_bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .mb_2()
            .children(focus_pills)
            .when_some(due_chip, |el, chip| el.child(chip));

        let query = self.search_query.as_str();
        let filtered: Vec<&GlobalTask> = tasks
            .iter()
            .filter(|t| self.global_status.matches(t.status()))
            .filter(|t| self.task_focus.matches(t, tasks, &today))
            .filter(|t| {
                self.due_date_filter
                    .as_deref()
                    .is_none_or(|d| t.due.as_deref() == Some(d))
            })
            .filter(|t| {
                query.is_empty()
                    || t.title.to_lowercase().contains(query)
                    || t.project.to_lowercase().contains(query)
            })
            .collect();

        let columns = vec![
            Column::new("done", "").width(rems(32. / 16.)).editable(),
            Column::new("title", "TASK").min_width(rems(15.)),
            Column::new("project", "PROJECT").width(rems(cols.project.unwrap_or(140. / 16.))),
            Column::new("status", "STATUS")
                .width(rems(cols.status.unwrap_or(100. / 16.)))
                .choices(TASK_STATUSES),
            Column::new("due", "DUE")
                .width(rems(cols.due / 16.))
                .editable(),
            Column::new("priority", "PRIO")
                .width(rems(cols.priority / 16.))
                .choices(["P1", "P2", "P3"]),
            Column::new("actions", "").width(rems(120. / 16.)).actions([
                RowAction::new("due", "Due"),
                RowAction::new("project", "Project"),
            ]),
        ];

        let mut hidden: Vec<&'static str> = Vec::new();
        if cols.project.is_none() {
            hidden.push("project");
        }
        if cols.status.is_none() {
            hidden.push("status");
        }

        let rows: Rc<Vec<Row>> = Rc::new(
            filtered
                .iter()
                .map(|task| {
                    let status = task.status().to_owned();
                    let closed = tasks::status_closes(&status);
                    Row::new(
                        task.id.clone(),
                        [
                            Cell::Check(task.done),
                            title_cell(&task.title, task.summary.as_deref()),
                            Cell::Tag(
                                task.project.clone().into(),
                                if closed { Tone::Neutral } else { Tone::Accent },
                            ),
                            Cell::Tag(status.clone().into(), status_tone(&status)),
                            due_cell(task.due.as_deref(), closed, &today),
                            Cell::Tag(
                                priority_label(task.priority).into(),
                                if closed {
                                    Tone::Neutral
                                } else {
                                    priority_tone(task.priority)
                                },
                            ),
                            Cell::Actions(vec!["due".into(), "project".into()]),
                        ],
                    )
                })
                .collect(),
        );

        let (sort, ascending) = self.task_sort;
        let sorts: Vec<SortKey> = match sort {
            TaskSort::Default => Vec::new(),
            _ => vec![SortKey {
                column: match sort {
                    TaskSort::Title => "title",
                    TaskSort::Project => "project",
                    TaskSort::Status => "status",
                    TaskSort::Due => "due",
                    TaskSort::Priority => "priority",
                    TaskSort::Default => unreachable!(),
                }
                .into(),
                rising: ascending,
            }],
        };

        let empty_msg = if query.is_empty() {
            "No tasks in this filter view.".to_string()
        } else {
            format!("No tasks matching \"{query}\".")
        };

        let meta_map: Rc<std::collections::HashMap<String, (bool, Option<String>, String)>> =
            Rc::new(
                filtered
                    .iter()
                    .map(|t| {
                        (
                            t.id.clone(),
                            (t.read_only(), t.due.clone(), t.project.clone()),
                        )
                    })
                    .collect(),
            );
        let edit_meta = meta_map.clone();

        let (on_curr, on_open, on_sort, on_edit, on_act) = (
            cx.entity(),
            cx.entity(),
            cx.entity(),
            cx.entity(),
            cx.entity(),
        );
        let table = DataTable::new("all-tasks", columns)
            .label("All tasks")
            .rows(rows)
            .hidden(hidden)
            .sorts(sorts)
            .when(self.group_by_project, |t| t.group_by("project"))
            .density(ely_density(cols.density))
            .current(self.selected_task_path.clone())
            .on_current(move |key, _, cx| {
                on_curr.update(cx, |view, cx| {
                    view.selected_task_path = Some(key.to_string());
                    cx.notify();
                });
            })
            .on_open(move |key, window, cx| {
                on_open.update(cx, |view, cx| {
                    view.selected_task_path = Some(key.to_string());
                    view.open_task_by_path(key, window, cx);
                });
            })
            .on_sort(move |keys, _, cx| {
                let next = keys
                    .first()
                    .and_then(|k| TaskSort::from_col_id(&k.column).map(|c| (c, k.rising)))
                    .unwrap_or((TaskSort::Default, true));
                on_sort.update(cx, |view, cx| {
                    view.task_sort = next;
                    cx.notify();
                });
            })
            .on_edit(move |row_key, col_key, value, window, cx| {
                let (row_key, value) = (row_key.to_string(), value.trim().to_string());
                let read_only = edit_meta.get(&row_key).is_some_and(|m| m.0);
                on_edit.update(cx, |view, cx| match col_key.as_ref() {
                    "done" if !read_only => {
                        view.toggle_global_task(row_key, value == "true", window, cx);
                    }
                    "status" => {
                        view.set_task_property(row_key, "status", value, window, cx);
                    }
                    "due" => {
                        if tasks::valid_due(&value) {
                            view.set_task_property(row_key, "due", value, window, cx);
                        }
                    }
                    "priority" => {
                        view.set_task_property(
                            row_key,
                            "priority",
                            priority_value(&value).to_owned(),
                            window,
                            cx,
                        );
                    }
                    _ => {}
                });
            })
            .on_action(move |row_key, action, window, cx| {
                let Some((_, due, proj)) = meta_map.get(row_key.as_ref()).cloned() else {
                    return;
                };
                let row_key = row_key.to_string();
                on_act.update(cx, |view, cx| match action.as_ref() {
                    "due" => view.edit_due(row_key, due, window, cx),
                    "project" => {
                        view.open_project(proj, cx);
                        view.tab = ProjectsTab::Hubs;
                    }
                    _ => {}
                });
            })
            .empty(empty_msg);

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(filter_bar)
            .child(focus_bar)
            .children(self.inline_edit_bar(palette, cx))
            .child(table)
            .when(cols.density != Density::Mobile, |el| {
                el.child(key_bar(
                    &[
                        ("J / K", "Move"),
                        ("Enter", "Open / Edit"),
                        ("Esc", "Close panel"),
                    ],
                    palette,
                ))
            })
            .into_any_element()
    }

    pub(super) fn all_notes_table(
        &self,
        notes: &[GlobalNote],
        search_input: &Entity<TextInput>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let hubs_count = notes.iter().filter(|n| n.kind == "project").count();
        let regular_count = notes.len().saturating_sub(hubs_count);

        let is_mobile = cols.density == Density::Mobile;
        let filter_bar = div()
            .flex()
            .when(is_mobile, |el| el.flex_col().items_stretch().gap_2())
            .when(!is_mobile, |el| el.items_center().justify_between().gap_2())
            .mb_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(filter_pill(
                        "All",
                        notes.len(),
                        self.note_kind_filter == NoteKindFilter::All,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::All;
                            cx.notify();
                        }),
                    ))
                    .child(filter_pill(
                        "Hubs ★",
                        hubs_count,
                        self.note_kind_filter == NoteKindFilter::Hubs,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::Hubs;
                            cx.notify();
                        }),
                    ))
                    .child(filter_pill(
                        "Notes ≡",
                        regular_count,
                        self.note_kind_filter == NoteKindFilter::Notes,
                        cx.listener(|view, _, _, cx| {
                            view.note_kind_filter = NoteKindFilter::Notes;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                search_box(cols.search)
                    .when(is_mobile, |el| el.w_full())
                    .child(Input::new(search_input).size(ControlSize::Sm)),
            );

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

        let columns = vec![
            Column::new("title", "NOTE"),
            Column::new("project", "PROJECT").width(rems(cols.project.unwrap_or(140. / 16.))),
            Column::new("kind", "TYPE").width(rems(cols.kind.unwrap_or(80. / 16.))),
            Column::new("actions", "")
                .width(rems(80. / 16.))
                .actions([RowAction::new("project", "Project")]),
        ];

        let mut hidden: Vec<&'static str> = Vec::new();
        if cols.project.is_none() {
            hidden.push("project");
        }
        if cols.kind.is_none() {
            hidden.push("kind");
        }

        let rows: Rc<Vec<Row>> = Rc::new(
            filtered
                .iter()
                .map(|note| {
                    let is_hub = note.kind == "project";
                    Row::new(
                        note.path.clone(),
                        [
                            Cell::Text(note.title.clone().into()),
                            Cell::Tag(note.project.clone().into(), Tone::Accent),
                            Cell::Tag(
                                note.kind.clone().into(),
                                if is_hub { Tone::Accent } else { Tone::Neutral },
                            ),
                            Cell::Actions(vec!["project".into()]),
                        ],
                    )
                })
                .collect(),
        );

        let (note_sort, ascending) = self.note_sort;
        let sorts: Vec<SortKey> = match note_sort {
            NoteSort::Default => Vec::new(),
            _ => vec![SortKey {
                column: match note_sort {
                    NoteSort::Title => "title",
                    NoteSort::Project => "project",
                    NoteSort::Kind => "kind",
                    NoteSort::Default => unreachable!(),
                }
                .into(),
                rising: ascending,
            }],
        };

        let proj_map: std::collections::HashMap<String, String> = filtered
            .iter()
            .map(|n| (n.path.clone(), n.project.clone()))
            .collect();

        let (on_open, on_sort, on_act) = (cx.entity(), cx.entity(), cx.entity());
        let table = DataTable::new("all-notes", columns)
            .label("All notes")
            .rows(rows)
            .hidden(hidden)
            .sorts(sorts)
            .density(ely_density(cols.density))
            .on_open(move |key, window, cx| {
                let path = key.to_string();
                on_open.update(cx, |view, cx| {
                    view.open_note(path, true, window, cx);
                });
            })
            .on_sort(move |keys, _, cx| {
                let next = keys
                    .first()
                    .and_then(|k| NoteSort::from_col_id(&k.column).map(|c| (c, k.rising)))
                    .unwrap_or((NoteSort::Default, true));
                on_sort.update(cx, |view, cx| {
                    view.note_sort = next;
                    cx.notify();
                });
            })
            .on_action(move |row_key, action, _, cx| {
                if action.as_ref() == "project" {
                    if let Some(proj) = proj_map.get(row_key.as_ref()).cloned() {
                        on_act.update(cx, |view, cx| {
                            view.open_project(proj, cx);
                            view.tab = ProjectsTab::Hubs;
                        });
                    }
                }
            })
            .empty(format!("No notes matching \"{query}\"."));

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(filter_bar)
            .child(table)
            .into_any_element()
    }

    pub(super) fn hubs_table(
        &self,
        filtered: &[sjel_gpui_core::shell::ProjectSummary],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let query = self.search_query.as_str();
        let columns = vec![
            Column::new("name", "PROJECT"),
            Column::new("project_tasks", "TASKS").width(rems(cols.project_tasks / 16.)),
            Column::new("project_progress", "PROGRESS")
                .width(rems(cols.project_progress.max(100.) / 16.)),
            Column::new("project_notes", "NOTES")
                .width(rems(cols.project_notes / 16.))
                .end(),
            Column::new("actions", "")
                .width(rems(5.))
                .actions([RowAction::new("task", "+ Task")]),
        ];

        let mut hidden: Vec<&'static str> = Vec::new();
        if cols.project_progress <= 0. {
            hidden.push("project_progress");
        }

        let rows: Rc<Vec<Row>> = Rc::new(
            filtered
                .iter()
                .map(|project| {
                    let total_tasks = project.open + project.done;
                    let tasks_cell = if project.open > 0 {
                        Cell::Tag(format!("● {} open", project.open).into(), Tone::Warning)
                    } else if project.done > 0 {
                        Cell::Tag(format!("✓ {} done", project.done).into(), Tone::Success)
                    } else {
                        Cell::Text("0 tasks".into())
                    };
                    let progress_cell = if total_tasks > 0 {
                        let pct =
                            ((project.done as f32 / total_tasks as f32) * 100.0).round() as u32;
                        Cell::Figure(format!("{pct}%").into(), pct as f64, Tone::Neutral)
                    } else {
                        Cell::Empty
                    };
                    let notes_label = format!(
                        "{} note{}",
                        project.notes,
                        if project.notes == 1 { "" } else { "s" }
                    );
                    Row::new(
                        project.name.clone(),
                        [
                            Cell::Text(project.name.clone().into()),
                            tasks_cell,
                            progress_cell,
                            Cell::Figure(notes_label.into(), project.notes as f64, Tone::Neutral),
                            Cell::Actions(vec!["task".into()]),
                        ],
                    )
                })
                .collect(),
        );

        let (sort_col, sort_asc) = self.project_sort;
        let sort_col_key = match sort_col {
            ProjectSort::Name => "name",
            ProjectSort::Tasks => "project_tasks",
            ProjectSort::Progress => "project_progress",
            ProjectSort::Notes => "project_notes",
        };
        let current_name = self
            .selected_index
            .and_then(|i| filtered.get(i).map(|p| p.name.clone()));
        let empty_msg = if query.is_empty() {
            "No projects in this filter view.".to_string()
        } else {
            format!("No projects matching \"{query}\".")
        };
        let _ = palette;

        let (on_open, on_sort, on_act) = (cx.entity(), cx.entity(), cx.entity());
        DataTable::new("projects-hub", columns)
            .label("Projects")
            .rows(rows)
            .hidden(hidden)
            .sorts([SortKey {
                column: sort_col_key.into(),
                rising: sort_asc,
            }])
            .density(ely_density(cols.density))
            .current(current_name)
            .on_open(move |key, _, cx| {
                let name = key.to_string();
                on_open.update(cx, |view, cx| {
                    view.open_project(name, cx);
                });
            })
            .on_sort(move |keys, _, cx| {
                if let Some(first) = keys.first() {
                    if let Some(col) = ProjectSort::from_col_id(first.column.as_ref()) {
                        on_sort.update(cx, |view, cx| {
                            view.project_sort = (col, first.rising);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action(move |key, action, _, cx| {
                if action.as_ref() == "task" {
                    let name = key.to_string();
                    on_act.update(cx, |view, cx| {
                        view.open_project(name, cx);
                        view.project_section = ProjectSection::Tasks;
                        cx.notify();
                    });
                }
            })
            .empty(empty_msg)
            .into_any_element()
    }
}
