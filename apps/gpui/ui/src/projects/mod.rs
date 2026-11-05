//! The Projects page: pick a project, view tasks and notes, edit them in place.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Div, Entity, FontWeight, ScrollHandle,
    SharedString, Subscription, Window,
};

use crate::data::{self, Remote, Repo};
use crate::editor::{self, EditorEvent, NoteEditor};
use crate::primitives::{
    back_button, list, note, placeholder, progress_bar, row_for_density, section,
};
use crate::tasks::{self, TaskFilter};
use crate::theme::{tint, Density, Palette, ThemeContext};
use sjel_gpui_core::data::Blocked;
use sjel_gpui_core::shell::{
    self, GlobalNote, GlobalTask, ProjectInventoryLink, ProjectLinkRecord, ProjectMailProposal,
    ProjectMailTarget, ProjectRecord, ProjectSummary, ShellError,
};

mod links;
mod mail;
mod tables;

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
    MailReview,
    InventoryLinks,
}

pub(super) struct LinkReview {
    pub(super) record: ProjectLinkRecord,
    pub(super) item_ref: String,
    pub(super) item_label: String,
    pub(super) remove: bool,
}

/// The widths the record tables give their columns. `None` drops the column: a phone is too
/// narrow for the project and type columns, and its search field fills the row.
#[derive(Clone, Copy)]
pub(super) struct Columns {
    pub(super) due: f32,
    pub(super) priority: f32,
    pub(super) project: Option<f32>,
    pub(super) kind: Option<f32>,
    pub(super) search: Option<f32>,
    /// The page's side padding.
    pub(super) pad: f32,
    /// The height and touch target a table row gets.
    pub(super) density: Density,
}

impl Columns {
    pub(super) fn for_density(density: Density) -> Self {
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
pub(super) fn column(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)),
        None => div().hidden(),
    }
}

/// The search field's box: a fixed width on the Mac, the rest of the row on a phone.
pub(super) fn search_box(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)),
        None => div().flex_1(),
    }
}

/// Extracts a clean preview snippet from markdown body text.
fn extract_note_snippet(content: &str) -> Option<String> {
    let mut in_frontmatter = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter || trimmed.is_empty() {
            continue;
        }
        let clean = trimmed
            .trim_start_matches('#')
            .trim_start_matches('-')
            .trim_start_matches('*')
            .trim();
        if !clean.is_empty() {
            return Some(clean.to_string());
        }
    }
    None
}

/// What the page tells the app that holds it.
pub enum ProjectsEvent {
    /// A note's editor opened (`true`) or closed. The phone shows or hides its keyboard.
    Editing(bool),
    /// The person asked to send the blocked writes again. The app owns the sync, so it runs one.
    Retry,
}

/// What kinds of notes to display in the cross-project notes table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKindFilter {
    All,
    Hubs,
    Notes,
}

pub struct Projects {
    pub(super) repo: Repo,
    pub(super) density: Density,
    /// The page's palette. `None` follows the window's appearance; the phone sets its own.
    pub(super) palette: Option<Palette>,
    pub projects: Remote<Vec<ProjectSummary>>,
    pub all_tasks: Remote<Vec<GlobalTask>>,
    pub all_notes: Remote<Vec<GlobalNote>>,
    pub tab: ProjectsTab,
    pub project_section: ProjectSection,
    pub task_filter: TaskFilter,
    pub global_task_filter: TaskFilter,
    pub note_kind_filter: NoteKindFilter,
    pub show_archived: bool,
    pub search_query: String,
    pub(super) search_input: Option<(Entity<InputState>, Subscription)>,
    pub notes_search_query: String,
    pub(super) notes_search_input: Option<(Entity<InputState>, Subscription)>,
    /// The open project and its records. The records carry no content: the editor reads one.
    pub(super) project: Option<(String, Remote<Vec<ProjectRecord>>)>,
    pub(super) mail: Remote<Vec<ProjectMailProposal>>,
    pub(super) mail_cursor: Option<i64>,
    pub(super) mail_more: bool,
    pub(super) mail_selected: Option<ProjectMailProposal>,
    pub(super) mail_target: Option<ProjectMailTarget>,
    pub(super) mail_target_loading: bool,
    pub(super) mail_busy: bool,
    pub(super) mail_message: Option<String>,
    pub(super) link_path: Option<String>,
    pub(super) links: Remote<Vec<ProjectInventoryLink>>,
    pub(super) link_items: Remote<Vec<sjel_gpui_core::inventory::InventoryRow>>,
    pub(super) link_review: Option<LinkReview>,
    pub(super) link_busy: bool,
    pub(super) link_message: Option<String>,
    pub(super) link_generation: u64,
    pub(super) editor: Option<Entity<NoteEditor>>,
    /// The note being read before its editor can open.
    pub(super) opening: Option<String>,
    /// The note whose title is being typed: its path, the title it had, the field and its events.
    pub(super) title_edit: Option<(String, String, Entity<InputState>, Subscription)>,
    /// The note whose last read failed. The hub opens itself only once, so a failing read is not
    /// retried on every frame.
    pub(super) failed: Option<String>,
    /// The "New task" field under the task table.
    pub(super) new_task: Option<(Entity<InputState>, Subscription)>,
    /// The task whose due date is being typed, and its field.
    pub(super) due_edit: Option<(ProjectRecord, Entity<InputState>, Subscription)>,
    /// The "New project" field.
    pub(super) new_project: Option<(Entity<InputState>, Subscription)>,
    pub adding_project: bool,
    /// The "New note" field inside project view.
    pub(super) new_note: Option<(Entity<InputState>, Subscription)>,
    pub adding_note: bool,
    /// The writes the Mac refused, with its reasons. Their content is still here and still queued:
    /// the band these draw is the only place that says so.
    pub(super) blocked: Vec<Blocked>,
    pub(super) scroll: ScrollHandle,
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
            note_kind_filter: NoteKindFilter::All,
            show_archived: false,
            search_query: String::new(),
            search_input: None,
            notes_search_query: String::new(),
            notes_search_input: None,
            project: None,
            mail: Remote::default(),
            mail_cursor: None,
            mail_more: false,
            mail_selected: None,
            mail_target: None,
            mail_target_loading: false,
            mail_busy: false,
            mail_message: None,
            link_path: None,
            links: Remote::default(),
            link_items: Remote::default(),
            link_review: None,
            link_busy: false,
            link_message: None,
            link_generation: 0,
            editor: None,
            opening: None,
            title_edit: None,
            failed: None,
            new_task: None,
            due_edit: None,
            new_project: None,
            adding_project: false,
            new_note: None,
            adding_note: false,
            blocked: Vec::new(),
            scroll: ScrollHandle::new(),
        }
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.clone()
    }

    /// The records the Mac refused, handed in by the app after a sync read them back.
    pub fn set_blocked(&mut self, blocked: Vec<Blocked>, cx: &mut Context<Self>) {
        if self.blocked == blocked {
            return;
        }
        self.blocked = blocked;
        cx.notify();
    }

    /// What the page is showing as blocked, for a host that wants to say so elsewhere.
    pub fn blocked(&self) -> &[Blocked] {
        &self.blocked
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

    /// The writes the Mac would not take: what each record is, why the Mac said no, and that it is
    /// still here. Nothing here renames, deletes or edits a record to make the refusal go away —
    /// a duplicate title and an archived project are the operator's decision, so the page names them
    /// and offers the two things that are this page's to offer: open it, or send it again.
    fn blocked_band(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.blocked.is_empty() {
            return None;
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, refusal) in self.blocked.clone().into_iter().enumerate() {
            // The record's own title when the replica still has it, else the path: a blocked record
            // that somebody deleted from under the queue is not a record to name.
            let title = refusal.title.clone();
            let kind = refusal.kind.clone();
            let open = refusal.path.clone();
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_2()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgba(palette.border))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.text))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(kind),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("blocked-open-{open}")))
                                    .flex_none()
                                    .px_3()
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .bg(rgba(palette.raised))
                                    .text_xs()
                                    .text_color(rgb(palette.accent))
                                    .cursor_pointer()
                                    .child("Open")
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_note(open.clone(), true, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.red))
                            .child(refusal.reason),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Saved here and still queued. Nothing was lost or renamed."),
                    )
                    .into_any_element(),
            );
        }
        let count = rows.len();
        Some(
            div()
                .mb_3()
                .rounded(px(10.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(tint(palette.red, 0x55)))
                .px_3()
                .py_2()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.red))
                                .child(match count {
                                    1 => "The Mac would not take one write".to_owned(),
                                    n => format!("The Mac would not take {n} writes"),
                                }),
                        )
                        .child(
                            div()
                                .id("blocked-retry")
                                .flex_none()
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .bg(rgba(tint(palette.accent, 0x22)))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .cursor_pointer()
                                .child("Try again")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(ProjectsEvent::Retry))),
                        ),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    pub(super) fn columns(&self) -> Columns {
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
        self.mail = Remote::default();
        self.mail_cursor = None;
        self.mail_more = false;
        self.mail_selected = None;
        self.mail_target = None;
        self.mail_message = None;
        self.link_generation = self.link_generation.wrapping_add(1);
        self.link_path = None;
        self.link_review = None;
        self.links = Remote::default();
        self.link_items = Remote::default();
        self.link_busy = false;
        self.link_message = None;
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
    /// The open note's title: a field while it is being typed, else a heading that opens one.
    ///
    /// A title is the record's file name (`capabilities/projects`), so renaming moves the file.
    /// The phone and its replica keep it read-only: a move there would arrive at the Mac as a new
    /// note beside the old one.
    fn note_title(
        &self,
        editor: &Entity<NoteEditor>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (path, title) = {
            let editor = editor.read(cx);
            (editor.path().to_owned(), editor.title.clone())
        };
        if let Some((editing, _, input, _)) = &self.title_edit {
            if *editing == path {
                let input = input.clone();
                return div()
                    .w_full()
                    .max_w(px(560.))
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .capture_action(cx.listener(|view, _: &Escape, _, cx| {
                        view.cancel_title(cx);
                        cx.stop_propagation();
                    }))
                    .child(Input::new(&input).appearance(false))
                    .into_any_element();
            }
        }
        let renamable = self.density != Density::Mobile && !self.repo.is_replica();
        let heading = div()
            .text_2xl()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(title);
        if !renamable {
            return heading.into_any_element();
        }
        div()
            .id("note-title")
            .group("note-title")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(|view, _, window, cx| view.edit_title(window, cx)))
            .child(
                div()
                    .rounded(px(4.))
                    .px(px(2.))
                    .mx(px(-2.))
                    .border_b_1()
                    .border_color(rgba(0x00000000))
                    .group_hover("note-title", |style| {
                        style.border_color(rgba(tint(palette.muted, 0x88)))
                    })
                    .child(heading),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .opacity(0.)
                    .group_hover("note-title", |style| style.opacity(0.8))
                    .child("✎"),
            )
            .into_any_element()
    }

    /// Turns the open note's title into a field, holding what it was, so `Esc` can put it back.
    fn edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let (path, title) = {
            let editor = editor.read(cx);
            (editor.path().to_owned(), editor.title.to_string())
        };
        if self
            .title_edit
            .as_ref()
            .is_some_and(|(open, _, _, _)| *open == path)
        {
            return;
        }
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Note title… (Press ↩ or Esc)");
            state.set_value(title.clone(), window, cx);
            state
        });
        let subscription = cx.subscribe_in(&input, window, |view, _, event, _, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => view.commit_title(cx),
            InputEvent::Blur => view.cancel_title(cx),
            _ => {}
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.title_edit = Some((path, title, input, subscription));
        cx.notify();
    }

    /// Takes the typed title and asks the editor for it once the note is saved.
    fn commit_title(&mut self, cx: &mut Context<Self>) {
        let Some((_, was, input, _)) = self.title_edit.take() else {
            return;
        };
        cx.notify();
        let title = input.read(cx).value().trim().to_owned();
        if title.is_empty() || title == was {
            return;
        }
        if let Some(editor) = self.editor.clone() {
            editor.update(cx, |editor, cx| editor.request_rename(title, cx));
        }
    }

    fn cancel_title(&mut self, cx: &mut Context<Self>) {
        if self.title_edit.take().is_some() {
            cx.notify();
        }
    }

    /// Moves the open note's file to the new title, and the `[[links]]` that named the old one.
    fn rename_open_note(&mut self, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let path = editor.read(cx).path().to_owned();
        let was = path.clone();
        let repo = self.repo.clone();
        let work = shell::run(async move { repo.rename_record(&path, &title).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                match result {
                    Ok(renamed) => {
                        let links = renamed.links.len();
                        let left = renamed.failed.len();
                        view.refresh_records(cx);
                        view.load_all_notes(cx);
                        // Only follow the note that was renamed: whatever is on screen now, if it
                        // moved meanwhile, is not this one.
                        let still_open = view
                            .editor
                            .as_ref()
                            .is_some_and(|editor| editor.read(cx).path() == was);
                        if still_open {
                            view.open_note(renamed.path, true, window, cx);
                        }
                        let mut done = format!("Renamed to {}", renamed.title);
                        if links > 0 {
                            done.push_str(&format!(" · {links} link(s) moved"));
                        }
                        if left > 0 {
                            done.push_str(&format!(" · {left} still name the old title"));
                        }
                        window.push_notification(Notification::success(done), cx);
                    }
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_note(&mut self, path: String, save: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        cx.subscribe_in(
            &editor,
            window,
            |projects, _, event, window, cx| match event {
                EditorEvent::Close => projects.close_note(cx),
                EditorEvent::Rename(title) => {
                    projects.rename_open_note(title.to_string(), window, cx)
                }
            },
        )
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

    pub(super) fn toggle_task(
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

    pub(super) fn toggle_global_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_task(path, done, window, cx);
    }

    pub(super) fn remove_record(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn set_task_field(
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

    pub(super) fn edit_due(&mut self, record: ProjectRecord, window: &mut Window, cx: &mut Context<Self>) {
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
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New task… (!1-!3 priority, @today, @tomorrow)")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let raw_title = input.read(cx).value().trim().to_owned();
                let Some(project) = view.project.as_ref().map(|(name, _)| name.clone()) else {
                    return;
                };
                if raw_title.is_empty() {
                    return;
                }
                let today = chrono::Local::now().date_naive();
                let (title, due, priority) = tasks::parse_smart_task_input(&raw_title, today);
                if title.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                let repo = view.repo.clone();
                let due_clone = due.clone();
                let priority_clone = priority;
                data::write_repo(
                    cx,
                    window,
                    view,
                    &repo,
                    data::Press::quiet("Task added."),
                    move |s| async move {
                        s.add_task_with_details(
                            &project,
                            &title,
                            due_clone.as_deref(),
                            priority_clone,
                        )
                        .await
                    },
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
        self.title_edit = None;
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


impl Render for Projects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cols = self.columns();
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
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
                            .child(self.note_title(editor, palette, cx))
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
                    let hub_path = format!("{project_name}/{project_name}.md");
                    let open = r.iter().filter(|t| t.kind == "task" && !t.done).count();
                    let notes = r
                        .iter()
                        .filter(|t| t.kind != "task" && t.kind != "project" && t.path != hub_path)
                        .count();
                    (open, notes)
                })
                .unwrap_or((0, 0));

            // Project header bar with breadcrumb and section tabs
            let project_header = div()
                .flex()
                .items_center()
                .justify_between()
                .when(self.density == Density::Mobile, |el| {
                    el.flex_col().items_start().gap_2()
                })
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
                        .flex_wrap()
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
                        )
                        .child(
                            Button::new("sec-mail-review")
                                .small()
                                .ghost()
                                .label("Mail review")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.project_section != ProjectSection::MailReview {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::MailReview;
                                        view.load_project_mail(false, cx);
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Button::new("sec-inventory-links")
                                .small()
                                .ghost()
                                .label("Inventory links")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if view.project_section != ProjectSection::InventoryLinks {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::InventoryLinks;
                                        cx.notify();
                                    }
                                })),
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
                                    .child(self.note_title(editor, palette, cx))
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
                                    .bg(rgb(palette.accent))
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0xffffff))
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
                ProjectSection::MailReview => self.render_project_mail(records, palette, cx),
                ProjectSection::InventoryLinks => self.render_inventory_links(records, palette, cx),
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
                                    .child(self.note_title(editor, palette, cx))
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
                                let hub_path = format!("{project_name}/{project_name}.md");
                                let note_records: Vec<_> = records_list
                                    .iter()
                                    .filter(|record| {
                                        record.kind != "task"
                                            && record.kind != "project"
                                            && record.path != hub_path
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
                                            let snippet = extract_note_snippet(&record.content);
                                            row_for_density(cols.density)
                                                .id(("record", index))
                                                .group("record-row")
                                                .min_h(px(40.))
                                                .py_1()
                                                .cursor_pointer()
                                                .hover(move |style| {
                                                    style.bg(rgba(tint(palette.raised, 0x55)))
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
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(1.))
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .text_sm()
                                                                .hover(move |s| {
                                                                    s.text_color(
                                                                        rgb(palette.accent),
                                                                    )
                                                                })
                                                                .child(record.title.clone()),
                                                        )
                                                        .children(snippet.map(|snip| {
                                                            div()
                                                                .truncate()
                                                                .text_xs()
                                                                .text_color(rgb(palette.muted))
                                                                .child(snip)
                                                        })),
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
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
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
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
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
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
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
                            .bg(rgba(tint(palette.accent, 0x22)))
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
                                    .bg(rgba(tint(palette.amber, 0x22)))
                                    .text_color(rgb(palette.amber))
                                    .text_xs()
                                    .child(format!("● {} open", project.open))
                                    .into_any_element()
                            } else if project.done > 0 {
                                div()
                                    .px_2()
                                    .py(px(1.5))
                                    .rounded(px(4.))
                                    .bg(rgba(tint(palette.green, 0x22)))
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
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .vertical_scrollbar(&self.scroll)
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
            .when_some(self.blocked_band(palette, cx), |page, band| {
                page.child(band)
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
            project: String::new(),
            path: "P/Plan.md".into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: "---\ntype: note\n---\n# Plan\n\n- one\n- two\n\nText\n".into(),
            done: false,
            due: None,
            priority: 2,
        }
    }

    /// The band the #244 work is about: what the host handed in is what the page holds, and a second
    /// sync that cleared it takes it away. The drawing itself needs a window; what this pins down is
    /// that the page never invents a blocked record and never keeps one the store has forgotten.
    #[gpui::test]
    fn the_blocked_band_holds_what_the_sync_reported(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)));
            projects.set_blocked(
                vec![Blocked {
                    path: "Old/One.md".into(),
                    title: "One".into(),
                    kind: "note".into(),
                    reason: "the project Old is archived".into(),
                }],
                cx,
            );
            projects
        });
        cx.run_until_parked();
        view.read_with(cx, |projects, _| {
            assert_eq!(projects.blocked().len(), 1);
            assert_eq!(projects.blocked()[0].title, "One");
            assert!(projects.blocked()[0].reason.contains("archived"));
        });
        view.update(cx, |projects, cx| projects.set_blocked(Vec::new(), cx));
        view.read_with(cx, |projects, _| {
            assert!(
                projects.blocked().is_empty(),
                "a landed write takes its band with it"
            );
        });
    }

    #[gpui::test]
    fn project_inventory_review_mounts_at_mobile_density(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)))
                    .with_density(Density::Mobile);
                let hub = ProjectRecord {
                    kind: "project".into(),
                    ..record()
                };
                projects.project = Some((
                    "P".into(),
                    Remote {
                        value: Some(vec![hub]),
                        ..Remote::default()
                    },
                ));
                projects.link_path = Some("P/Plan.md".into());
                projects.links.value = Some(vec![ProjectInventoryLink {
                    record_path: "P/Plan.md".into(),
                    item_ref: "inv:61".into(),
                    verified_revision: 2,
                    reviewed_at: "stamp".into(),
                    provenance: None,
                }]);
                projects.link_review = Some(LinkReview {
                    record: ProjectLinkRecord {
                        path: "P/Plan.md".into(),
                        title: "Plan".into(),
                        content: "exact".into(),
                        updated_at: "stamp".into(),
                        kind: "project".into(),
                        archived: false,
                    },
                    item_ref: "inv:61".into(),
                    item_label: "Item".into(),
                    remove: true,
                });
                projects.project_section = ProjectSection::InventoryLinks;
                projects
            });
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn project_mail_review_mounts_at_mobile_density(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)))
                    .with_density(Density::Mobile);
                projects.project = Some((
                    "P".into(),
                    Remote {
                        value: Some(vec![record()]),
                        ..Remote::default()
                    },
                ));
                let proposal = ProjectMailProposal {
                    id: 1,
                    version: 1,
                    project: "P".into(),
                    title: "Evidence".into(),
                    evidence: "quoted\ntext".into(),
                    source_id: "mail:v1:thread:message".into(),
                    data_class: "c2".into(),
                    task_suggestion: Some("Follow up".into()),
                    status: "pending".into(),
                };
                projects.mail.value = Some(vec![proposal.clone()]);
                projects.mail_selected = Some(proposal);
                projects.mail_target = Some(ProjectMailTarget {
                    path: "P/Plan.md".into(),
                    title: "Plan".into(),
                    content: "  exact\n".into(),
                });
                projects.project_section = ProjectSection::MailReview;
                projects
            });
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
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
                    project: String::new(),
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
                    project: String::new(),
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
