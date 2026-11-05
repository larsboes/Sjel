//! The Sjel Clipper in the app: Settings › Clipper edits the templates and shows the Interpreter
//! model comms owns; Feed › Clips lists and reads saved clips.
//!
//! Both read and write over comms' owner-only clip socket through `sjel_gpui_core::clipper`,
//! off the UI thread. The browser extension pulls the templates from comms, so a save here
//! reaches every browser at its next sync.

mod composer;
mod reader;
mod settings;
mod types;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, FontWeight, IntoElement, Render, Window,
};
use sjel_gpui_core::blocks::Doc;
use sjel_gpui_core::clipper::{self, ClipDocument};

use crate::primitives::{list, note};
use crate::theme::{Palette, ThemeContext};

use composer::ComposerState;

pub use settings::ClipperSettings;
pub use types::*;

pub struct Clips {
    pub(super) documents: Option<Result<Vec<ClipDocument>, String>>,
    pub(super) loading: bool,
    pub(super) open: Option<Result<ClipDocument, String>>,
    pub(super) parsed_doc: Option<Doc>,
    pub(super) reader_page: usize,
    pub(super) confirm_delete: Option<String>,
    pub(super) confirm_delete_all: bool,
    pub(super) delete_status: Option<(bool, String)>,
    pub(super) palette: Option<Palette>,
    composer: Option<ComposerState>,
}

impl Clips {
    pub fn new() -> Self {
        Self {
            documents: None,
            loading: false,
            open: None,
            parsed_doc: None,
            reader_page: 0,
            confirm_delete: None,
            confirm_delete_all: false,
            delete_status: None,
            palette: None,
            composer: None,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::documents(200) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.documents = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open(&mut self, key: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::document(&key) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.parsed_doc = result
                    .as_ref()
                    .ok()
                    .map(|document| Doc::parse(&document.markdown));
                view.reader_page = 0;
                view.confirm_delete = None;
                view.open = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_document(&mut self, key: String, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.delete_status = Some((true, "Removing clip…".into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::delete_document(&key) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.confirm_delete = None;
                match result {
                    Ok(count) => {
                        view.open = None;
                        view.parsed_doc = None;
                        view.delete_status =
                            Some((true, format!("Removed {count} clip version(s).")));
                        view.load(cx);
                    }
                    Err(error) => view.delete_status = Some((false, error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_all_documents(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.delete_status = Some((true, "Removing all clips…".into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::delete_all_documents() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.confirm_delete_all = false;
                match result {
                    Ok(count) => {
                        view.open = None;
                        view.parsed_doc = None;
                        view.documents = Some(Ok(Vec::new()));
                        view.delete_status =
                            Some((true, format!("Removed {count} clip version(s).")));
                    }
                    Err(error) => view.delete_status = Some((false, error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn clips_header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let clip_count = self
            .documents
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map_or(0, Vec::len);
        let delete_all_label = if self.confirm_delete_all {
            format!("Confirm remove all ({clip_count})")
        } else {
            "Remove all".to_string()
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .py_1()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_sm()
                            .child("Saved Clips"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Web pages and multi-turn LLM thread clips"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children((clip_count > 0).then(|| {
                        Button::new("clips-delete-all")
                            .small()
                            .ghost()
                            .label(delete_all_label)
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.confirm_delete_all {
                                    view.delete_all_documents(cx);
                                } else {
                                    view.confirm_delete_all = true;
                                    cx.notify();
                                }
                            }))
                    }))
                    .child(
                        Button::new("clips-refresh")
                            .small()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|view, _, _, cx| view.load(cx))),
                    )
                    .child(
                        Button::new("clips-new-thread")
                            .small()
                            .primary()
                            .label("+ New Thread Clip")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.start_composer("".into(), "".into(), vec![], window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Clips {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        if self.documents.is_none() && !self.loading {
            self.load(cx);
        }

        if self.composer.is_some() {
            let body = self.render_composer(cx, palette);
            return div().w_full().child(body);
        }

        let body = match (&self.open, &self.documents) {
            (Some(Ok(document)), _) => match self.parsed_doc.as_ref() {
                Some(doc) => self.reader(document, doc, palette, cx),
                None => note("Preparing clip…", palette.muted),
            },
            (Some(Err(error)), _) => note(error.clone(), palette.red),
            (None, None) => note("Reading clips from comms…", palette.muted),
            (None, Some(Err(error))) => note(error.clone(), palette.red),
            (None, Some(Ok(documents))) if documents.is_empty() => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.clips_header(palette, cx))
                .child(note(
                    "No clips yet. Save a page with the Sjel Clipper in Chrome, or create a thread clip.",
                    palette.muted,
                ))
                .into_any_element(),
            (None, Some(Ok(documents))) => {
                let rows = documents
                    .iter()
                    .enumerate()
                    .map(|(index, document)| {
                        let key = document.key.clone();
                        div()
                            .id(("clip-row", index))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| view.open(key.clone(), cx)))
                            .child(div().font_weight(FontWeight::MEDIUM).child(
                                if document.title.is_empty() {
                                    document.key.clone()
                                } else {
                                    document.title.clone()
                                },
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "{} · {} · v{}",
                                        document.updated_at, document.url, document.versions
                                    )),
                            )
                            .into_any_element()
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.clips_header(palette, cx))
                    .child(list(rows, palette))
                    .into_any_element()
            }
        };
        let status = self.delete_status.as_ref().map(|(ok, message)| {
            note(
                message.clone(),
                if *ok { palette.muted } else { palette.red },
            )
        });
        div().w_full().child(
            div()
                .id("clips-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .max_w(px(860.))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .children(status)
                        .child(body),
                ),
        )
    }
}
