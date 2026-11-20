//! The Projects page: pick a project, view tasks and notes, edit them in place.

use std::rc::Rc;

use crate::ely::data_display::Tone;
use crate::ely::feedback::Toast;
use crate::ely::primitives::Severity;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use crate::feedback::WindowToastExt;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, MouseButton, ScrollHandle, SharedString, Subscription, Window,
};

use crate::data::{self, Remote, Repo};
use crate::editor::{self, EditorEvent, NoteEditor};
use crate::primitives::{back_button, note, placeholder, section, sub_tabs};
use crate::tasks::{self, NoteSort, StatusFilter, TaskFilter, TaskSort};
use crate::theme::{tint, Density, Palette, ThemeContext};
use sjel_gpui_core::data::Blocked;
use sjel_gpui_core::shell::{
    self, GlobalNote, GlobalTask, ProjectInventoryLink, ProjectLinkRecord, ProjectMailProposal,
    ProjectMailTarget, ProjectRecord, ProjectSummary, ShellError,
};

pub mod actions;
mod board;
mod charts;
mod links;
mod mail;
mod review;
mod tables;
mod task_panel;

pub use actions::bind_keys;
pub(super) use task_panel::TaskPanel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskViewMode {
    Table,
    Board,
}

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

/// A column the Projects hub table sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectSort {
    Name,
    Tasks,
    Progress,
    Notes,
}

impl ProjectSort {
    pub fn from_col_id(id: &str) -> Option<Self> {
        match id {
            "name" => Some(Self::Name),
            "project_tasks" => Some(Self::Tasks),
            "project_progress" => Some(Self::Progress),
            "project_notes" => Some(Self::Notes),
            _ => None,
        }
    }
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
    pub(super) status: Option<f32>,
    pub(super) kind: Option<f32>,
    pub(super) search: Option<f32>,
    pub(super) project_tasks: f32,
    pub(super) project_progress: f32,
    pub(super) project_notes: f32,
    /// The height and touch target a table row gets.
    pub(super) density: Density,
}

impl Columns {
    pub(super) fn for_density(density: Density) -> Self {
        match density {
            Density::Mobile => Self {
                due: 76.,
                priority: 48.,
                project: None,
                status: None,
                kind: None,
                search: None,
                project_tasks: 80.,
                project_progress: 0.,
                project_notes: 60.,
                density,
            },
            _ => Self {
                due: 120.,
                priority: 84.,
                project: Some(140.),
                status: Some(100.),
                kind: Some(80.),
                search: Some(220.),
                project_tasks: 110.,
                project_progress: 140.,
                project_notes: 85.,
                density,
            },
        }
    }
}

/// The search field's box: a fixed width on the Mac, the rest of the row on a phone.
pub(super) fn search_box(width: Option<f32>) -> Div {
    match width {
        Some(width) => div().w(px(width)),
        None => div().flex_1(),
    }
}

/// Extracts unchecked markdown checklist items (`- [ ] ...` or `* [ ] ...`).
fn extract_open_checklist_items(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed
                .strip_prefix("- [ ] ")
                .or_else(|| trimmed.strip_prefix("* [ ] "))?;
            let clean = rest.trim();
            (!clean.is_empty()).then(|| clean.to_string())
        })
        .collect()
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

/// Question-driven focus filter for the cross-project task table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFocusFilter {
    All,
    NeedsMeToday,
    Unblockers,
    BlockedOnOthers,
    Drifting,
}

impl TaskFocusFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::NeedsMeToday,
        Self::Unblockers,
        Self::BlockedOnOthers,
        Self::Drifting,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All focus",
            Self::NeedsMeToday => "Needs me today",
            Self::Unblockers => "Unblockers",
            Self::BlockedOnOthers => "Blocked on others",
            Self::Drifting => "Drifting",
        }
    }

    pub fn matches(self, task: &GlobalTask, all_tasks: &[GlobalTask], today: &str) -> bool {
        let status = task.status();
        let active = status != "done" && status != "cancelled";
        match self {
            Self::All => true,
            Self::NeedsMeToday => {
                active
                    && task
                        .due
                        .as_deref()
                        .is_some_and(|d| !d.is_empty() && d <= today)
            }
            Self::Unblockers => active && !unblocked_by_task(task, all_tasks).is_empty(),
            Self::BlockedOnOthers => {
                active
                    && (status == "waiting"
                        || status == "blocked"
                        || task
                            .fields
                            .get("blocked_by")
                            .is_some_and(|v| !v.trim().is_empty()))
            }
            Self::Drifting => {
                active
                    && task
                        .due
                        .as_deref()
                        .is_some_and(|d| !d.is_empty() && d < today)
            }
        }
    }
}

/// Active downstream tasks whose `blocked_by` references `task`.
pub(super) fn unblocked_by_task<'a>(
    task: &GlobalTask,
    all_tasks: &'a [GlobalTask],
) -> Vec<&'a GlobalTask> {
    let needle_id = task.id.trim().to_lowercase();
    let needle_title = task.title.trim().to_lowercase();
    all_tasks
        .iter()
        .filter(|other| {
            if other.id == task.id {
                return false;
            }
            let status = other.status();
            if status == "done" || status == "cancelled" {
                return false;
            }
            let Some(raw) = other.fields.get("blocked_by") else {
                return false;
            };
            raw.split(',').any(|part| {
                let token = part.trim().to_lowercase();
                !token.is_empty()
                    && (token == needle_id || (!needle_title.is_empty() && token == needle_title))
            })
        })
        .collect()
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
    pub task_view_mode: TaskViewMode,
    pub task_filter: TaskFilter,
    /// The statuses the cross-project table shows.
    pub global_status: StatusFilter,
    /// Question-driven focus filter on the cross-project task table.
    pub task_focus: TaskFocusFilter,
    /// Optional `YYYY-MM-DD` due-date filter set from charts or review bars.
    pub due_date_filter: Option<String>,
    /// The cross-project table's sort column, and whether it ascends.
    pub task_sort: (TaskSort, bool),
    /// The cross-project notes table's sort column, and whether it ascends.
    pub note_sort: (NoteSort, bool),
    /// The projects hub table's sort column, and whether it ascends.
    pub project_sort: (ProjectSort, bool),
    pub group_by_project: bool,
    /// The task the side panel shows, with its text fields.
    pub(super) task_panel: Option<TaskPanel>,
    /// The task whose status chips stand open in its row.
    pub(super) status_edit: Option<String>,
    pub note_kind_filter: NoteKindFilter,
    pub show_archived: bool,
    pub review_mode: bool,
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
    /// The task whose due date is being typed (its path), and its field.
    pub(super) due_edit: Option<(String, Entity<InputState>, Subscription)>,
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
    pub focus: FocusHandle,
    pub selected_index: Option<usize>,
    pub selected_task_index: Option<usize>,
    pub selected_task_path: Option<String>,
    pub(super) task_title_edit: Option<(String, String, Entity<InputState>, Subscription)>,
}

impl gpui_kit::EventEmitter<ProjectsEvent> for Projects {}

impl Projects {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            density: Density::Desktop,
            palette: None,
            projects: Remote::default(),
            all_tasks: Remote::default(),
            all_notes: Remote::default(),
            tab: ProjectsTab::Hubs,
            project_section: ProjectSection::Note,
            task_view_mode: TaskViewMode::Table,
            task_filter: TaskFilter::Open,
            global_status: StatusFilter::Active,
            task_focus: TaskFocusFilter::All,
            due_date_filter: None,
            task_sort: (TaskSort::Default, true),
            note_sort: (NoteSort::Default, true),
            project_sort: (ProjectSort::Name, true),
            group_by_project: false,
            task_panel: None,
            status_edit: None,
            note_kind_filter: NoteKindFilter::All,
            show_archived: false,
            review_mode: false,
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
            focus: cx.focus_handle(),
            selected_index: None,
            selected_task_index: None,
            selected_task_path: None,
            task_title_edit: None,
        }
    }

    pub fn active_project_name(&self) -> Option<String> {
        self.project.as_ref().map(|(name, _)| name.clone())
    }

    pub fn set_project_section(&mut self, section: ProjectSection, cx: &mut Context<Self>) {
        if self.project_section != section {
            self.put_away_editor(cx);
            self.project_section = section;
            if section == ProjectSection::MailReview {
                self.load_project_mail(false, cx);
            }
            cx.notify();
        }
    }

    pub fn close_project(&mut self, cx: &mut Context<Self>) {
        self.put_away_editor(cx);
        self.task_panel = None;
        self.status_edit = None;
        self.due_edit = None;
        self.selected_task_index = None;
        self.selected_task_path = None;
        self.task_title_edit = None;
        self.project = None;
        cx.notify();
    }

    fn on_close_project(
        &mut self,
        _: &actions::CloseProject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.task_title_edit.is_some() {
            self.cancel_task_title(cx);
        } else if self.task_panel.is_some() {
            self.task_panel = None;
            cx.notify();
        } else if self.status_edit.is_some() {
            self.status_edit = None;
            cx.notify();
        } else if self.due_edit.is_some() {
            self.due_edit = None;
            cx.notify();
        } else if self.editor.is_some() && self.project_section == ProjectSection::Notes {
            self.put_away_editor(cx);
            cx.notify();
        } else if self.project.is_some() {
            self.close_project(cx);
        } else if self.tab != ProjectsTab::Hubs {
            self.tab = ProjectsTab::Hubs;
            cx.notify();
        } else if self.adding_project {
            self.adding_project = false;
            cx.notify();
        }
    }

    fn on_select_tab_1(
        &mut self,
        _: &actions::SelectTab1,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() {
            if self.project_section != ProjectSection::Note {
                self.put_away_editor(cx);
                self.project_section = ProjectSection::Note;
                cx.notify();
            }
        } else if self.tab != ProjectsTab::Hubs {
            self.tab = ProjectsTab::Hubs;
            cx.notify();
        }
    }

    fn on_select_tab_2(
        &mut self,
        _: &actions::SelectTab2,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() {
            if self.project_section != ProjectSection::Tasks {
                self.put_away_editor(cx);
                self.project_section = ProjectSection::Tasks;
                cx.notify();
            }
        } else if self.tab != ProjectsTab::AllTasks {
            self.tab = ProjectsTab::AllTasks;
            cx.notify();
        }
    }

    fn on_select_tab_3(
        &mut self,
        _: &actions::SelectTab3,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() {
            if self.project_section != ProjectSection::Notes {
                self.put_away_editor(cx);
                self.project_section = ProjectSection::Notes;
                cx.notify();
            }
        } else if self.tab != ProjectsTab::AllNotes {
            self.tab = ProjectsTab::AllNotes;
            cx.notify();
        }
    }

    fn on_select_tab_4(
        &mut self,
        _: &actions::SelectTab4,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section != ProjectSection::MailReview {
            self.put_away_editor(cx);
            self.project_section = ProjectSection::MailReview;
            self.load_project_mail(false, cx);
            cx.notify();
        }
    }

    fn on_select_tab_5(
        &mut self,
        _: &actions::SelectTab5,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section != ProjectSection::InventoryLinks {
            self.put_away_editor(cx);
            self.project_section = ProjectSection::InventoryLinks;
            cx.notify();
        }
    }

    fn on_toggle_task_view_mode(
        &mut self,
        _: &actions::ToggleTaskViewMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section == ProjectSection::Tasks {
            self.task_view_mode = match self.task_view_mode {
                TaskViewMode::Table => TaskViewMode::Board,
                TaskViewMode::Board => TaskViewMode::Table,
            };
            cx.notify();
        }
    }

    fn on_cycle_task_filter(
        &mut self,
        _: &actions::CycleTaskFilter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section == ProjectSection::Tasks {
            self.task_filter = match self.task_filter {
                TaskFilter::Open => TaskFilter::All,
                TaskFilter::All => TaskFilter::Done,
                TaskFilter::Done => TaskFilter::Open,
            };
            cx.notify();
        }
    }

    fn on_new_item(&mut self, _: &actions::NewItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_some() {
            match self.project_section {
                ProjectSection::Tasks => {
                    if let Some((input, _)) = &self.new_task {
                        let handle = input.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                }
                ProjectSection::Notes => {
                    self.adding_note = true;
                    if let Some((input, _)) = &self.new_note {
                        let handle = input.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                    cx.notify();
                }
                _ => {}
            }
        } else if self.tab == ProjectsTab::Hubs {
            self.adding_project = true;
            if let Some((input, _)) = &self.new_project {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            cx.notify();
        }
    }

    fn on_focus_search(
        &mut self,
        _: &actions::FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab == ProjectsTab::AllTasks {
            if let Some((input, _)) = &self.search_input {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        } else if self.tab == ProjectsTab::AllNotes {
            if let Some((input, _)) = &self.notes_search_input {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
    }

    pub fn filtered_projects(&self) -> Vec<ProjectSummary> {
        let Some(projects) = &self.projects.value else {
            return Vec::new();
        };
        let query = self.search_query.as_str();
        let (sort_col, sort_asc) = self.project_sort;
        let mut filtered: Vec<_> = projects
            .iter()
            .filter(|p| p.archived == self.show_archived)
            .filter(|p| query.is_empty() || p.name.to_lowercase().contains(query))
            .cloned()
            .collect();

        filtered.sort_by(|a, b| {
            let order = match sort_col {
                ProjectSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                ProjectSort::Tasks => (a.open, a.open + a.done).cmp(&(b.open, b.open + b.done)),
                ProjectSort::Progress => {
                    let p_a = if a.open + a.done > 0 {
                        a.done as f32 / (a.open + a.done) as f32
                    } else {
                        -1.0
                    };
                    let p_b = if b.open + b.done > 0 {
                        b.done as f32 / (b.open + b.done) as f32
                    } else {
                        -1.0
                    };
                    p_a.partial_cmp(&p_b).unwrap_or(std::cmp::Ordering::Equal)
                }
                ProjectSort::Notes => a.notes.cmp(&b.notes),
            };
            if sort_asc {
                order
            } else {
                order.reverse()
            }
        });
        filtered
    }

    pub fn current_project_tasks(&self) -> Vec<ProjectRecord> {
        let Some((_, remote)) = &self.project else {
            return Vec::new();
        };
        let Some(records) = &remote.value else {
            return Vec::new();
        };
        tasks::ordered(records)
            .into_iter()
            .filter(|t| self.task_filter.matches(t.done))
            .cloned()
            .collect()
    }

    pub fn filtered_all_tasks(&self) -> Vec<GlobalTask> {
        let Some(tasks) = &self.all_tasks.value else {
            return Vec::new();
        };
        let query = self.search_query.as_str();
        let (sort, ascending) = self.task_sort;
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
        filtered.into_iter().cloned().collect()
    }

    fn on_select_next(
        &mut self,
        _: &actions::SelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_none() && self.tab == ProjectsTab::Hubs {
            let projects = self.filtered_projects();
            if !projects.is_empty() {
                self.selected_index = Some(match self.selected_index {
                    Some(i) => (i + 1).min(projects.len().saturating_sub(1)),
                    None => 0,
                });
                cx.notify();
            }
        }
    }

    fn on_select_prev(
        &mut self,
        _: &actions::SelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_none() && self.tab == ProjectsTab::Hubs {
            let projects = self.filtered_projects();
            if !projects.is_empty() {
                self.selected_index = Some(match self.selected_index {
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                });
                cx.notify();
            }
        }
    }

    fn on_activate_selected(
        &mut self,
        _: &actions::ActivateSelected,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_none() && self.tab == ProjectsTab::Hubs {
            let projects = self.filtered_projects();
            if let Some(i) = self.selected_index {
                if let Some(p) = projects.get(i) {
                    self.open_project(p.name.clone(), cx);
                }
            }
        }
    }

    fn selected_project_task(&self) -> Option<ProjectRecord> {
        let tasks = self.current_project_tasks();
        if let Some(path) = &self.selected_task_path {
            if let Some(found) = tasks.iter().find(|t| t.path == *path) {
                return Some(found.clone());
            }
        }
        self.selected_task_index.and_then(|i| tasks.get(i).cloned())
    }

    fn selected_global_task(&self) -> Option<GlobalTask> {
        let tasks = self.filtered_all_tasks();
        if let Some(path) = &self.selected_task_path {
            if let Some(found) = tasks.iter().find(|t| t.id == *path) {
                return Some((*found).clone());
            }
        }
        self.selected_task_index
            .and_then(|i| tasks.get(i).map(|t| (*t).clone()))
    }

    pub(super) fn open_task_by_path(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() {
            if let Some(task) = self
                .current_project_tasks()
                .into_iter()
                .find(|t| t.path == path)
            {
                let global = tables::project_record_to_global_task(&task);
                self.select_task(&global, window, cx);
            }
        } else if let Some(task) = self.filtered_all_tasks().into_iter().find(|t| t.id == path) {
            let owned = task.clone();
            self.select_task(&owned, window, cx);
        }
    }

    fn on_toggle_selected(
        &mut self,
        _: &actions::ToggleSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() {
            if self.project_section == ProjectSection::Tasks {
                if let Some(task) = self.selected_project_task() {
                    self.toggle_task(task.path, !task.done, window, cx);
                }
            }
        } else if self.tab == ProjectsTab::AllTasks {
            if let Some(task) = self.selected_global_task() {
                self.toggle_global_task(task.id, !task.done, window, cx);
            }
        }
    }

    fn on_edit_task_title_action(
        &mut self,
        _: &actions::EditTaskTitle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section == ProjectSection::Tasks {
            if let Some(task) = self.selected_project_task() {
                self.edit_task_title(task.path, task.title, window, cx);
            }
        }
    }

    fn on_move_task_next_stage(
        &mut self,
        _: &actions::MoveTaskNextStage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section == ProjectSection::Tasks {
            if let Some(task) = self.selected_project_task() {
                let status = tables::parse_frontmatter_status(&task.content, task.done);
                let next = match status.as_str() {
                    "open" | "" => "doing",
                    "doing" => "done",
                    "waiting" | "blocked" => "doing",
                    "done" | "cancelled" => "open",
                    _ => "doing",
                };
                self.set_task_property(task.path, "status", next.to_string(), window, cx);
            }
        }
    }

    fn on_move_task_prev_stage(
        &mut self,
        _: &actions::MoveTaskPrevStage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.is_some() && self.project_section == ProjectSection::Tasks {
            if let Some(task) = self.selected_project_task() {
                let status = tables::parse_frontmatter_status(&task.content, task.done);
                let prev = match status.as_str() {
                    "done" | "cancelled" => "doing",
                    "doing" => "open",
                    "waiting" | "blocked" => "open",
                    _ => "open",
                };
                self.set_task_property(task.path, "status", prev.to_string(), window, cx);
            }
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
        self.selected_task_index = None;
        self.selected_task_path = None;
        self.task_title_edit = None;
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
    /// A title is the record's file name (`crates/projects`), so renaming moves the file.
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
            if editor.renaming() {
                return;
            }
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
                        if view.editor.as_ref() == Some(&editor) {
                            view.open_note(renamed.path, true, window, cx);
                        }
                        let mut done = format!("Renamed to {}", renamed.title);
                        if links > 0 {
                            done.push_str(&format!(" · {links} link(s) moved"));
                        }
                        if left > 0 {
                            done.push_str(&format!(" · {left} still name the old title"));
                        }
                        window.push_toast(Toast::new(done).severity(Severity::Success), cx);
                    }
                    Err(error) => {
                        editor.update(cx, |editor, cx| editor.rename_failed(cx));
                        window.push_toast(
                            Toast::new(error.to_string()).severity(Severity::Danger),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_note(
        &mut self,
        path: String,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                window.push_toast(Toast::new(error.to_string()).severity(Severity::Danger), cx);
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
                        window.push_toast(
                            Toast::new(error.to_string()).severity(Severity::Danger),
                            cx,
                        )
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
        cx.subscribe_in(&editor, window, |projects, source, event, window, cx| {
            if projects.editor.as_ref() != Some(source) {
                return;
            }
            match event {
                EditorEvent::Close => projects.close_note(cx),
                EditorEvent::Rename(title) => {
                    projects.rename_open_note(title.to_string(), window, cx)
                }
            }
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
                row.fields = task.fields;
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

    pub(super) fn remove_record(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// Writes one frontmatter field of the task at `path`. An empty value clears the field.
    pub(super) fn set_task_property(
        &mut self,
        path: String,
        key: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn edit_due(
        &mut self,
        path: String,
        due: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let due = due.unwrap_or_default();
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
                    let Some((path, _, _)) = view.due_edit.take() else {
                        return;
                    };
                    if tasks::valid_due(&due) {
                        view.set_task_property(path, "due", due, window, cx);
                    } else {
                        window.push_toast(
                            Toast::new("A due date is YYYY-MM-DD, or empty for none.")
                                .severity(Severity::Danger),
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
        self.due_edit = Some((path, input, subscription));
        cx.notify();
    }

    pub(super) fn edit_task_title(
        &mut self,
        path: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .task_title_edit
            .as_ref()
            .is_some_and(|(p, ..)| *p == path)
        {
            return;
        }
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Task title… (Press ↩ or Esc)");
            state.set_value(title.clone(), window, cx);
            state
        });
        let subscription =
            cx.subscribe_in(&input, window, |view, _, event, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => view.commit_task_title(window, cx),
                InputEvent::Blur => view.cancel_task_title(cx),
                _ => {}
            });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.task_title_edit = Some((path, title, input, subscription));
        cx.notify();
    }

    pub(super) fn commit_task_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, was, input, _)) = self.task_title_edit.take() else {
            return;
        };
        cx.notify();
        let title = input.read(cx).value().trim().to_owned();
        if title.is_empty() || title == was {
            return;
        }
        self.rename_task(path, title, window, cx);
    }

    pub(super) fn cancel_task_title(&mut self, cx: &mut Context<Self>) {
        if self.task_title_edit.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn rename_task(
        &mut self,
        path: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let was = path.clone();
        let work = shell::run(async move { repo.rename_record(&path, &title).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                match result {
                    Ok(renamed) => {
                        view.refresh_records(cx);
                        view.load_all_tasks(cx);
                        if let Some(panel) = &mut view.task_panel {
                            if panel.path == was {
                                panel.path = renamed.path;
                            }
                        }
                        window.push_toast(
                            Toast::new(format!("Renamed task to {}", renamed.title))
                                .severity(Severity::Success),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_toast(
                            Toast::new(error.to_string()).severity(Severity::Danger),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn promote_checklist_strip(
        &self,
        project: &str,
        editor: &Entity<NoteEditor>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let items = extract_open_checklist_items(&editor.read(cx).base);
        if items.is_empty() {
            return None;
        }
        let buttons: Vec<AnyElement> = items
            .into_iter()
            .take(5)
            .enumerate()
            .map(|(ix, item)| {
                let project_name = project.to_string();
                let task_text = item.clone();
                crate::ely::buttons::Button::new(
                    SharedString::from(format!("promote-checklist-{ix}")),
                    format!("+ {item}"),
                )
                .size(crate::ely::theme::ControlSize::Sm)
                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.create_task_from_text(project_name.clone(), task_text.clone(), window, cx);
                }))
                .into_any_element()
            })
            .collect();

        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Promote to task:"),
                )
                .children(buttons)
                .into_any_element(),
        )
    }

    pub(super) fn create_task_from_text(
        &mut self,
        project: String,
        raw_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if raw_title.trim().is_empty() {
            return;
        }
        let today = chrono::Local::now().date_naive();
        let (title, due, priority) = tasks::parse_smart_task_input(raw_title.trim(), today);
        if title.is_empty() {
            return;
        }
        let repo = self.repo.clone();
        data::write_repo(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Task added."),
            move |s| async move {
                s.add_task_with_details(&project, &title, due.as_deref(), priority)
                    .await
            },
            |this, _, cx| {
                this.refresh_records(cx);
                this.load_projects(cx);
            },
        );
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
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.create_task_from_text(project, raw_title, window, cx);
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
        self.task_title_edit = None;
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

    fn database_toolbar(
        &self,
        records: &[ProjectRecord],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_count = records
            .iter()
            .filter(|r| r.kind == "task" && !r.done)
            .count();
        let all_count = records.iter().filter(|r| r.kind == "task").count();
        let done_count = all_count.saturating_sub(open_count);

        let filter_btn = |id: &'static str,
                          label: &'static str,
                          count: usize,
                          target_filter: TaskFilter,
                          cx: &mut Context<Self>| {
            let active = self.task_filter == target_filter;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_1p5()
                .px_2p5()
                .py(px(3.))
                .rounded(px(5.))
                .text_xs()
                .cursor_pointer()
                .when(active, |el| {
                    el.bg(rgba(palette.surface))
                        .text_color(rgb(palette.text))
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow_sm()
                })
                .when(!active, |el| {
                    el.text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .on_click(cx.listener(move |view: &mut Self, _, _, cx| {
                    view.task_filter = target_filter;
                    cx.notify();
                }))
                .child(label)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(if active { palette.text } else { palette.muted }))
                        .child(format!("({count})")),
                )
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .mb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .p(px(2.))
                            .rounded(px(6.))
                            .bg(rgba(palette.raised))
                            .child(
                                div()
                                    .id("view-mode-table")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(2.5))
                                    .rounded(px(4.))
                                    .text_xs()
                                    .cursor_pointer()
                                    .when(self.task_view_mode == TaskViewMode::Table, |el| {
                                        el.bg(rgba(palette.surface))
                                            .text_color(rgb(palette.text))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .shadow_sm()
                                    })
                                    .when(self.task_view_mode != TaskViewMode::Table, |el| {
                                        el.text_color(rgb(palette.muted))
                                            .hover(|s| s.text_color(rgb(palette.text)))
                                    })
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.task_view_mode = TaskViewMode::Table;
                                        cx.notify();
                                    }))
                                    .child("Table ☷"),
                            )
                            .child(
                                div()
                                    .id("view-mode-board")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py(px(2.5))
                                    .rounded(px(4.))
                                    .text_xs()
                                    .cursor_pointer()
                                    .when(self.task_view_mode == TaskViewMode::Board, |el| {
                                        el.bg(rgba(palette.surface))
                                            .text_color(rgb(palette.text))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .shadow_sm()
                                    })
                                    .when(self.task_view_mode != TaskViewMode::Board, |el| {
                                        el.text_color(rgb(palette.muted))
                                            .hover(|s| s.text_color(rgb(palette.text)))
                                    })
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.task_view_mode = TaskViewMode::Board;
                                        cx.notify();
                                    }))
                                    .child("Board ⊞")
                                    .child(
                                        div()
                                            .px_1()
                                            .py(px(0.5))
                                            .rounded(px(3.))
                                            .bg(rgba(palette.border))
                                            .text_size(px(9.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child("B"),
                                    ),
                            ),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(rgba(palette.border)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(filter_btn(
                                "filter-open",
                                "Open",
                                open_count,
                                TaskFilter::Open,
                                cx,
                            ))
                            .child(filter_btn(
                                "filter-all",
                                "All",
                                all_count,
                                TaskFilter::All,
                                cx,
                            ))
                            .child(filter_btn(
                                "filter-done",
                                "Done",
                                done_count,
                                TaskFilter::Done,
                                cx,
                            ))
                            .child(
                                div()
                                    .px_1()
                                    .py(px(0.5))
                                    .rounded(px(3.))
                                    .bg(rgba(palette.border))
                                    .text_size(px(9.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.muted))
                                    .child("F"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("db-add-task-btn")
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .hover(move |s| s.text_color(rgb(palette.text)))
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.on_new_item(&actions::NewItem, window, cx);
                    }))
                    .child("+ New Task")
                    .child(
                        div()
                            .px_1()
                            .py(px(0.5))
                            .rounded(px(3.))
                            .bg(rgba(palette.border))
                            .text_size(px(9.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("N"),
                    ),
            )
            .into_any_element()
    }
}

impl Projects {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cols = self.columns();
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        // 1. Note editor open with NO project selected (e.g. opened from All Notes)
        if self.project.is_none() {
            if let Some(editor) = &self.editor {
                let title = editor.read(cx).title.clone();
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(840.))
                    .mx_auto()
                    .pt_2()
                    .pb_6()
                    .child(
                        div()
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

            let sec_entity = cx.entity();

            // Project header bar with breadcrumb and section tabs
            let project_header = div()
                .flex()
                .items_center()
                .justify_between()
                .when(self.density == Density::Mobile, |el| {
                    el.flex_col().items_start().gap_2()
                })
                .mb_2()
                .child(
                    div()
                        .id("project-breadcrumb-host")
                        .debug_selector(|| "project-breadcrumb-host".to_owned())
                        .child({
                            let owner = cx.entity();
                            crate::ely::navigation::Breadcrumb::new(
                                "project-breadcrumb",
                                [
                                    crate::ely::navigation::Crumb::new("projects", "Projects"),
                                    crate::ely::navigation::Crumb::new(
                                        "current",
                                        project_name.clone(),
                                    ),
                                ],
                            )
                            .on_select(move |level, _, _, cx| {
                                if level == 0 {
                                    owner.update(cx, |view, cx| view.close_project(cx));
                                }
                            })
                        }),
                )
                .child(sub_tabs(
                    "project-section-tabs",
                    vec![
                        (
                            "Project Note",
                            None,
                            self.project_section == ProjectSection::Note,
                        ),
                        (
                            "Tasks",
                            Some(open_tasks),
                            self.project_section == ProjectSection::Tasks,
                        ),
                        (
                            "Notes",
                            Some(supporting_notes_count),
                            self.project_section == ProjectSection::Notes,
                        ),
                        (
                            "Mail review",
                            None,
                            self.project_section == ProjectSection::MailReview,
                        ),
                        (
                            "Inventory links",
                            None,
                            self.project_section == ProjectSection::InventoryLinks,
                        ),
                    ],
                    move |ix, _, cx| {
                        sec_entity.update(cx, |view, cx| {
                            let target_sec = match ix {
                                1 => ProjectSection::Tasks,
                                2 => ProjectSection::Notes,
                                3 => ProjectSection::MailReview,
                                4 => ProjectSection::InventoryLinks,
                                _ => ProjectSection::Note,
                            };
                            if view.project_section != target_sec {
                                view.put_away_editor(cx);
                                view.project_section = target_sec;
                                if target_sec == ProjectSection::MailReview {
                                    view.load_project_mail(false, cx);
                                }
                                cx.notify();
                            }
                        });
                    },
                ));

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
                            .w_full()
                            .max_w(px(840.))
                            .mx_auto()
                            .pt_2()
                            .pb_6()
                            .child(
                                div()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(self.note_title(editor, palette, cx))
                                    .when_some(
                                        self.promote_checklist_strip(
                                            &project_name,
                                            editor,
                                            palette,
                                            cx,
                                        ),
                                        |el, strip| el.child(strip),
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
                        let toolbar = self.database_toolbar(records_list, palette, cx);
                        let content = match self.task_view_mode {
                            TaskViewMode::Table => {
                                self.task_table(records_list, &new_task, palette, cx)
                            }
                            TaskViewMode::Board => {
                                self.task_board(records_list, &new_task, palette, cx)
                            }
                        };

                        let selected = self.task_panel.as_ref().and_then(|panel| {
                            records_list
                                .iter()
                                .find(|t| t.path == panel.path)
                                .map(|rec| (panel, tables::project_record_to_global_task(rec)))
                        });

                        let inner = match selected {
                            Some((panel, task)) if self.density == Density::Mobile => {
                                self.task_panel_view(panel, &task, palette, cx)
                            }
                            Some((panel, task)) => div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(div().flex_1().min_w_0().child(content))
                                .child(self.task_panel_view(panel, &task, palette, cx))
                                .into_any_element(),
                            None => content,
                        };

                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .size_full()
                            .child(toolbar)
                            .child(inner)
                            .into_any_element()
                    }
                },
                ProjectSection::Notes => {
                    if let Some(editor) = &self.editor {
                        let active_path = editor.read(cx).path.clone();
                        let hub_path = format!("{project_name}/{project_name}.md");
                        let rail = (self.density != Density::Mobile)
                            .then(|| {
                                records.value.as_deref().map(|list| {
                                    let items: Vec<AnyElement> = list
                                        .iter()
                                        .filter(|rec| {
                                            rec.kind != "task"
                                                && rec.kind != "project"
                                                && rec.path != hub_path
                                        })
                                        .map(|rec| {
                                            let path = rec.path.clone();
                                            let is_current = path == active_path;
                                            div()
                                                .id(SharedString::from(format!(
                                                    "notes-rail-{}",
                                                    rec.path
                                                )))
                                                .px_2()
                                                .py_1p5()
                                                .rounded(px(6.))
                                                .cursor_pointer()
                                                .text_xs()
                                                .truncate()
                                                .when(is_current, |el| {
                                                    el.bg(rgba(tint(palette.accent, 0x26)))
                                                        .text_color(rgb(palette.accent))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                })
                                                .when(!is_current, |el| {
                                                    el.text_color(rgb(palette.text))
                                                        .hover(move |s| s.bg(rgba(palette.raised)))
                                                })
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        view.open_note(
                                                            path.clone(),
                                                            false,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                ))
                                                .child(rec.title.clone())
                                                .into_any_element()
                                        })
                                        .collect();
                                    div()
                                        .w(px(220.))
                                        .flex_none()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .pr_3()
                                        .border_r_1()
                                        .border_color(rgba(palette.border))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(palette.muted))
                                                .mb_1()
                                                .child("NOTES"),
                                        )
                                        .children(items)
                                })
                            })
                            .flatten();

                        let editor_pane = div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
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
                                        div().text_xs().text_color(rgb(palette.muted)).child("/"),
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
                            .when_some(
                                self.promote_checklist_strip(&project_name, editor, palette, cx),
                                |el, strip| el.child(strip),
                            )
                            .child(div().flex_1().min_h_0().child(editor.clone()));

                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap_4()
                            .w_full()
                            .pt_2()
                            .pb_6()
                            .when_some(rail, |el, r| el.child(r))
                            .child(editor_pane)
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

                                let columns = vec![
                                    Column::new("title", "NOTE").min_width(rems(14.)),
                                    Column::new("preview", "PREVIEW").min_width(rems(16.)),
                                    Column::new("kind", "TYPE").width(rems(6.)),
                                    Column::new("actions", "")
                                        .width(rems(6.))
                                        .actions([RowAction::new("delete", "Delete")]),
                                ];
                                let rows: Rc<Vec<Row>> = Rc::new(
                                    note_records
                                        .iter()
                                        .map(|record| {
                                            let snippet = extract_note_snippet(&record.content)
                                                .unwrap_or_else(|| "—".into());
                                            Row::new(
                                                record.path.clone(),
                                                [
                                                    Cell::Text(record.title.clone().into()),
                                                    Cell::Text(snippet.into()),
                                                    Cell::Tag(
                                                        record.kind.clone().into(),
                                                        Tone::Neutral,
                                                    ),
                                                    Cell::Actions(vec!["delete".into()]),
                                                ],
                                            )
                                        })
                                        .collect(),
                                );
                                let (on_open, on_act) = (cx.entity(), cx.entity());
                                let notes_content =
                                    DataTable::new("project-supporting-notes-table", columns)
                                        .label("Supporting notes")
                                        .rows(rows)
                                        .on_open(move |key, window, cx| {
                                            let path = key.to_string();
                                            on_open.update(cx, |view, cx| {
                                                view.open_note(path, true, window, cx);
                                            });
                                        })
                                        .on_action(move |key, action, window, cx| {
                                            if action.as_ref() == "delete" {
                                                let path = key.to_string();
                                                on_act.update(cx, |view, cx| {
                                                    view.remove_record(path, window, cx);
                                                });
                                            }
                                        })
                                        .empty("No supporting notes yet in this project.")
                                        .into_any_element();

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

            let summary_strip = records.value.as_deref().map(|r| {
                let mut open = 0usize;
                let mut blocked = 0usize;
                let mut done = 0usize;
                let mut next_due: Option<&str> = None;
                for rec in r.iter().filter(|rec| rec.kind == "task") {
                    let status = tables::parse_frontmatter_status(&rec.content, rec.done);
                    if status == "done" || status == "cancelled" {
                        done += 1;
                    } else {
                        open += 1;
                        if status == "blocked" || status == "waiting" {
                            blocked += 1;
                        }
                        if let Some(d) = rec.due.as_deref().filter(|d| !d.is_empty()) {
                            if next_due.is_none_or(|cur| d < cur) {
                                next_due = Some(d);
                            }
                        }
                    }
                }
                let total = open + done;
                let ratio = if total > 0 {
                    done as f32 / total as f32
                } else {
                    0.0
                };
                let mail_count = self.mail.value.as_ref().map(|m| m.len());
                let links_count = self.links.value.as_ref().map(|l| l.len());
                let next_due_label = next_due.map(str::to_string);

                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(tint(palette.raised, 0x55)))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(div().w(px(110.)).child(crate::primitives::progress_bar(
                                ratio,
                                palette.green,
                                palette,
                            )))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.text))
                                    .child(format!("{open} open · {done} done")),
                            )
                            .when(blocked > 0, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.amber))
                                        .child(format!("{blocked} blocked")),
                                )
                            })
                            .when_some(next_due_label, |el, due| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Next due {due}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-touch-mail",
                                    match mail_count {
                                        Some(n) if n > 0 => format!("Mail ({n})"),
                                        _ => "Mail".to_string(),
                                    },
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::MailReview;
                                        view.load_project_mail(false, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-touch-inventory",
                                    match links_count {
                                        Some(n) if n > 0 => format!("Inventory ({n})"),
                                        _ => "Inventory".to_string(),
                                    },
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::InventoryLinks;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-primary-add-task",
                                    "+ Task",
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Primary)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::Tasks;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
            });

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
                .children(summary_strip)
                .child(content)
                .into_any_element();
        }

        if self.review_mode && self.project.is_none() {
            return self.review_view(palette, cx);
        }

        // Top navigation tabs: rendered inside the page only when mounted standalone or on mobile
        // (on Desktop, `header.rs` renders the unified tab strip in the window header).
        let show_inline_tabs = self.density == Density::Mobile || cfg!(test);
        let projects_count = self
            .projects
            .value
            .as_ref()
            .map(|list| list.iter().filter(|p| !p.archived).count())
            .filter(|&c| c > 0);
        let tasks_count = self
            .all_tasks
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let notes_count = self
            .all_notes
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let tab_entity = cx.entity();
        let tab_bar = show_inline_tabs.then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_3()
                .child(sub_tabs(
                    "projects-main-tabs",
                    vec![
                        ("Projects", projects_count, self.tab == ProjectsTab::Hubs),
                        ("All Tasks", tasks_count, self.tab == ProjectsTab::AllTasks),
                        ("All Notes", notes_count, self.tab == ProjectsTab::AllNotes),
                    ],
                    move |ix, _, cx| {
                        tab_entity.update(cx, |view, cx| {
                            view.tab = match ix {
                                1 => ProjectsTab::AllTasks,
                                2 => ProjectsTab::AllNotes,
                                _ => ProjectsTab::Hubs,
                            };
                            cx.notify();
                        });
                    },
                ))
        });

        if self.tab == ProjectsTab::AllNotes {
            let search_input = self.notes_search_input_field(window, cx);
            let body = match (&self.all_notes.value, &self.all_notes.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(notes), None) => self.all_notes_table(notes, &search_input, cx),
            };
            return div()
                .id("all-notes")
                .size_full()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
                .children(tab_bar)
                .child(section("Cross-Project Notes", body, palette))
                .into_any_element();
        }

        if self.tab == ProjectsTab::AllTasks {
            let search_input = self.search_input_field(window, cx);
            let body = match (&self.all_tasks.value, &self.all_tasks.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(tasks), None) => {
                    let pressure = charts::tasks_pressure_charts(tasks, self.density, palette, cx);
                    let table = self.all_tasks_table(tasks, &search_input, palette, cx);
                    let selected = self.task_panel.as_ref().and_then(|panel| {
                        tasks
                            .iter()
                            .find(|t| t.id == panel.path)
                            .map(|task| (panel, task))
                    });
                    let content = match selected {
                        // A phone has no room beside the table: the panel takes its place.
                        Some((panel, task)) if self.density == Density::Mobile => {
                            self.task_panel_view(panel, task, palette, cx)
                        }
                        Some((panel, task)) => div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(div().flex_1().min_w_0().child(table))
                            .child(self.task_panel_view(panel, task, palette, cx))
                            .into_any_element(),
                        None => table,
                    };
                    div()
                        .flex()
                        .flex_col()
                        .when_some(pressure, |el, strip| el.child(strip))
                        .child(content)
                        .into_any_element()
                }
            };
            return div()
                .id("all-tasks")
                .size_full()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
                .children(tab_bar)
                .child(section("Cross-Project Tasks", body, palette))
                .into_any_element();
        }

        // Projects Hub View
        let search_input = self.search_input_field(window, cx);
        let new_project = self.new_project_field(window, cx);

        let archived_count = self
            .projects
            .value
            .as_ref()
            .map(|list| list.iter().filter(|p| p.archived).count())
            .unwrap_or(0);

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
                                "Show Active".to_string()
                            } else if archived_count > 0 {
                                format!("Show Archived ({archived_count})")
                            } else {
                                "Show Archived".to_string()
                            }),
                    )
                    .child(
                        crate::ely::buttons::Button::new("open-review-btn", "Guided Review")
                            .size(crate::ely::theme::ControlSize::Sm)
                            .variant(crate::ely::buttons::ButtonVariant::Ghost)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.review_mode = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("new-project-btn")
                            .px_3()
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_xs()
                            .cursor_pointer()
                            .hover(move |s| s.opacity(0.9))
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
            (Some(_), None) => {
                let filtered = self.filtered_projects();
                let portfolio = charts::hubs_portfolio_charts(&filtered, self.density, palette, cx);
                let table = self.hubs_table(&filtered, palette, cx);
                div()
                    .flex()
                    .flex_col()
                    .when_some(portfolio, |el, charts_row| el.child(charts_row))
                    .child(table)
                    .into_any_element()
            }
        };

        div()
            .id("projects")
            .size_full()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .vertical_scrollbar(&self.scroll)
            .children(tab_bar)
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

impl Render for Projects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let in_ely_table = if self.project.is_some() {
            self.project_section == ProjectSection::Tasks
                && self.task_view_mode == TaskViewMode::Table
        } else {
            matches!(self.tab, ProjectsTab::AllTasks | ProjectsTab::AllNotes)
        };
        let body = self.render_body(window, cx);
        div()
            .key_context("Projects")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close_project))
            .on_action(cx.listener(Self::on_select_tab_1))
            .on_action(cx.listener(Self::on_select_tab_2))
            .on_action(cx.listener(Self::on_select_tab_3))
            .on_action(cx.listener(Self::on_select_tab_4))
            .on_action(cx.listener(Self::on_select_tab_5))
            .on_action(cx.listener(Self::on_toggle_task_view_mode))
            .on_action(cx.listener(Self::on_cycle_task_filter))
            .on_action(cx.listener(Self::on_new_item))
            .on_action(cx.listener(Self::on_focus_search))
            .when(!in_ely_table, |d| {
                d.on_action(cx.listener(Self::on_select_next))
                    .on_action(cx.listener(Self::on_select_prev))
                    .on_action(cx.listener(Self::on_activate_selected))
            })
            .on_action(cx.listener(Self::on_toggle_selected))
            .on_action(cx.listener(Self::on_edit_task_title_action))
            .on_action(cx.listener(Self::on_move_task_next_stage))
            .on_action(cx.listener(Self::on_move_task_prev_stage))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    window.focus(&view.focus, cx);
                }),
            )
            .size_full()
            .child(body)
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
        crate::test_support::init(cx);
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
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
        crate::test_support::init(cx);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx)
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
        crate::test_support::init(cx);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx)
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
    fn ely_breadcrumb_closes_the_project_by_pointer_and_keyboard(cx: &mut TestAppContext) {
        for pointer in [true, false] {
            let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
                let mut page = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                page.project = Some((
                    "P".into(),
                    Remote {
                        value: Some(vec![record()]),
                        ..Remote::default()
                    },
                ));
                page.project_section = ProjectSection::Tasks;
                page.selected_task_path = Some("P/Plan.md".into());
                page
            });
            if pointer {
                let bounds = cx
                    .debug_bounds("project-breadcrumb-host")
                    .expect("breadcrumb renders");
                cx.simulate_click(
                    bounds.origin + gpui::point(px(8.), px(8.)),
                    gpui::Modifiers::none(),
                );
            } else {
                cx.update(|window, cx| {
                    window.blur(cx);
                    window.focus_next(cx);
                });
                cx.simulate_keystrokes("enter");
                cx.run_until_parked();
                cx.simulate_event(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse("enter").unwrap(),
                });
            }
            cx.run_until_parked();
            page.read_with(cx, |page, _| {
                assert!(page.project.is_none(), "pointer={pointer}");
                assert!(
                    page.selected_task_path.is_none(),
                    "close clears project selection"
                );
            });
        }
    }

    #[gpui::test]
    fn a_project_page_draws_its_tasks_as_a_table(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
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
        crate::test_support::init(cx);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.tab = ProjectsTab::AllTasks;
                projects.search_query = "berlin".into();
                projects.all_tasks = Remote {
                    value: Some(vec![
                        GlobalTask {
                            fields: Default::default(),
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
                            fields: Default::default(),
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
    fn all_tasks_panel_opens_on_a_task_and_draws_beside_the_table(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let task = |id: &str, status: &str| GlobalTask {
            id: format!("P/Tasks/{id}.md"),
            kind: "task".into(),
            title: id.into(),
            done: tasks::status_closes(status),
            due: None,
            priority: 2,
            summary: None,
            project: "P".into(),
            fields: [
                ("status".to_owned(), status.to_owned()),
                ("context".to_owned(), "Werkstatt".to_owned()),
            ]
            .into(),
        };
        let blocked = task("blocked", "blocked");
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let rows = vec![blocked.clone(), task("cancelled", "cancelled")];
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.tab = ProjectsTab::AllTasks;
                projects.group_by_project = true;
                projects.all_tasks = Remote {
                    value: Some(rows),
                    ..Remote::default()
                };
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        cx.update(|window, cx| {
            projects.update(cx, |projects, cx| {
                projects.select_task(&blocked, window, cx)
            })
        });
        cx.run_until_parked();
        projects.read_with(cx, |projects, cx| {
            let panel = projects.task_panel.as_ref().expect("the panel opened");
            assert_eq!(panel.path, "P/Tasks/blocked.md");
            let context = panel.input("context").expect("a context field");
            assert_eq!(context.read(cx).value().to_string(), "Werkstatt");
            let shown: Vec<&str> = projects
                .all_tasks
                .value
                .iter()
                .flatten()
                .filter(|t| projects.global_status.matches(t.status()))
                .map(|t| t.title.as_str())
                .collect();
            assert_eq!(shown, ["blocked"]);
        });
        cx.update(|window, cx| {
            projects.update(cx, |projects, cx| {
                projects.select_task(&blocked, window, cx)
            })
        });
        projects.read_with(cx, |projects, _| assert!(projects.task_panel.is_none()));
    }

    #[gpui::test]
    fn ely_grouping_toggle_preserves_the_projects_table_state(cx: &mut TestAppContext) {
        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            page.tab = ProjectsTab::AllTasks;
            page.all_tasks.value = Some(Vec::new());
            page.group_by_project = false;
            page
        });
        for expected in [true, false] {
            let toggle = cx
                .debug_bounds("toggle Group by project")
                .expect("grouping toggle renders");
            cx.simulate_click(toggle.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            page.read_with(cx, |page, _| assert_eq!(page.group_by_project, expected));
        }
    }

    #[gpui::test]
    fn all_notes_tab_filters_by_query(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.tab = ProjectsTab::AllNotes;
                projects.notes_search_query = "architecture".into();
                projects.all_notes = Remote {
                    value: Some(vec![
                        GlobalNote {
                            path: "Sjel/Architecture.md".into(),
                            kind: "note".into(),
                            title: "Architecture".into(),
                            project: "Sjel".into(),
                            url: None,
                        },
                        GlobalNote {
                            path: "BetterCode/BetterCode.md".into(),
                            kind: "project".into(),
                            title: "BetterCode".into(),
                            project: "BetterCode".into(),
                            url: None,
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
        crate::test_support::init(cx);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
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

    #[gpui::test]
    fn projects_hub_renders_table_header_and_sorts_projects(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let p1 = ProjectSummary {
            name: "Hearth".into(),
            archived: false,
            open: 3,
            done: 2,
            notes: 1,
        };
        let p2 = ProjectSummary {
            name: "Atlas".into(),
            archived: false,
            open: 0,
            done: 5,
            notes: 171,
        };
        let p3 = ProjectSummary {
            name: "ArchiveOld".into(),
            archived: true,
            open: 0,
            done: 10,
            notes: 4,
        };

        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.projects = Remote {
                    value: Some(vec![p1, p2, p3]),
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
            assert_eq!(projects.project_sort, (ProjectSort::Name, true));
            assert_eq!(projects.tab, ProjectsTab::Hubs);
        });

        // Test sorting behavior
        projects.update(cx, |projects, cx| {
            projects.project_sort = (ProjectSort::Notes, false);
            cx.notify();
        });
        cx.run_until_parked();
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.project_sort, (ProjectSort::Notes, false));
        });

        // Test sort cycling: flipping to ascending, then back to Name
        projects.update(cx, |projects, cx| {
            projects.project_sort = (ProjectSort::Tasks, true);
            cx.notify();
        });
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.project_sort, (ProjectSort::Tasks, true));
        });
    }

    #[gpui::test]
    fn project_tasks_supports_board_mode_and_task_panel_inspector(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let task1 = ProjectRecord {
            project: "P".into(),
            path: "P/Tasks/T1.md".into(),
            kind: "task".into(),
            title: "T1".into(),
            content: "---\nstatus: doing\ncontext: Terminal\n---\n".into(),
            done: false,
            due: Some("2026-11-10".into()),
            priority: 2,
        };
        let task2 = ProjectRecord {
            project: "P".into(),
            path: "P/Tasks/T2.md".into(),
            kind: "task".into(),
            title: "T2".into(),
            content: "---\nstatus: open\n---\n".into(),
            done: false,
            due: None,
            priority: 1,
        };

        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let records = vec![task1.clone(), task2.clone()];
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.project = Some((
                    "P".into(),
                    Remote {
                        value: Some(records),
                        ..Remote::default()
                    },
                ));
                projects.project_section = ProjectSection::Tasks;
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();

        // Starts in table mode
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.task_view_mode, TaskViewMode::Table);
            assert!(projects.task_panel.is_none());
        });

        // Open task panel on task1
        let global = tables::project_record_to_global_task(&task1);
        cx.update(|window, cx| {
            projects.update(cx, |projects, cx| {
                projects.select_task(&global, window, cx);
            });
        });
        cx.run_until_parked();
        projects.read_with(cx, |projects, cx| {
            let panel = projects.task_panel.as_ref().expect("task panel opened");
            assert_eq!(panel.path, "P/Tasks/T1.md");
            let context = panel.input("context").expect("context input present");
            assert_eq!(context.read(cx).value().to_string(), "Terminal");
        });

        // Toggle to board mode
        projects.update(cx, |projects, cx| {
            projects.task_view_mode = TaskViewMode::Board;
            cx.notify();
        });
        cx.run_until_parked();
        projects.read_with(cx, |projects, _| {
            assert_eq!(projects.task_view_mode, TaskViewMode::Board);
            assert!(
                projects.task_panel.is_some(),
                "inspector persists across view modes"
            );
        });
    }

    #[gpui::test]
    fn keyboard_navigation_and_inline_task_editing_in_projects(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let p1 = ProjectSummary {
            name: "Hearth".into(),
            archived: false,
            open: 2,
            done: 1,
            notes: 1,
        };
        let p2 = ProjectSummary {
            name: "Atlas".into(),
            archived: false,
            open: 0,
            done: 5,
            notes: 10,
        };
        let (projects, cx) = cx.add_window_view(move |_window, cx| {
            let mut view = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            view.projects.value = Some(vec![p1, p2]);
            view
        });
        cx.run_until_parked();

        // 1. Hub navigation with j / k
        projects.update(cx, |view, _cx| {
            assert_eq!(view.selected_index, None);
        });
        cx.update(|window, cx| {
            projects.update(cx, |view, cx| {
                view.on_select_next(&actions::SelectNext, window, cx);
            });
        });
        projects.read_with(cx, |view, _| {
            assert_eq!(view.selected_index, Some(0));
        });
        cx.update(|window, cx| {
            projects.update(cx, |view, cx| {
                view.on_select_next(&actions::SelectNext, window, cx);
            });
        });
        projects.read_with(cx, |view, _| {
            assert_eq!(view.selected_index, Some(1));
        });
        cx.update(|window, cx| {
            projects.update(cx, |view, cx| {
                view.on_select_prev(&actions::SelectPrev, window, cx);
            });
        });
        projects.read_with(cx, |view, _| {
            assert_eq!(view.selected_index, Some(0));
        });

        // 2. Open project and navigate tasks
        let task1 = ProjectRecord {
            project: "Hearth".into(),
            path: "Hearth/Tasks/T1.md".into(),
            kind: "task".into(),
            title: "T1".into(),
            content: "---\ntype: task\nstatus: open\n---\n# T1\n".into(),
            done: false,
            due: None,
            priority: 2,
        };
        let task2 = ProjectRecord {
            project: "Hearth".into(),
            path: "Hearth/Tasks/T2.md".into(),
            kind: "task".into(),
            title: "T2".into(),
            content: "---\ntype: task\nstatus: doing\n---\n# T2\n".into(),
            done: false,
            due: None,
            priority: 1,
        };
        projects.update(cx, |view, cx| {
            view.project = Some((
                "Hearth".into(),
                Remote {
                    value: Some(vec![task1.clone(), task2.clone()]),
                    ..Remote::default()
                },
            ));
            view.project_section = ProjectSection::Tasks;
            cx.notify();
        });
        cx.run_until_parked();

        // Driving task actions from the current row path
        projects.update(cx, |view, cx| {
            view.selected_task_path = Some("Hearth/Tasks/T1.md".into());
            cx.notify();
        });
        cx.run_until_parked();

        // Inline task title editing via `e` action on current row
        cx.update(|window, cx| {
            projects.update(cx, |view, cx| {
                view.on_edit_task_title_action(&actions::EditTaskTitle, window, cx);
            });
        });
        projects.read_with(cx, |view, _| {
            assert!(view.task_title_edit.is_some());
            let (path, original, _, _) = view.task_title_edit.as_ref().unwrap();
            assert_eq!(path, "Hearth/Tasks/T1.md");
            assert_eq!(original, "T1");
        });

        // Cancel editing
        projects.update(cx, |view, cx| {
            view.cancel_task_title(cx);
            assert!(view.task_title_edit.is_none());
        });
    }

    #[test]
    fn column_specs_and_sort_col_ids() {
        assert_eq!(ProjectSort::from_col_id("name"), Some(ProjectSort::Name));
        assert_eq!(
            ProjectSort::from_col_id("project_tasks"),
            Some(ProjectSort::Tasks)
        );
        assert_eq!(TaskSort::from_col_id("due"), Some(TaskSort::Due));
        assert_eq!(NoteSort::from_col_id("kind"), Some(NoteSort::Kind));
    }

    #[test]
    fn task_focus_filters_and_unblocker_chain() {
        let mut blocked_fields = std::collections::BTreeMap::new();
        blocked_fields.insert("blocked_by".into(), "Root Task".into());
        blocked_fields.insert("status".into(), "blocked".into());

        let root = GlobalTask {
            id: "P/root.md".into(),
            kind: "task".into(),
            title: "Root Task".into(),
            done: false,
            due: Some("2026-11-20".into()),
            priority: 1,
            summary: None,
            project: "P".into(),
            fields: Default::default(),
        };
        let downstream = GlobalTask {
            id: "P/down.md".into(),
            kind: "task".into(),
            title: "Downstream".into(),
            done: false,
            due: Some("2026-11-18".into()),
            priority: 2,
            summary: None,
            project: "P".into(),
            fields: blocked_fields,
        };
        let all = vec![root.clone(), downstream.clone()];
        let today = "2026-11-20";

        assert_eq!(unblocked_by_task(&root, &all).len(), 1);
        assert!(TaskFocusFilter::NeedsMeToday.matches(&root, &all, today));
        assert!(TaskFocusFilter::Unblockers.matches(&root, &all, today));
        assert!(!TaskFocusFilter::Drifting.matches(&root, &all, today));

        assert!(TaskFocusFilter::BlockedOnOthers.matches(&downstream, &all, today));
        assert!(TaskFocusFilter::Drifting.matches(&downstream, &all, today));
        assert!(!TaskFocusFilter::Unblockers.matches(&downstream, &all, today));
    }

    #[test]
    fn extract_open_checklist_items_parses_unchecked_only() {
        let md = "# Notes\n- [ ] Book train\n- [x] Done item\n* [ ] Pack adapter\n- [ ]   \n";
        assert_eq!(
            extract_open_checklist_items(md),
            vec!["Book train".to_string(), "Pack adapter".to_string()]
        );
    }

    #[gpui::test]
    fn guided_review_mode_mounts_and_exits(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                projects.review_mode = true;
                projects.all_tasks = Remote {
                    value: Some(vec![GlobalTask {
                        id: "P/1.md".into(),
                        kind: "task".into(),
                        title: "Overdue item".into(),
                        done: false,
                        due: Some("2026-01-01".into()),
                        priority: 1,
                        summary: None,
                        project: "P".into(),
                        fields: Default::default(),
                    }]),
                    ..Remote::default()
                };
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let projects = slot.take().unwrap();
        projects.read_with(cx, |p, _| assert!(p.review_mode));
    }
}
