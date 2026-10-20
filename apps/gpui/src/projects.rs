//! The Projects page: pick a project, view tasks and notes, edit them in place.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Subscription, Window,
};

use crate::data::{self, Remote, Repo};
use crate::editor::NoteEditor;
use crate::shell::{self, GlobalTask, ProjectRecord, ProjectSummary, ShellError};
use crate::tasks::{self, TaskFilter};
use crate::theme::Palette;
use crate::ui::{
    back_button, due_badge, filter_pill, list, note, placeholder, priority_pill, progress_bar, row,
    section,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectsTab {
    Hubs,
    AllTasks,
}

pub struct Projects {
    repo: Repo,
    pub projects: Remote<Vec<ProjectSummary>>,
    pub all_tasks: Remote<Vec<GlobalTask>>,
    pub tab: ProjectsTab,
    pub task_filter: TaskFilter,
    pub global_task_filter: TaskFilter,
    pub show_archived: bool,
    pub search_query: String,
    search_input: Option<(Entity<InputState>, Subscription)>,
    /// The open project and its records. The records carry no content: the editor reads one.
    project: Option<(String, Remote<Vec<ProjectRecord>>)>,
    editor: Option<Entity<NoteEditor>>,
    /// The note being read before its editor can open.
    opening: Option<String>,
    /// The "New task" field under the task table.
    new_task: Option<(Entity<InputState>, Subscription)>,
    /// The task whose due date is being typed, and its field.
    due_edit: Option<(ProjectRecord, Entity<InputState>, Subscription)>,
    /// The "New project" field.
    new_project: Option<(Entity<InputState>, Subscription)>,
    pub adding_project: bool,
    /// The "New note" field inside project view.
    new_note: Option<(Entity<InputState>, Subscription)>,
    pub adding_note: bool,
}

impl Projects {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            projects: Remote::default(),
            all_tasks: Remote::default(),
            tab: ProjectsTab::Hubs,
            task_filter: TaskFilter::Open,
            global_task_filter: TaskFilter::Open,
            show_archived: false,
            search_query: String::new(),
            search_input: None,
            project: None,
            editor: None,
            opening: None,
            new_task: None,
            due_edit: None,
            new_project: None,
            adding_project: false,
            new_note: None,
            adding_note: false,
        }
    }

    pub fn load_projects(&mut self, cx: &mut Context<Self>) {
        self.load_all_tasks(cx);
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |p| &mut p.projects,
            &repo,
            |s| async move { s.projects().await },
        );
    }

    pub fn load_all_tasks(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |p| &mut p.all_tasks,
            &repo,
            |s| async move { s.tasks(None).await },
        );
    }

    /// Reads the open project again, keeping the rows on screen until the answer arrives.
    fn refresh_records(&mut self, cx: &mut Context<Self>) {
        if let Some((name, _)) = &self.project {
            let name = name.clone();
            self.open_project(name, cx);
        }
    }

    fn open_project(&mut self, name: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.project = Some((name.clone(), Remote::new_loading()));
        self.editor = None;
        self.opening = None;
        cx.notify();
        let request = name.clone();
        let work = shell::run(async move { shell.project(&request).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if let Some((open, records)) = &mut view.project {
                    if *open == name {
                        records.settle(result);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads one note before opening it. A list read carries no content, so this is the only
    /// read that does.
    fn open_note(&mut self, path: String, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.opening = Some(path.clone());
        cx.notify();
        let work = shell::run(async move { shell.record(&path).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                view.opening = None;
                match result {
                    Ok(record) => view.show_note(record, save, window, cx),
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_note(
        &mut self,
        record: ProjectRecord,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let notes = cx.entity().downgrade();
        let editor = cx.new(|cx| NoteEditor::new(record, shell, notes, window, cx));
        if !save {
            editor.update(cx, |editor, _| editor.editable_saves = false);
        }
        editor.update(cx, |editor, cx| editor.focus(0, 0, window, cx));
        self.editor = Some(editor);
        cx.notify();
    }

    /// Bench only (`SJEL_GPUI_BENCH_NOTE`): opens the record at `path` once its project has
    /// loaded, with saving off. False while the project or the record is still being read.
    pub(crate) fn bench_open(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editor.is_some() {
            return true;
        }
        let project = path.split('/').next().unwrap_or_default().to_owned();
        match self.project.as_ref() {
            Some((open, records)) if *open == project => {
                if records.loading || self.opening.is_some() {
                    return false;
                }
                let listed = records
                    .value
                    .as_ref()
                    .is_some_and(|records| records.iter().any(|record| record.path == path));
                if !listed {
                    return false;
                }
                self.open_note(path.to_owned(), false, window, cx);
            }
            _ => self.open_project(project, cx),
        }
        false
    }

    pub(crate) fn editor(&self) -> Option<Entity<NoteEditor>> {
        self.editor.clone()
    }

    /// Copies the frontmatter a task write answered with into the row it belongs to, so the
    /// table shows the write without reading the project again.
    fn patch_task(&mut self, task: GlobalTask, cx: &mut Context<Self>) {
        if let Some((_, records)) = &mut self.project {
            if let Some(records) = &mut records.value {
                if let Some(record) = records.iter_mut().find(|record| record.path == task.id) {
                    record.title = task.title;
                    record.done = task.done;
                    record.due = task.due.clone();
                    record.priority = task.priority;
                }
            }
        }
        if let Some(tasks) = &mut self.all_tasks.value {
            if let Some(row) = tasks.iter_mut().find(|row| row.id == task.id) {
                row.done = task.done;
                row.due = task.due;
                row.priority = task.priority;
            }
        }
        self.load_projects(cx);
        cx.notify();
    }

    fn toggle_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let done_text = if done { "Done." } else { "Open again." };
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(done_text),
            move |s| async move { s.set_task_done(&path, done).await },
            |this, task, cx| {
                if let Some(task) = task {
                    this.patch_task(task, cx);
                }
            },
        );
    }

    fn toggle_global_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_task(path, done, window, cx);
    }

    fn remove_record(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Removed."),
            move |s| async move { s.remove_record(&path).await },
            |this, _, cx| {
                this.refresh_records(cx);
                this.load_projects(cx);
            },
        );
    }

    /// Writes one frontmatter field of a task through the capability, which owns the edit and
    /// answers with the record's new frontmatter.
    fn set_task_field(
        &mut self,
        record: &ProjectRecord,
        key: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = record.path.clone();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Saved."),
            move |s| async move { s.set_property(&path, key, &value).await },
            |this, task, cx| {
                if let Some(task) = task {
                    this.patch_task(task, cx);
                }
            },
        );
    }

    fn edit_due(&mut self, record: ProjectRecord, window: &mut Window, cx: &mut Context<Self>) {
        let due = record.due.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("YYYY-MM-DD");
            state.set_value(due, window, cx);
            state
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    let due = input.read(cx).value().trim().to_owned();
                    let Some((record, _, _)) = view.due_edit.take() else {
                        return;
                    };
                    if tasks::valid_due(&due) {
                        view.set_task_field(&record, "due", due, window, cx);
                    } else {
                        window.push_notification(
                            Notification::error("A due date is YYYY-MM-DD, or empty for none."),
                            cx,
                        );
                    }
                    cx.notify();
                }
                // Leaving the field without Enter keeps the old date.
                InputEvent::Blur => {
                    view.due_edit = None;
                    cx.notify();
                }
                _ => {}
            }
        });
        self.due_edit = Some((record, input, subscription));
        cx.notify();
    }

    fn new_task_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.new_task {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New task…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let title = input.read(cx).value().trim().to_owned();
                let Some(project) = view.project.as_ref().map(|(name, _)| name.clone()) else {
                    return;
                };
                if title.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                let repo = view.repo.clone();
                data::write(
                    cx,
                    window,
                    view,
                    &repo,
                    data::Press::quiet("Task added."),
                    move |s| async move { s.add_task(&project, &title).await },
                    |this, _, cx| {
                        this.refresh_records(cx);
                        this.load_projects(cx);
                    },
                );
            }
        });
        self.new_task = Some((input.clone(), subscription));
        input
    }

    fn search_input_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search projects…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn new_project_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.new_project {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name… (Press ↩)"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |view, input, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let name = input.read(cx).value().trim().to_owned();
                        if !name.is_empty() {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                            view.adding_project = false;
                            let repo = view.repo.clone();
                            data::write(
                                cx,
                                window,
                                view,
                                &repo,
                                data::Press::quiet("Project added."),
                                move |s| async move { s.add_project(&name, "").await },
                                |this, _, cx| this.load_projects(cx),
                            );
                        }
                    }
                    InputEvent::Blur if input.read(cx).value().trim().is_empty() => {
                        view.adding_project = false;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        self.new_project = Some((input.clone(), subscription));
        input
    }

    fn new_note_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.new_note {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Note title… (Press ↩)"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |view, input, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let title = input.read(cx).value().trim().to_owned();
                        let Some(project) = view.project.as_ref().map(|(name, _)| name.clone())
                        else {
                            return;
                        };
                        if !title.is_empty() {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                            view.adding_note = false;
                            let repo = view.repo.clone();
                            data::write(
                                cx,
                                window,
                                view,
                                &repo,
                                data::Press::quiet("Note added."),
                                move |s| async move { s.add_note(&project, &title).await },
                                |this, _, cx| {
                                    this.refresh_records(cx);
                                    this.load_projects(cx);
                                },
                            );
                        }
                    }
                    InputEvent::Blur if input.read(cx).value().trim().is_empty() => {
                        view.adding_note = false;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        self.new_note = Some((input.clone(), subscription));
        input
    }

    /// Back to the record list. The project reloads so its list shows what was just saved.
    pub(crate) fn close_note(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.take() {
            editor.update(cx, |editor, cx| editor.save(cx));
        }
        if let Some(name) = self.project.as_ref().map(|(name, _)| name.clone()) {
            self.open_project(name, cx);
        }
        cx.notify();
    }
}

impl Projects {
    fn task_table(
        &self,
        records: &[ProjectRecord],
        new_task: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let open_count = records
            .iter()
            .filter(|r| r.kind == "task" && !r.done)
            .count();
        let done_count = records
            .iter()
            .filter(|r| r.kind == "task" && r.done)
            .count();
        let all_count = open_count + done_count;

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

        let header = row()
            .min_h(px(32.))
            .py_1()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(div().w(px(20.)))
            .child(div().flex_1().child("Task"))
            .child(div().w(px(120.)).child("Due"))
            .child(div().w(px(64.)).child("Priority"))
            .child(div().w(px(24.)))
            .into_any_element();
        let mut rows = vec![header];

        let filtered: Vec<_> = tasks::ordered(records)
            .into_iter()
            .filter(|t| self.task_filter.matches(t.done))
            .collect();

        for (index, task) in filtered.into_iter().enumerate() {
            let (toggle, open, del) = (task.clone(), task.clone(), task.clone());
            let due = task.due.clone().filter(|d| !d.is_empty());
            let editing = self
                .due_edit
                .as_ref()
                .filter(|(record, _, _)| record.path == task.path)
                .map(|(_, input, _)| input.clone());
            let due_cell = match editing {
                Some(input) => div()
                    .w(px(120.))
                    .child(Input::new(&input).small())
                    .into_any_element(),
                None => {
                    let edit = task.clone();
                    div()
                        .id(("due", index))
                        .w(px(120.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit_due(edit.clone(), window, cx)
                        }))
                        .child(due_badge(due.as_deref(), task.done, &today, palette))
                        .into_any_element()
                }
            };
            let cycle = task.clone();
            let priority = task.priority.clamp(1, 3);
            rows.push(
                row()
                    .min_h(px(40.))
                    .py_1()
                    .when(task.done, |row| row.opacity(0.55))
                    .child(
                        div().w(px(20.)).child(
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
                            .truncate()
                            .text_sm()
                            .cursor_pointer()
                            .when(task.done, |title| title.line_through())
                            .hover(move |style| style.text_color(rgb(palette.accent)))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.open_note(open.path.clone(), true, window, cx)
                            }))
                            .child(task.title.clone()),
                    )
                    .child(due_cell)
                    .child(
                        div()
                            .id(("priority", index))
                            .w(px(64.))
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
                        div().w(px(24.)).flex().justify_center().child(
                            div()
                                .id(("del-task", index))
                                .cursor_pointer()
                                .text_color(rgb(palette.muted))
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
            row()
                .min_h(px(40.))
                .py_1()
                .child(div().w(px(20.)).text_color(rgb(palette.muted)).child("+"))
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

    fn all_tasks_table(
        &self,
        tasks: &[GlobalTask],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let open_count = tasks.iter().filter(|t| !t.done).count();
        let done_count = tasks.iter().filter(|t| t.done).count();
        let all_count = tasks.len();

        let filter_bar = div()
            .flex()
            .items_center()
            .gap_2()
            .mb_2()
            .child(filter_pill(
                "Open",
                open_count,
                self.global_task_filter == TaskFilter::Open,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.global_task_filter = TaskFilter::Open;
                    cx.notify();
                }),
            ))
            .child(filter_pill(
                "All",
                all_count,
                self.global_task_filter == TaskFilter::All,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.global_task_filter = TaskFilter::All;
                    cx.notify();
                }),
            ))
            .child(filter_pill(
                "Done",
                done_count,
                self.global_task_filter == TaskFilter::Done,
                palette,
                cx.listener(|view, _, _, cx| {
                    view.global_task_filter = TaskFilter::Done;
                    cx.notify();
                }),
            ));

        let header = row()
            .min_h(px(32.))
            .py_1()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(div().w(px(20.)))
            .child(div().flex_1().child("Task"))
            .child(div().w(px(140.)).child("Project"))
            .child(div().w(px(120.)).child("Due"))
            .child(div().w(px(64.)).child("Priority"))
            .into_any_element();
        let mut rows = vec![header];

        let filtered: Vec<_> = tasks
            .iter()
            .filter(|t| self.global_task_filter.matches(t.done))
            .collect();

        for (index, task) in filtered.into_iter().enumerate() {
            let toggle = task.clone();
            let proj_name = task.project.clone();
            let due = task.due.clone().filter(|d| !d.is_empty());
            let priority = task.priority.clamp(1, 3);
            rows.push(
                row()
                    .min_h(px(40.))
                    .py_1()
                    .when(task.done, |row| row.opacity(0.55))
                    .child(
                        div().w(px(20.)).child(
                            Checkbox::new(("all-task-done", index))
                                .checked(task.done)
                                .on_click(cx.listener(move |view, done: &bool, window, cx| {
                                    view.toggle_global_task(toggle.id.clone(), *done, window, cx)
                                })),
                        ),
                    )
                    .child(
                        div()
                            .id(("all-task", index))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(task.done, |title| title.line_through())
                            .child(task.title.clone()),
                    )
                    .child(
                        div().w(px(140.)).child(
                            div()
                                .id(("proj-tag", index))
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
                                    view.open_project(proj_name.clone(), cx);
                                    view.tab = ProjectsTab::Hubs;
                                }))
                                .child(task.project.clone()),
                        ),
                    )
                    .child(div().w(px(120.)).child(due_badge(
                        due.as_deref(),
                        task.done,
                        &today,
                        palette,
                    )))
                    .child(div().w(px(64.)).child(priority_pill(priority, palette)))
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

impl Render for Projects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        if let Some(editor) = &self.editor {
            let title = editor.read(cx).title.clone();
            let project_name = self.project.as_ref().map_or("", |(n, _)| n.as_str());
            // An opaque card: the window is translucent, and a wallpaper behind it must not
            // show through the text.
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_3()
                .px_8()
                .pt_5()
                .pb_4()
                .rounded(px(12.))
                .bg(rgba(palette.surface | 0xff))
                .border_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(back_button(
                            format!("‹ {project_name}"),
                            cx.listener(|view, _, _, cx| view.close_note(cx)),
                            palette,
                        ))
                        .child(div().text_xs().text_color(rgb(palette.muted)).child("/"))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.muted))
                                .child(title.clone()),
                        ),
                )
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().flex_1().min_h_0().child(editor.clone()))
                .into_any_element();
        }
        if self.project.is_some() {
            let new_task = self.new_task_field(window, cx);
            let new_note = self.new_note_field(window, cx);
            let Some((name, records)) = &self.project else {
                return div().into_any_element();
            };
            let table = records
                .value
                .as_ref()
                .map(|records| self.task_table(records, &new_task, palette, cx));

            let open_tasks = records
                .value
                .as_ref()
                .map(|r| r.iter().filter(|t| t.kind == "task" && !t.done).count())
                .unwrap_or(0);
            let done_tasks = records
                .value
                .as_ref()
                .map(|r| r.iter().filter(|t| t.kind == "task" && t.done).count())
                .unwrap_or(0);
            let notes_count = records
                .value
                .as_ref()
                .map(|r| r.iter().filter(|t| t.kind != "task").count())
                .unwrap_or(0);

            let body = match (&records.value, &records.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(240.),
                (Some(records), None) => {
                    let note_records: Vec<_> = records
                        .iter()
                        .filter(|record| record.kind != "task")
                        .collect();
                    if note_records.is_empty() {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .py_2()
                            .child("No notes yet in this project.")
                            .into_any_element()
                    } else {
                        let rows = note_records
                            .into_iter()
                            .enumerate()
                            .map(|(index, record)| {
                                let open = record.clone();
                                let del = record.clone();
                                row()
                                    .id(("record", index))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgba(palette.raised)))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_note(open.path.clone(), true, window, cx)
                                    }))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .child(record.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(record.kind.clone()),
                                    )
                                    .child(
                                        div().w(px(24.)).flex().justify_center().child(
                                            div()
                                                .id(("del-note", index))
                                                .cursor_pointer()
                                                .text_color(rgb(palette.muted))
                                                .hover(move |s| s.text_color(rgb(palette.red)))
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        view.remove_record(
                                                            del.path.clone(),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                ))
                                                .child("×"),
                                        ),
                                    )
                                    .into_any_element()
                            })
                            .collect();
                        list(rows, palette)
                    }
                }
            };
            return div()
                .id("records")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3()
                .child(back_button(
                    "‹ Projects",
                    cx.listener(|view, _, _, cx| {
                        view.project = None;
                        cx.notify();
                    }),
                    palette,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!(
                                            "{open_tasks} open · {done_tasks} done · {notes_count} note{}",
                                            if notes_count == 1 { "" } else { "s" }
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .id("add-note-btn")
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .bg(rgba(palette.raised))
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .cursor_pointer()
                                .hover(move |s| s.text_color(rgb(palette.text)))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.adding_note = !view.adding_note;
                                    cx.notify();
                                }))
                                .child("+ Add note"),
                        ),
                )
                .children(table.map(|table| section("Tasks", table, palette)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .when(self.adding_note, |sec| {
                            sec.child(
                                div()
                                    .p_2()
                                    .rounded(px(8.))
                                    .bg(rgba(palette.raised))
                                    .child(Input::new(&new_note).small()),
                            )
                        })
                        .child(section("Notes", body, palette)),
                )
                .into_any_element();
        }

        // Top navigation tabs: Projects | All Tasks
        let tab_bar = div().flex().items_center().justify_between().mb_3().child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .p(px(2.))
                .rounded(px(8.))
                .bg(rgba(palette.raised))
                .child(
                    div()
                        .id("tab-hubs")
                        .px_3()
                        .py(px(4.))
                        .rounded(px(6.))
                        .text_xs()
                        .cursor_pointer()
                        .when(self.tab == ProjectsTab::Hubs, |el| {
                            el.bg(rgba(palette.surface))
                                .text_color(rgb(palette.text))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(self.tab != ProjectsTab::Hubs, |el| {
                            el.text_color(rgb(palette.muted))
                                .hover(|s| s.text_color(rgb(palette.text)))
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.tab = ProjectsTab::Hubs;
                            cx.notify();
                        }))
                        .child("Projects"),
                )
                .child(
                    div()
                        .id("tab-all-tasks")
                        .px_3()
                        .py(px(4.))
                        .rounded(px(6.))
                        .text_xs()
                        .cursor_pointer()
                        .when(self.tab == ProjectsTab::AllTasks, |el| {
                            el.bg(rgba(palette.surface))
                                .text_color(rgb(palette.text))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(self.tab != ProjectsTab::AllTasks, |el| {
                            el.text_color(rgb(palette.muted))
                                .hover(|s| s.text_color(rgb(palette.text)))
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.tab = ProjectsTab::AllTasks;
                            cx.notify();
                        }))
                        .child("All Tasks"),
                ),
        );

        if self.tab == ProjectsTab::AllTasks {
            let body = match (&self.all_tasks.value, &self.all_tasks.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(tasks), None) => self.all_tasks_table(tasks, palette, cx),
            };
            return div()
                .id("all-tasks")
                .size_full()
                .overflow_y_scroll()
                .child(tab_bar)
                .child(section("Cross-Project Tasks", body, palette))
                .into_any_element();
        }

        // Projects Hub View
        let search_input = self.search_input_field(window, cx);
        let new_project = self.new_project_field(window, cx);

        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .mb_2()
            .child(div().w(px(240.)).child(Input::new(&search_input).small()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("toggle-archived")
                            .px_2()
                            .py(px(4.))
                            .rounded(px(6.))
                            .text_xs()
                            .cursor_pointer()
                            .bg(rgba(palette.raised))
                            .text_color(rgb(if self.show_archived {
                                palette.accent
                            } else {
                                palette.muted
                            }))
                            .hover(|s| s.text_color(rgb(palette.text)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_archived = !view.show_archived;
                                cx.notify();
                            }))
                            .child(if self.show_archived {
                                "Show Active"
                            } else {
                                "Show Archived"
                            }),
                    )
                    .child(
                        div()
                            .id("new-project-btn")
                            .px_3()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(rgba(palette.accent & 0xffffff | 0x22000000))
                            .text_color(rgb(palette.accent))
                            .font_weight(FontWeight::MEDIUM)
                            .text_xs()
                            .cursor_pointer()
                            .hover(move |s| s.text_color(rgb(palette.text)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.adding_project = !view.adding_project;
                                cx.notify();
                            }))
                            .child("+ New Project"),
                    ),
            );

        let body = match (&self.projects.value, &self.projects.error) {
            (_, Some(error)) => note(error.to_string(), palette.red),
            (None, None) => placeholder(320.),
            (Some(projects), None) => {
                let query = self.search_query.as_str();
                let filtered: Vec<_> = projects
                    .iter()
                    .filter(|p| p.archived == self.show_archived)
                    .filter(|p| query.is_empty() || p.name.to_lowercase().contains(query))
                    .collect();

                if filtered.is_empty() {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .py_4()
                        .child("No matching projects found.")
                        .into_any_element()
                } else {
                    let rows = filtered
                        .into_iter()
                        .map(|project| {
                            let name = project.name.clone();
                            let total_tasks = project.open + project.done;
                            let progress = if total_tasks > 0 {
                                (project.done as f32 / total_tasks as f32).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            let task_badge = if project.open > 0 {
                                div()
                                    .px_2()
                                    .py(px(1.5))
                                    .rounded(px(4.))
                                    .bg(rgba(palette.amber & 0xffffff | 0x22000000))
                                    .text_color(rgb(palette.amber))
                                    .text_xs()
                                    .child(format!("● {} open", project.open))
                                    .into_any_element()
                            } else if project.done > 0 {
                                div()
                                    .px_2()
                                    .py(px(1.5))
                                    .rounded(px(4.))
                                    .bg(rgba(palette.green & 0xffffff | 0x22000000))
                                    .text_color(rgb(palette.green))
                                    .text_xs()
                                    .child(format!("✓ {} done", project.done))
                                    .into_any_element()
                            } else {
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child("0 tasks")
                                    .into_any_element()
                            };
                            row()
                                .id(SharedString::from(format!("project-{}", project.name)))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(palette.raised)))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_project(name.clone(), cx)
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(project.name.clone()),
                                )
                                .child(task_badge)
                                .when(total_tasks > 0, |r| {
                                    let bar_color = if project.open == 0 {
                                        palette.green
                                    } else {
                                        palette.accent
                                    };
                                    r.child(progress_bar(progress, bar_color, palette))
                                })
                                .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                    format!(
                                        "{} note{}",
                                        project.notes,
                                        if project.notes == 1 { "" } else { "s" }
                                    ),
                                ))
                                .child(div().text_color(rgb(palette.muted)).child("›"))
                                .into_any_element()
                        })
                        .collect();
                    list(rows, palette)
                }
            }
        };

        div()
            .id("projects")
            .size_full()
            .overflow_y_scroll()
            .child(tab_bar)
            .child(toolbar)
            .when(self.adding_project, |p| {
                p.child(
                    div()
                        .mb_3()
                        .p_2()
                        .rounded(px(8.))
                        .bg(rgba(palette.raised))
                        .child(Input::new(&new_project).small()),
                )
            })
            .child(section("Projects", body, palette))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::shell::{ProjectRecord, ShellError};

    fn record() -> ProjectRecord {
        ProjectRecord {
            path: "P/Plan.md".into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: "---\ntype: note\n---\n# Plan\n\n- one\n- two\n\nText\n".into(),
            done: false,
            due: None,
            priority: 2,
        }
    }

    #[gpui::test]
    fn a_project_page_draws_its_tasks_as_a_table(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)));
                let task = |title: &str, done: bool| ProjectRecord {
                    path: format!("P/{title}.md"),
                    kind: "task".into(),
                    title: title.into(),
                    content: "---\ntype: task\n---\n".into(),
                    done,
                    due: Some("2026-10-24".into()),
                    priority: 1,
                };
                let records = Remote {
                    value: Some(vec![task("open", false), task("closed", true), record()]),
                    ..Remote::default()
                };
                projects.project = Some(("P".into(), records));
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        projects.read_with(cx, |projects, _| assert!(projects.new_task.is_some()));
    }
}
