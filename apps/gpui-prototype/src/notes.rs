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
    Backspace, InputEvent, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::{
    actions, div, list as block_list, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity,
    Focusable, FontWeight, KeyBinding, ListAlignment, ListState, SharedString, Subscription,
    Window,
};

use crate::blocks::{Block, Doc, Kind};
use crate::shell::{self, ProjectRecord, ProjectSummary, Shell, ShellError};
use crate::theme::Palette;
use crate::ui::{list, note, placeholder, row, section};
use crate::workspace::Remote;

actions!(notes, [SaveNote, CloseNote]);

/// How long typing pauses before the note saves.
const SAVE_AFTER: Duration = Duration::from_millis(700);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", SaveNote, Some("NoteEditor")),
        KeyBinding::new("cmd-[", CloseNote, Some("NoteEditor")),
    ]);
}

pub struct Notes {
    shell: Result<Arc<Shell>, ShellError>,
    pub projects: Remote<Vec<ProjectSummary>>,
    /// The open project and its records.
    project: Option<(String, Remote<Vec<ProjectRecord>>)>,
    editor: Option<Entity<NoteEditor>>,
}

impl Notes {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self {
            shell,
            projects: Remote::default(),
            project: None,
            editor: None,
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
        let first = editor.read(cx).rows.first().map(|row| row.input.clone());
        if let Some(first) = first {
            first.update(cx, |input, cx| input.focus(window, cx));
        }
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
        if let Some((name, records)) = &self.project {
            let body = match (&records.value, &records.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(240.),
                (Some(records), None) => {
                    let rows = records
                        .iter()
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
                .child(section("Records", body, palette))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(name.clone()),
                )
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
    /// False only in the typing bench, which must never write a note.
    editable_saves: bool,
    /// Bumped on every change, so only the last pause saves.
    revision: u64,
    /// The revision the capability last confirmed.
    saved_revision: u64,
    /// Lays out only the blocks on screen, so a keystroke costs the same in a long note.
    list: ListState,
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
            editable_saves: true,
            revision: 0,
            saved_revision: 0,
            list: ListState::new(0, ListAlignment::Top, px(600.)),
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
                self.split(index, window, cx)
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
        self.changed(cx);
    }

    /// Bench only: types one character at the end of the last block, the way a keystroke does.
    pub(crate) fn bench_type(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.rows.len() - 1;
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

    fn focus(&self, index: usize, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
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
        let field = Textarea::new(&row.input)
            .appearance(false)
            .readonly(!self.editable);
        let field = match block.kind {
            // The textarea sets its own text size, so a heading styles the textarea itself.
            Kind::Heading(1) => div().child(field.text_2xl().font_weight(FontWeight::SEMIBOLD)),
            Kind::Heading(2) => div().child(field.text_xl().font_weight(FontWeight::SEMIBOLD)),
            Kind::Heading(_) => div().child(field.text_lg().font_weight(FontWeight::SEMIBOLD)),
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
        div()
            .w_full()
            .flex()
            .items_start()
            .gap_2()
            .pl(px(indent as f32 * 8.))
            .mt(px(if block.gap > 0 { 10. } else { 2. }))
            .capture_action(cx.listener(move |editor, _: &Backspace, window, cx| {
                if editor.on_backspace(index, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(move |editor, _: &MoveUp, window, cx| {
                if editor.on_cross(index, true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(move |editor, _: &MoveDown, window, cx| {
                if editor.on_cross(index, false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .w(px(20.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .pt(px(2.))
                    .text_color(rgb(palette.muted))
                    .child(gutter),
            )
            .child(div().flex_1().min_w_0().child(field))
            .into_any_element()
    }
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
}
