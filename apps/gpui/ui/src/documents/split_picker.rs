//! Visual PDF split review over the document reader's thumbnail cache.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use gpui::{
    div, img, prelude::*, px, rgb, rgba, Context, Entity, FontWeight, ObjectFit, ScrollHandle,
    StyledImage, Window,
};
use sjel_gpui_core::document_queue::SavedSplit;
use sjel_gpui_core::shell::{
    self, DocumentCollection, DocumentDetail, DocumentPageRange, DocumentSplitChildInput,
    DocumentSplitRequest, DocumentSummary, ShellError,
};

use crate::data::Remote;
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Choice, Input, Select, TextInput};
use crate::ely::primitives::Tooltip;
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

use super::{
    split_preview::BoundaryPreview,
    viewer::{Reader, Zoom},
    Documents,
};

pub(super) const PAGES_PER_GRID: usize = 12;

#[derive(Default)]
pub(super) struct SplitPicker {
    pub(super) offset: usize,
    draft: Option<SplitDraft>,
    collections: Remote<Vec<DocumentCollection>>,
    collection_id: Option<String>,
    scroll: ScrollHandle,
    pub(super) preview: Option<BoundaryPreview>,
    matching: Remote<Option<DocumentPageRange>>,
    saved: Option<SavedSplit>,
}

impl SplitPicker {
    pub(super) fn matched_span(&self) -> Option<DocumentPageRange> {
        self.matching.value.flatten()
    }

    pub(super) fn has_cut(&self, before: u32) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| draft.cuts.contains(&before))
    }
}

struct SplitDraft {
    cuts: BTreeSet<u32>,
    names: BTreeMap<u32, Entity<TextInput>>,
    edited: bool,
    pending_names: BTreeMap<u32, String>,
}

impl SplitDraft {
    fn new(detail: &DocumentDetail, pages: u32) -> Self {
        let cuts = detail
            .split_suggestions
            .as_ref()
            .filter(|suggestion| suggestion.page_count == pages)
            .into_iter()
            .flat_map(|suggestion| &suggestion.boundaries)
            .map(|boundary| boundary.before_page)
            .filter(|page| (2..=pages).contains(page))
            .collect();
        Self {
            cuts,
            names: BTreeMap::new(),
            edited: false,
            pending_names: BTreeMap::new(),
        }
    }

    fn toggle(&mut self, before: u32) {
        self.edited = true;
        if self.cuts.remove(&before) {
            self.names.remove(&before);
            self.pending_names.remove(&before);
        } else {
            self.cuts.insert(before);
        }
    }

    fn ranges(&self, pages: u32) -> impl Iterator<Item = DocumentPageRange> + '_ {
        let starts = std::iter::once(1).chain(self.cuts.iter().copied());
        let ends = self.cuts.iter().map(|before| before - 1).chain([pages]);
        starts
            .zip(ends)
            .map(|(first, last)| DocumentPageRange { first, last })
    }
}

pub(super) fn can_split(document: &DocumentSummary) -> bool {
    document.status != "archived"
        && document.parent_id.is_none()
        && document.media_type == "application/pdf"
        && document.workflow_stage != "split"
}

fn default_name(filename: &str, range: DocumentPageRange) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Document");
    let suffix = format!(" pages {}-{}", range.first, range.last);
    let mut end = stem.len().min(251 - suffix.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &stem[..end])
}

fn pdf_filename(name: &str) -> String {
    if name
        .as_bytes()
        .get(name.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".pdf"))
    {
        name.to_owned()
    } else {
        format!("{name}.pdf")
    }
}

impl Documents {
    pub(super) fn begin_split_picker(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.split_picker.is_some() {
            return;
        }
        self.workflow_error = None;
        let saved = self.split_queue.state.drafts.get(&id).cloned();
        self.split_picker = Some(SplitPicker {
            offset: saved.as_ref().map_or(
                self.reader.page / PAGES_PER_GRID * PAGES_PER_GRID,
                |draft| draft.offset,
            ),
            collection_id: saved.as_ref().and_then(|draft| draft.collection_id.clone()),
            saved,
            ..SplitPicker::default()
        });
        if self.filter == super::Filter::BigDocuments {
            self.split_queue.state.resume = Some(id);
            self.save_split_queue(cx);
        }
        self.load_split_collections(cx);
        self.request_thumbnails(cx);
        cx.notify();
    }

    pub(super) fn open_contained_split(
        &mut self,
        id: String,
        part: String,
        cx: &mut Context<Self>,
    ) {
        self.set_duplicate_mode(false, cx);
        self.open(id.clone(), cx);
        if self.selected.as_ref() != Some(&id) {
            return;
        }
        self.begin_split_picker(cx);
        let (Some(picker), Ok(shell)) = (&mut self.split_picker, self.repo.shell()) else {
            return;
        };
        picker.matching = Remote::new_loading();
        let expected = id.clone();
        let work = shell::run(async move { shell.contained_document_pages(&id, &part).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_ref() != Some(&expected) {
                    return;
                }
                let Some(picker) = &mut view.split_picker else {
                    return;
                };
                let auto_apply = picker.saved.is_none()
                    && picker.draft.as_ref().is_none_or(|draft| !draft.edited);
                picker.matching.settle(result);
                if auto_apply {
                    view.apply_matched_span(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_matched_span(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.split_picker else {
            return;
        };
        if let Some(Some(range)) = picker.matching.value {
            if let Some(draft) = &mut picker.draft {
                add_matched_cuts(draft, range, self.reader.pages as u32);
            }
            picker.offset = (range.first as usize - 1) / PAGES_PER_GRID * PAGES_PER_GRID;
            self.request_thumbnails(cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_split_cut(&mut self, before: u32, cx: &mut Context<Self>) {
        if self.workflow_busy.as_deref() == Some("split") {
            return;
        }
        if let Some(draft) = self
            .split_picker
            .as_mut()
            .and_then(|picker| picker.draft.as_mut())
        {
            draft.toggle(before);
            self.workflow_error = None;
            self.remember_split_draft(cx);
            cx.notify();
        }
    }

    fn remember_split_draft(&mut self, cx: &mut Context<Self>) {
        if self.filter != super::Filter::BigDocuments {
            return;
        }
        let (Some(id), Some(picker)) = (self.selected.clone(), &self.split_picker) else {
            return;
        };
        let Some(draft) = &picker.draft else {
            return;
        };
        let mut names = draft.pending_names.clone();
        for (start, input) in &draft.names {
            names.insert(*start, input.read(cx).text().to_owned());
        }
        self.split_queue.state.drafts.insert(
            id,
            SavedSplit {
                cuts: draft.cuts.clone(),
                names,
                offset: picker.offset,
                collection_id: picker.collection_id.clone(),
            },
        );
        self.save_split_queue(cx);
    }

    fn defer_split(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.split_queue.state.deferred.insert(id.clone());
        self.close_split_picker(cx);
        self.save_split_queue(cx);
        self.advance_split_queue(&id, cx);
    }

    pub(super) fn close_split_picker(&mut self, cx: &mut Context<Self>) {
        self.remember_split_draft(cx);
        self.split_picker = None;
        self.workflow_error = None;
        if self.reader.show_thumbnails {
            self.request_thumbnails(cx);
        } else {
            self.reader.thumbnail_task = None;
            self.reader.thumbnail_pending.clear();
        }
        cx.notify();
    }

    fn load_split_collections(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(picker)) = (self.selected.clone(), &mut self.split_picker) else {
            return;
        };
        match self.repo.shell() {
            Ok(shell) => {
                picker.collections = Remote::new_loading();
                let work = shell::run(async move { shell.document_collections().await });
                cx.spawn(async move |this, cx| {
                    let result = work.await.unwrap_or(Err(ShellError::Unreachable));
                    let _ = this.update(cx, |view, cx| {
                        if view.selected.as_ref() == Some(&id) {
                            if let Some(picker) = &mut view.split_picker {
                                picker.collections.settle(result);
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
            Err(error) => picker.collections.settle(Err(error)),
        }
        cx.notify();
    }

    fn step_split_grid(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.split_picker {
            let next = if forward {
                picker.offset + PAGES_PER_GRID
            } else {
                picker.offset.saturating_sub(PAGES_PER_GRID)
            };
            if next < self.reader.pages {
                picker.offset = next;
                picker.scroll.set_offset(gpui::point(px(0.), px(0.)));
                self.remember_split_draft(cx);
                self.request_thumbnails(cx);
                cx.notify();
            }
        }
    }

    fn submit_split(&mut self, next: bool, cx: &mut Context<Self>) {
        let (Some(id), Some(detail), Some(picker)) = (
            self.selected.clone(),
            &self.detail.value,
            &self.split_picker,
        ) else {
            return;
        };
        let (Some(draft), Ok(page_count)) = (&picker.draft, u32::try_from(self.reader.pages))
        else {
            return;
        };
        if self.workflow_busy.is_some() || draft.cuts.is_empty() || !split_ready(detail, page_count)
        {
            return;
        }
        let children: Vec<_> = draft
            .ranges(page_count)
            .map(|range| {
                let name = draft
                    .names
                    .get(&range.first)
                    .map_or("", |name| name.read(cx).text().trim())
                    .to_owned();
                let name = if name.is_empty() {
                    default_name(&detail.document.filename, range)
                } else {
                    name
                };
                DocumentSplitChildInput {
                    first: range.first,
                    last: range.last,
                    filename: pdf_filename(&name),
                }
            })
            .collect();
        let mut filenames = BTreeSet::new();
        for child in &children {
            let name = &child.filename;
            if name.len() > 255 || name.contains(['/', '\\']) || name.chars().any(char::is_control)
            {
                self.workflow_error = Some(format!(
                    "Enter a valid PDF filename for the piece starting on page {}.",
                    child.first
                ));
                cx.notify();
                return;
            }
            if !filenames.insert(name) {
                self.workflow_error = Some("Give each piece a different filename.".into());
                cx.notify();
                return;
            }
        }
        let collection_id = picker.collection_id.clone();
        let request = DocumentSplitRequest {
            page_count,
            children,
            collection_id,
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.workflow_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        for name in draft.names.values() {
            name.update(cx, |name, cx| name.set_disabled(true, cx));
        }
        self.workflow_busy = Some("split".into());
        self.workflow_error = None;
        cx.notify();
        let expected = id.clone();
        let work = shell::run(async move {
            let children = shell.split_document(&id, &request).await?;
            let parent = shell.document_detail(&id).await;
            Ok((children, parent))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() != Some(expected.as_str()) {
                    return;
                }
                view.workflow_busy = None;
                match result {
                    Ok((children, parent)) => {
                        view.split_picker = None;
                        view.close_split_picker(cx);
                        let queue = &mut view.split_queue.state;
                        let was_resumed = queue.resume.as_ref() == Some(&expected);
                        if was_resumed {
                            queue.resume = None;
                        }
                        let removed = queue.drafts.remove(&expected).is_some();
                        let deferred = queue.deferred.remove(&expected);
                        if was_resumed || removed || deferred {
                            view.save_split_queue(cx);
                        }
                        let detail = view.detail.value.as_mut();
                        if let Some(detail) = detail {
                            match parent {
                                Ok(parent) => *detail = parent,
                                Err(error) => {
                                    detail.document.workflow_stage = "split".into();
                                    detail.children.clone_from(&children);
                                    view.workflow_error = Some(format!(
                                        "Split saved. The source record could not refresh: {error}"
                                    ));
                                }
                            }
                            if let Some(rows) = &mut view.rows.value {
                                if let Some(row) = rows.iter_mut().find(|row| row.id == expected) {
                                    row.clone_from(&detail.document);
                                }
                                if !view.search_applied {
                                    for child in children.into_iter().rev() {
                                        super::upsert_uploaded_document(rows, child);
                                    }
                                }
                            }
                        }
                        view.reader_tab = 0;
                        view.page_offset = 0;
                        view.order_rev = view.order_rev.wrapping_add(1);
                        if next {
                            view.advance_split_queue(&expected, cx);
                        }
                    }
                    Err(error) => {
                        if let Some(draft) = view
                            .split_picker
                            .as_ref()
                            .and_then(|picker| picker.draft.as_ref())
                        {
                            for name in draft.names.values() {
                                name.update(cx, |name, cx| name.set_disabled(false, cx));
                            }
                        }
                        view.workflow_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn split_picker_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let saving = self.workflow_busy.as_deref() == Some("split");
        let queue = self.filter == super::Filter::BigDocuments;
        let title = self.detail.value.as_ref().map_or_else(
            || "Split PDF".to_owned(),
            |detail| format!("Split · {}", detail.document.filename),
        );
        let base = div().size_full().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .when(queue, |bar| {
                    bar.child(
                        Button::new("split-defer", "Defer")
                            .size(ControlSize::Sm)
                            .disabled(saving)
                            .on_click(cx.listener(|view, _, _, cx| view.defer_split(cx))),
                    )
                })
                .child(
                    Button::new("split-back", "Back to reader")
                        .size(ControlSize::Sm)
                        .disabled(saving)
                        .on_click(cx.listener(|view, _, _, cx| view.close_split_picker(cx))),
                ),
        );
        let Some(detail) = &self.detail.value else {
            let message = self
                .detail
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "Opening document…".into());
            return base.child(message);
        };
        if !can_split(&detail.document) || !detail.children.is_empty() {
            return base.child("Choose an unsplit original PDF.");
        }
        let Ok(pages) = u32::try_from(self.reader.pages) else {
            return base.child("This PDF has too many pages to split.");
        };
        if pages == 0 {
            let error = self.reader.error.as_ref().or(self.renderer_error.as_ref());
            return base.child(
                error
                    .cloned()
                    .unwrap_or_else(|| "Opening PDF pages…".into()),
            );
        }
        if pages == 1 {
            return base.child("This PDF has one page.");
        }
        if let Some(preview) = self
            .split_picker
            .as_ref()
            .and_then(|picker| picker.preview.as_ref())
        {
            return base.child(self.preview_boundary(preview, cx));
        }
        let Some(picker) = &mut self.split_picker else {
            return base;
        };
        let draft = picker.draft.get_or_insert_with(|| {
            let mut draft = SplitDraft::new(detail, pages);
            if let Some(saved) = picker.saved.take() {
                draft.edited = true;
                draft.cuts = saved
                    .cuts
                    .into_iter()
                    .filter(|page| (2..=pages).contains(page))
                    .collect();
                draft.pending_names = saved.names;
            } else if let Some(Some(range)) = picker.matching.value {
                add_matched_cuts(&mut draft, range, pages);
            }
            draft
        });
        let count = draft.cuts.len() + 1;
        let start = picker.offset;
        let end = (start + PAGES_PER_GRID).min(self.reader.pages);
        let grid = render_grid(
            start..end,
            &self.reader,
            detail,
            &draft.cuts,
            saving,
            palette,
            cx,
        );
        let pieces = render_pieces(draft, detail, pages, palette, window, cx)
            .children(render_collection(picker, saving, palette, cx));
        let ready = split_ready(detail, pages);
        let wide = window.viewport_size().width >= px(720.);
        let match_status = if picker.matching.loading {
            Some("Locating matching pages…".to_owned())
        } else if let Some(error) = &picker.matching.error {
            Some(format!("Could not locate the matching pages: {error}"))
        } else {
            picker.matching.value.map(|range| {
                range.map_or_else(
                    || "No unambiguous matching page span found.".into(),
                    |range| {
                        format!(
                            "Contained document matches pages {}–{}",
                            range.first, range.last
                        )
                    },
                )
            })
        };
        base.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("split-grid-prev", "Previous pages")
                        .size(ControlSize::Sm)
                        .disabled(start == 0)
                        .on_click(cx.listener(|view, _, _, cx| view.step_split_grid(false, cx))),
                )
                .child(
                    div()
                        .text_sm()
                        .child(format!("Pages {}–{end} of {pages}", start + 1)),
                )
                .child(
                    Button::new("split-grid-next", "Next pages")
                        .size(ControlSize::Sm)
                        .disabled(end == self.reader.pages)
                        .on_click(cx.listener(|view, _, _, cx| view.step_split_grid(true, cx))),
                ),
        )
        .children(match_status.map(|status| {
            div().flex().items_center().gap_2().child(status).when(
                picker.matching.value.flatten().is_some(),
                |bar| {
                    bar.child(
                        Button::new("split-apply-matched", "Use matched page span")
                            .size(ControlSize::Sm)
                            .disabled(saving)
                            .on_click(cx.listener(|view, _, _, cx| view.apply_matched_span(cx))),
                    )
                },
            )
        }))
        .child(
            div()
                .id("split-content")
                .flex_1()
                .min_h_0()
                .track_scroll(&picker.scroll)
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .gap_4()
                        .when(!wide, |body| body.flex_col())
                        .child(div().flex_1().min_w_0().child(grid))
                        .child(pieces.when(wide, |pieces| pieces.w(px(264.)).flex_none())),
                ),
        )
        .children(self.workflow_error.as_ref().map(|error| {
            div()
                .text_sm()
                .text_color(rgb(palette.red))
                .child(error.clone())
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .flex_wrap()
                .child(
                    Button::new(
                        "split-save",
                        if saving { "Splitting…" } else { "Split PDF" },
                    )
                    .primary()
                    .disabled(count < 2 || !ready || self.workflow_busy.is_some())
                    .on_click(cx.listener(|view, _, _, cx| view.submit_split(false, cx))),
                )
                .when(queue, |bar| {
                    bar.child(
                        Button::new("split-save-next", "Save and open next")
                            .disabled(count < 2 || !ready || self.workflow_busy.is_some())
                            .on_click(cx.listener(|view, _, _, cx| view.submit_split(true, cx))),
                    )
                })
                .when(!ready, |bar| {
                    bar.child(
                        Button::new(
                            "split-analyze",
                            if self.workflow_busy.is_some() {
                                "Analyzing…"
                            } else {
                                "Analyze pages"
                            },
                        )
                        .size(ControlSize::Sm)
                        .disabled(self.workflow_busy.is_some())
                        .on_click(cx.listener(|view, _, _, cx| view.analyze_selected_document(cx))),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(if !ready {
                            "Analyze every page before splitting."
                        } else if count < 2 {
                            "Add a cut between pages."
                        } else {
                            "The original stays in the library."
                        }),
                ),
        )
    }
}

fn render_grid(
    pages: Range<usize>,
    reader: &Reader,
    detail: &DocumentDetail,
    cuts: &BTreeSet<u32>,
    saving: bool,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> gpui::Div {
    let mut grid = div().flex().flex_wrap().gap_2().items_start();
    for page in pages {
        let number = page + 1;
        let preview = div()
            .w(px(94.))
            .h(px(114.))
            .bg(rgba(palette.surface))
            .rounded(px(4.))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        let preview = match reader.thumbnails.get(&page) {
            Some(image) => preview.bg(rgb(0xffffff)).child(
                img(image.clone())
                    .object_fit(ObjectFit::Contain)
                    .size_full(),
            ),
            None => preview.text_xs().text_color(rgb(palette.muted)).child(
                if reader.thumbnail_pending.contains(&page) {
                    "Loading…"
                } else {
                    "No preview"
                },
            ),
        };
        grid = grid.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .w(px(157.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(preview)
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("Page {number}")),
                        ),
                )
                .when(number < reader.pages, |card| {
                    let before = number as u32 + 1;
                    let cut = cuts.contains(&before);
                    let suggestion = detail.split_suggestions.as_ref().and_then(|suggestion| {
                        suggestion
                            .boundaries
                            .iter()
                            .find(|boundary| boundary.before_page == before)
                    });
                    let action = if cut { "Remove cut" } else { "Add cut" };
                    card.child(
                        div()
                            .id(("split-boundary", page))
                            .tooltip(Tooltip::text(suggestion.map_or_else(
                                || format!("{action} after page {number}"),
                                |evidence| {
                                    format!(
                                        "{action} after page {number} · {} · {}",
                                        evidence.marker, evidence.quote
                                    )
                                },
                            )))
                            .child(
                                div()
                                    .debug_selector(move || format!("split-cut-{before}"))
                                    .child(
                                        Button::new(
                                            ("split-cut", page),
                                            if cut { "Cut" } else { "+ Cut" },
                                        )
                                        .size(ControlSize::Sm)
                                        .variant(if cut {
                                            ButtonVariant::Secondary
                                        } else {
                                            ButtonVariant::Ghost
                                        })
                                        .disabled(saving)
                                        .on_click(
                                            cx.listener(move |view, _, _, cx| {
                                                view.toggle_split_cut(before, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .debug_selector(move || format!("split-inspect-{before}"))
                                    .child(
                                        Button::new(("inspect-split-boundary", page), "Inspect")
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .disabled(saving)
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.inspect_boundary(page + 1, Zoom::Width, cx)
                                            })),
                                    ),
                            ),
                    )
                }),
        );
    }
    grid
}

fn render_pieces(
    draft: &mut SplitDraft,
    detail: &DocumentDetail,
    pages: u32,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Documents>,
) -> gpui::Div {
    let count = draft.cuts.len() + 1;
    let ranges: Vec<_> = draft.ranges(pages).collect();
    let mut pieces =
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                "{count} {}",
                if count == 1 { "piece" } else { "pieces" }
            )));
    for range in ranges {
        let first = range.first;
        let last = range.last;
        let pages = last - first + 1;
        let placeholder = default_name(&detail.document.filename, range);
        let name = draft.names.entry(first).or_insert_with(|| {
            let name = cx.new(|cx| {
                TextInput::new(window, cx)
                    .placeholder(placeholder.clone())
                    .label(format!("Filename for piece starting on page {first}"))
            });
            if let Some(text) = draft.pending_names.remove(&first) {
                name.update(cx, |input, cx| input.set_text(text, cx));
            }
            name
        });
        if name.read(cx).placeholder_text().as_ref() != placeholder {
            name.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        }
        pieces = pieces.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(if first == last {
                            format!("Page {first} · 1 page")
                        } else {
                            format!("Pages {first}–{last} · {pages} pages")
                        }),
                )
                .child(Input::new(name).size(ControlSize::Sm)),
        );
    }

    pieces
}

fn render_collection(
    picker: &SplitPicker,
    saving: bool,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> Option<gpui::Div> {
    let field = div().flex().flex_col().gap_1();
    if let Some(collections) = picker
        .collections
        .value
        .as_ref()
        .filter(|items| !items.is_empty())
    {
        let choices = std::iter::once(Choice::new("", "No collection")).chain(
            collections
                .iter()
                .map(|collection| Choice::new(collection.id.clone(), collection.name.clone())),
        );
        let entity = cx.entity();
        return Some(
            field.child(
                Select::new("split-collection", choices)
                    .selected(picker.collection_id.clone().unwrap_or_default())
                    .label("Collection for all pieces")
                    .size(ControlSize::Sm)
                    .disabled(saving)
                    .on_change(move |id, _, cx| {
                        entity.update(cx, |view, cx| {
                            if let Some(picker) = &mut view.split_picker {
                                picker.collection_id = (!id.is_empty()).then(|| id.to_string());
                                view.remember_split_draft(cx);
                                cx.notify();
                            }
                        });
                    }),
            ),
        );
    }
    picker.collections.error.as_ref().map(|error| {
        field
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("Collections unavailable: {error}")),
            )
            .child(
                Button::new("split-collections-retry", "Retry collections")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(saving)
                    .on_click(cx.listener(|view, _, _, cx| view.load_split_collections(cx))),
            )
    })
}

fn add_matched_cuts(draft: &mut SplitDraft, range: DocumentPageRange, pages: u32) {
    if range.first == 0 || range.last > pages || range.first > range.last {
        return;
    }
    draft
        .cuts
        .retain(|cut| *cut <= range.first || *cut > range.last);
    draft
        .names
        .retain(|start, _| *start == 1 || draft.cuts.contains(start));
    draft
        .pending_names
        .retain(|start, _| *start == 1 || draft.cuts.contains(start));
    if range.first > 1 {
        draft.cuts.insert(range.first);
    }
    if range.last < pages {
        draft.cuts.insert(range.last + 1);
    }
}

fn split_ready(detail: &DocumentDetail, pages: u32) -> bool {
    detail.pages.len() == pages as usize
        && detail
            .pages
            .iter()
            .enumerate()
            .all(|(index, page)| page.page_number == index as u32 + 1)
        && detail.children.is_empty()
        && can_split(&detail.document)
}
