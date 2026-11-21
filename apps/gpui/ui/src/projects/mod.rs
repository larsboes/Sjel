//! The Projects page: pick a project, view tasks and notes, edit them in place.

use std::rc::Rc;

use crate::ely::data_display::Tone;
use crate::ely::feedback::Toast;
use crate::ely::forms::{Input, InputEvent, TextInput};
use crate::ely::primitives::Severity;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use crate::ely::theme::ControlSize;
use crate::feedback::WindowToastExt;
use gpui_kit::component::scroll::ScrollableElement;
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
mod render;
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
    pub(super) search_input: Option<(Entity<TextInput>, Subscription)>,
    pub notes_search_query: String,
    pub(super) notes_search_input: Option<(Entity<TextInput>, Subscription)>,
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
    pub(super) title_edit: Option<(String, String, Entity<TextInput>, Subscription)>,
    /// The note whose last read failed. The hub opens itself only once, so a failing read is not
    /// retried on every frame.
    pub(super) failed: Option<String>,
    /// The "New task" field under the task table.
    pub(super) new_task: Option<(Entity<TextInput>, Subscription)>,
    /// The task whose due date is being typed (its path), and its field.
    pub(super) due_edit: Option<(String, Entity<TextInput>, Subscription)>,
    /// The "New project" field.
    pub(super) new_project: Option<(Entity<TextInput>, Subscription)>,
    pub adding_project: bool,
    /// The "New note" field inside project view.
    pub(super) new_note: Option<(Entity<TextInput>, Subscription)>,
    pub adding_note: bool,
    /// The writes the Mac refused, with its reasons. Their content is still here and still queued:
    /// the band these draw is the only place that says so.
    pub(super) blocked: Vec<Blocked>,
    pub(super) scroll: ScrollHandle,
    pub focus: FocusHandle,
    pub selected_index: Option<usize>,
    pub selected_task_index: Option<usize>,
    pub selected_task_path: Option<String>,
    pub(super) task_title_edit: Option<(String, String, Entity<TextInput>, Subscription)>,
    pub show_charts: bool,
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
            show_charts: false,
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
        if self.title_edit.is_some() {
            self.cancel_title(cx);
        } else if self.task_title_edit.is_some() {
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
                    .key_context("Input")
                    .w_full()
                    .max_w(px(560.))
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(Input::new(&input))
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
            let mut state = TextInput::new(window, cx).placeholder("Note title… (Press ↩ or Esc)");
            state.set_text(title.clone(), cx);
            state
        });
        let subscription = cx.subscribe_in(&input, window, |view, _, event, _, cx| match event {
            InputEvent::Submit => view.commit_title(cx),
            InputEvent::Blur => view.cancel_title(cx),
            _ => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.title_edit = Some((path, title, input, subscription));
        cx.notify();
    }

    /// Takes the typed title and asks the editor for it once the note is saved.
    fn commit_title(&mut self, cx: &mut Context<Self>) {
        let Some((_, was, input, _)) = self.title_edit.take() else {
            return;
        };
        cx.notify();
        let title = input.read(cx).text().trim().to_owned();
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
        let density = self.density;
        editor.update(cx, |editor, cx| editor.set_density(density, cx));
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
            let mut state = TextInput::new(window, cx).placeholder("YYYY-MM-DD");
            state.set_text(due, cx);
            state
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            match event {
                InputEvent::Submit => {
                    let due = input.read(cx).text().trim().to_owned();
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
            let mut state = TextInput::new(window, cx).placeholder("Task title… (Press ↩ or Esc)");
            state.set_text(title.clone(), cx);
            state
        });
        let subscription =
            cx.subscribe_in(&input, window, |view, _, event, window, cx| match event {
                InputEvent::Submit => view.commit_task_title(window, cx),
                InputEvent::Blur => view.cancel_task_title(cx),
                _ => {}
            });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.task_title_edit = Some((path, title, input, subscription));
        cx.notify();
    }

    pub(super) fn commit_task_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, was, input, _)) = self.task_title_edit.take() else {
            return;
        };
        cx.notify();
        let title = input.read(cx).text().trim().to_owned();
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

    fn new_task_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.new_task {
            return input.clone();
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx).placeholder("New task… (!1-!3 priority, @today, @tomorrow)")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::Submit = event {
                let raw_title = input.read(cx).text().trim().to_owned();
                let Some(project) = view.project.as_ref().map(|(name, _)| name.clone()) else {
                    return;
                };
                if raw_title.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_text(String::new(), cx));
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Search projects…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                view.search_query = input.read(cx).text().trim().to_lowercase();
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.notes_search_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Search all notes…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                view.notes_search_query = input.read(cx).text().trim().to_lowercase();
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.new_project {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Project name… (Press ↩)"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |view, input, event, window, cx| match event {
                    InputEvent::Submit => {
                        let name = input.read(cx).text().trim().to_owned();
                        if !name.is_empty() {
                            input.update(cx, |input, cx| input.set_text(String::new(), cx));
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
                    InputEvent::Blur if input.read(cx).text().trim().is_empty() => {
                        view.adding_project = false;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        self.new_project = Some((input.clone(), subscription));
        input
    }

    fn new_note_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.new_note {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Note title… (Press ↩)"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |view, input, event, window, cx| match event {
                    InputEvent::Submit => {
                        let title = input.read(cx).text().trim().to_owned();
                        let Some(project) = view.project.as_ref().map(|(name, _)| name.clone())
                        else {
                            return;
                        };
                        if !title.is_empty() {
                            input.update(cx, |input, cx| input.set_text(String::new(), cx));
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
                    InputEvent::Blur if input.read(cx).text().trim().is_empty() => {
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
mod tests;
