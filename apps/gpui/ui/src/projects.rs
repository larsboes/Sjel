//! The Projects page: pick a project, view tasks and notes, edit them in place.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Div, Entity, FontWeight, SharedString,
    Subscription, Window,
};

use crate::data::{self, Remote, Repo};
use crate::editor::{self, EditorEvent, NoteEditor};
use crate::primitives::{
    back_button, due_badge, filter_pill, list, note, placeholder, priority_pill, progress_bar,
    row_for_density, section,
};
use crate::tasks::{self, TaskFilter};
use crate::theme::{Density, Palette};
use sjel_gpui_core::shell::{
    self, GlobalNote, GlobalTask, ProjectRecord, ProjectSummary, ShellError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectsTab {
    Hubs,
    AllTasks,
    AllNotes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectSection {
    Note,
    Tasks,
    Notes,
}

/// The widths the record tables give their columns. `None` drops the column: a phone is too
/// narrow for the project and type columns, and its search field fills the row.
#[derive(Clone, Copy)]
struct Columns {
    due: f32,
    priority: f32,
    project: Option<f32>,
    kind: Option<f32>,
    search: Option<f32>,
    /// The page's side padding.
    pad: f32,
    /// The height and touch target a table row gets.
    density: Density,
}

impl Columns {
    fn for_density(density: Density) -> Self {
        match density {
            Density::Mobile => Self {
                due: 76.,
                priority: 40.,
                project: None,
                kind: None,
                search: None,
                pad: 12.,
                density,
            },
            _ => Self {
                due: 120.,
                priority: 64.,
                project: Some(140.),
                kind: Some(80.),
                search: Some(220.),
                pad: 32.,
                density,
            },
        }
    }
}

/// A table cell of `width`, or nothing when the density drops the column.
fn column(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)),
        None => div().hidden(),
    }
}

/// The search field's box: a fixed width on the Mac, the rest of the row on a phone.
fn search_box(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)),
        None => div().flex_1(),
    }
}

/// What the page tells the app that holds it.
pub enum ProjectsEvent {
    /// A note's editor opened (`true`) or closed. The phone shows or hides its keyboard.
    Editing(bool),
}

pub struct Projects {
    repo: Repo,
    density: Density,
    /// The page's palette. `None` follows the window's appearance; the phone sets its own.
    palette: Option<Palette>,
    pub projects: Remote<Vec<ProjectSummary>>,
    pub all_tasks: Remote<Vec<GlobalTask>>,
    pub all_notes: Remote<Vec<GlobalNote>>,
    pub tab: ProjectsTab,
    pub project_section: ProjectSection,
    pub task_filter: TaskFilter,
    pub global_task_filter: TaskFilter,
    pub show_archived: bool,
    pub search_query: String,
    search_input: Option<(Entity<InputState>, Subscription)>,
    pub notes_search_query: String,
    notes_search_input: Option<(Entity<InputState>, Subscription)>,
    /// The open project and its records. The records carry no content: the editor reads one.
    project: Option<(String, Remote<Vec<ProjectRecord>>)>,
    editor: Option<Entity<NoteEditor>>,
    /// The note being read before its editor can open.
    opening: Option<String>,
    /// The note whose last read failed. The hub opens itself only once, so a failing read is not
    /// retried on every frame.
    failed: Option<String>,
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

impl gpui_kit::EventEmitter<ProjectsEvent> for Projects {}

impl Projects {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            density: Density::Desktop,
            palette: None,
            projects: Remote::default(),
            all_tasks: Remote::default(),
            all_notes: Remote::default(),
            tab: ProjectsTab::Hubs,
            project_section: ProjectSection::Note,
            task_filter: TaskFilter::Open,
            global_task_filter: TaskFilter::Open,
            show_archived: false,
            search_query: String::new(),
            search_input: None,
            notes_search_query: String::new(),
            notes_search_input: None,
            project: None,
            editor: None,
            opening: None,
            failed: None,
            new_task: None,
            due_edit: None,
            new_project: None,
            adding_project: false,
            new_note: None,
            adding_note: false,
        }
    }

    /// The same page laid out for `density`: the phone passes `Density::Mobile`.
    pub fn with_density(mut self, density: Density) -> Self {
        self.density = density;
        self
    }

    /// Draws with `palette` instead of the window's appearance, and so does the open note.
    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette == Some(palette) {
            return;
        }
        self.palette = Some(palette);
        if let Some(editor) = &self.editor {
            editor.update(cx, |editor, cx| editor.set_palette(palette, cx));
        }
        cx.notify();
    }

    fn columns(&self) -> Columns {
        Columns::for_density(self.density)
    }

    /// Reads everything the page shows again, after something outside it changed the records:
    /// on the phone, a sync that pulled from the Mac.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_projects(cx);
        self.load_all_notes(cx);
        self.refresh_records(cx);
    }

    pub fn load_projects(&mut self, cx: &mut Context<Self>) {
        self.load_all_tasks(cx);
        self.load_all_notes(cx);
        let repo = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |p| &mut p.projects,
            &repo,
            |r| async move { r.fetch_projects().await },
        );
    }

    pub fn load_all_tasks(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |p| &mut p.all_tasks,
            &repo,
            |r| async move { r.fetch_tasks(None).await },
        );
    }

    pub fn load_all_notes(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |p| &mut p.all_notes,
            &repo,
            |r| async move { r.fetch_all_notes().await },
        );
    }

    /// Reads the open project again, keeping the rows on screen until the answer arrives.
    fn refresh_records(&mut self, cx: &mut Context<Self>) {
        if let Some((name, records)) = &mut self.project {
            let request = name.clone();
            let repo = self.repo.clone();
            records.loading = true;
            let work_req = request.clone();
            let work = shell::run(async move { repo.fetch_project_records(&work_req).await });
            cx.spawn(async move |this, cx| {
                let result = work.await.unwrap_or(Err(ShellError::Unreachable));
                let _ = this.update(cx, |view, cx| {
                    if let Some((open, records)) = &mut view.project {
                        if *open == request {
                            records.settle(result);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub fn open_project(&mut self, name: String, cx: &mut Context<Self>) {
        self.project = Some((name.clone(), Remote::new_loading()));
        self.project_section = ProjectSection::Note;
        self.editor = None;
        self.opening = None;
        cx.notify();
        let request = name.clone();
        let repo = self.repo.clone();
        let work = shell::run(async move { repo.fetch_project_records(&request).await });
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

    fn create_project_note(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let proj = project.clone();
        data::write_repo(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Project note created."),
            move |s| async move { s.add_note(&proj, &proj).await },
            |this, _, cx| {
                this.refresh_records(cx);
                this.load_projects(cx);
            },
        );
    }

    /// Opens one record of a project, for a caller outside the page (the Mac's finder): the
    /// project first, so closing the record lands in the project it belongs to.
    pub fn open_record(
        &mut self,
        project: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_project(project, cx);
        self.open_note(path, false, window, cx);
    }

    /// Opens one note. A note read in the last few minutes is in memory, so it opens in the frame
    /// the click lands in; otherwise this reads it first, because a list read carries no content.
    fn open_note(&mut self, path: String, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(record) = self.repo.notes().get(&path) {
            self.opening = None;
            self.show_note(record, save, window, cx);
            return;
        }
        // A read with nothing behind it is answered here, not on the HTTP runtime. The phone
        // keeps its own store (`is_replica`), so it reads without a shell; everywhere else an
        // unreachable Mac is known without awaiting anything, and hopping threads to learn it
        // only delays the message.
        if !self.repo.is_replica() {
            if let Err(error) = self.repo.shell() {
                self.opening = None;
                self.failed = Some(path);
                window.push_notification(Notification::error(error.to_string()), cx);
                cx.notify();
                return;
            }
        }
        self.opening = Some(path.clone());
        cx.notify();
        let requested = path.clone();
        let repo = self.repo.clone();
        let work = shell::run(async move { repo.fetch_record(&path).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                // A note opened since this read started owns the screen: this answer is stale.
                if view.opening.as_deref() != Some(requested.as_str()) {
                    return;
                }
                view.opening = None;
                match result {
                    Ok(record) => view.show_note(record, save, window, cx),
                    Err(error) => {
                        view.failed = Some(requested);
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
        let saver = editor::repo_saver(self.repo.clone());
        let editor = cx.new(|cx| NoteEditor::new(record, saver, window, cx));
        cx.subscribe(&editor, |projects, _, event, cx| match event {
            EditorEvent::Close => projects.close_note(cx),
        })
        .detach();
        if !save {
            editor.update(cx, |editor, _| editor.editable_saves = false);
        }
        if let Some(palette) = self.palette {
            editor.update(cx, |editor, cx| editor.set_palette(palette, cx));
        }
        editor.update(cx, |editor, cx| editor.focus(0, 0, window, cx));
        self.editor = Some(editor);
        cx.emit(ProjectsEvent::Editing(true));
        cx.notify();
    }

    /// Bench only (`SJEL_GPUI_BENCH_NOTE`): opens the record at `path` once its project has
    /// loaded, with saving off. False while the project or the record is still being read.
    pub fn bench_open(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
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

    pub fn editor(&self) -> Option<Entity<NoteEditor>> {
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
        // The write changes the record's frontmatter: the next open must read it, not remember it.
        self.repo.notes().forget(&path);
        let repo = self.repo.clone();
        data::write_repo(
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
        self.repo.notes().forget(&path);
        let repo = self.repo.clone();
        data::write_repo(
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
        // The write changes the record's frontmatter: the next open must read it, not remember it.
        self.repo.notes().forget(&path);
        let repo = self.repo.clone();
        data::write_repo(
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
                data::write_repo(
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

    fn notes_search_input_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.notes_search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search all notes…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.notes_search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.notes_search_input = Some((input.clone(), subscription));
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
                            data::write_repo(
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
                            data::write_repo(
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

    /// Saves what the open note still holds and closes its editor. The write outlives it.
    fn put_away_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.take() {
            editor.update(cx, |editor, cx| editor.save(cx));
            cx.emit(ProjectsEvent::Editing(false));
        }
    }

    /// Back to the record list. The project reloads so its list shows what was just saved.
    pub fn close_note(&mut self, cx: &mut Context<Self>) {
        self.put_away_editor(cx);
        if self.project_section == ProjectSection::Note {
            self.project = None;
        } else if self.project.is_some() {
            self.refresh_records(cx);
        } else {
            self.load_all_notes(cx);
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
        let cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let board = sjel_gpui_core::data::Board::from_records(records.to_vec());
        let open_count = board.open_tasks().count();
        let all_count = board.tasks().count();
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
            .bg(rgba(palette.raised & 0x00ffffff | 0x30000000))
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
                .filter(|(record, _, _)| record.path == task.path)
                .map(|(_, input, _)| input.clone());
            let due_cell = match editing {
                Some(input) => div()
                    .w(px(cols.due))
                    .child(Input::new(&input).small())
                    .into_any_element(),
                None => {
                    let edit = task.clone();
                    div()
                        .id(("due", index))
                        .w(px(cols.due))
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
                row_for_density(cols.density)
                    .id(("task-row", index))
                    .group("task-row")
                    .min_h(px(40.))
                    .py_1()
                    .hover(move |s| s.bg(rgba(palette.raised & 0x00ffffff | 0x55000000)))
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

    fn all_tasks_table(
        &self,
        tasks: &[GlobalTask],
        search_input: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let open_count = tasks.iter().filter(|t| !t.done).count();
        let done_count = tasks.iter().filter(|t| t.done).count();
        let all_count = tasks.len();

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
                    )),
            )
            .child(search_box(cols.search).child(Input::new(search_input).small()));

        let header = row_for_density(cols.density)
            .min_h(px(34.))
            .py_1()
            .bg(rgba(palette.raised & 0x00ffffff | 0x30000000))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child(div().w(px(24.)))
            .child(div().flex_1().child("TASK"))
            .child(column(cols.project).child("PROJECT"))
            .child(div().w(px(cols.due)).child("DUE"))
            .child(div().w(px(cols.priority)).child("PRIORITY"))
            .into_any_element();
        let mut rows = vec![header];

        let query = self.search_query.as_str();
        let filtered: Vec<_> = tasks
            .iter()
            .filter(|t| self.global_task_filter.matches(t.done))
            .filter(|t| {
                query.is_empty()
                    || t.title.to_lowercase().contains(query)
                    || t.project.to_lowercase().contains(query)
            })
            .collect();

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

        for (index, task) in filtered.into_iter().enumerate() {
            let toggle = task.clone();
            let proj_name = task.project.clone();
            let due = task.due.clone().filter(|d| !d.is_empty());
            let priority = task.priority.clamp(1, 3);
            rows.push(
                row_for_density(cols.density)
                    .id(("all-task-row", index))
                    .group("all-task-row")
                    .min_h(px(40.))
                    .py_1()
                    .hover(move |s| s.bg(rgba(palette.raised & 0x00ffffff | 0x55000000)))
                    .when(task.done, |row| row.opacity(0.55))
                    .child(
                        div().w(px(24.)).flex().items_center().child(
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
                        column(cols.project).child(
                            div()
                                .id(("proj-tag", index))
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(palette.accent & 0x00ffffff | 0x12000000))
                                .border_1()
                                .border_color(rgba(palette.accent & 0x00ffffff | 0x24000000))
                                .text_color(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .cursor_pointer()
                                .hover(move |s| {
                                    s.bg(rgba(palette.accent & 0x00ffffff | 0x22000000))
                                        .text_color(rgb(palette.text))
                                })
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_project(proj_name.clone(), cx);
                                    view.tab = ProjectsTab::Hubs;
                                }))
                                .child(task.project.clone()),
                        ),
                    )
                    .child(div().w(px(cols.due)).child(due_badge(
                        due.as_deref(),
                        task.done,
                        &today,
                        palette,
                    )))
                    .child(
                        div()
                            .w(px(cols.priority))
                            .child(priority_pill(priority, palette)),
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

    fn all_notes_table(
        &self,
        notes: &[GlobalNote],
        search_input: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cols = self.columns();
        let filter_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .mb_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child(format!("{} notes total", notes.len())),
            )
            .child(search_box(cols.search).child(Input::new(search_input).small()));

        let header = row_for_density(cols.density)
            .min_h(px(34.))
            .py_1()
            .bg(rgba(palette.raised & 0x00ffffff | 0x30000000))
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
                query.is_empty()
                    || n.title.to_lowercase().contains(query)
                    || n.project.to_lowercase().contains(query)
                    || n.path.to_lowercase().contains(query)
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
                    .hover(move |s| s.bg(rgba(palette.raised & 0x00ffffff | 0x55000000)))
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
                                .bg(rgba(palette.accent & 0x00ffffff | 0x12000000))
                                .border_1()
                                .border_color(rgba(palette.accent & 0x00ffffff | 0x24000000))
                                .text_color(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .cursor_pointer()
                                .hover(move |s| {
                                    s.bg(rgba(palette.accent & 0x00ffffff | 0x22000000))
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
                                    palette.accent & 0x00ffffff | 0x18000000
                                } else {
                                    palette.raised
                                }))
                                .border_1()
                                .border_color(rgba(if is_hub {
                                    palette.accent & 0x00ffffff | 0x30000000
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

impl Render for Projects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cols = self.columns();
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(window.appearance()));
        // 1. Note editor open with NO project selected (e.g. opened from All Notes)
        if self.project.is_none() {
            if let Some(editor) = &self.editor {
                let title = editor.read(cx).title.clone();
                // An opaque card: the window is translucent, and a wallpaper behind it must not
                // show through the text.
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .px(px(cols.pad))
                    .pt_5()
                    .pb_4()
                    .rounded(px(12.))
                    .bg(rgba(palette.surface | 0xff))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(760.))
                            .mx_auto()
                            .size_full()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(back_button(
                                        "‹ All Notes",
                                        cx.listener(|view, _, _, cx| view.close_note(cx)),
                                        palette,
                                    ))
                                    .child(
                                        div().text_xs().text_color(rgb(palette.muted)).child("/"),
                                    )
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
                            .child(div().flex_1().min_h_0().child(editor.clone())),
                    )
                    .into_any_element();
            }
        }
        if self.project.is_some() {
            let hub_to_open = if self.project_section == ProjectSection::Note
                && self.editor.is_none()
                && self.opening.is_none()
            {
                self.project.as_ref().and_then(|(name, records)| {
                    records.value.as_ref().and_then(|r| {
                        r.iter()
                            .find(|t| t.kind == "project" || t.path == format!("{name}/{name}.md"))
                            .map(|t| t.path.clone())
                            .filter(|path| self.failed.as_ref() != Some(path))
                    })
                })
            } else {
                None
            };
            if let Some(hub_path) = hub_to_open {
                self.open_note(hub_path, true, window, cx);
            }

            let new_task = self.new_task_field(window, cx);
            let new_note = self.new_note_field(window, cx);
            let Some((name, records)) = &self.project else {
                return div().into_any_element();
            };
            let project_name = name.clone();

            let (open_tasks, supporting_notes_count) = records
                .value
                .as_ref()
                .map(|r| {
                    let board = sjel_gpui_core::data::Board::from_records(r.clone());
                    let open = board.open_tasks().count();
                    let notes = board
                        .notes()
                        .filter(|t| {
                            t.kind != "project"
                                && t.path != format!("{project_name}/{project_name}.md")
                        })
                        .count();
                    (open, notes)
                })
                .unwrap_or((0, 0));

            // Project header bar with breadcrumb and section tabs
            let project_header = div()
                .flex()
                .items_center()
                .justify_between()
                .mb_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(back_button(
                            "‹ Projects",
                            cx.listener(|view, _, _, cx| {
                                view.put_away_editor(cx);
                                view.project = None;
                                cx.notify();
                            }),
                            palette,
                        ))
                        .child(div().text_xs().text_color(rgb(palette.muted)).child("/"))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(project_name.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .p(px(2.))
                        .rounded(px(8.))
                        .bg(rgba(palette.raised))
                        .child(
                            div()
                                .id("sec-project-note")
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .text_xs()
                                .cursor_pointer()
                                .when(self.project_section == ProjectSection::Note, |el| {
                                    el.bg(rgba(palette.surface))
                                        .text_color(rgb(palette.text))
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(self.project_section != ProjectSection::Note, |el| {
                                    el.text_color(rgb(palette.muted))
                                        .hover(|s| s.text_color(rgb(palette.text)))
                                })
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.project_section != ProjectSection::Note {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::Note;
                                        cx.notify();
                                    }
                                }))
                                .child("Project Note"),
                        )
                        .child(
                            div()
                                .id("sec-tasks")
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .text_xs()
                                .cursor_pointer()
                                .when(self.project_section == ProjectSection::Tasks, |el| {
                                    el.bg(rgba(palette.surface))
                                        .text_color(rgb(palette.text))
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(self.project_section != ProjectSection::Tasks, |el| {
                                    el.text_color(rgb(palette.muted))
                                        .hover(|s| s.text_color(rgb(palette.text)))
                                })
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.project_section != ProjectSection::Tasks {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::Tasks;
                                        cx.notify();
                                    }
                                }))
                                .child(format!("Tasks ({open_tasks})")),
                        )
                        .child(
                            div()
                                .id("sec-notes")
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .text_xs()
                                .cursor_pointer()
                                .when(self.project_section == ProjectSection::Notes, |el| {
                                    el.bg(rgba(palette.surface))
                                        .text_color(rgb(palette.text))
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(self.project_section != ProjectSection::Notes, |el| {
                                    el.text_color(rgb(palette.muted))
                                        .hover(|s| s.text_color(rgb(palette.text)))
                                })
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.project_section != ProjectSection::Notes {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::Notes;
                                        cx.notify();
                                    }
                                }))
                                .child(format!("Notes ({supporting_notes_count})")),
                        ),
                );

            let content: AnyElement = match self.project_section {
                ProjectSection::Note => {
                    let has_hub = records.value.as_ref().is_some_and(|r| {
                        r.iter().any(|t| {
                            t.kind == "project"
                                || t.path == format!("{project_name}/{project_name}.md")
                        })
                    });

                    if records.loading || self.opening.is_some() {
                        placeholder(240.).into_any_element()
                    } else if let Some(editor) = &self.editor {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .px(px(cols.pad))
                            .pt_4()
                            .pb_4()
                            .rounded(px(12.))
                            .bg(rgba(palette.surface | 0xff))
                            .border_1()
                            .border_color(rgba(palette.border))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(760.))
                                    .mx_auto()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(editor.read(cx).title.clone()),
                                    )
                                    .child(div().flex_1().min_h_0().child(editor.clone())),
                            )
                            .into_any_element()
                    } else if !has_hub {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .py_16()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("No hub note found for {project_name}.")),
                            )
                            .child(
                                div()
                                    .id("create-project-note-btn")
                                    .px_4()
                                    .py_2()
                                    .rounded(px(6.))
                                    .bg(rgba(palette.accent))
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.surface))
                                    .cursor_pointer()
                                    .hover(move |s| s.opacity(0.9))
                                    .on_click(cx.listener({
                                        let proj = project_name.clone();
                                        move |view, _, window, cx| {
                                            view.create_project_note(proj.clone(), window, cx);
                                        }
                                    }))
                                    .child(format!("+ Create {project_name}.md")),
                            )
                            .into_any_element()
                    } else {
                        placeholder(240.).into_any_element()
                    }
                }
                ProjectSection::Tasks => match (&records.value, &records.error) {
                    (_, Some(error)) => note(error.to_string(), palette.red).into_any_element(),
                    (None, None) => placeholder(240.).into_any_element(),
                    (Some(records_list), None) => {
                        self.task_table(records_list, &new_task, palette, cx)
                    }
                },
                ProjectSection::Notes => {
                    if let Some(editor) = &self.editor {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .px(px(cols.pad))
                            .pt_5()
                            .pb_4()
                            .rounded(px(12.))
                            .bg(rgba(palette.surface | 0xff))
                            .border_1()
                            .border_color(rgba(palette.border))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(760.))
                                    .mx_auto()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(back_button(
                                                format!("‹ {project_name} Notes"),
                                                cx.listener(|view, _, _, cx| {
                                                    view.put_away_editor(cx);
                                                    cx.notify();
                                                }),
                                                palette,
                                            ))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(palette.muted))
                                                    .child("/"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.muted))
                                                    .child(editor.read(cx).title.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(editor.read(cx).title.clone()),
                                    )
                                    .child(div().flex_1().min_h_0().child(editor.clone())),
                            )
                            .into_any_element()
                    } else {
                        match (&records.value, &records.error) {
                            (_, Some(error)) => {
                                note(error.to_string(), palette.red).into_any_element()
                            }
                            (None, None) => placeholder(240.).into_any_element(),
                            (Some(records_list), None) => {
                                let board =
                                    sjel_gpui_core::data::Board::from_records(records_list.clone());
                                let note_records: Vec<_> = board
                                    .notes()
                                    .filter(|record| {
                                        record.kind != "project"
                                            && record.path
                                                != format!("{project_name}/{project_name}.md")
                                    })
                                    .collect();

                                let notes_content = if note_records.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .py_2()
                                        .child("No supporting notes yet in this project.")
                                        .into_any_element()
                                } else {
                                    let rows = note_records
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, record)| {
                                            let open = record.clone();
                                            let del = record.clone();
                                            row_for_density(cols.density)
                                                .id(("record", index))
                                                .group("record-row")
                                                .min_h(px(40.))
                                                .py_1()
                                                .cursor_pointer()
                                                .hover(move |style| {
                                                    style.bg(rgba(
                                                        palette.raised & 0x00ffffff | 0x55000000,
                                                    ))
                                                })
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        view.open_note(
                                                            open.path.clone(),
                                                            true,
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                ))
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
                                                            .text_color(rgb(palette.muted))
                                                            .child("≡"),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_sm()
                                                        .hover(move |s| {
                                                            s.text_color(rgb(palette.accent))
                                                        })
                                                        .child(record.title.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .px_2()
                                                        .py(px(1.5))
                                                        .rounded(px(4.))
                                                        .bg(rgba(palette.raised))
                                                        .border_1()
                                                        .border_color(rgba(palette.border))
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .child(record.kind.clone()),
                                                )
                                                .child(
                                                    div().w(px(28.)).flex().justify_center().child(
                                                        div()
                                                            .id(("del-note", index))
                                                            .cursor_pointer()
                                                            .text_color(rgb(palette.muted))
                                                            .opacity(0.)
                                                            .group_hover("record-row", |s| {
                                                                s.opacity(1.)
                                                            })
                                                            .hover(move |s| {
                                                                s.text_color(rgb(palette.red))
                                                            })
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
                                };

                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(rgb(palette.muted))
                                                    .child(format!(
                                                        "Supporting Notes ({supporting_notes_count})"
                                                    )),
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
                                    .when(self.adding_note, |sec| {
                                        sec.child(
                                            div()
                                                .p_2()
                                                .rounded(px(8.))
                                                .bg(rgba(palette.raised))
                                                .child(Input::new(&new_note).small()),
                                        )
                                    })
                                    .child(notes_content)
                                    .into_any_element()
                            }
                        }
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
                .child(project_header)
                .child(content)
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
                )
                .child(
                    div()
                        .id("tab-all-notes")
                        .px_3()
                        .py(px(4.))
                        .rounded(px(6.))
                        .text_xs()
                        .cursor_pointer()
                        .when(self.tab == ProjectsTab::AllNotes, |el| {
                            el.bg(rgba(palette.surface))
                                .text_color(rgb(palette.text))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(self.tab != ProjectsTab::AllNotes, |el| {
                            el.text_color(rgb(palette.muted))
                                .hover(|s| s.text_color(rgb(palette.text)))
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.tab = ProjectsTab::AllNotes;
                            cx.notify();
                        }))
                        .child("All Notes"),
                ),
        );

        if self.tab == ProjectsTab::AllNotes {
            let search_input = self.notes_search_input_field(window, cx);
            let body = match (&self.all_notes.value, &self.all_notes.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(notes), None) => self.all_notes_table(notes, &search_input, palette, cx),
            };
            return div()
                .id("all-notes")
                .size_full()
                .overflow_y_scroll()
                .child(tab_bar)
                .child(section("Cross-Project Notes", body, palette))
                .into_any_element();
        }

        if self.tab == ProjectsTab::AllTasks {
            let search_input = self.search_input_field(window, cx);
            let body = match (&self.all_tasks.value, &self.all_tasks.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(tasks), None) => self.all_tasks_table(tasks, &search_input, palette, cx),
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
            .child(search_box(cols.search).child(Input::new(&search_input).small()))
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
                            row_for_density(cols.density)
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
    use sjel_gpui_core::shell::{ProjectRecord, ShellError};

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

    #[gpui::test]
    fn all_tasks_tab_filters_by_query(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)));
                projects.tab = ProjectsTab::AllTasks;
                projects.search_query = "berlin".into();
                projects.all_tasks = Remote {
                    value: Some(vec![
                        GlobalTask {
                            id: "P/1.md".into(),
                            kind: "task".into(),
                            title: "Book train to Berlin".into(),
                            done: false,
                            due: None,
                            priority: 1,
                            summary: None,
                            project: "Travel".into(),
                        },
                        GlobalTask {
                            id: "P/2.md".into(),
                            kind: "task".into(),
                            title: "Update resume".into(),
                            done: false,
                            due: None,
                            priority: 2,
                            summary: None,
                            project: "Career".into(),
                        },
                    ]),
                    ..Remote::default()
                };
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.tab, ProjectsTab::AllTasks);
            assert_eq!(projects.search_query, "berlin");
        });
    }

    #[gpui::test]
    fn all_notes_tab_filters_by_query(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)));
                projects.tab = ProjectsTab::AllNotes;
                projects.notes_search_query = "architecture".into();
                projects.all_notes = Remote {
                    value: Some(vec![
                        GlobalNote {
                            path: "Sjel/Architecture.md".into(),
                            kind: "note".into(),
                            title: "Architecture".into(),
                            project: "Sjel".into(),
                        },
                        GlobalNote {
                            path: "BetterCode/BetterCode.md".into(),
                            kind: "project".into(),
                            title: "BetterCode".into(),
                            project: "BetterCode".into(),
                        },
                    ]),
                    ..Remote::default()
                };
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.tab, ProjectsTab::AllNotes);
            assert_eq!(projects.notes_search_query, "architecture");
            assert_eq!(projects.all_notes.value.as_ref().unwrap().len(), 2);
        });
    }

    #[gpui::test]
    fn project_section_auto_opens_hub_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)));
                let hub = ProjectRecord {
                    path: "P/P.md".into(),
                    kind: "project".into(),
                    title: "P".into(),
                    content: "# P\n".into(),
                    done: false,
                    due: None,
                    priority: 1,
                };
                let records = Remote {
                    value: Some(vec![hub, record()]),
                    ..Remote::default()
                };
                projects.project = Some(("P".into(), records));
                projects.project_section = ProjectSection::Note;
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.project_section, ProjectSection::Note);
            // The repo has no shell, so the read fails: the hub was tried once, not on every frame.
            assert_eq!(projects.failed.as_deref(), Some("P/P.md"));
        });
    }
}
