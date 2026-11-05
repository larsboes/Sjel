//! Types, actions, save contracts, and internal row/menu states for NoteEditor.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::TextareaState;
use gpui_kit::component::Size;
use gpui_kit::prelude::*;
use gpui_kit::{
    actions, div, px, rgba, App, Context, Entity, HighlightStyle, IntoElement, KeyBinding, Render,
    SharedString, Subscription, Window,
};
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::inline::Inline;
use sjel_gpui_core::shell::ShellError;

use crate::block_menu::{self, Choice};
use crate::theme::Palette;

actions!(editor, [SaveNote, CloseNote, MoveBlockUp, MoveBlockDown]);

/// The size every block's textarea has, which sets its editor's padding.
pub const TEXTAREA: Size = Size::Medium;

/// How long typing pauses before the note saves.
pub const SAVE_AFTER: Duration = Duration::from_millis(700);

/// One save: the note, its new markdown, and the content the editor started from.
pub struct Save {
    pub path: String,
    pub content: String,
    pub base: String,
}

/// Why a save did not land.
pub enum SaveError {
    /// The note changed elsewhere since the editor opened it. The editor turns read-only.
    Conflict,
    /// Anything else. The editor keeps the text and shows the sentence.
    Failed(String),
}

/// Where an editor writes. It runs on the shell's runtime (`shell::run`), off the UI thread.
pub type Saver =
    Arc<dyn Fn(Save) -> Pin<Box<dyn Future<Output = Result<(), SaveError>> + Send>> + Send + Sync>;

/// Saves through `Repo`. On the Mac the capability refuses (409) an edit made over a newer
/// version; on the phone the replica takes it and the sync settles a conflict.
pub fn repo_saver(repo: Repo) -> Saver {
    Arc::new(move |save: Save| {
        let repo = repo.clone();
        Box::pin(async move {
            match repo
                .save_record(&save.path, &save.content, &save.base)
                .await
            {
                Ok(()) => Ok(()),
                Err(ShellError::Refused(409, _)) => Err(SaveError::Conflict),
                Err(error) => Err(SaveError::Failed(error.to_string())),
            }
        })
    })
}

/// What the editor asks of the page that holds it.
pub enum EditorEvent {
    /// `⌘[` or the back control: the page closes the note.
    Close,
    /// The note is saved and its title should become this. The page owns the rename, because it
    /// is the one that moves bundles of records and links; the editor only settles its text.
    Rename(SharedString),
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", SaveNote, Some("NoteEditor")),
        KeyBinding::new("cmd-[", CloseNote, Some("NoteEditor")),
        // On the textarea's own context, bound after gpui-kit's select-to-start/end, so these win.
        KeyBinding::new("cmd-shift-up", MoveBlockUp, Some("NoteEditor > Input")),
        KeyBinding::new("cmd-shift-down", MoveBlockDown, Some("NoteEditor > Input")),
    ]);
}

pub(crate) struct Row {
    pub(crate) input: Entity<TextareaState>,
    pub(crate) _changes: Subscription,
}

/// A block as a reader sees it, kept between frames: its plain text, its marks, and the place
/// every shown byte came from. A re-render then parses and shapes nothing; only the text element
/// that draws it is built again.
pub(crate) struct Shown {
    /// The block text and palette this was built from. Either one moving rebuilds it.
    pub(crate) text: String,
    pub(crate) palette: Palette,
    /// The block's text without its marks, which is what a reader sees.
    pub(crate) label: SharedString,
    /// Where each shown byte came from in the markdown, for a click.
    pub(crate) inline: Arc<Inline>,
    pub(crate) highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>,
}

/// The kind menu under a block: from `/` in an empty paragraph, or from the block handle.
pub(crate) struct Menu {
    pub(crate) block: usize,
    pub(crate) selected: usize,
    /// The text after `/`. `None` when the handle opened it: the block keeps its text.
    pub(crate) query: Option<String>,
}

impl Menu {
    pub(crate) fn choices(&self) -> Vec<&'static Choice> {
        block_menu::matching(self.query.as_deref().unwrap_or_default())
    }
}

/// A block being dragged by its handle. It draws itself as the drag preview.
#[derive(Clone)]
pub(crate) struct DraggedBlock {
    pub(crate) index: usize,
    pub(crate) label: SharedString,
    pub(crate) palette: Palette,
}

impl Render for DraggedBlock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
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
