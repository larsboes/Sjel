//! The Notes page: pick a project, pick a note, edit it in place as blocks.
//!
//! One textarea per block (`crate::blocks`). Enter splits a block, Backspace at its start merges
//! it into the one above, Up and Down at its edge cross into the neighbour. Every change saves
//! after a short pause through `POST /projects/api/record` with the content it started from, so
//! an edit made elsewhere in the meantime is refused, never overwritten.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Backspace, Escape, Input, InputEvent, InputState, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, Size, WindowExt};
use gpui_kit::{
    actions, anchored, deferred, div, list as block_list, prelude::*, px, relative, rgb, rgba,
    AnyElement, App, Context, Entity, Focusable, FontStyle, FontWeight, HighlightStyle, KeyBinding,
    ListAlignment, ListState, MouseButton, MouseDownEvent, SharedString, StrikethroughStyle,
    StyledText, Subscription, UnderlineStyle, Window,
};

use crate::block_menu::{self, Choice};
use crate::blocks::{Block, Doc, Kind};
use crate::inline::{Inline, Mark};
use crate::shell::{self, ProjectRecord, ProjectSummary, Shell, ShellError, ShellResult};
use crate::tasks;
use crate::theme::Palette;
use crate::ui::{list, note, placeholder, row, section};
use crate::workspace::Remote;

actions!(notes, [SaveNote, CloseNote, MoveBlockUp, MoveBlockDown]);

/// The size every block's textarea has, which sets its editor's padding.
const TEXTAREA: Size = Size::Medium;

/// How long typing pauses before the note saves.
const SAVE_AFTER: Duration = Duration::from_millis(700);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", SaveNote, Some("NoteEditor")),
        KeyBinding::new("cmd-[", CloseNote, Some("NoteEditor")),
        // On the textarea's own context, bound after gpui-kit's select-to-start/end, so these win.
        KeyBinding::new("cmd-shift-up", MoveBlockUp, Some("NoteEditor > Input")),
        KeyBinding::new("cmd-shift-down", MoveBlockDown, Some("NoteEditor > Input")),
    ]);
}

pub struct Notes {
    shell: Result<Arc<Shell>, ShellError>,
    pub projects: Remote<Vec<ProjectSummary>>,
    /// The open project and its records.
    project: Option<(String, Remote<Vec<ProjectRecord>>)>,
    editor: Option<Entity<NoteEditor>>,
    /// The "New task" field under the task table.
    new_task: Option<(Entity<InputState>, Subscription)>,
    /// The task whose due date is being typed, and its field.
    due_edit: Option<(ProjectRecord, Entity<InputState>, Subscription)>,
}

impl Notes {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self {
            shell,
            projects: Remote::default(),
            project: None,
            editor: None,
            new_task: None,
            due_edit: None,
        }
    }

    pub fn load_projects(&mut self, cx: &mut Context<Self>) {
        let shell = match &self.shell {
            Ok(shell) => shell.clone(),
            Err(error) => return self.projects.settle(Err(error.clone())),
        };
        if self.projects.loading {
            return;
        }
        self.projects.loading = true;
        let work = shell::run(async move { shell.projects().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.projects.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_project(&mut self, name: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.shell.clone() else {
            return;
        };
        let records = Remote {
            loading: true,
            ..Remote::default()
        };
        self.project = Some((name.clone(), records));
        self.editor = None;
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

    fn open_note(&mut self, record: ProjectRecord, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(shell) = self.shell.clone() else {
            return;
        };
        let notes = cx.entity().downgrade();
        let editor = cx.new(|cx| NoteEditor::new(record, shell, notes, window, cx));
        editor.update(cx, |editor, cx| editor.focus(0, 0, window, cx));
        self.editor = Some(editor);
        cx.notify();
    }

    /// Bench only (`SJEL_GPUI_BENCH_NOTE`): opens the record at `path` once its project has loaded,
    /// with saving off. False while the project is still loading.
    pub(crate) fn bench_open(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let project = path.split('/').next().unwrap_or_default().to_owned();
        let record = match &self.project {
            Some((open, records)) if *open == project => {
                match records
                    .value
                    .as_ref()
                    .and_then(|r| r.iter().find(|r| r.path == path))
                {
                    Some(record) => record.clone(),
                    None => return false,
                }
            }
            _ => {
                self.open_project(project, cx);
                return false;
            }
        };
        self.open_note(record, window, cx);
        if let Some(editor) = &self.editor {
            editor.update(cx, |editor, _| editor.editable_saves = false);
        }
        true
    }

    pub(crate) fn editor(&self) -> Option<Entity<NoteEditor>> {
        self.editor.clone()
    }

    /// Reads the open project again, keeping the rows on screen until the answer arrives.
    fn refresh_records(&mut self, cx: &mut Context<Self>) {
        let (Ok(shell), Some((name, records))) = (self.shell.clone(), self.project.as_mut()) else {
            return;
        };
        records.loading = true;
        let (request, name) = (name.clone(), name.clone());
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

    /// Runs one task write, toasts a failure, then reads the project again.
    fn task_write<F, Fut>(&mut self, request: F, window: &mut Window, cx: &mut Context<Self>)
    where
        F: FnOnce(Arc<Shell>) -> Fut,
        Fut: std::future::Future<Output = ShellResult<()>> + Send + 'static,
    {
        let Ok(shell) = self.shell.clone() else {
            return;
        };
        let work = shell::run(request(shell));
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                if let Err(error) = result {
                    window.push_notification(Notification::error(error.to_string()), cx);
                }
                view.refresh_records(cx);
            });
        })
        .detach();
    }

    fn toggle_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_write(
            move |s| async move { s.set_task_done(&path, done).await },
            window,
            cx,
        );
    }

    /// Writes one frontmatter field of a task, refused if the note changed since it was read.
    fn set_task_field(
        &mut self,
        record: &ProjectRecord,
        key: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(content) = tasks::set_field(&record.content, key, &value) else {
            window.push_notification(Notification::error("This task has no frontmatter."), cx);
            return;
        };
        let (path, base) = (record.path.clone(), record.content.clone());
        self.task_write(
            move |s| async move { s.save_record(&path, &content, &base).await },
            window,
            cx,
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
                view.task_write(
                    move |s| async move { s.add_task(&project, &title).await },
                    window,
                    cx,
                );
            }
        });
        self.new_task = Some((input.clone(), subscription));
        input
    }

    /// Back to the record list. The project reloads so its list shows what was just saved.
    fn close_note(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.take() {
            editor.update(cx, |editor, cx| editor.save(cx));
        }
        if let Some(name) = self.project.as_ref().map(|(name, _)| name.clone()) {
            self.open_project(name, cx);
        }
        cx.notify();
    }
}

impl Notes {
    fn task_table(
        &self,
        records: &[ProjectRecord],
        new_task: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let header = row()
            .min_h(px(32.))
            .py_1()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(div().w(px(20.)))
            .child(div().flex_1().child("Task"))
            .child(div().w(px(110.)).child("Due"))
            .child(div().w(px(56.)).child("Priority"))
            .into_any_element();
        let mut rows = vec![header];
        for (index, task) in tasks::ordered(records).into_iter().enumerate() {
            let (toggle, open) = (task.clone(), task.clone());
            let due = task.due.clone().filter(|d| !d.is_empty());
            let due_tone = match due.as_deref() {
                Some(d) if !task.done && *d < *today.as_str() => palette.red,
                Some(d) if !task.done && d == today => palette.amber,
                _ => palette.muted,
            };
            let editing = self
                .due_edit
                .as_ref()
                .filter(|(record, _, _)| record.path == task.path)
                .map(|(_, input, _)| input.clone());
            let due_cell = match editing {
                Some(input) => div()
                    .w(px(110.))
                    .child(Input::new(&input).small())
                    .into_any_element(),
                None => {
                    let edit = task.clone();
                    div()
                        .id(("due", index))
                        .w(px(110.))
                        .text_sm()
                        .text_color(rgb(due_tone))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit_due(edit.clone(), window, cx)
                        }))
                        .child(due.unwrap_or_else(|| "—".into()))
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
                                view.open_note(open.clone(), window, cx)
                            }))
                            .child(task.title.clone()),
                    )
                    .child(due_cell)
                    .child(
                        div()
                            .id(("priority", index))
                            .w(px(56.))
                            .text_sm()
                            .cursor_pointer()
                            .text_color(rgb(match priority {
                                1 => palette.red,
                                2 => palette.text,
                                _ => palette.muted,
                            }))
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
                            .child(format!("P{priority}")),
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
        list(rows, palette)
    }
}

impl Render for Notes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        if let Some(editor) = &self.editor {
            let title = editor.read(cx).title.clone();
            // An opaque card: the window is translucent, and a wallpaper behind it must not
            // show through the text.
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_3()
                .px_8()
                .pt_5()
                .rounded(px(12.))
                .bg(rgba(palette.surface | 0xff))
                .border_1()
                .border_color(rgba(palette.border))
                .child(back(
                    format!(
                        "‹ {}",
                        self.project.as_ref().map_or("", |(n, _)| n.as_str())
                    ),
                    cx.listener(|view, _, _, cx| view.close_note(cx)),
                    palette,
                ))
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().flex_1().min_h_0().child(editor.clone()))
                .into_any_element();
        }
        if self.project.is_some() {
            let new_task = self.new_task_field(window, cx);
            let Some((name, records)) = &self.project else {
                return div().into_any_element();
            };
            let table = records
                .value
                .as_ref()
                .map(|records| self.task_table(records, &new_task, palette, cx));
            let body = match (&records.value, &records.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(240.),
                (Some(records), None) => {
                    let rows = records
                        .iter()
                        .filter(|record| record.kind != "task")
                        .enumerate()
                        .map(|(index, record)| {
                            let open = record.clone();
                            row()
                                .id(("record", index))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(palette.raised)))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.open_note(open.clone(), window, cx)
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
                                .into_any_element()
                        })
                        .collect();
                    list(rows, palette)
                }
            };
            return div()
                .id("records")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3()
                .child(back(
                    "‹ Projects".into(),
                    cx.listener(|view, _, _, cx| {
                        view.project = None;
                        cx.notify();
                    }),
                    palette,
                ))
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.clone()),
                )
                .children(table.map(|table| section("Tasks", table, palette)))
                .child(section("Notes", body, palette))
                .into_any_element();
        }
        let body = match (&self.projects.value, &self.projects.error) {
            (_, Some(error)) => note(error.to_string(), palette.red),
            (None, None) => placeholder(320.),
            (Some(projects), None) => {
                let rows =
                    projects
                        .iter()
                        .filter(|p| !p.archived)
                        .map(|project| {
                            let name = project.name.clone();
                            row()
                                .id(SharedString::from(format!("project-{}", project.name)))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(palette.raised)))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_project(name.clone(), cx)
                                }))
                                .child(div().flex_1().text_sm().child(project.name.clone()))
                                .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                    format!(
                                        "{} note{}",
                                        project.notes,
                                        if project.notes == 1 { "" } else { "s" }
                                    ),
                                ))
                                .into_any_element()
                        })
                        .collect();
                list(rows, palette)
            }
        };
        div()
            .id("projects")
            .size_full()
            .overflow_y_scroll()
            .child(section("Projects", body, palette))
            .into_any_element()
    }
}

fn back(
    label: String,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    palette: Palette,
) -> AnyElement {
    div()
        .id("back")
        .text_sm()
        .text_color(rgb(palette.accent))
        .cursor_pointer()
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

struct Row {
    input: Entity<TextareaState>,
    _changes: Subscription,
}

/// The kind menu under a block: from `/` in an empty paragraph, or from the block handle.
struct Menu {
    block: usize,
    selected: usize,
    /// The text after `/`. `None` when the handle opened it: the block keeps its text.
    query: Option<String>,
}

impl Menu {
    fn choices(&self) -> Vec<&'static Choice> {
        block_menu::matching(self.query.as_deref().unwrap_or_default())
    }
}

/// A block being dragged by its handle. It draws itself as the drag preview.
#[derive(Clone)]
struct DraggedBlock {
    index: usize,
    label: SharedString,
}

impl Render for DraggedBlock {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        div()
            .max_w(px(360.))
            .truncate()
            .px_3()
            .py_1()
            .rounded(px(6.))
            .bg(rgba(palette.surface | 0xff))
            .border_1()
            .border_color(rgba(palette.border))
            .text_sm()
            .child(self.label.clone())
    }
}

pub struct NoteEditor {
    path: String,
    title: SharedString,
    doc: Doc,
    /// One per block, in the same order.
    rows: Vec<Row>,
    /// The content the capability holds, as far as this editor knows.
    base: String,
    /// False when the note does not round-trip or changed elsewhere: nothing is saved.
    editable: bool,
    /// Why the note is read-only, or the last save error.
    problem: Option<String>,
    saving: bool,
    /// False in the typing bench and under `SJEL_GPUI_NO_SAVE`: nothing is written.
    editable_saves: bool,
    /// Bumped on every change, so only the last pause saves.
    revision: u64,
    /// The revision the capability last confirmed.
    saved_revision: u64,
    /// The block with the caret. It shows its markdown; every other block shows formatted text.
    active: Option<usize>,
    /// Lays out only the blocks on screen, so a keystroke costs the same in a long note.
    list: ListState,
    menu: Option<Menu>,
    shell: Arc<Shell>,
    notes: gpui_kit::WeakEntity<Notes>,
}

impl NoteEditor {
    fn new(
        record: ProjectRecord,
        shell: Arc<Shell>,
        notes: gpui_kit::WeakEntity<Notes>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let doc = Doc::parse(&record.content);
        let editable = doc.lossless(&record.content);
        let mut editor = Self {
            path: record.path,
            title: record.title.into(),
            rows: Vec::new(),
            base: record.content,
            editable,
            problem: (!editable).then(|| {
                "This note has markdown the editor cannot write back exactly, so it is read-only."
                    .to_owned()
            }),
            saving: false,
            editable_saves: std::env::var_os("SJEL_GPUI_NO_SAVE").is_none(),
            revision: 0,
            saved_revision: 0,
            active: None,
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            menu: None,
            shell,
            notes,
            doc,
        };
        if editor.doc.blocks.is_empty() {
            editor.doc.blocks.push(Block::paragraph("", 0));
        }
        editor.rows = (0..editor.doc.blocks.len())
            .map(|index| editor.row(index, window, cx))
            .collect();
        let handles: Vec<_> = editor
            .rows
            .iter()
            .map(|row| Some(row.input.read(cx).focus_handle(cx)))
            .collect();
        editor.list.splice_focusable(0..0, handles);
        editor
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Row {
        let block = &self.doc.blocks[index];
        let code = block.kind == Kind::Code;
        let text = block.text.clone();
        let input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .auto_grow(1, 100_000)
                // Enter splits the block; Shift+Enter, and Enter in code, adds a line.
                .submit_on_enter(!code);
            state.set_value(text, window, cx);
            state
        });
        let changes = cx.subscribe_in(&input, window, Self::on_input);
        Row {
            input,
            _changes: changes,
        }
    }

    fn index_of(&self, input: &Entity<TextareaState>) -> Option<usize> {
        // ponytail: linear scan per event; an id map if notes reach thousands of blocks.
        self.rows.iter().position(|row| &row.input == input)
    }

    fn on_input(
        &mut self,
        input: &Entity<TextareaState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.index_of(input) else {
            return;
        };
        match event {
            InputEvent::Change => self.text_changed(index, window, cx),
            InputEvent::PressEnter { shift: false, .. } if self.editable => {
                match self.menu.as_ref().filter(|menu| menu.block == index) {
                    Some(menu) => {
                        if let Some(choice) = menu.choices().get(menu.selected) {
                            self.apply(index, choice.kind, window, cx);
                        }
                    }
                    None => self.split(index, window, cx),
                }
            }
            InputEvent::Focus => {
                self.active = Some(index);
                if self.menu.as_ref().is_some_and(|menu| menu.block != index) {
                    self.menu = None;
                }
                cx.notify();
            }
            InputEvent::Blur if self.active == Some(index) => {
                self.active = None;
                cx.notify();
            }
            _ => {}
        }
    }

    fn text_changed(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.rows[index].input.clone();
        let block = &mut self.doc.blocks[index];
        block.text = input.read(cx).value().to_string();
        if block.apply_shortcut() {
            let text = block.text.clone();
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        let query = block_menu::slash_query(block.kind, &block.text).map(str::to_owned);
        let handle_menu = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.block == index && menu.query.is_none());
        if !handle_menu {
            self.menu = query
                .filter(|query| !block_menu::matching(query).is_empty())
                .map(|query| Menu {
                    block: index,
                    selected: 0,
                    query: Some(query),
                });
        }
        self.changed(cx);
    }

    /// Turn block `index` into `kind`. From `/`, the command text goes; from the handle, the
    /// block keeps its text.
    fn apply(&mut self, index: usize, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let slash = self.menu.take().is_some_and(|menu| menu.query.is_some());
        let block = &mut self.doc.blocks[index];
        if slash {
            block.text.clear();
        }
        block.set_kind(kind);
        // Code needs a textarea where Enter adds a line, so the block gets a new one.
        let row = self.row(index, window, cx);
        let handle = row.input.read(cx).focus_handle(cx);
        self.rows[index] = row;
        self.list.splice_focusable(index..index + 1, [Some(handle)]);
        let caret = match kind {
            Kind::Code => "```\n".len(),
            _ => self.doc.blocks[index].text.len(),
        };
        self.focus(index, caret, window, cx);
        self.changed(cx);
    }

    /// Opens the kind menu for a block from its handle.
    fn open_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let end = self.doc.blocks[index].text.len();
        self.focus(index, end, window, cx);
        self.menu = Some(Menu {
            block: index,
            selected: 0,
            query: None,
        });
        cx.notify();
    }

    /// Moves the menu's selection; false when no menu is open on this block.
    fn step_menu(&mut self, index: usize, step: isize, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.menu.as_mut().filter(|menu| menu.block == index) else {
            return false;
        };
        let last = menu.choices().len().saturating_sub(1) as isize;
        menu.selected = (menu.selected as isize + step).clamp(0, last) as usize;
        cx.notify();
        true
    }

    /// Moves block `from` to `to`, counted before the move.
    fn move_block(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.rows.len().saturating_sub(1);
        let to = to.min(last);
        if from == to || !self.editable || from > last {
            return;
        }
        self.menu = None;
        // Blank lines belong to places, not blocks: swapping two list items keeps the list tight.
        let gaps: Vec<usize> = self.doc.blocks.iter().map(|b| b.gap).collect();
        let block = self.doc.blocks.remove(from);
        let row = self.rows.remove(from);
        let handle = row.input.read(cx).focus_handle(cx);
        self.list.splice(from..from + 1, 0);
        self.doc.blocks.insert(to, block);
        for (block, gap) in self.doc.blocks.iter_mut().zip(gaps) {
            block.gap = gap;
        }
        self.rows.insert(to, row);
        self.list.splice_focusable(to..to, [Some(handle)]);
        let end = self.doc.blocks[to].text.len();
        self.focus(to, end, window, cx);
        self.changed(cx);
    }

    fn move_active(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active {
            let to = if up {
                index.saturating_sub(1)
            } else {
                index + 1
            };
            self.move_block(index, to, window, cx);
        }
    }

    /// Bench only: types one character at the end of the last block, the way a keystroke does.
    pub(crate) fn bench_type(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.rows.len() - 1;
        if self.active != Some(last) {
            let end = self.doc.blocks[last].text.len();
            self.focus(last, end, window, cx);
        }
        let input = self.rows[last].input.clone();
        input.update(cx, |input, cx| input.insert("x", window, cx));
        self.text_changed(last, window, cx);
    }

    fn split(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.rows[index].input.clone();
        let (text, at) = {
            let state = input.read(cx);
            (state.value().to_string(), state.cursor())
        };
        let block = &mut self.doc.blocks[index];
        // Enter in an empty list item ends the list, as in Notion.
        if block.kind.is_list() && text.is_empty() {
            *block = Block::paragraph("", 1);
            self.changed(cx);
            return;
        }
        let tail = text[at..].to_owned();
        block.text.truncate(at);
        let head = block.text.clone();
        let next = block.next(tail);
        input.update(cx, |input, cx| input.set_value(head, window, cx));
        self.doc.blocks.insert(index + 1, next);
        let row = self.row(index + 1, window, cx);
        let handle = row.input.read(cx).focus_handle(cx);
        self.rows.insert(index + 1, row);
        self.list
            .splice_focusable(index + 1..index + 1, [Some(handle)]);
        self.focus(index + 1, 0, window, cx);
        self.changed(cx);
    }

    /// Backspace at the start of a block: a list item or heading becomes a paragraph first;
    /// a paragraph joins the block above. Anywhere else the textarea deletes as usual.
    fn on_backspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let at_start = {
            let state = self.rows[index].input.read(cx);
            state.cursor() == 0 && state.selected_range().is_empty()
        };
        if !at_start || !self.editable {
            return false;
        }
        let block = &mut self.doc.blocks[index];
        if !matches!(block.kind, Kind::Paragraph | Kind::Code) {
            // A paragraph right under a list line would read back as part of that item.
            let gap = if index > 0 {
                block.gap.max(1)
            } else {
                block.gap
            };
            *block = Block::paragraph(std::mem::take(&mut block.text), gap);
            self.changed(cx);
            return true;
        }
        if index == 0 {
            return false;
        }
        let removed = self.doc.blocks.remove(index);
        self.rows.remove(index);
        self.list.splice(index..index + 1, 0);
        let above = &mut self.doc.blocks[index - 1];
        let joint = above.text.len();
        above.text.push_str(&removed.text);
        let text = above.text.clone();
        self.rows[index - 1]
            .input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.focus(index - 1, joint, window, cx);
        self.changed(cx);
        true
    }

    /// Up on a block's first visual line, or Down on its last, moves to the neighbour.
    fn on_cross(
        &mut self,
        index: usize,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let target = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1)
        };
        let Some(target) = target.filter(|t| *t < self.rows.len()) else {
            return false;
        };
        let state = self.rows[index].input.read(cx);
        let at_edge = match (state.cursor_layout(), state.text_bounds()) {
            (Some((caret, line)), Some(bounds)) => {
                if up {
                    caret.top() - bounds.top() < line
                } else {
                    bounds.bottom() - caret.bottom() < line
                }
            }
            // No layout yet: fall back to logical lines.
            _ => {
                let line = state.cursor_position().line as usize;
                if up {
                    line == 0
                } else {
                    line + 1 >= state.value().lines().count().max(1)
                }
            }
        };
        if !at_edge {
            return false;
        }
        let offset = if up {
            self.doc.blocks[target].text.len()
        } else {
            0
        };
        self.focus(target, offset, window, cx);
        true
    }

    fn focus(&mut self, index: usize, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Set before the focus event, so the next frame already draws this block's textarea.
        self.active = Some(index);
        cx.notify();
        self.list.scroll_to_reveal_item(index);
        self.rows[index].input.update(cx, |input, cx| {
            input.set_selected_range(offset..offset, cx);
            input.focus(window, cx);
        });
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.editable {
            self.doc.blocks[index].toggle_done();
            self.changed(cx);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        // A moved or retyped block must not read back as part of the one above it.
        self.doc.repair_gaps();
        cx.notify();
        if !self.editable {
            return;
        }
        self.revision += 1;
        let revision = self.revision;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            let _ = this.update(cx, |editor, cx| {
                if editor.revision == revision {
                    editor.save(cx);
                }
            });
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let content = self.doc.markdown();
        if !self.editable || !self.editable_saves || self.saving || content == self.base {
            if content == self.base {
                self.saved_revision = self.revision;
            }
            return;
        }
        self.saving = true;
        cx.notify();
        let (shell, path, base) = (self.shell.clone(), self.path.clone(), self.base.clone());
        let saved = content.clone();
        let revision = self.revision;
        let work = shell::run(async move { shell.save_record(&path, &saved, &base).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |editor, cx| {
                editor.saving = false;
                match result {
                    Ok(()) => {
                        editor.base = content;
                        editor.saved_revision = revision;
                        editor.problem = None;
                        // Typing during the save left more to send.
                        editor.save(cx);
                    }
                    Err(ShellError::Refused(409, _)) => {
                        editor.editable = false;
                        editor.problem = Some(
                            "This note changed elsewhere since you opened it. Nothing was overwritten; open it again to see the current text."
                                .to_owned(),
                        );
                    }
                    Err(error) => editor.problem = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn status(&self) -> (&str, bool) {
        match (&self.problem, self.saving) {
            (Some(problem), _) => (problem.as_str(), true),
            (None, true) => ("Saving…", false),
            (None, false) if !self.editable_saves => ("Not saving (SJEL_GPUI_NO_SAVE)", false),
            (None, false) if self.revision != self.saved_revision => ("Edited", false),
            (None, false) => ("Saved", false),
        }
    }
}

impl NoteEditor {
    fn render_block(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(block), Some(row)) = (self.doc.blocks.get(index), self.rows.get(index)) else {
            return div().into_any_element();
        };
        let palette = Palette::for_appearance(window.appearance());
        let mono = cx.theme().mono_font_family.clone();
        let indent = block.prefix.len() - block.prefix.trim_start().len();
        let gutter: AnyElement = match block.kind {
            Kind::Bullet => div().child("•").into_any_element(),
            Kind::Numbered => div()
                .child(block.prefix.trim().to_owned())
                .into_any_element(),
            Kind::Todo(done) => {
                let editor = cx.entity().downgrade();
                Checkbox::new(("todo", index))
                    .checked(done)
                    .on_click(move |_, _, cx| {
                        let _ = editor.update(cx, |editor, cx| editor.toggle(index, cx));
                    })
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };
        // Code always shows its source; prose shows markdown only while the caret is in it.
        let field = if self.active == Some(index) || block.kind == Kind::Code {
            let field = Textarea::new(&row.input)
                .appearance(false)
                // Cancels the padding its editor gets from its size (`gpui-component`
                // input.rs `set_editor_paddings`), so a block keeps its box when it gains the
                // caret.
                .mx(-TEXTAREA.input_px())
                .my(-TEXTAREA.input_py())
                .line_height(relative(1.25))
                .readonly(!self.editable);
            // The textarea sets its own text size, so a heading styles the textarea itself.
            match block.kind {
                Kind::Heading(1) => field.text_2xl().font_weight(FontWeight::SEMIBOLD),
                Kind::Heading(2) => field.text_xl().font_weight(FontWeight::SEMIBOLD),
                Kind::Heading(_) => field.text_lg().font_weight(FontWeight::SEMIBOLD),
                _ => field,
            }
            .into_any_element()
        } else {
            formatted(index, &block.text, palette, cx)
        };
        let field = match block.kind {
            Kind::Heading(1) => div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(field),
            Kind::Heading(2) => div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(field),
            Kind::Heading(_) => div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(field),
            Kind::Quote => div()
                .border_l_2()
                .border_color(rgba(palette.border))
                .pl_3()
                .text_color(rgb(palette.muted))
                .child(field),
            Kind::Todo(true) => div()
                .text_color(rgb(palette.muted))
                .line_through()
                .child(field),
            Kind::Code => div()
                .font_family(mono.clone())
                .text_sm()
                .p_2()
                .rounded(px(8.))
                .bg(rgba(palette.raised))
                .child(field),
            _ => div().child(field),
        };
        let label: SharedString = Inline::parse(&block.text).text.into();
        let handle = div()
            .id(("handle", index))
            .absolute()
            .left(px(-22.))
            .top(px(if block.gap > 0 { 10. } else { 2. }))
            .w(px(18.))
            .line_height(relative(1.25))
            .text_color(rgb(palette.muted))
            .cursor_grab()
            .opacity(0.)
            .group_hover("block", |style| style.opacity(1.))
            .hover(move |style| style.text_color(rgb(palette.text)))
            .child("⋮⋮")
            .on_click(cx.listener(move |editor, _, window, cx| editor.open_menu(index, window, cx)))
            .on_drag(DraggedBlock { index, label }, |dragged, _, _, cx| {
                cx.new(|_| dragged.clone())
            });
        let menu = self
            .menu
            .as_ref()
            .filter(|menu| menu.block == index)
            .map(|menu| self.render_menu(menu, palette, cx));
        div()
            .id(("block", index))
            .group("block")
            .relative()
            .w_full()
            .flex()
            .items_start()
            .gap_2()
            .pl(px(indent as f32 * 8.))
            .drag_over::<DraggedBlock>(move |style, _, _, _| style.bg(rgba(palette.raised)))
            .on_drop(
                cx.listener(move |editor, dragged: &DraggedBlock, window, cx| {
                    editor.move_block(dragged.index, index, window, cx)
                }),
            )
            .child(handle)
            .pt(px(if block.gap > 0 { 10. } else { 2. }))
            .capture_action(cx.listener(move |editor, _: &Backspace, window, cx| {
                if editor.on_backspace(index, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(move |editor, _: &MoveUp, window, cx| {
                if editor.step_menu(index, -1, cx) || editor.on_cross(index, true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(move |editor, _: &MoveDown, window, cx| {
                if editor.step_menu(index, 1, cx) || editor.on_cross(index, false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(move |editor, _: &Escape, _, cx| {
                if editor.menu.as_ref().is_some_and(|menu| menu.block == index) {
                    editor.menu = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .w(px(20.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    // The text's line height, so the marker sits on the first line.
                    .line_height(relative(1.25))
                    .text_color(rgb(palette.muted))
                    .child(gutter),
            )
            .child(div().flex_1().min_w_0().child(field).children(menu))
            .into_any_element()
    }

    fn render_menu(&self, menu: &Menu, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let index = menu.block;
        let items = menu.choices().into_iter().enumerate().map(|(at, choice)| {
            let kind = choice.kind;
            div()
                .id(("choice", at))
                .px_3()
                .py_1()
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .when(at == menu.selected, |item| item.bg(rgba(palette.raised)))
                .hover(move |style| style.bg(rgba(palette.raised)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |editor, _, window, cx| {
                        editor.apply(index, kind, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(choice.label)
        });
        let title = if menu.query.is_some() {
            "Insert"
        } else {
            "Turn into"
        };
        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                div()
                    .mt_1()
                    .w(px(220.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .rounded(px(10.))
                    .bg(rgba(palette.surface | 0xff))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .shadow_lg()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(title),
                    )
                    .children(items),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }
}

/// A block as a reader sees it. A click puts the caret at the same place in the markdown.
fn formatted(
    index: usize,
    text: &str,
    palette: Palette,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let inline = Inline::parse(text);
    // An empty block keeps one line's height.
    let shown: SharedString = if inline.text.is_empty() {
        " ".into()
    } else {
        inline.text.clone().into()
    };
    let styled = StyledText::new(shown).with_highlights(runs(&inline.marks, palette));
    let layout = styled.layout().clone();
    div()
        .id(("formatted", index))
        .w_full()
        // The textarea's line height, so a block keeps its height when it gains the caret.
        .line_height(relative(1.25))
        .cursor_text()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |editor, event: &MouseDownEvent, window, cx| {
                let shown = layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|nearest| nearest);
                editor.focus(index, inline.source_offset(shown), window, cx);
                cx.stop_propagation();
            }),
        )
        .child(styled)
        .into_any_element()
}

/// Marks as non-overlapping highlight runs; where marks nest, their styles combine.
fn runs(
    marks: &[(std::ops::Range<usize>, Mark)],
    palette: Palette,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let style = |mark: Mark| match mark {
        Mark::Bold => HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        Mark::Italic => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        Mark::Code => HighlightStyle {
            background_color: Some(rgba(palette.raised).into()),
            ..Default::default()
        },
        Mark::Strike => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            ..Default::default()
        },
        Mark::Link => HighlightStyle {
            color: Some(rgb(palette.accent).into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            }),
            ..Default::default()
        },
    };
    let mut cuts: Vec<usize> = marks.iter().flat_map(|(r, _)| [r.start, r.end]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            marks
                .iter()
                .filter(|(r, _)| r.start <= range.start && range.end <= r.end)
                .map(|(_, mark)| style(*mark))
                .reduce(HighlightStyle::highlight)
                .map(|style| (range, style))
        })
        .collect()
}

impl Render for NoteEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let (status, bad) = self.status();
        let status = div()
            .text_xs()
            .text_color(rgb(if bad { palette.red } else { palette.muted }))
            .child(status.to_owned());
        let editor = cx.entity().downgrade();
        let blocks = block_list(self.list.clone(), move |index, window, cx| {
            editor
                .update(cx, |editor, cx| editor.render_block(index, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .pb_8();
        div()
            .key_context("NoteEditor")
            .on_action(cx.listener(|editor, _: &SaveNote, _, cx| editor.save(cx)))
            .on_action(cx.listener(|editor, _: &MoveBlockUp, window, cx| {
                editor.move_active(true, window, cx)
            }))
            .on_action(cx.listener(|editor, _: &MoveBlockDown, window, cx| {
                editor.move_active(false, window, cx)
            }))
            .on_action(cx.listener(|editor, _: &CloseNote, window, cx| {
                if editor.problem.is_some() && editor.editable {
                    window.push_notification(
                        Notification::error("The last save failed; the text is still here."),
                        cx,
                    );
                }
                let _ = editor.notes.update(cx, |notes, cx| notes.close_note(cx));
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(status)
            .child(blocks)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    const CONTENT: &str = "---\ntype: note\n---\n# Plan\n\n- one\n- two\n\nText\n";

    fn record() -> ProjectRecord {
        ProjectRecord {
            path: "P/Plan.md".into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: CONTENT.into(),
            done: false,
            due: None,
            priority: 2,
        }
    }

    /// The editor in a window, with a shell that refuses at once, so nothing reaches the network.
    fn open(cx: &mut TestAppContext) -> (Entity<NoteEditor>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let shell = Arc::new(Shell::new("http://127.0.0.1:9").unwrap());
            let view = cx.new(|cx| {
                NoteEditor::new(
                    record(),
                    shell,
                    gpui_kit::WeakEntity::new_invalid(),
                    window,
                    cx,
                )
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        let editor = slot.take().unwrap();
        (editor, cx)
    }

    fn focus_end(editor: &Entity<NoteEditor>, index: usize, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let end = editor.doc.blocks[index].text.len();
                editor.focus(index, end, window, cx)
            })
        });
        cx.run_until_parked();
    }

    fn markdown(editor: &Entity<NoteEditor>, cx: &mut VisualTestContext) -> String {
        editor.read_with(cx, |editor, _| editor.doc.markdown())
    }

    #[gpui::test]
    fn enter_continues_the_list_and_typing_lands_in_the_new_item(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        focus_end(&editor, 2, cx);
        cx.simulate_keystrokes("enter t h r e e");
        assert_eq!(
            markdown(&editor, cx),
            "---\ntype: note\n---\n# Plan\n\n- one\n- two\n- three\n\nText\n"
        );
    }

    #[gpui::test]
    fn backspace_ends_a_list_item_then_joins_the_line_above(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(2, 0, window, cx)));
        cx.run_until_parked();
        cx.simulate_keystrokes("backspace");
        assert_eq!(
            markdown(&editor, cx),
            "---\ntype: note\n---\n# Plan\n\n- one\n\ntwo\n\nText\n"
        );
        cx.simulate_keystrokes("backspace");
        assert_eq!(
            markdown(&editor, cx),
            "---\ntype: note\n---\n# Plan\n\n- onetwo\n\nText\n"
        );
    }

    #[gpui::test]
    fn a_shortcut_typed_in_an_empty_block_makes_a_todo(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        focus_end(&editor, 3, cx);
        cx.simulate_keystrokes("enter [ ] space m i l k");
        let block = editor.read_with(cx, |e, _| e.doc.blocks[4].clone());
        assert_eq!(
            (block.kind, block.text.as_str()),
            (Kind::Todo(false), "milk")
        );
    }

    #[gpui::test]
    fn slash_opens_the_menu_and_enter_applies_the_filtered_choice(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        focus_end(&editor, 3, cx);
        cx.simulate_keystrokes("enter / h 2");
        assert!(editor.read_with(cx, |e, _| e.menu.is_some()));
        cx.simulate_keystrokes("enter P l a n");
        let block = editor.read_with(cx, |e, _| e.doc.blocks[4].clone());
        assert_eq!(
            (block.kind, block.text.as_str()),
            (Kind::Heading(2), "Plan")
        );
        assert!(editor.read_with(cx, |e, _| e.menu.is_none()));
    }

    #[gpui::test]
    fn arrows_pick_in_the_menu_and_escape_closes_it(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        focus_end(&editor, 3, cx);
        cx.simulate_keystrokes("enter / l i s t down");
        assert_eq!(
            editor.read_with(cx, |e, _| e.menu.as_ref().map(|m| m.selected)),
            Some(1)
        );
        cx.simulate_keystrokes("escape");
        assert!(editor.read_with(cx, |e, _| e.menu.is_none()));
        // With the menu closed, Enter splits again.
        cx.simulate_keystrokes("enter");
        assert_eq!(editor.read_with(cx, |e, _| e.doc.blocks.len()), 6);
    }

    #[gpui::test]
    fn cmd_shift_arrows_move_the_active_block(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        focus_end(&editor, 2, cx);
        cx.simulate_keystrokes("cmd-shift-up");
        assert_eq!(
            markdown(&editor, cx),
            "---\ntype: note\n---\n# Plan\n\n- two\n- one\n\nText\n"
        );
        // The heading moved under a list item gets a blank line, so it stays a heading.
        focus_end(&editor, 0, cx);
        cx.simulate_keystrokes("cmd-shift-down cmd-shift-down cmd-shift-down");
        let reread = Doc::parse(&markdown(&editor, cx));
        assert_eq!(reread.blocks.len(), 4);
        assert_eq!(reread.blocks[3].kind, Kind::Heading(1));
    }

    #[gpui::test]
    fn down_on_the_last_line_moves_to_the_next_block(cx: &mut TestAppContext) {
        let (editor, cx) = open(cx);
        cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(1, 0, window, cx)));
        cx.run_until_parked();
        cx.simulate_keystrokes("down x");
        assert_eq!(
            editor.read_with(cx, |e, _| e.doc.blocks[2].text.clone()),
            "xtwo"
        );
    }

    #[gpui::test]
    fn a_project_page_draws_its_tasks_as_a_table(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| {
                let mut notes = Notes::new(Err(ShellError::NotLoopback));
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
                notes.project = Some(("P".into(), records));
                notes
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let notes = slot.take().unwrap();
        notes.read_with(cx, |notes, _| assert!(notes.new_task.is_some()));
    }
}
