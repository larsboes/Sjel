//! Document PDF viewer, page rendering, caching, and zoom controls.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use futures::channel::oneshot;
use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, Context, ObjectFit, RenderImage, StyledImage,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::theme::Palette;

use super::Documents;

/// Why a page could not be drawn. Every variant is shown to the reader rather than swallowed into
/// an empty pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// The bytes are not a PDF the platform renderer can open.
    Unreadable,
    /// The page number is outside the document.
    NoSuchPage,
    /// The page box is empty, or drawing it would exceed the maximum pixel budget.
    TooLarge,
    /// The platform renderer could not build a bitmap for this page.
    NoBitmap,
    /// This platform has no PDF renderer.
    Unsupported,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "This file is not a PDF this Mac can read.",
            Self::NoSuchPage => "That page is not in this document.",
            Self::TooLarge => "This page is too large to draw here.",
            Self::NoBitmap => "This page could not be drawn.",
            Self::Unsupported => "PDF pages cannot be drawn on this platform yet.",
        })
    }
}

/// Platform-backed PDF page renderer supplied by the host shell.
pub trait PdfRenderer: Send + Sync {
    /// How many pages the opened bytes hold. Asked once, so the reader can size its page strip.
    fn info(&self, bytes: Arc<Vec<u8>>) -> oneshot::Receiver<Result<usize, PdfError>>;

    /// Ask for one page at `width` pixels wide.
    fn render(
        &self,
        bytes: Arc<Vec<u8>>,
        page: usize,
        width: u32,
    ) -> oneshot::Receiver<Result<Arc<RenderImage>, PdfError>>;

    /// Open the document original in the host platform's viewer when supported.
    fn open_external(&self, _filename: &str, _bytes: &[u8]) {}
}

pub(super) type RendererFactory = fn() -> Result<Arc<dyn PdfRenderer>, String>;

pub(super) static DEFAULT_RENDERER: OnceLock<RendererFactory> = OnceLock::new();

/// Register the host shell's [`PdfRenderer`] factory so [`Documents::new`] wires it without
/// importing platform UI crates directly.
pub fn set_default_pdf_renderer(factory: RendererFactory) {
    let _ = DEFAULT_RENDERER.set(factory);
}

pub fn register_renderer(factory: RendererFactory) {
    set_default_pdf_renderer(factory);
}

pub(crate) const CACHED_PAGES: usize = 6;

/// The zoom level the reader draws at. The bitmap is re-rendered at the width Zoom specifies, so
/// the difference the reader sees is sharpness, not layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Zoom {
    #[default]
    Page,
    Close,
}

impl Zoom {
    pub(crate) fn width(self) -> u32 {
        match self {
            Self::Page => 1200,
            Self::Close => 2400,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Page => "Fit",
            Self::Close => "2×",
        }
    }

    pub(crate) fn other(self) -> Self {
        match self {
            Self::Page => Self::Close,
            Self::Close => Self::Page,
        }
    }
}

/// The document on screen: its bytes, and the pages of it that have been drawn.
#[derive(Default)]
pub(crate) struct Reader {
    pub(crate) id: Option<String>,
    pub(crate) bytes: Option<Arc<Vec<u8>>>,
    pub(crate) pages: usize,
    pub(crate) page: usize,
    pub(crate) first_page: usize,
    pub(crate) last_page: Option<usize>,
    pub(crate) zoom: Zoom,
    /// Keyed by page and drawn width, so toggling zoom back reuses what was already drawn.
    pub(crate) drawn: BTreeMap<(usize, u32), Arc<RenderImage>>,
    /// Why the original could not be read or drawn. Shown with the text, never instead of it.
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    /// The extracted text instead of the page. Text stays available when drawing fails.
    pub(crate) reading_text: bool,
}

impl Reader {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn next_page(&mut self, by: isize) -> bool {
        if self.pages == 0 {
            return false;
        }
        let first = self.first_page.min(self.pages - 1);
        let last = self
            .last_page
            .unwrap_or(self.pages - 1)
            .min(self.pages - 1)
            .max(first);
        let next = (self.page as isize + by).clamp(first as isize, last as isize) as usize;
        if next == self.page {
            return false;
        }
        self.page = next;
        true
    }

    pub(super) fn set_page(&mut self, target: usize) -> bool {
        if self.pages == 0 {
            return false;
        }
        let first = self.first_page.min(self.pages - 1);
        let last = self
            .last_page
            .unwrap_or(self.pages - 1)
            .min(self.pages - 1)
            .max(first);
        let next = target.clamp(first, last);
        if next == self.page {
            return false;
        }
        self.page = next;
        true
    }

    /// Up to seven one-based page indices around the active page (always including first and last)
    /// for quick jump pills in the reader bar.
    pub(super) fn jump_pages(&self) -> Vec<usize> {
        if self.pages <= 1 {
            return Vec::new();
        }
        let first = self.first_page.min(self.pages - 1);
        let last = self
            .last_page
            .unwrap_or(self.pages - 1)
            .min(self.pages - 1)
            .max(first);
        if first == last {
            return Vec::new();
        }
        let mut out = vec![first, last, self.page];
        for offset in 1..=2usize {
            out.push(self.page.saturating_sub(offset).max(first));
            out.push((self.page + offset).min(last));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub(super) fn image(&self) -> Option<Arc<RenderImage>> {
        self.drawn.get(&(self.page, self.zoom.width())).cloned()
    }

    /// Drop the drawn pages furthest from the one on screen.
    pub(super) fn trim(&mut self) {
        while self.drawn.len() > CACHED_PAGES {
            let current = self.page;
            let furthest = self
                .drawn
                .keys()
                .max_by_key(|key| (key.0.abs_diff(current), key.0, key.1))
                .copied();
            match furthest {
                Some(key) => {
                    self.drawn.remove(&key);
                }
                None => return,
            }
        }
    }
}

pub(super) fn reader_page_element(reader: &Reader, palette: Palette) -> AnyElement {
    if let Some(image) = reader.image() {
        if reader.zoom == Zoom::Close {
            return div()
                .id("document-page-zoom-scroll")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_scroll()
                .p_2()
                .child(img(image).w(px(1200.)))
                .into_any_element();
        }
        return div()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .p_2()
            .child(img(image).object_fit(ObjectFit::Contain).size_full())
            .into_any_element();
    }
    let message = if reader.loading {
        "Reading the original…".to_owned()
    } else if let Some(error) = &reader.error {
        error.clone()
    } else if reader.pages == 0 {
        "No page is on screen yet.".to_owned()
    } else {
        "Drawing this page…".to_owned()
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(message)
        .into_any_element()
}

impl Documents {
    pub(super) fn open_original(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.reader.loading = true;
        self.reader.id = Some(id.clone());
        cx.notify();

        let expected = id.clone();
        let work = shell::run(async move { shell.document_original(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.reader.id.as_deref() != Some(expected.as_str()) {
                    return;
                }
                match result {
                    Ok(bytes) => {
                        view.reader.bytes = Some(Arc::new(bytes));
                        view.reader.error = None;
                        view.reader.loading = false;
                        view.ask_page_count(cx);
                    }
                    Err(error) => {
                        view.reader.loading = false;
                        view.reader.error = Some(error.to_string());
                        view.reader.reading_text = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The page count comes from the renderer, because the document belongs to its thread.
    pub(super) fn ask_page_count(&mut self, cx: &mut Context<Self>) {
        let (Some(renderer), Some(bytes), Some(id)) = (
            self.renderer.as_ref(),
            self.reader.bytes.clone(),
            self.reader.id.clone(),
        ) else {
            return;
        };
        let answer = renderer.info(bytes);
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                if view.reader.id.as_deref() != Some(id.as_str()) {
                    return;
                }
                match result {
                    Ok(pages) => {
                        view.reader.pages = pages;
                        if let Some(range) = view
                            .detail
                            .value
                            .as_ref()
                            .and_then(|detail| detail.document.page_range)
                        {
                            view.reader.first_page = 0;
                            view.reader.last_page = Some(
                                usize::try_from(range.last.saturating_sub(range.first))
                                    .unwrap_or_default(),
                            );
                        } else {
                            view.reader.first_page = 0;
                            view.reader.last_page = None;
                        }
                        view.reader.page = view.reader.first_page.min(pages.saturating_sub(1));
                    }
                    Err(error) => {
                        view.reader.error = Some(error.to_string());
                        view.reader.reading_text = true;
                    }
                }
                view.request_page(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Ask for the page on screen unless it is drawn already.
    pub(super) fn request_page(&mut self, cx: &mut Context<Self>) {
        let (Some(renderer), Some(bytes), Some(id)) = (
            self.renderer.as_ref(),
            self.reader.bytes.clone(),
            self.reader.id.clone(),
        ) else {
            return;
        };
        if let Some(error) = &self.renderer_error {
            self.reader.error = Some(error.clone());
            return;
        }
        let width = self.reader.zoom.width();
        if self.reader.drawn.contains_key(&(self.reader.page, width)) {
            return;
        }
        let page = self.reader.page;
        let answer = renderer.render(bytes, page, width);
        let expected = (id, page, width);
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                if view.reader.id.as_deref() != Some(expected.0.as_str())
                    || view.reader.page != expected.1
                {
                    return;
                }
                match result {
                    Ok(image) => {
                        view.reader.error = None;
                        view.reader.drawn.insert((expected.1, expected.2), image);
                        view.reader.trim();
                    }
                    Err(error) => {
                        view.reader.error = Some(error.to_string());
                        view.reader.reading_text = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn go_to_page(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.reader.next_page(by) {
            self.request_page(cx);
            cx.notify();
        }
    }

    pub fn go_to_page_index(&mut self, target: usize, cx: &mut Context<Self>) {
        if self.reader.set_page(target) {
            self.request_page(cx);
            cx.notify();
        }
    }

    pub fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        let next = self.reader.zoom.other();
        self.set_zoom(next, cx);
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(Zoom::Close, cx);
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(Zoom::Page, cx);
    }

    pub fn on_wheel_zoom(&mut self, event: &gpui::ScrollEvent, cx: &mut Context<Self>) {
        let dy = match event.delta {
            gpui::ScrollDelta::Pixels(d) => f32::from(d.y),
            gpui::ScrollDelta::Lines(d) => d.y,
        };
        if dy > 0. {
            self.zoom_in(cx);
        } else if dy < 0. {
            self.zoom_out(cx);
        }
    }

    pub(super) fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        if self.reader.zoom != zoom {
            self.reader.zoom = zoom;
            self.request_page(cx);
            cx.notify();
        }
    }

    pub(super) fn show_text(&mut self, show: bool, cx: &mut Context<Self>) {
        self.reader.reading_text = show;
        cx.notify();
    }

    /// The page, or the reason there is none. A drawn page is the whole of this element when it
    /// exists; otherwise the caller shows the text beside it.
    pub(super) fn page_element(&self, palette: Palette) -> AnyElement {
        reader_page_element(&self.reader, palette)
    }
}
