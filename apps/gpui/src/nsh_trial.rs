//! Opens the NSH Documents & Mail screens (`sjel_gpui_ui::nsh_documents`, `sjel_gpui_ui::nsh_mail`)
//! in a window of their own, drawn by GPUI Fast (`docs/plans/gpui.md` §11).
//! `SJEL_NSH_TRIAL=1` (or `documents` / `mail`) asks for it at launch, with the `sjel.pdf` native
//! component registered so PDF originals and offline keeping work on the Mac trial window as well
//! as on the phone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, img, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FontWeight, ObjectFit,
    RenderImage, ScrollHandle, StyledImage, Window,
};
use nsh_platform_gpui::{
    open_app_window, set_ui_sets, AppOptions, AppSource, ComponentContext, NativeComponent, Root,
};
use sjel_gpui_core::device::Source;
use sjel_gpui_core::originals::Originals;
use sjel_gpui_core::shell::{self, Shell};
use sjel_gpui_ui::documents::PdfRenderer;
use sjel_gpui_ui::nsh_documents::{
    connect, set_nsh_page, set_originals, set_pdf_viewer, NshHostScreen, NshPage, PDF_COMPONENT,
};
use sjel_gpui_ui::nsh_mail;
use sjel_gpui_ui::theme::{tint, Palette};

const PAGE_FIT_WIDTH: u32 = 1200;
const PAGE_ZOOM_WIDTH: u32 = 2400;
const THUMB_WIDTH: u32 = 160;

fn mac_originals() -> Option<Originals> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let cache = home.join("Library/Caches/dev.sjel.desktop/nsh-originals");
    let kept = home.join("Library/Application Support/dev.sjel.desktop/nsh-kept-originals");
    Some(Originals::new(cache, kept))
}

pub fn open(cx: &mut App) {
    set_ui_sets(vec![nsh_base::registry()]);
    if let Some(originals) = mac_originals() {
        set_originals(originals);
    }
    set_pdf_viewer();
    if std::env::var("SJEL_NSH_TRIAL").as_deref() == Ok("mail") {
        set_nsh_page(NshPage::Mail);
    } else {
        set_nsh_page(NshPage::Documents);
    }
    let endpoint = &sjel_gpui_core::environment::current().endpoint;
    if let Ok(shell) = Shell::new(endpoint) {
        let shell = Arc::new(shell);
        connect(Source::Shell(shell.clone()));
        nsh_mail::connect(Source::Shell(shell));
    }
    let options = AppOptions {
        title: "Sjel (NSH)".to_owned(),
        ..AppOptions::default()
    };
    options.components.register(
        PDF_COMPONENT,
        Rc::new(|nsh_cx: ComponentContext| -> Box<dyn NativeComponent> {
            Box::new(MacPdfComponent::new(nsh_cx))
        }),
    );
    let source = AppSource::Rust(Root::of::<NshHostScreen>());
    if let Err(error) = open_app_window(source, options, cx) {
        eprintln!("sjel-gpui: the NSH trial window did not open: {error}");
    }
}

struct MacPdfComponent {
    nsh_cx: ComponentContext,
    path: Option<String>,
    view: Option<Entity<NshPdfView>>,
}

impl MacPdfComponent {
    fn new(nsh_cx: ComponentContext) -> Self {
        Self {
            nsh_cx,
            path: None,
            view: None,
        }
    }
}

impl NativeComponent for MacPdfComponent {
    fn render(&mut self, _window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        self.view
            .as_ref()
            .map(|view| view.clone().into_any_element())
    }

    fn set_prop(
        &mut self,
        name: &str,
        value: Option<&serde_json::Value>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        if name != "path" {
            return;
        }
        let next_path = value
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if self.path == next_path && self.view.is_some() {
            return;
        }
        self.path = next_path.clone();
        let nsh_cx = self.nsh_cx.clone();
        match &self.view {
            Some(entity) => {
                entity.update(cx, |view, cx| {
                    view.load_path(next_path, cx);
                });
            }
            None => {
                let entity = cx.new(|cx| {
                    let mut view = NshPdfView::new(nsh_cx);
                    view.load_path(next_path, cx);
                    view
                });
                self.view = Some(entity);
            }
        }
        self.nsh_cx.redraw();
    }
}

struct NshPdfView {
    nsh_cx: ComponentContext,
    renderer: Option<Arc<dyn PdfRenderer>>,
    path: Option<String>,
    bytes: Option<Arc<Vec<u8>>>,
    pages: usize,
    page: usize,
    zoomed: bool,
    show_thumbnails: bool,
    drawn: BTreeMap<(usize, u32), Arc<RenderImage>>,
    thumbnails: BTreeMap<usize, Arc<RenderImage>>,
    thumb_scroll: ScrollHandle,
    loading: bool,
    error: Option<String>,
}

impl NshPdfView {
    fn new(nsh_cx: ComponentContext) -> Self {
        let (renderer, error) = match sjel_gpui_ui::pdf_kit::default_renderer() {
            Ok(renderer) => (Some(renderer), None),
            Err(err) => (None, Some(err)),
        };
        Self {
            nsh_cx,
            renderer,
            path: None,
            bytes: None,
            pages: 0,
            page: 0,
            zoomed: false,
            show_thumbnails: false,
            drawn: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            thumb_scroll: ScrollHandle::new(),
            loading: false,
            error,
        }
    }

    fn target_width(&self) -> u32 {
        if self.zoomed {
            PAGE_ZOOM_WIDTH
        } else {
            PAGE_FIT_WIDTH
        }
    }

    fn load_path(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.path = path.clone();
        self.bytes = None;
        self.pages = 0;
        self.page = 0;
        self.drawn.clear();
        self.thumbnails.clear();
        self.error = None;
        let Some(path) = path else {
            self.loading = false;
            self.nsh_cx.redraw();
            cx.notify();
            return;
        };
        let Some(renderer) = self.renderer.clone() else {
            self.loading = false;
            self.nsh_cx.redraw();
            cx.notify();
            return;
        };
        self.loading = true;
        self.nsh_cx.redraw();
        cx.notify();

        let expected = path.clone();
        let read_work = shell::run(async move {
            tokio::task::spawn_blocking(move || std::fs::read(&path))
                .await
                .map_err(|_| "Could not read PDF file.".to_owned())
                .and_then(|res| res.map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let loaded = read_work
                .await
                .unwrap_or_else(|| Err("Could not read PDF file.".to_owned()));
            let bytes = match loaded {
                Ok(bytes) => Arc::new(bytes),
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        if view.path.as_deref() == Some(expected.as_str()) {
                            view.loading = false;
                            view.error = Some(err);
                            view.nsh_cx.redraw();
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let info = renderer.info(bytes.clone()).await;
            let _ = this.update(cx, |view, cx| {
                if view.path.as_deref() != Some(expected.as_str()) {
                    return;
                }
                match info {
                    Ok(Ok(pages)) => {
                        view.bytes = Some(bytes);
                        view.pages = pages;
                        view.page = 0;
                        view.request_page(cx);
                    }
                    Ok(Err(err)) => {
                        view.loading = false;
                        view.error = Some(err.to_string());
                    }
                    Err(_) => {
                        view.loading = false;
                        view.error = Some("The PDF worker stopped.".into());
                    }
                }
                view.nsh_cx.redraw();
                cx.notify();
            });
        })
        .detach();
    }

    fn request_page(&mut self, cx: &mut Context<Self>) {
        if self.show_thumbnails {
            self.request_thumbnails(cx);
        }
        let width = self.target_width();
        let page = self.page;
        if self.drawn.contains_key(&(page, width)) {
            self.loading = false;
            return;
        }
        let (Some(renderer), Some(bytes), Some(expected)) =
            (self.renderer.clone(), self.bytes.clone(), self.path.clone())
        else {
            return;
        };
        self.loading = true;
        let rx = renderer.render(bytes, page, width);
        cx.spawn(async move |this, cx| {
            let rendered = rx.await;
            let _ = this.update(cx, |view, cx| {
                if view.path.as_deref() != Some(expected.as_str()) {
                    return;
                }
                view.loading = false;
                match rendered {
                    Ok(Ok(image)) => {
                        view.drawn.insert((page, width), image);
                    }
                    Ok(Err(err)) => view.error = Some(err.to_string()),
                    Err(_) => view.error = Some("The PDF worker stopped.".into()),
                }
                view.nsh_cx.redraw();
                cx.notify();
            });
        })
        .detach();
    }

    fn request_thumbnails(&mut self, cx: &mut Context<Self>) {
        if !self.show_thumbnails || self.pages == 0 {
            return;
        }
        let (Some(renderer), Some(bytes), Some(expected)) =
            (self.renderer.clone(), self.bytes.clone(), self.path.clone())
        else {
            return;
        };
        let start = self.page.saturating_sub(8);
        let end = (self.page + 8).min(self.pages - 1);
        for page in start..=end {
            if self.thumbnails.contains_key(&page) {
                continue;
            }
            let rx = renderer.render(bytes.clone(), page, THUMB_WIDTH);
            let expected = expected.clone();
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(image)) = rx.await {
                    let _ = this.update(cx, |view, cx| {
                        if view.path.as_deref() == Some(expected.as_str()) {
                            view.thumbnails.insert(page, image);
                            view.nsh_cx.redraw();
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    fn step_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.pages == 0 {
            return;
        }
        let next = (self.page as isize + delta).clamp(0, self.pages as isize - 1) as usize;
        if next != self.page {
            self.page = next;
            self.request_page(cx);
            self.nsh_cx.redraw();
            cx.notify();
        }
    }

    fn go_to_page(&mut self, target: usize, cx: &mut Context<Self>) {
        if self.pages == 0 {
            return;
        }
        let next = target.min(self.pages - 1);
        if next != self.page {
            self.page = next;
            self.request_page(cx);
            self.nsh_cx.redraw();
            cx.notify();
        }
    }

    fn set_zoom(&mut self, zoomed: bool, cx: &mut Context<Self>) {
        if self.zoomed != zoomed {
            self.zoomed = zoomed;
            self.request_page(cx);
            self.nsh_cx.redraw();
            cx.notify();
        }
    }

    fn toggle_thumbnails(&mut self, cx: &mut Context<Self>) {
        self.show_thumbnails = !self.show_thumbnails;
        if self.show_thumbnails {
            self.request_thumbnails(cx);
        }
        self.nsh_cx.redraw();
        cx.notify();
    }

    fn open_in_preview(&self) {
        let (Some(renderer), Some(bytes), Some(path)) =
            (&self.renderer, &self.bytes, self.path.as_deref())
        else {
            return;
        };
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("document.pdf");
        renderer.open_external(filename, bytes.as_slice());
    }
}

impl Render for NshPdfView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::dark();
        let page = self.page;
        let pages = self.pages;
        let zoomed = self.zoomed;
        let show_thumbnails = self.show_thumbnails;
        let page_label = if pages == 0 {
            "Page —".to_owned()
        } else {
            format!("Page {} of {}", page + 1, pages)
        };

        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .bg(rgba(palette.surface))
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                Button::new("nsh-pdf-prev")
                    .small()
                    .label("‹ Page")
                    .disabled(page == 0)
                    .on_click(cx.listener(|view, _, _, cx| view.step_page(-1, cx))),
            )
            .child(
                Button::new("nsh-pdf-next")
                    .small()
                    .label("Page ›")
                    .disabled(pages == 0 || page + 1 >= pages)
                    .on_click(cx.listener(|view, _, _, cx| view.step_page(1, cx))),
            )
            .child(
                div()
                    .min_w(px(84.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(page_label),
            )
            .child(
                Button::new("nsh-pdf-zoom")
                    .small()
                    .label(if zoomed { "2×" } else { "Fit" })
                    .on_click(cx.listener(move |view, _, _, cx| view.set_zoom(!zoomed, cx))),
            )
            .when(pages > 1, |bar| {
                bar.child(
                    Button::new("nsh-pdf-thumbs")
                        .small()
                        .label(if show_thumbnails {
                            "Hide thumbs"
                        } else {
                            "Thumbnails"
                        })
                        .on_click(cx.listener(|view, _, _, cx| view.toggle_thumbnails(cx))),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("nsh-pdf-preview")
                    .small()
                    .label("Open in Preview")
                    .disabled(self.bytes.is_none())
                    .on_click(cx.listener(|view, _, _, _| view.open_in_preview())),
            );

        let rail: Option<AnyElement> = (show_thumbnails && pages > 1).then(|| {
            let mut list = div().flex().flex_col().gap_2().p_2();
            for page_ix in 0..pages {
                let active = page_ix == page;
                let page_num = page_ix + 1;
                let preview = match self.thumbnails.get(&page_ix).cloned() {
                    Some(image) => div()
                        .w_full()
                        .h(px(112.))
                        .rounded(px(3.))
                        .bg(rgb(0xffffff))
                        .overflow_hidden()
                        .child(img(image).size_full().object_fit(ObjectFit::Contain))
                        .into_any_element(),
                    None => div()
                        .w_full()
                        .h(px(112.))
                        .rounded(px(3.))
                        .bg(rgba(palette.raised))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("p.{page_num}"))
                        .into_any_element(),
                };
                list = list.child(
                    div()
                        .id(("nsh-pdf-thumb", page_ix))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .p_1p5()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .bg(rgba(if active {
                            tint(palette.accent, 0x22)
                        } else {
                            palette.surface
                        }))
                        .border_1()
                        .border_color(rgba(if active {
                            palette.accent
                        } else {
                            palette.border
                        }))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.go_to_page(page_ix, cx);
                        }))
                        .child(preview)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .text_color(rgb(if active {
                                    palette.accent
                                } else {
                                    palette.muted
                                }))
                                .child(format!("p.{page_num}")),
                        ),
                );
            }
            div()
                .id("nsh-pdf-thumb-rail")
                .w(px(124.))
                .flex_shrink_0()
                .h_full()
                .min_h_0()
                .rounded(px(6.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .overflow_y_scroll()
                .track_scroll(&self.thumb_scroll)
                .vertical_scrollbar(&self.thumb_scroll)
                .child(list)
                .into_any_element()
        });

        let width = self.target_width();
        let canvas = if let Some(error) = &self.error {
            div()
                .flex_1()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.red))
                .child(error.clone())
                .into_any_element()
        } else if let Some(image) = self.drawn.get(&(page, width)).cloned() {
            if zoomed {
                div()
                    .id("nsh-pdf-zoom-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.surface))
                    .child(img(image).w(px(1200.)))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.surface))
                    .child(img(image).size_full().object_fit(ObjectFit::Contain))
                    .into_any_element()
            }
        } else {
            div()
                .flex_1()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(if self.loading {
                    "Drawing PDF page…"
                } else {
                    "Select a PDF original to view."
                })
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.panel_surface))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_2()
                    .p_2()
                    .when_some(rail, |el, rail| el.child(rail))
                    .child(canvas),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_register_sjel_pdf_component() {
        let options = AppOptions::default();
        options.components.register(
            PDF_COMPONENT,
            Rc::new(|nsh_cx: ComponentContext| -> Box<dyn NativeComponent> {
                Box::new(MacPdfComponent::new(nsh_cx))
            }),
        );
        let debug = format!("{:?}", options.components);
        assert!(debug.contains(PDF_COMPONENT));
    }
}
