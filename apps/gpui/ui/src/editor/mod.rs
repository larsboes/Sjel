//! The block note editor.
//!
//! One textarea per block (`sjel_gpui_core::blocks`). Enter splits a block, Backspace at its
//! start merges it into the one above, Up and Down at its edge cross into the neighbour. Every
//! change saves after a short pause through the [`Saver`] the app hands in, with the content it
//! started from. The Mac sends it to the capability, which refuses an edit made elsewhere in the
//! meantime; the phone writes its replica and queues the write for the Mac.

mod block_view;
mod preview;
mod slash_menu;
pub(crate) mod table;
mod types;

use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::{
    div, list as block_list, prelude::*, px, rgb, rgba, Context, Entity, EventEmitter, Focusable,
    FontWeight, HighlightStyle, IntoElement, ListAlignment, ListState, Render, SharedString,
    Window,
};
use sjel_gpui_core::blocks::{Block, Doc, Kind};
use sjel_gpui_core::inline::Inline;
use sjel_gpui_core::shell::{self, ProjectRecord};

use crate::block_menu;
use crate::theme::{tint, Palette};

pub use types::{
    bind_keys, repo_saver, CloseNote, EditorEvent, MoveBlockDown, MoveBlockUp, Save, SaveError,
    SaveNote, Saver,
};

pub(crate) use block_view::runs;
pub(crate) use types::{Menu, Row, Shown, SAVE_AFTER, TEXTAREA};

pub struct NoteEditor {
    pub(crate) path: String,
    pub title: SharedString,
    pub(crate) doc: Doc,
    /// One per block, in the same order.
    pub(crate) rows: Vec<Row>,
    /// The content the capability holds, as far as this editor knows.
    pub(crate) base: String,
    /// False when the note does not round-trip or changed elsewhere: nothing is saved.
    pub(crate) editable: bool,
    /// Why the note is read-only, or the last save error.
    pub(crate) problem: Option<String>,
    pub(crate) saving: bool,
    /// False in the typing bench and under `SJEL_GPUI_NO_SAVE`: nothing is written.
    pub editable_saves: bool,
    /// Bumped on every change, so only the last pause saves.
    pub(crate) revision: u64,
    /// The revision the capability last confirmed.
    pub(crate) saved_revision: u64,
    /// The block with the caret. It shows its markdown; every other block shows formatted text.
    pub(crate) active: Option<usize>,
    /// Lays out only the blocks on screen, so a keystroke costs the same in a long note.
    pub(crate) list: ListState,
    pub(crate) menu: Option<Menu>,
    pub(crate) saver: Saver,
    /// The page's palette. `None` follows the window's appearance; the phone sets its own.
    pub(crate) palette: Option<Palette>,
    /// A title waiting for the last keystroke to be saved before the page renames the note.
    pub(crate) pending_rename: Option<SharedString>,
    /// One per block: the reader view built last time. `None` until a block is drawn.
    pub(crate) shown: Vec<Option<Shown>>,
    pub(crate) preview: Option<Entity<preview::LiveBlock>>,
}

impl EventEmitter<EditorEvent> for NoteEditor {}

impl NoteEditor {
    pub fn new(
        record: ProjectRecord,
        saver: Saver,
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
            saver,
            palette: None,
            pending_rename: None,
            shown: Vec::new(),
            preview: None,
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
        if std::env::var("SJEL_GPUI_LIVE_PREVIEW").as_deref() == Ok("1") {
            editor.enable_preview(window, cx);
        }
        editor
    }

    fn enable_preview(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .doc
            .blocks
            .iter()
            .position(|block| block.kind == Kind::Paragraph)
        {
            let input = self.rows[index].input.clone();
            let palette = self.palette(window);
            self.preview = Some(cx.new(|cx| preview::LiveBlock::new(input, palette, cx)));
        }
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
                let moved = self.active != Some(index);
                let closes_menu = self.menu.as_ref().is_some_and(|menu| menu.block != index);
                self.active = Some(index);
                if closes_menu {
                    self.menu = None;
                }
                // A click already notified; this would be a second full render of the note.
                if moved || closes_menu {
                    cx.notify();
                }
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
    pub(crate) fn apply(
        &mut self,
        index: usize,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(crate) fn open_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(crate) fn step_menu(&mut self, index: usize, step: isize, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.menu.as_mut().filter(|menu| menu.block == index) else {
            return false;
        };
        let last = menu.choices().len().saturating_sub(1) as isize;
        menu.selected = (menu.selected as isize + step).clamp(0, last) as usize;
        cx.notify();
        true
    }

    /// Moves block `from` to `to`, counted before the move.
    pub(crate) fn move_block(
        &mut self,
        from: usize,
        to: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Ask for the note's title to become `title` once nothing is left to save. The answer is
    /// [`EditorEvent::Rename`], so the page renames a settled note rather than one whose last
    /// keystroke is still in the air.
    pub fn request_rename(&mut self, title: String, cx: &mut Context<Self>) {
        self.pending_rename = Some(title.into());
        self.save(cx);
        self.settled(cx);
    }

    /// Answers a rename that is waiting, once the text is saved and nothing is in flight.
    fn settled(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.revision != self.saved_revision {
            return;
        }
        if let Some(title) = self.pending_rename.take() {
            cx.emit(EditorEvent::Rename(title));
        }
    }

    /// Bench only: types one character at the end of the last block, the way a keystroke does.
    /// Draws with `palette` instead of the window's appearance.
    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub(crate) fn palette(&self, window: &Window) -> Palette {
        self.palette
            .unwrap_or_else(|| Palette::for_appearance(window.appearance()))
    }

    /// Bench only: puts the caret in a block the way a click on its rendered text does, cycling
    /// through the note. Measures what leaving the reader view for the markdown costs.
    pub fn bench_click(&mut self, step: usize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.rows.len();
        if count == 0 {
            return;
        }
        let index = step % count;
        self.focus(index, 0, window, cx);
    }

    pub fn bench_type(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(crate) fn on_backspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
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
    pub(crate) fn on_cross(
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

    pub fn focus(
        &mut self,
        index: usize,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Set before the focus event, so the next frame already draws this block's textarea.
        self.active = Some(index);
        self.list.scroll_to_reveal_item(index);
        self.rows[index].input.update(cx, |input, cx| {
            input.set_selected_range(offset..offset, cx);
            input.focus(window, cx);
        });
        // One frame: the focus event's own notify is skipped while `active` already says so.
        cx.notify();
    }

    pub(crate) fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
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

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let content = self.doc.markdown();
        if !self.editable || !self.editable_saves || self.saving || content == self.base {
            if content == self.base {
                self.saved_revision = self.revision;
            }
            self.settled(cx);
            return;
        }
        self.saving = true;
        cx.notify();
        let revision = self.revision;
        let work = shell::run((self.saver)(Save {
            path: self.path.clone(),
            content: content.clone(),
            base: self.base.clone(),
        }));
        cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|| Err(SaveError::Failed("The save stopped.".to_owned())));
            let _ = this.update(cx, |editor, cx| {
                editor.saving = false;
                match result {
                    Ok(()) => {
                        editor.base = content;
                        editor.saved_revision = revision;
                        editor.problem = None;
                        // Typing during the save left more to send.
                        editor.save(cx);
                        editor.settled(cx);
                    }
                    Err(SaveError::Conflict) => {
                        // A rename waiting on this write has nothing to settle on a read-only note.
                        editor.pending_rename = None;
                        editor.editable = false;
                        editor.problem = Some(
                            "This note changed elsewhere since you opened it. Nothing was overwritten; open it again to see the current text."
                                .to_owned(),
                        );
                    }
                    Err(SaveError::Failed(error)) => editor.problem = Some(error),
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

impl Render for NoteEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette(window);
        let mono = cx.theme().mono_font_family.clone();
        let (status_text, bad) = self.status();
        let dot_color = match (bad, self.saving, self.revision != self.saved_revision) {
            (true, _, _) => palette.red,
            (false, true, _) => palette.amber,
            (false, false, true) => palette.amber,
            (false, false, false) => palette.green,
        };
        let block_count = self.doc.blocks.len();
        let word_count = self
            .doc
            .blocks
            .iter()
            .map(|b| b.text.split_whitespace().count())
            .sum::<usize>();

        let est_minutes = word_count.div_ceil(200);
        let reading_time = if est_minutes <= 1 {
            "1 min read".to_string()
        } else {
            format!("{est_minutes} min read")
        };

        let total_todos = self
            .doc
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, Kind::Todo(_)))
            .count();
        let done_todos = self
            .doc
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, Kind::Todo(true)))
            .count();

        let tasks_badge = (total_todos > 0).then(|| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_1p5()
                .py(px(0.5))
                .rounded(px(3.))
                .bg(rgba(if done_todos == total_todos {
                    tint(palette.green, 0x22)
                } else {
                    palette.raised
                }))
                .text_color(rgb(if done_todos == total_todos {
                    palette.green
                } else {
                    palette.muted
                }))
                .child(format!("{done_todos}/{total_todos} tasks"))
        });

        let status_badge = div()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py(px(1.5))
            .rounded_full()
            .bg(rgba(tint(dot_color, 0x18)))
            .child(div().size(px(6.)).rounded_full().bg(rgb(dot_color)))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(if bad { palette.red } else { palette.muted }))
                    .child(status_text.to_owned()),
            );

        let status_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .pb_2p5()
            .border_b_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(status_badge)
                    .children(tasks_badge),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{word_count} words · {block_count} blocks · {reading_time}"
                    ))
                    .child(div().w(px(1.)).h(px(10.)).bg(rgba(palette.border)))
                    .child("Esc to close · / for blocks"),
            );

        let headings: Vec<(usize, u8, String)> = self
            .doc
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(idx, b)| match b.kind {
                Kind::Heading(level) => {
                    let label = b.text.trim();
                    if !label.is_empty() {
                        Some((idx, level, label.to_string()))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();

        let outline_bar = if headings.len() >= 2 {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .py_1()
                    .overflow_x_hidden()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.muted))
                            .child("§"),
                    )
                    .children(headings.into_iter().map(|(h_idx, level, label)| {
                        div()
                            .id(("heading-chip", h_idx))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(1.5))
                            .rounded(px(4.))
                            .bg(rgba(tint(palette.raised, 0x66)))
                            .hover(move |s| {
                                s.bg(rgba(palette.raised)).text_color(rgb(palette.accent))
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |editor, _, window, cx| {
                                editor.focus(h_idx, 0, window, cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("H{level}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .max_w(px(140.))
                                    .child(label),
                            )
                    })),
            )
        } else {
            None
        };

        let editor = cx.entity().downgrade();
        let blocks = block_list(self.list.clone(), move |index, window, cx| {
            editor
                .update(cx, |editor, cx| {
                    block_view::render_block(editor, index, window, cx)
                })
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .pb_4();

        let footer = div()
            .pt_2p5()
            .border_t_1()
            .border_color(rgba(palette.border))
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(rgb(palette.muted))
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
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(3.))
                                    .bg(rgba(palette.raised))
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .child("⌘S"),
                            )
                            .child("Save"),
                    )
                    .child("·")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(3.))
                                    .bg(rgba(palette.raised))
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .child("⌘["),
                            )
                            .child("Close"),
                    )
                    .child("·")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(3.))
                                    .bg(rgba(palette.raised))
                                    .font_family(mono)
                                    .text_xs()
                                    .child("⌘⇧↑/↓"),
                            )
                            .child("Move block"),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .opacity(0.8)
                    .child(self.path.clone()),
            );

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
                cx.emit(EditorEvent::Close);
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(status_bar)
            .children(outline_bar)
            .child(blocks)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{MouseButton, TestAppContext, VisualTestContext};
    use gpui_kit::Entity;
    use sjel_gpui_core::inline::Mark;
    use std::sync::Arc;

    use super::*;

    const CONTENT: &str = "---\ntype: note\n---\n# Plan\n\n- one\n- two\n\nText\n";

    fn record() -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: "P/Plan.md".into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: CONTENT.into(),
            done: false,
            due: None,
            priority: 2,
        }
    }

    /// The editor in a window, with a saver that refuses, so nothing is written.
    fn open(cx: &mut TestAppContext) -> (Entity<NoteEditor>, &mut VisualTestContext) {
        open_with(cx, record(), false)
    }

    fn open_with(
        cx: &mut TestAppContext,
        record: ProjectRecord,
        preview: bool,
    ) -> (Entity<NoteEditor>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let saver: Saver =
                Arc::new(|_| Box::pin(async { Err(SaveError::Failed("test".to_owned())) }));
            let view = cx.new(|cx| {
                let mut editor = NoteEditor::new(record, saver, window, cx);
                if preview {
                    editor.enable_preview(window, cx);
                }
                editor
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

    fn live_record() -> ProjectRecord {
        ProjectRecord {
            content: "---\ntype: note\n---\n# Plan\n\nBefore **bold** and [link](url).\n\nOther *block*.\n".into(),
            ..record()
        }
    }

    #[gpui::test]
    fn live_preview_focus_outside_a_span_keeps_the_readers_wrapping(cx: &mut TestAppContext) {
        let mut record = live_record();
        record.content = format!(
            "{}\n",
            "Before **bold** and [link](https://example.org/a/long/target). ".repeat(30)
        );
        let (editor, cx) = open_with(cx, record, true);
        cx.run_until_parked();
        let before = cx.debug_bounds("formatted-0").unwrap();
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.focus(0, 0, window, cx)));
        cx.run_until_parked();
        let after = cx.debug_bounds("live-block").unwrap();
        assert_eq!(before.size, after.size);
    }

    #[gpui::test]
    fn live_preview_keeps_markdown_typing_and_undo_in_the_native_input(cx: &mut TestAppContext) {
        let (editor, cx) = open_with(cx, live_record(), true);
        focus_end(&editor, 1, cx);
        let before = markdown(&editor, cx);
        cx.simulate_keystrokes("x");
        assert!(markdown(&editor, cx).contains("[link](url).x"));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(markdown(&editor, cx), before);
        cx.simulate_keystrokes("cmd-shift-z");
        assert!(markdown(&editor, cx).contains("[link](url).x"));
        assert!(editor.read_with(cx, |editor, cx| {
            let live = editor.preview.as_ref().unwrap().read(cx);
            live.input == editor.rows[1].input && live.input != editor.rows[2].input
        }));
    }

    #[gpui::test]
    fn live_preview_click_maps_formatted_text_to_source_and_keeps_bold(cx: &mut TestAppContext) {
        let (editor, cx) = open_with(cx, live_record(), true);
        focus_end(&editor, 1, cx);
        let preview = editor.read_with(cx, |editor, _| editor.preview.clone().unwrap());
        let position = preview.read_with(cx, |live, _| {
            live.layout.position_for_index(9).unwrap() + gpui_kit::point(px(0.), px(2.))
        });
        cx.simulate_mouse_down(position, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Default::default());
        cx.run_until_parked();
        let (source, caret) = preview.read_with(cx, |live, cx| {
            (
                live.input.read(cx).value().to_string(),
                live.input.read(cx).cursor(),
            )
        });
        assert_eq!(caret, 11);
        let inline = Inline::preview(&source, caret);
        assert_eq!(inline.text, "Before **bold** and link.");
        assert!(inline
            .marks
            .iter()
            .any(|(range, mark)| *mark == Mark::Bold && &inline.text[range.clone()] == "bold"));
        cx.simulate_keystrokes("x");
        assert!(markdown(&editor, cx).contains("Before **boxld** and [link](url)."));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(markdown(&editor, cx), live_record().content);
    }

    #[gpui::test]
    fn live_preview_composition_preserves_source_and_undo(cx: &mut TestAppContext) {
        use gpui_kit::EntityInputHandler;

        let (editor, cx) = open_with(cx, live_record(), true);
        focus_end(&editor, 1, cx);
        let before = markdown(&editor, cx);
        let input = editor.read_with(cx, |editor, _| editor.rows[1].input.clone());
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "名前", Some(2..2), window, cx);
            })
        });
        cx.run_until_parked();
        assert!(input.read_with(cx, |input, _| input.value().ends_with("名前")));
        assert_eq!(markdown(&editor, cx), before);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_text_in_range(None, "名前", window, cx);
                input.unmark_text(window, cx);
            })
        });
        cx.run_until_parked();
        assert!(markdown(&editor, cx).contains("[link](url).名前"));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(markdown(&editor, cx), before);
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
    fn markdown_table_renders_formatted_and_focuses_to_edit(cx: &mut TestAppContext) {
        let mut record = live_record();
        record.content = "---\ntype: note\n---\n# Table Note\n\n| Frage | Stand |\n|---|---|\n| Master | Geprüft |\n".into();
        let (editor, cx) = open_with(cx, record, false);
        cx.run_until_parked();
        // The table is rendered as block 1
        assert_eq!(editor.read_with(cx, |e, _| e.doc.blocks.len()), 2);
        assert_eq!(editor.read_with(cx, |e, _| e.active), None);
        // Focusing block 1 opens the textarea for editing
        cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(1, 0, window, cx)));
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |e, _| e.active), Some(1));
    }

    #[gpui::test]
    fn code_block_renders_formatted_and_focuses_to_edit(cx: &mut TestAppContext) {
        let mut record = live_record();
        record.content = "---\ntype: note\n---\n# Code Note\n\n```rust\nfn main() {}\n```\n".into();
        let (editor, cx) = open_with(cx, record, false);
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |e, _| e.doc.blocks.len()), 2);
        assert_eq!(editor.read_with(cx, |e, _| e.doc.blocks[1].kind), Kind::Code);
        assert_eq!(editor.read_with(cx, |e, _| e.active), None);
        // Focusing block 1 opens the code textarea for editing
        cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(1, 0, window, cx)));
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |e, _| e.active), Some(1));
    }
}
