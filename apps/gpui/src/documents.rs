//! Native Documents library and reader for the GPUI desktop surface.
//!
//! The list is metadata only. Opening a record reads its locally extracted text and its
//! hash-verified stored PDF, and draws that record's pages through `crate::pdf_kit`. Proposal
//! reviews and regenerations go through the Documents API; filing remains a separate explicit
//! `PATCH /api/documents/{id}` action.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, ObjectFit,
    RenderImage, StyledImage, Subscription, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::shell::{
    self, DocumentDetail, DocumentSplitChildInput, DocumentSplitRequest, DocumentSummary,
    DuplicateCandidate, DuplicateCandidateKind, DuplicateCandidatePage, DuplicateDecision,
    DuplicateDecisionAction, DuplicatePageRange, ProposalEvidenceSource, ProposalOutcome,
    ProposalReview, ProposalSource, ShellError,
};

use crate::data::{Remote, Repo};
use crate::pdf_kit::PageRenderer;
use crate::theme::Palette;
use sjel_gpui_ui::filter_pill;

/// How many drawn pages the reader keeps for the document on screen. One page ahead and one back is
/// what a reader flips through; more than that is memory a 24-page scan would happily take.
const CACHED_PAGES: usize = 6;

/// How many rows the list draws at once. The list holds every record so the counts and the search
/// stay honest, but a library of a few thousand draws in batches: the page is rebuilt every frame,
/// and ten thousand elements per frame is what makes it stutter. `Show more` adds another batch.
const ROW_BATCH: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Inbox,
    Filed,
    All,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Grouping {
    #[default]
    None,
    Category,
    Project,
    Year,
    Status,
    Workflow,
}

impl Grouping {
    fn next(self) -> Self {
        match self {
            Self::None => Self::Category,
            Self::Category => Self::Project,
            Self::Project => Self::Year,
            Self::Year => Self::Status,
            Self::Status => Self::Workflow,
            Self::Workflow => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "No groups",
            Self::Category => "Group: category",
            Self::Project => "Group: project",
            Self::Year => "Group: year",
            Self::Status => "Group: status",
            Self::Workflow => "Group: workflow",
        }
    }
}

impl Filter {
    fn matches(self, row: &DocumentSummary) -> bool {
        match self {
            Self::Inbox => row.status == "inbox",
            Self::Filed => row.status == "filed",
            Self::All => row.status != "archived",
            Self::NeedsReview => {
                row.status == "inbox"
                    && matches!(
                        row.workflow_stage.as_str(),
                        "analyze" | "review_split" | "review_enrichment" | "file" | ""
                    )
            }
        }
    }
}

fn grouping_key(row: &DocumentSummary, grouping: Grouping) -> String {
    match grouping {
        Grouping::None => String::new(),
        Grouping::Category => row.category.clone().unwrap_or_else(|| "Unfiled".into()),
        Grouping::Project => row.project.clone().unwrap_or_else(|| "No project".into()),
        Grouping::Year => row
            .imported_at
            .get(..4)
            .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
            .unwrap_or("Unknown year")
            .to_owned(),
        Grouping::Status => match row.status.as_str() {
            "inbox" => "Inbox".into(),
            "filed" => "Filed".into(),
            "archived" => "Archived".into(),
            status => status.to_owned(),
        },
        Grouping::Workflow => workflow_label(&row.workflow_stage).into(),
    }
}

fn workflow_label(stage: &str) -> &'static str {
    match stage {
        "analyze" => "Needs analysis",
        "review_split" => "Review split",
        "split" => "Split into children",
        "review_enrichment" => "Review enrichment",
        "file" => "Ready to file",
        "filed" => "Filed",
        "archived" => "Archived",
        _ => "Needs review",
    }
}

/// How wide a page is drawn before the pane scales it. Both are well past a 5 K display column, so
/// the difference the reader sees is sharpness, not layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Zoom {
    #[default]
    Page,
    Close,
}

impl Zoom {
    fn width(self) -> u32 {
        match self {
            Self::Page => 1200,
            Self::Close => 2400,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Page => "Fit",
            Self::Close => "2×",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Page => Self::Close,
            Self::Close => Self::Page,
        }
    }
}

/// The document on screen: its bytes, and the pages of it that have been drawn.
#[derive(Default)]
struct Reader {
    id: Option<String>,
    bytes: Option<Arc<Vec<u8>>>,
    pages: usize,
    page: usize,
    first_page: usize,
    last_page: Option<usize>,
    zoom: Zoom,
    /// Keyed by page and drawn width, so toggling zoom back reuses what was already drawn.
    drawn: BTreeMap<(usize, u32), Arc<RenderImage>>,
    /// Why the original could not be read or drawn. Shown with the text, never instead of it.
    error: Option<String>,
    loading: bool,
    /// The extracted text instead of the page. Text stays available when drawing fails.
    reading_text: bool,
}

impl Reader {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn next_page(&mut self, by: isize) -> bool {
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

    fn image(&self) -> Option<Arc<RenderImage>> {
        self.drawn.get(&(self.page, self.zoom.width())).cloned()
    }

    /// Drop the drawn pages furthest from the one on screen.
    fn trim(&mut self) {
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

pub struct Documents {
    repo: Repo,
    palette: Option<Palette>,
    pub rows: Remote<Vec<DocumentSummary>>,
    pub detail: Remote<DocumentDetail>,
    pub filter: Filter,
    pub search_query: String,
    grouping: Grouping,
    category_filter: Option<String>,
    search_revision: u64,
    search_applied: bool,
    search_input: Option<(Entity<InputState>, Subscription)>,
    sort_by_name: bool,
    selected: Option<String>,
    /// How many rows the list draws now. Raised by `Show more`, reset whenever the view changes.
    visible: usize,
    /// The list is its own scroll area: the page column is bounded so the reader can fill it.
    list_scroll: gpui::ScrollHandle,
    /// Extracted text is its own scroll area when reading text mode is on.
    text_scroll: gpui::ScrollHandle,
    reader: Reader,
    renderer: Option<PageRenderer>,
    renderer_error: Option<String>,
    duplicate_mode: bool,
    duplicate_kind: Option<DuplicateCandidateKind>,
    duplicate_offset: usize,
    duplicate_page: Remote<DuplicateCandidatePage>,
    duplicate_selection: Option<DuplicateCandidate>,
    compare_readers: [Reader; 2],
    preferred_id: Option<String>,
    split_ranges: String,
    duplicate_error: Option<String>,
    split_input: Option<(Entity<InputState>, Subscription)>,
    proposal_busy: Option<String>,
    proposal_error: Option<String>,
    proposal_notice: Option<String>,
    show_proposal_history: bool,
    workflow_busy: Option<String>,
    workflow_error: Option<String>,
}

impl Documents {
    pub fn new(repo: Repo) -> Self {
        let renderer = PageRenderer::new();
        let (renderer, renderer_error) = match renderer {
            Ok(renderer) => (Some(renderer), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            repo,
            palette: None,
            rows: Remote::default(),
            detail: Remote::default(),
            filter: Filter::Inbox,
            search_query: String::new(),
            grouping: Grouping::None,
            category_filter: None,
            search_revision: 0,
            search_applied: false,
            search_input: None,
            sort_by_name: false,
            selected: None,
            visible: ROW_BATCH,
            list_scroll: gpui::ScrollHandle::new(),
            text_scroll: gpui::ScrollHandle::new(),
            reader: Reader::default(),
            renderer,
            renderer_error,
            duplicate_mode: false,
            duplicate_kind: None,
            duplicate_offset: 0,
            duplicate_page: Remote::default(),
            duplicate_selection: None,
            compare_readers: [Reader::default(), Reader::default()],
            preferred_id: None,
            split_ranges: String::new(),
            duplicate_error: None,
            split_input: None,
            proposal_busy: None,
            proposal_error: None,
            proposal_notice: None,
            show_proposal_history: false,
            workflow_busy: None,
            workflow_error: None,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.rows.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.rows.loading = true;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let work = shell::run(async move { shell.documents_stream(tx).await });
        cx.spawn(async move |this, cx| {
            while let Some(batch) = rx.recv().await {
                let _ = this.update(cx, |view, cx| {
                    match &mut view.rows.value {
                        Some(rows) => rows.extend(batch),
                        None => view.rows.value = Some(batch),
                    }
                    cx.notify();
                });
            }
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.rows.loading = false;
                if let Err(err) = result {
                    if view.rows.value.is_none() {
                        view.rows.settle(Err(err));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_duplicate_candidates(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.duplicate_page.loading = true;
        self.duplicate_error = None;
        let kind = self.duplicate_kind;
        let offset = self.duplicate_offset;
        let expected_kind = kind;
        let expected_offset = offset;
        let work = shell::run(async move { shell.duplicate_candidates(kind, offset, 50).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.duplicate_kind != expected_kind || view.duplicate_offset != expected_offset
                {
                    return;
                }
                view.duplicate_page.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_duplicate_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.duplicate_mode = enabled;
        if enabled {
            self.duplicate_offset = 0;
            self.duplicate_selection = None;
            self.load_duplicate_candidates(cx);
        }
        if !enabled {
            self.duplicate_selection = None;
            self.compare_readers = [Reader::default(), Reader::default()];
        }
        cx.notify();
    }

    fn set_duplicate_kind(&mut self, kind: Option<DuplicateCandidateKind>, cx: &mut Context<Self>) {
        self.duplicate_kind = kind;
        self.duplicate_offset = 0;
        self.duplicate_page = Remote::default();
        self.duplicate_selection = None;
        self.compare_readers = [Reader::default(), Reader::default()];
        self.load_duplicate_candidates(cx);
    }

    fn open_duplicate_candidate(&mut self, candidate: DuplicateCandidate, cx: &mut Context<Self>) {
        self.preferred_id = candidate
            .member_ids
            .first()
            .cloned()
            .filter(|_| candidate.member_ids.len() == 1);
        self.split_ranges.clear();
        self.split_input = None;
        self.duplicate_error = None;
        self.duplicate_selection = Some(candidate.clone());
        self.compare_readers = [Reader::default(), Reader::default()];
        for side in 0..2 {
            let Some(id) = candidate.member_ids.get(side).cloned() else {
                continue;
            };
            self.load_compare_original(side, id, cx);
        }
        cx.notify();
    }

    fn load_compare_original(&mut self, side: usize, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        self.compare_readers[side] = Reader {
            id: Some(id.clone()),
            loading: true,
            ..Reader::default()
        };
        let expected_id = id.clone();
        let work = shell::run(async move { shell.document_original(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || view.compare_readers[side].id.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                match result {
                    Ok(bytes) => {
                        view.compare_readers[side].bytes = Some(Arc::new(bytes));
                        view.compare_readers[side].loading = false;
                        view.ask_compare_page_count(side, cx);
                    }
                    Err(error) => {
                        view.compare_readers[side].loading = false;
                        view.compare_readers[side].error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ask_compare_page_count(&mut self, side: usize, cx: &mut Context<Self>) {
        let Some(renderer) = self.renderer.as_ref() else {
            self.compare_readers[side].error = self.renderer_error.clone();
            return;
        };
        let (Some(bytes), Some(id)) = (
            self.compare_readers[side].bytes.clone(),
            self.compare_readers[side].id.clone(),
        ) else {
            return;
        };
        let answer = renderer.info(bytes);
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || view.compare_readers[side].id.as_deref() != Some(id.as_str())
                {
                    return;
                }
                match result {
                    Ok(pages) => view.compare_readers[side].pages = pages,
                    Err(error) => view.compare_readers[side].error = Some(error.to_string()),
                }
                view.request_compare_page(side, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn request_compare_page(&mut self, side: usize, cx: &mut Context<Self>) {
        let reader = &self.compare_readers[side];
        let (Some(renderer), Some(bytes), Some(id)) = (
            self.renderer.as_ref(),
            reader.bytes.clone(),
            reader.id.clone(),
        ) else {
            return;
        };
        let page = reader.page;
        let width = reader.zoom.width();
        if reader.drawn.contains_key(&(page, width)) {
            return;
        }
        let answer = renderer.render(bytes, page, width);
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                let reader = &mut view.compare_readers[side];
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || reader.id.as_deref() != Some(id.as_str())
                    || reader.page != page
                {
                    return;
                }
                match result {
                    Ok(image) => {
                        reader
                            .drawn
                            .insert((page, width), image.into_render_image());
                        reader.trim();
                    }
                    Err(error) => reader.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn move_compare_page(&mut self, side: usize, by: isize, cx: &mut Context<Self>) {
        if self.compare_readers[side].next_page(by) {
            self.request_compare_page(side, cx);
            cx.notify();
        }
    }

    fn duplicate_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let rows = self.rows.value.clone().unwrap_or_default();
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap_3();
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(duplicate_filter_button(
                    "duplicate-all",
                    "All candidates",
                    self.duplicate_kind.is_none(),
                    cx.listener(|view, _, _, cx| view.set_duplicate_kind(None, cx)),
                ))
                .child(duplicate_filter_button(
                    "duplicate-exact",
                    "Exact copies",
                    self.duplicate_kind == Some(DuplicateCandidateKind::ExactCopy),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::ExactCopy), cx)
                    }),
                ))
                .child(duplicate_filter_button(
                    "duplicate-rescans",
                    "Rescans",
                    self.duplicate_kind == Some(DuplicateCandidateKind::Rescan),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::Rescan), cx)
                    }),
                ))
                .child(duplicate_filter_button(
                    "duplicate-parts",
                    "Possible parts",
                    self.duplicate_kind == Some(DuplicateCandidateKind::Part),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::Part), cx)
                    }),
                ))
                .child(div().flex_1())
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} · no files are changed",
                    self.duplicate_page
                        .value
                        .as_ref()
                        .map_or(0, |page| page.total)
                )),
        );

        if let Some(error) = &self.duplicate_error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        if let Some(candidate) = self.duplicate_selection.clone() {
            let titles: Vec<_> = candidate
                .member_ids
                .iter()
                .take(2)
                .map(|id| {
                    rows.iter()
                        .find(|row| &row.id == id)
                        .map(|row| row.filename.clone())
                        .unwrap_or_else(|| id.chars().take(12).collect())
                })
                .collect();
            let label = match candidate.kind {
                DuplicateCandidateKind::ExactCopy => {
                    "Identical source bytes · SHA-256 match".to_owned()
                }
                DuplicateCandidateKind::Rescan => format!(
                    "Possible rescan · {:.0}% text overlap",
                    candidate.score.unwrap_or_default() * 100.0
                ),
                DuplicateCandidateKind::Part => format!(
                    "Possible part · {:.0}% text overlap",
                    candidate.score.unwrap_or_default() * 100.0
                ),
            };
            let mut panes = div().flex_1().min_h_0().flex().gap_3();
            for side in 0..2 {
                let reader = &self.compare_readers[side];
                let title = titles
                    .get(side)
                    .cloned()
                    .unwrap_or_else(|| "No second document record".into());
                let title = candidate.part.as_ref().map_or(title.clone(), |part| {
                    match candidate.member_ids.get(side).map(String::as_str) {
                        Some(id) if id == part.part_id => format!("Likely part · {title}"),
                        Some(id) if id == part.parent_id => format!("Likely parent · {title}"),
                        _ => title,
                    }
                });
                let id = reader.id.clone();
                let page = reader.page;
                let pages = reader.pages;
                let chosen = id
                    .as_ref()
                    .is_some_and(|id| self.preferred_id.as_ref() == Some(id));
                let pane = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .when_some(id.clone(), |bar, id| {
                                bar.child(
                                    Button::new(format!("duplicate-prefer-{side}"))
                                        .small()
                                        .label(if chosen { "Preferred" } else { "Prefer" })
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.preferred_id = Some(id.clone());
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(format!("duplicate-prev-{side}"))
                                    .small()
                                    .label("‹")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.move_compare_page(side, -1, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(76.))
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(if pages == 0 {
                                        "—".into()
                                    } else {
                                        format!("{} / {}", page + 1, pages)
                                    }),
                            )
                            .child(
                                Button::new(format!("duplicate-next-{side}"))
                                    .small()
                                    .label("›")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.move_compare_page(side, 1, cx)
                                    })),
                            ),
                    )
                    .child(reader_page_element(reader, palette));
                panes = panes.child(pane);
            }
            let receipts = candidate
                .locations
                .iter()
                .map(|location| {
                    location
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&location.path)
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(" · ");
            body = body
                .child(div().text_sm().text_color(rgb(palette.amber)).child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} recorded source location(s): {}",
                            candidate.locations.len(),
                            if receipts.is_empty() {
                                "no source receipt"
                            } else {
                                &receipts
                            }
                        )),
                )
                .child(panes);

            if candidate.kind != DuplicateCandidateKind::Part && candidate.member_ids.len() > 2 {
                let mut members = div().flex().items_center().gap_2().flex_wrap();
                for (index, id) in candidate.member_ids.iter().enumerate() {
                    let filename = rows
                        .iter()
                        .find(|row| &row.id == id)
                        .map(|row| row.filename.clone())
                        .unwrap_or_else(|| id.chars().take(12).collect());
                    let preferred = self.preferred_id.as_ref() == Some(id);
                    let id = id.clone();
                    members = members.child(
                        Button::new(format!("duplicate-member-{index}"))
                            .small()
                            .label(if preferred {
                                format!("✓ {filename}")
                            } else {
                                filename
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.preferred_id = Some(id.clone());
                                cx.notify();
                            })),
                    );
                }
                body = body.child(members);
            }

            if candidate.kind == DuplicateCandidateKind::Part {
                let input = self.split_range_field(window, cx);
                let parent = candidate.part.as_ref();
                let parent_pages = parent
                    .and_then(|part| {
                        self.compare_readers
                            .iter()
                            .find(|reader| reader.id.as_deref() == Some(part.parent_id.as_str()))
                            .map(|reader| reader.pages)
                    })
                    .unwrap_or(0);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Split page ranges · parent has {parent_pages} pages"
                        ))
                        .child(div().w(px(220.)).child(Input::new(&input).small()))
                        .when(parent_pages > 0, |bar| {
                            let parent_id = parent
                                .map(|part| part.parent_id.clone())
                                .unwrap_or_default();
                            bar.child(
                                Button::new("duplicate-record-split")
                                    .small()
                                    .label("Record split relationship")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        match parse_page_ranges(&view.split_ranges) {
                                            Ok(ranges) => view.record_duplicate_decision(
                                                DuplicateDecisionAction::Split {
                                                    parent_id: parent_id.clone(),
                                                    page_count: u32::try_from(parent_pages)
                                                        .unwrap_or(u32::MAX),
                                                    ranges,
                                                },
                                                cx,
                                            ),
                                            Err(error) => {
                                                view.duplicate_error = Some(error);
                                                cx.notify();
                                            }
                                        }
                                    })),
                            )
                        })
                        .child(
                            Button::new("duplicate-keep-separate")
                                .small()
                                .label("Keep separate")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.record_duplicate_decision(
                                        DuplicateDecisionAction::Dismiss,
                                        cx,
                                    )
                                })),
                        ),
                );
            } else {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(self.preferred_id.is_some(), |bar| {
                            bar.child(
                                Button::new("duplicate-confirm-group")
                                    .small()
                                    .label("Confirm group")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        if let Some(survivor_id) = view.preferred_id.clone() {
                                            view.record_duplicate_decision(
                                                DuplicateDecisionAction::ConfirmGroup {
                                                    survivor_id,
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                        })
                        .child(
                            Button::new("duplicate-keep-separate")
                                .small()
                                .label("Keep separate")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.record_duplicate_decision(
                                        DuplicateDecisionAction::Dismiss,
                                        cx,
                                    )
                                })),
                        ),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("duplicate-back-queue")
                            .small()
                            .label("Back to candidates")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.duplicate_selection = None;
                                view.compare_readers = [Reader::default(), Reader::default()];
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                        "Source records stay intact; approved page splits are stored as child PDFs in Sjel.",
                    )),
            );
        } else {
            if self.duplicate_page.loading && self.duplicate_page.value.is_none() {
                body = body.child(
                    div()
                        .py_5()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Finding duplicate candidates…"),
                );
            }
            if let Some(error) = &self.duplicate_page.error {
                body = body.child(
                    div()
                        .py_3()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error.to_string()),
                );
            }
            let candidates = self
                .duplicate_page
                .value
                .as_ref()
                .map(|page| page.candidates.clone())
                .unwrap_or_default();
            if candidates.is_empty()
                && !self.duplicate_page.loading
                && self.duplicate_page.error.is_none()
            {
                body = body.child(
                    div()
                        .py_5()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No unresolved candidates in this filter."),
                );
            }
            for (candidate_index, candidate) in candidates.into_iter().enumerate() {
                let candidate_for_click = candidate.clone();
                let names = candidate
                    .member_ids
                    .iter()
                    .take(2)
                    .map(|id| {
                        rows.iter()
                            .find(|row| &row.id == id)
                            .map(|row| row.filename.clone())
                            .unwrap_or_else(|| id.chars().take(12).collect::<String>())
                    })
                    .collect::<Vec<_>>();
                let evidence = match candidate.kind {
                    DuplicateCandidateKind::ExactCopy => "Exact SHA-256 match".to_owned(),
                    DuplicateCandidateKind::Rescan => format!(
                        "Possible rescan · {:.0}%",
                        candidate.score.unwrap_or_default() * 100.0
                    ),
                    DuplicateCandidateKind::Part => format!(
                        "Possible part · {:.0}%",
                        candidate.score.unwrap_or_default() * 100.0
                    ),
                };
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded(px(5.))
                        .child(
                            Button::new(format!("duplicate-candidate-{candidate_index}"))
                                .small()
                                .label(names.join("  ↔  "))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_duplicate_candidate(candidate_for_click.clone(), cx)
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(
                                    if candidate.kind == DuplicateCandidateKind::ExactCopy {
                                        palette.green
                                    } else {
                                        palette.amber
                                    },
                                ))
                                .child(evidence),
                        ),
                );
            }
            let offset = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(0, |page| page.offset);
            let total = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(0, |page| page.total);
            let limit = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(50, |page| page.limit);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .when(offset > 0, |bar| {
                        bar.child(
                            Button::new("duplicate-prev-batch")
                                .small()
                                .label("Previous")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.duplicate_offset =
                                        view.duplicate_offset.saturating_sub(50);
                                    view.load_duplicate_candidates(cx);
                                })),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{}–{} of {}",
                                if total == 0 { 0 } else { offset + 1 },
                                (offset + limit).min(total),
                                total
                            )),
                    )
                    .when(offset + limit < total, |bar| {
                        bar.child(
                            Button::new("duplicate-next-batch")
                                .small()
                                .label("Next")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.duplicate_offset += 50;
                                    view.load_duplicate_candidates(cx);
                                })),
                        )
                    }),
            );
        }
        body.into_any_element()
    }

    fn record_duplicate_decision(
        &mut self,
        action: DuplicateDecisionAction,
        cx: &mut Context<Self>,
    ) {
        let Some(candidate) = self.duplicate_selection.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        let decision = DuplicateDecision {
            candidate_key: candidate.candidate_key,
            kind: candidate.kind,
            member_ids: candidate.member_ids,
            action,
        };
        let work = shell::run(async move { shell.review_duplicate(decision).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(()) => {
                        view.duplicate_selection = None;
                        view.compare_readers = [Reader::default(), Reader::default()];
                        view.duplicate_offset = 0;
                        view.load_duplicate_candidates(cx);
                    }
                    Err(error) => view.duplicate_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Clear the active document and close the reader, returning the view to the full table.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_some() {
            self.selected = None;
            self.detail = Remote::default();
            self.reader.clear();
            self.proposal_busy = None;
            self.workflow_busy = None;
            self.workflow_error = None;
            self.proposal_error = None;
            self.proposal_notice = None;
            self.show_proposal_history = false;
            cx.notify();
        }
    }

    /// Step through documents in the active filter/search list by `by` (+1 next, -1 prev).
    pub fn select_relative(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(rows) = self.rows.value.as_ref() else {
            return;
        };
        let mut filtered: Vec<_> =
            rows.iter()
                .filter(|row| self.filter.matches(row))
                .filter(|row| {
                    self.search_query.is_empty()
                        || row.filename.to_lowercase().contains(&self.search_query)
                        || row
                            .extracted_title
                            .as_deref()
                            .is_some_and(|title| title.to_lowercase().contains(&self.search_query))
                        || row.category.as_deref().is_some_and(|category| {
                            category.to_lowercase().contains(&self.search_query)
                        })
                        || row.project.as_deref().is_some_and(|project| {
                            project.to_lowercase().contains(&self.search_query)
                        })
                })
                .collect();
        if self.sort_by_name {
            filtered.sort_by_key(|row| row.filename.to_lowercase());
        }
        if filtered.is_empty() {
            return;
        }
        let current_index = self
            .selected
            .as_deref()
            .and_then(|sel| filtered.iter().position(|r| r.id == sel))
            .unwrap_or(0);
        let next_index =
            (current_index as isize + by).clamp(0, filtered.len() as isize - 1) as usize;
        if let Some(next_row) = filtered.get(next_index) {
            let next_id = next_row.id.clone();
            self.open(next_id, cx);
        }
    }

    /// Open the current document original in the default macOS application (e.g. Preview).
    pub fn open_externally(&mut self) {
        let (Some(bytes), Some(detail)) = (&self.reader.bytes, self.detail.value.as_ref()) else {
            return;
        };
        if detail.document.parent_id.is_some() {
            return;
        }
        let dir = std::env::temp_dir().join("sjel-documents");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(&detail.document.filename);
        if std::fs::write(&path, bytes.as_slice()).is_ok() {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(&path)
                .spawn();
        }
    }

    /// Open one record: its metadata and text first, then the original for the page view.
    fn open(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.selected = Some(id.clone());
        self.detail = Remote::new_loading();
        self.reader.clear();
        self.split_ranges.clear();
        self.split_input = None;
        self.workflow_busy = None;
        self.workflow_error = None;
        self.proposal_busy = None;
        self.proposal_error = None;
        self.proposal_notice = None;
        self.show_proposal_history = false;
        cx.notify();

        let expected = id.clone();
        let work = shell::run(async move { shell.document_detail(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected.as_str()) {
                    view.detail.settle(result);
                    if let Some(detail) = &view.detail.value {
                        if let Some(range) = detail.document.page_range {
                            view.reader.first_page = 0;
                            view.reader.last_page = Some(
                                usize::try_from(range.last.saturating_sub(range.first))
                                    .unwrap_or_default(),
                            );
                            view.reader.page = 0;
                        }
                    }
                    view.request_page(cx);
                    cx.notify();
                }
            });
        })
        .detach();

        self.open_original(cx);
    }

    fn open_original(&mut self, cx: &mut Context<Self>) {
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
    fn ask_page_count(&mut self, cx: &mut Context<Self>) {
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
    fn request_page(&mut self, cx: &mut Context<Self>) {
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
                        view.reader
                            .drawn
                            .insert((expected.1, expected.2), image.into_render_image());
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

    pub fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        let next = self.reader.zoom.other();
        self.set_zoom(next, cx);
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        if self.reader.zoom != zoom {
            self.reader.zoom = zoom;
            self.request_page(cx);
            cx.notify();
        }
    }

    fn show_text(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.reader.reading_text != show {
            self.reader.reading_text = show;
            cx.notify();
        }
    }

    fn split_range_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.split_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("1-3, 4-8"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.split_ranges = input.read(cx).value().trim().to_owned();
                cx.notify();
            }
        });
        self.split_input = Some((input.clone(), subscription));
        input
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search all document text…"));
        let subscription =
            cx.subscribe_in(&input, window, |view, input, event, _, cx| match event {
                InputEvent::Change => {
                    view.search_query = input.read(cx).value().trim().to_lowercase();
                    view.search_applied = false;
                    view.visible = ROW_BATCH;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => view.search_documents(cx),
                _ => {}
            });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn review_proposal(
        &mut self,
        proposal_id: String,
        outcome: ProposalReview,
        cx: &mut Context<Self>,
    ) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some(proposal_id.clone());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let expected_proposal_id = proposal_id.clone();
        let work = shell::run(async move {
            let reviewed = shell
                .review_document_proposal(&document_id, &proposal_id, outcome)
                .await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((reviewed, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((updated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                if let Some(record) = detail
                                    .proposals
                                    .iter_mut()
                                    .find(|record| record.id == expected_proposal_id)
                                {
                                    *record = updated;
                                }
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "Review was recorded, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice =
                                Some("Review recorded; filing is unchanged.".into());
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn regenerate_proposals(&mut self, cx: &mut Context<Self>) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some("regenerate".into());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let work = shell::run(async move {
            let generated = shell.regenerate_document_proposals(&document_id).await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((generated, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((mut generated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                detail.proposals.append(&mut generated);
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "New proposals were saved, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice = Some(
                                "New rule run saved; earlier proposals and reviews remain.".into(),
                            );
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn generate_model_proposal(&mut self, cx: &mut Context<Self>) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some("model".into());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let work = shell::run(async move {
            let generated = shell.generate_document_model_proposal(&document_id).await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((generated, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((mut generated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                detail.proposals.append(&mut generated);
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "The model proposal was saved, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice = Some(
                                "Local model proposal saved for review; filing remains separate.".into(),
                            );
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn toggle_proposal_history(&mut self, cx: &mut Context<Self>) {
        self.show_proposal_history = !self.show_proposal_history;
        cx.notify();
    }

    fn file_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        let Some(category) = detail.document.suggested_category.clone() else {
            return;
        };
        let id = detail.document.id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        let work = shell::run(async move { shell.review_document(&id, &category, None).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if let Ok(updated) = result {
                    if let Some(detail) = &mut view.detail.value {
                        if detail.document.id == updated.id {
                            detail.document = updated.clone();
                        }
                    }
                    if let Some(rows) = &mut view.rows.value {
                        if let Some(row) = rows.iter_mut().find(|row| row.id == updated.id) {
                            *row = updated;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.visible = ROW_BATCH;
        cx.notify();
    }

    fn search_documents(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let query = self.search_query.trim().to_owned();
        self.search_revision = self.search_revision.wrapping_add(1);
        let revision = self.search_revision;
        self.rows.loading = true;
        self.rows.error = None;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let work = shell::run(async move { shell.documents_search_stream(&query, tx).await });
        cx.spawn(async move |this, cx| {
            let mut received = false;
            while let Some(batch) = rx.recv().await {
                let first_batch = !received;
                received = true;
                let _ = this.update(cx, |view, cx| {
                    if view.search_revision != revision {
                        return;
                    }
                    if first_batch {
                        view.rows.value = Some(batch);
                    } else if let Some(rows) = &mut view.rows.value {
                        rows.extend(batch);
                    } else {
                        view.rows.value = Some(batch);
                    }
                    cx.notify();
                });
            }
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.search_revision != revision {
                    return;
                }
                view.rows.loading = false;
                match result {
                    Ok(_) => {
                        if !received {
                            view.rows.value = Some(Vec::new());
                        }
                        view.search_applied = true;
                    }
                    Err(error) => view.rows.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn analyze_selected_document(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.workflow_busy.is_some() {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.workflow_busy = Some("analyze".into());
        self.workflow_error = None;
        cx.notify();
        let expected = id.clone();
        let work = shell::run(async move { shell.analyze_document(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() != Some(expected.as_str()) {
                    return;
                }
                view.workflow_busy = None;
                match result {
                    Ok(detail) => {
                        if let Some(rows) = &mut view.rows.value {
                            if let Some(row) =
                                rows.iter_mut().find(|row| row.id == detail.document.id)
                            {
                                *row = detail.document.clone();
                            }
                        }
                        view.detail.settle(Ok(detail));
                    }
                    Err(error) => view.workflow_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn split_selected_document(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.workflow_busy.is_some() {
            return;
        }
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        if detail.document.parent_id.is_some() || detail.document.page_count < 2 {
            return;
        }
        let page_count = detail.document.page_count;
        let filename = detail.document.filename.clone();
        let ranges = match parse_page_ranges(&self.split_ranges) {
            Ok(ranges) => ranges,
            Err(error) => {
                self.workflow_error = Some(error);
                cx.notify();
                return;
            }
        };
        let stem = std::path::Path::new(&filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Document");
        let request = DocumentSplitRequest {
            page_count: u32::try_from(page_count).unwrap_or(u32::MAX),
            children: ranges
                .into_iter()
                .map(|range| DocumentSplitChildInput {
                    first: range.first,
                    last: range.last,
                    filename: format!("{stem} pages {}-{}.pdf", range.first, range.last),
                })
                .collect(),
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.workflow_busy = Some("split".into());
        self.workflow_error = None;
        cx.notify();
        let expected = id.clone();
        let refresh_id = id.clone();
        let work = shell::run(async move {
            let children = shell.split_document(&id, &request).await?;
            let parent = shell.document_detail(&refresh_id).await;
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
                        if let Some(detail) = &mut view.detail.value {
                            detail.document.workflow_stage = "split".into();
                            for child in &children {
                                if !detail
                                    .children
                                    .iter()
                                    .any(|existing| existing.id == child.id)
                                {
                                    detail.children.push(child.clone());
                                }
                            }
                        }
                        if let Some(rows) = &mut view.rows.value {
                            for child in children {
                                if !rows.iter().any(|existing| existing.id == child.id) {
                                    rows.push(child);
                                }
                            }
                        }
                        match parent {
                            Ok(detail) => {
                                if let Some(rows) = &mut view.rows.value {
                                    if let Some(row) = rows
                                        .iter_mut()
                                        .find(|row| row.id == detail.document.id)
                                    {
                                        *row = detail.document.clone();
                                    }
                                }
                                view.detail.settle(Ok(detail));
                            }
                            Err(error) => {
                                view.workflow_error = Some(format!(
                                    "Split was saved, but the parent workflow status did not refresh: {error}"
                                ));
                            }
                        }
                    }
                    Err(error) => view.workflow_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn use_split_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        let Some(suggestion) = detail.split_suggestions.as_ref() else {
            return;
        };
        if suggestion.ranges.is_empty() {
            return;
        }
        let value = suggestion
            .ranges
            .iter()
            .map(|range| format!("{}-{}", range.first, range.last))
            .collect::<Vec<_>>()
            .join(", ");
        self.split_ranges = value.clone();
        if let Some((input, _)) = &self.split_input {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        cx.notify();
    }

    fn cycle_grouping(&mut self, cx: &mut Context<Self>) {
        self.grouping = self.grouping.next();
        self.visible = ROW_BATCH;
        self.list_scroll = gpui::ScrollHandle::new();
        cx.notify();
    }

    fn cycle_category_filter(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows.value.as_deref().unwrap_or_default();
        let categories: Vec<_> = rows
            .iter()
            .filter_map(|row| row.category.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let next = match self.category_filter.as_deref() {
            None => Some(String::new()),
            Some("") => categories.first().cloned(),
            Some(current) => categories
                .iter()
                .position(|category| category == current)
                .and_then(|index| categories.get(index + 1).cloned()),
        };
        self.category_filter = next;
        self.visible = ROW_BATCH;
        self.list_scroll = gpui::ScrollHandle::new();
        cx.notify();
    }

    fn show_more(&mut self, cx: &mut Context<Self>) {
        self.visible += ROW_BATCH;
        cx.notify();
    }

    /// The page, or the reason there is none. A drawn page is the whole of this element when it
    /// exists; otherwise the caller shows the text beside it.
    fn page_element(&self, palette: Palette) -> AnyElement {
        reader_page_element(&self.reader, palette)
    }
}

impl Render for Documents {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        if self.duplicate_mode {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Duplicate review"),
                        )
                        .child(
                            Button::new("documents-back-library")
                                .small()
                                .label("Back to library")
                                .on_click(
                                    cx.listener(|view, _, _, cx| {
                                        view.set_duplicate_mode(false, cx)
                                    }),
                                ),
                        ),
                )
                .child(self.duplicate_workspace(window, cx));
        }
        let rows = self.rows.value.clone().unwrap_or_default();
        let inbox = rows.iter().filter(|d| d.status == "inbox").count();
        let filed = rows.iter().filter(|d| d.status == "filed").count();
        let active = rows.iter().filter(|d| d.status != "archived").count();
        let next = rows
            .iter()
            .filter(|row| Filter::NeedsReview.matches(row))
            .count();
        let selected = self.selected.clone();
        // With a document open the list keeps a column beside the reader. That column is too
        // narrow for four columns and a search box, so the view drops what it cannot show rather
        // than squeezing every column into nothing.
        let narrow = selected.is_some();
        let category_label = match self.category_filter.as_deref() {
            None => "Category: all".to_owned(),
            Some("") => "Category: unfiled".to_owned(),
            Some(category) => format!("Category: {category}"),
        };

        let mut table = div().flex().flex_col().gap_1();
        let search = self.search_field(window, cx);
        let split_input = self.split_range_field(window, cx);
        table = table.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mb_2()
                .child(filter_pill(
                    "Inbox",
                    inbox,
                    self.filter == Filter::Inbox,
                    palette,
                    cx.listener(|view, _, _, cx| view.set_filter(Filter::Inbox, cx)),
                ))
                .child(filter_pill(
                    "Filed",
                    filed,
                    self.filter == Filter::Filed,
                    palette,
                    cx.listener(|view, _, _, cx| view.set_filter(Filter::Filed, cx)),
                ))
                .child(filter_pill(
                    "All",
                    active,
                    self.filter == Filter::All,
                    palette,
                    cx.listener(|view, _, _, cx| view.set_filter(Filter::All, cx)),
                ))
                .child(filter_pill(
                    "Needs review",
                    next,
                    self.filter == Filter::NeedsReview,
                    palette,
                    cx.listener(|view, _, _, cx| view.set_filter(Filter::NeedsReview, cx)),
                ))
                .child(
                    Button::new("documents-category-filter")
                        .small()
                        .label(category_label)
                        .on_click(cx.listener(|view, _, _, cx| view.cycle_category_filter(cx))),
                )
                .child(
                    Button::new("documents-grouping")
                        .small()
                        .label(self.grouping.label())
                        .on_click(cx.listener(|view, _, _, cx| view.cycle_grouping(cx))),
                )
                .child(div().flex_1())
                .when(!narrow, |bar| {
                    bar.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().w(px(220.)).child(Input::new(&search).small()))
                            .child(
                                Button::new("documents-search-text")
                                    .small()
                                    .label("Search")
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.search_documents(cx)),
                                    ),
                            ),
                    )
                })
                .when(!narrow, |bar| {
                    bar.child(
                        Button::new("documents-sort")
                            .small()
                            .label(if self.sort_by_name {
                                "Name A–Z"
                            } else {
                                "Newest"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.sort_by_name = !view.sort_by_name;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    div()
                        .max_w(px(170.))
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .when(!narrow, |count| {
                            count.child(format!("{} documents · local only", rows.len()))
                        }),
                ),
        );
        if narrow {
            table = table.child(
                div()
                    .flex()
                    .gap_1()
                    .mb_2()
                    .child(div().flex_1().child(Input::new(&search).small()))
                    .child(
                        Button::new("documents-search-text-narrow")
                            .small()
                            .label("Search")
                            .on_click(cx.listener(|view, _, _, cx| view.search_documents(cx))),
                    ),
            );
        }
        table = table.child(
            div()
                .flex()
                .items_center()
                .min_h(px(32.))
                .px_3()
                .rounded(px(5.))
                .bg(rgba(palette.raised))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(div().flex_1().child("DOCUMENT"))
                .when(!narrow, |head| {
                    head.child(div().w(px(110.)).child("CATEGORY"))
                })
                .child(div().w(px(76.)).child("STATUS"))
                .when(!narrow, |head| head.child(div().w(px(110.)).child("ADDED"))),
        );
        if self.rows.value.is_none() && self.rows.loading {
            table = table.child(
                div()
                    .py_5()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Loading documents…"),
            );
        }
        if let Some(error) = &self.rows.error {
            table = table.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        let mut filtered: Vec<_> =
            rows.iter()
                .filter(|row| self.filter.matches(row))
                .filter(|row| match self.category_filter.as_deref() {
                    None => true,
                    Some(category) => row.category.as_deref().unwrap_or("") == category,
                })
                .filter(|row| {
                    self.search_applied
                        || self.search_query.is_empty()
                        || row.filename.to_lowercase().contains(&self.search_query)
                        || row
                            .extracted_title
                            .as_deref()
                            .is_some_and(|title| title.to_lowercase().contains(&self.search_query))
                        || row.category.as_deref().is_some_and(|category| {
                            category.to_lowercase().contains(&self.search_query)
                        })
                        || row.project.as_deref().is_some_and(|project| {
                            project.to_lowercase().contains(&self.search_query)
                        })
                })
                .collect();
        if self.grouping != Grouping::None {
            filtered.sort_by(|left, right| {
                grouping_key(left, self.grouping)
                    .cmp(&grouping_key(right, self.grouping))
                    .then_with(|| {
                        if self.sort_by_name {
                            left.filename
                                .to_lowercase()
                                .cmp(&right.filename.to_lowercase())
                        } else {
                            right.imported_at.cmp(&left.imported_at)
                        }
                    })
            });
        } else if self.sort_by_name {
            filtered.sort_by_key(|row| row.filename.to_lowercase());
        }
        if filtered.is_empty() && !self.rows.loading {
            table = table.child(div().py_5().text_sm().text_color(rgb(palette.muted)).child(
                if rows.is_empty() {
                    "No documents yet."
                } else {
                    "No documents in this view."
                },
            ));
        }
        let in_view = filtered.len();
        let shown = in_view.min(self.visible);
        let mut previous_group: Option<String> = None;
        let mut group_counts = BTreeMap::new();
        if self.grouping != Grouping::None {
            for row in &filtered {
                *group_counts
                    .entry(grouping_key(row, self.grouping))
                    .or_insert(0usize) += 1;
            }
        }
        for (i, row) in filtered.into_iter().take(self.visible).enumerate() {
            let group = (self.grouping != Grouping::None).then(|| grouping_key(row, self.grouping));
            if let Some(group) = group
                .as_ref()
                .filter(|group| previous_group.as_ref() != Some(group))
            {
                let count = group_counts.get(group).copied().unwrap_or(0);
                table = table.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .px_3()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(format!("{group} · {count}")),
                );
                previous_group = Some(group.clone());
            }
            let id = row.id.clone();
            let chosen = selected.as_deref() == Some(row.id.as_str());
            let category = row
                .category
                .as_deref()
                .or(row.suggested_category.as_deref())
                .unwrap_or("—");
            let date = row.imported_at.get(..10).unwrap_or("—").to_owned();
            let workflow = workflow_label(&row.workflow_stage);
            table = table.child(
                div()
                    .id(("document-row", i))
                    .flex()
                    .items_center()
                    .min_h(px(40.))
                    .px_3()
                    .rounded(px(5.))
                    .when(chosen, |el| el.bg(rgba(palette.raised)))
                    .hover(move |style| style.bg(rgba(palette.raised)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.selected.as_deref() == Some(id.as_str()) {
                            view.close(cx);
                        } else {
                            view.open(id.clone(), cx);
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_sm().child(row.filename.clone()))
                            .when(row.status == "inbox", |column| {
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(workflow),
                                )
                            })
                            .when(!row.matched_pages.is_empty(), |column| {
                                let pages = row
                                    .matched_pages
                                    .iter()
                                    .map(u32::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Matches on page {pages}")),
                                )
                            }),
                    )
                    .when(!narrow, |cells| {
                        cells.child(
                            div()
                                .w(px(110.))
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(category.to_owned()),
                        )
                    })
                    .child(div().w(px(76.)).child(status_chip(&row.status, palette)))
                    .when(!narrow, |cells| {
                        cells.child(
                            div()
                                .w(px(110.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(date),
                        )
                    }),
            );
        }
        if in_view > 0 {
            let remaining = in_view - shown;
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("Showing {shown} of {in_view} in this view"))
                    .when(remaining > 0, |footer| {
                        footer.child(
                            Button::new("documents-show-more")
                                .small()
                                .label(format!(
                                    "Show {} more · {remaining} left",
                                    ROW_BATCH.min(remaining)
                                ))
                                .on_click(cx.listener(|view, _, _, cx| view.show_more(cx))),
                        )
                    }),
            );
        }

        // With a record open the list keeps a column and the reader takes the rest; with nothing
        // open the list is the page.
        let reader: Option<AnyElement> = selected.as_ref().map(|_| {
            let page = self.reader.page;
            let pages = self.reader.pages;
            let reading_text = self.reader.reading_text;
            let zoom = self.reader.zoom;
            let suggestion = self
                .detail
                .value
                .as_ref()
                .and_then(|detail| detail.document.suggested_category.clone());
            let can_open_externally = self
                .detail
                .value
                .as_ref()
                .is_some_and(|detail| detail.document.parent_id.is_none());
            let page_label = if pages == 0 {
                "Page —".to_owned()
            } else if let Some(range) = self
                .detail
                .value
                .as_ref()
                .and_then(|detail| detail.document.page_range)
            {
                let source_page = range
                    .first
                    .saturating_add(u32::try_from(page).unwrap_or(u32::MAX));
                format!(
                    "Source page {source_page} · child {}-{}",
                    range.first, range.last
                )
            } else {
                format!("Page {} of {}", page + 1, pages)
            };
            let detail: AnyElement = match (&self.detail.value, &self.detail.error) {
                (Some(detail), _) => div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(detail_panel(detail, palette))
                    .child(workflow_panel(self, detail, &split_input, palette, cx))
                    .child(proposal_panel(self, detail, palette, cx))
                    .into_any_element(),
                (_, Some(error)) => div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string())
                    .into_any_element(),
                _ => div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Opening document…")
                    .into_any_element(),
            };
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(detail))
                        .child(
                            Button::new("document-close")
                                .small()
                                .label("Close ✕ (Esc)")
                                .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("document-page-prev")
                                .small()
                                .label("‹ Page")
                                .on_click(cx.listener(|view, _, _, cx| view.go_to_page(-1, cx))),
                        )
                        .child(
                            Button::new("document-page-next")
                                .small()
                                .label("Page ›")
                                .on_click(cx.listener(|view, _, _, cx| view.go_to_page(1, cx))),
                        )
                        .child(
                            div()
                                .min_w(px(80.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(page_label),
                        )
                        .child(
                            Button::new("document-zoom")
                                .small()
                                .label(zoom.label())
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.set_zoom(zoom.other(), cx);
                                })),
                        )
                        .child(
                            Button::new("document-read-text")
                                .small()
                                .label(if reading_text { "Pages" } else { "Text" })
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.show_text(!reading_text, cx);
                                })),
                        )
                        .when(can_open_externally, |bar| {
                            bar.child(
                                Button::new("document-open-external")
                                    .small()
                                    .label("Open in Preview")
                                    .on_click(cx.listener(|view, _, _, _| view.open_externally())),
                            )
                        })
                        .child(div().flex_1())
                        .when_some(suggestion, |bar, category| {
                            bar.child(
                                Button::new("document-file-suggestion")
                                    .small()
                                    .label(format!("File as {category}"))
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.file_suggestion(cx)),
                                    ),
                            )
                        }),
                )
                .when(reading_text, |el| el.child(text_panel(self, palette)))
                .when(!reading_text, |el| el.child(self.page_element(palette)))
                .into_any_element()
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Documents"),
                    )
                    .child(
                        Button::new("documents-duplicate-review")
                            .small()
                            .label("Review duplicates")
                            .on_click(
                                cx.listener(|view, _, _, cx| view.set_duplicate_mode(true, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .when(reader.is_some(), |el| el.w(px(380.)).min_w(px(320.)))
                            .when(reader.is_none(), |el| el.flex_1())
                            .flex()
                            .flex_col()
                            // A scroll area's min-content width is the table's whole width; without
                            // this the column refuses to shrink and pushes the reader off screen.
                            .min_w_0()
                            .min_h_0()
                            .id("documents-list")
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("documents-rows")
                                    .flex_1()
                                    .min_h_0()
                                    .w_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.list_scroll)
                                    .vertical_scrollbar(&self.list_scroll)
                                    .child(table.w_full()),
                            ),
                    )
                    .when_some(reader, |el, reader| el.child(reader)),
            )
    }
}

fn duplicate_filter_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    Button::new(id)
        .small()
        .label(if selected {
            format!("✓ {label}")
        } else {
            label.to_owned()
        })
        .on_click(on_click)
        .into_any_element()
}

fn parse_page_ranges(value: &str) -> Result<Vec<DuplicatePageRange>, String> {
    let mut ranges = Vec::new();
    for part in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (first, last) = part
            .split_once('-')
            .ok_or_else(|| "Use inclusive ranges such as 1-3, 4-8.".to_owned())?;
        let first = first
            .trim()
            .parse::<u32>()
            .map_err(|_| "Page range start must be a positive number.".to_owned())?;
        let last = last
            .trim()
            .parse::<u32>()
            .map_err(|_| "Page range end must be a positive number.".to_owned())?;
        if first == 0 || last < first {
            return Err("Ranges must be positive and ordered, for example 1-3, 4-8.".into());
        }
        ranges.push(DuplicatePageRange { first, last });
    }
    if ranges.is_empty() {
        return Err("Enter every child range, for example 1-3, 4-8.".into());
    }
    Ok(ranges)
}

fn reader_page_element(reader: &Reader, palette: Palette) -> AnyElement {
    if let Some(image) = reader.image() {
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

fn format_bytes(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn status_chip(status: &str, palette: Palette) -> AnyElement {
    let (color, text) = match status {
        "inbox" => (palette.amber, "inbox"),
        "filed" => (palette.green, "filed"),
        _ => (palette.muted, status),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgba((color & 0xffffff) | 0x22000000))
        .text_color(rgb(color))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(text.to_owned())
        .into_any_element()
}

/// The locally extracted text, which is the only view a format without a renderer has.
fn text_panel(view: &Documents, palette: Palette) -> AnyElement {
    let text = view
        .detail
        .value
        .as_ref()
        .and_then(|detail| detail.text.clone());
    div()
        .id("text-scroll-panel")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&view.text_scroll)
        .vertical_scrollbar(&view.text_scroll)
        .p_4()
        .rounded(px(6.))
        .bg(rgba(palette.surface))
        .text_sm()
        .line_height(px(22.))
        .text_color(rgb(palette.text))
        .child(text.unwrap_or_else(|| "No text extraction is available for this file.".into()))
        .into_any_element()
}

fn proposal_source_label(source: &ProposalSource) -> String {
    match source {
        ProposalSource::Rules { identity, version } => format!("Rules · {identity} · {version}"),
        ProposalSource::Model {
            role,
            model,
            revision,
        } => format!(
            "Model · {role} · {model}{}",
            revision
                .as_deref()
                .map(|revision| format!(" · {revision}"))
                .unwrap_or_default()
        ),
    }
}

fn proposal_outcome_label(outcome: &ProposalOutcome) -> String {
    match outcome {
        ProposalOutcome::Proposed { category, .. } => category.clone(),
        ProposalOutcome::Abstained { reason } => format!(
            "No category · {}",
            match reason {
                shell::ProposalAbstentionReason::NoMatch => "no rule matched",
                shell::ProposalAbstentionReason::MissingEvidence => "evidence missing",
                shell::ProposalAbstentionReason::ModelUnavailable => "source unavailable",
                shell::ProposalAbstentionReason::CategoryNotConfigured => "category not configured",
            }
        ),
        ProposalOutcome::Invalid { reason } => format!(
            "Invalid · {}",
            match reason {
                shell::ProposalInvalidReason::MalformedOutput => "malformed output",
                shell::ProposalInvalidReason::EvidenceNotExact => "evidence was not exact",
                shell::ProposalInvalidReason::EvidenceTooLong => "evidence exceeded limits",
                shell::ProposalInvalidReason::InvalidCategory => "category was invalid",
            }
        ),
    }
}

fn proposal_review_label(review: ProposalReview) -> &'static str {
    match review {
        ProposalReview::Accepted => "Agreed",
        ProposalReview::Rejected => "Rejected",
        ProposalReview::Overridden => "Overridden",
    }
}

fn proposal_panel(
    view: &Documents,
    detail: &DocumentDetail,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let latest_generation = detail
        .proposals
        .iter()
        .max_by(|left, right| {
            left.generated_at
                .cmp(&right.generated_at)
                .then_with(|| left.generation_id.cmp(&right.generation_id))
        })
        .map(|record| record.generation_id.as_str());
    let latest: Vec<_> = detail
        .proposals
        .iter()
        .filter(|record| Some(record.generation_id.as_str()) == latest_generation)
        .collect();
    let mut categories = BTreeMap::<String, usize>::new();
    for record in &latest {
        if let ProposalOutcome::Proposed { category, .. } = &record.proposal.outcome {
            *categories.entry(category.clone()).or_default() += 1;
        }
    }
    let proposed_count: usize = categories.values().sum();
    let summary = if categories.len() > 1 {
        format!(
            "Sources disagree: {}",
            categories.keys().cloned().collect::<Vec<_>>().join(" · ")
        )
    } else if proposed_count > 1 {
        format!(
            "Sources agree: {}",
            categories.keys().next().cloned().unwrap_or_default()
        )
    } else if proposed_count == 1 {
        format!(
            "One proposal: {}",
            categories.keys().next().cloned().unwrap_or_default()
        )
    } else if latest.is_empty() {
        "No proposal records yet".into()
    } else {
        "No category proposed in this run".into()
    };
    let older_runs = detail
        .proposals
        .iter()
        .filter(|record| Some(record.generation_id.as_str()) != latest_generation)
        .map(|record| record.generation_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut visible: Vec<_> = detail
        .proposals
        .iter()
        .filter(|record| {
            view.show_proposal_history || Some(record.generation_id.as_str()) == latest_generation
        })
        .collect();
    visible.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let busy = view.proposal_busy.is_some();
    let mut panel = div()
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded(px(6.))
        .bg(rgba(palette.raised))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Classification proposals"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("document-regenerate-proposals")
                        .small()
                        .ghost()
                        .label(if view.proposal_busy.as_deref() == Some("regenerate") {
                            "Saving…"
                        } else {
                            "Regenerate rules"
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.regenerate_proposals(cx))),
                )
                .child(
                    Button::new("document-model-proposal")
                        .small()
                        .ghost()
                        .label(if view.proposal_busy.as_deref() == Some("model") {
                            "Asking local model…"
                        } else {
                            "Ask local model"
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.generate_model_proposal(cx))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("A local model can suggest a category and exact text evidence. Review it before filing."),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(if categories.len() > 1 {
                    palette.amber
                } else {
                    palette.muted
                }))
                .child(summary),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Review records an assessment only. Filing remains a separate action."),
        );
    if let Some(error) = view.proposal_error.as_ref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(format!("Proposal action failed: {error}")),
        );
    }
    if let Some(notice) = view.proposal_notice.as_ref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(notice.clone()),
        );
    }
    if older_runs > 0 {
        panel = panel.child(
            Button::new("document-toggle-proposal-history")
                .small()
                .ghost()
                .label(if view.show_proposal_history {
                    "Show latest run only"
                } else {
                    "Show earlier runs"
                })
                .on_click(cx.listener(|view, _, _, cx| view.toggle_proposal_history(cx))),
        );
    }
    if visible.is_empty() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Generate a rules run to record a source result."),
        );
    }
    panel
        .children(visible.into_iter().map(|record| {
            let source = proposal_source_label(&record.proposal.source);
            let outcome = proposal_outcome_label(&record.proposal.outcome);
            let evidence = match &record.proposal.outcome {
                ProposalOutcome::Proposed { evidence, .. } => Some((
                    match evidence.source {
                        ProposalEvidenceSource::Filename => "Filename evidence",
                        ProposalEvidenceSource::ExtractedText => "Text evidence",
                    },
                    evidence.quote.clone(),
                )),
                _ => None,
            };
            let review_history = if record.review_history.is_empty() {
                "Not reviewed".to_owned()
            } else {
                record
                    .review_history
                    .iter()
                    .map(|event| {
                        format!(
                            "{} · {}",
                            proposal_review_label(event.outcome),
                            event.reviewed_at.get(..10).unwrap_or("date unknown")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            let proposal_id = record.id.clone();
            let source_id = record.id.clone();
            let override_id = record.id.clone();
            let latest_review = record
                .proposal
                .review
                .map(proposal_review_label)
                .unwrap_or("Not reviewed");
            let confidence = record
                .proposal
                .confidence
                .map(|confidence| format!("Confidence {confidence:.2}"));
            let generated = record
                .generated_at
                .get(..16)
                .unwrap_or(&record.generated_at);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(4.))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(source),
                        )
                        .child(div().text_xs().child(outcome))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(latest_review),
                        )
                        .when_some(confidence, |row, confidence| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(confidence),
                            )
                        }),
                )
                .when_some(evidence, |row, (label, quote)| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{label}: “{quote}”")),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Run {generated} · review history: {review_history}"
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new(format!("document-proposal-agree-{}", record.id))
                                .xsmall()
                                .ghost()
                                .label("Agree")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        proposal_id.clone(),
                                        ProposalReview::Accepted,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new(format!("document-proposal-reject-{}", source_id))
                                .xsmall()
                                .ghost()
                                .label("Reject")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        source_id.clone(),
                                        ProposalReview::Rejected,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new(format!("document-proposal-override-{}", override_id))
                                .xsmall()
                                .ghost()
                                .label("Override")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        override_id.clone(),
                                        ProposalReview::Overridden,
                                        cx,
                                    )
                                })),
                        ),
                )
        }))
        .into_any_element()
}

fn workflow_panel(
    view: &Documents,
    detail: &DocumentDetail,
    split_input: &Entity<InputState>,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let document = &detail.document;
    let is_pdf = document.media_type == "application/pdf";
    let is_original = document.parent_id.is_none();
    let mut panel = div().flex().flex_col().gap_1();

    if is_pdf {
        let analyzing = view.workflow_busy.as_deref() == Some("analyze");
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("document-analyze-pages")
                        .small()
                        .label(if analyzing {
                            "Analyzing locally…"
                        } else if document.page_count == 0 {
                            "Analyze pages locally"
                        } else {
                            "Refresh page analysis"
                        })
                        .on_click(cx.listener(|view, _, _, cx| view.analyze_selected_document(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} analyzed page(s)", document.page_count)),
                ),
        );

        if let Some(suggestion) = detail.split_suggestions.as_ref() {
            if suggestion.boundaries.is_empty() {
                if let Some(reason) = suggestion.abstention.as_ref() {
                    panel = panel.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("No automatic cut: {reason}")),
                    );
                }
            } else {
                let cuts = suggestion
                    .boundaries
                    .iter()
                    .map(|boundary| boundary.before_page.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("Page-one markers suggest cuts before pages {cuts}. Review each page before splitting.")),
                );
                for boundary in &suggestion.boundaries {
                    panel = panel.child(div().text_xs().text_color(rgb(palette.muted)).child(
                        format!(
                            "Page {} · {} · {}",
                            boundary.before_page, boundary.marker, boundary.quote
                        ),
                    ));
                }
            }
        }

        if is_original && document.page_count > 1 {
            let splitting = view.workflow_busy.as_deref() == Some("split");
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mt(px(2.))
                    .child(div().w(px(220.)).child(Input::new(split_input).small()))
                    .child(
                        Button::new("document-create-split")
                            .small()
                            .label(if splitting {
                                "Creating children…"
                            } else {
                                "Split into records"
                            })
                            .on_click(
                                cx.listener(|view, _, _, cx| view.split_selected_document(cx)),
                            ),
                    )
                    .when_else(
                        detail
                            .split_suggestions
                            .as_ref()
                            .is_some_and(|suggestion| !suggestion.ranges.is_empty()),
                        |row| {
                            row.child(
                                Button::new("document-use-split-suggestions")
                                    .small()
                                    .ghost()
                                    .label("Use page-marker suggestion")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.use_split_suggestions(window, cx)
                                    })),
                            )
                        },
                        |row| row,
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Use every page once, e.g. 1-2,3-5. Original stays intact."),
                    ),
            );
        }

        if !detail.children.is_empty() {
            panel = panel.child(
                div()
                    .mt(px(3.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Child records · independent PDFs linked to the source"),
            );
            for child in &detail.children {
                let id = child.id.clone();
                let label = child.page_range.map_or_else(
                    || child.filename.clone(),
                    |range| format!("Pages {}-{} · {}", range.first, range.last, child.filename),
                );
                panel = panel.child(
                    Button::new(format!("document-child-{}", &child.id[..12]))
                        .small()
                        .label(label)
                        .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx))),
                );
            }
        }

        if !detail.pages.is_empty() {
            panel = panel.child(
                div()
                    .mt(px(3.))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("Page evidence"),
            );
            for page in &detail.pages {
                let preview = page
                    .text
                    .split_whitespace()
                    .take(28)
                    .collect::<Vec<_>>()
                    .join(" ");
                let provenance = page.failure.as_ref().map_or_else(
                    || format!("{} · {} chars", page.engine, page.text.chars().count()),
                    |failure| format!("{} · extraction issue: {failure}", page.engine),
                );
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(4.))
                        .px_2()
                        .py(px(2.))
                        .bg(rgba(palette.raised))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("Page {} · {provenance}", page.page_number)),
                        )
                        .child(div().text_xs().child(preview)),
                );
            }
        }
    }

    if let Some(error) = &view.workflow_error {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(error.clone()),
        );
    }
    if is_pdf && !is_original {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "Child of {} · pages {}-{}",
                    document.parent_id.as_deref().unwrap_or("unknown"),
                    document.page_range.map_or(0, |range| range.first),
                    document.page_range.map_or(0, |range| range.last)
                )),
        );
    }
    panel.into_any_element()
}

fn detail_panel(detail: &DocumentDetail, palette: Palette) -> AnyElement {
    let doc = &detail.document;
    let has_distinct_title = doc
        .extracted_title
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty() && t.trim() != doc.filename.trim());

    div()
        .flex()
        .flex_col()
        .gap_1()
        .when(has_distinct_title, |col| {
            col.child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(doc.extracted_title.clone().unwrap_or_default()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .child(doc.filename.clone()),
            )
        })
        .when(!has_distinct_title, |col| {
            col.child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(doc.filename.clone()),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pt(px(2.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(status_chip(&doc.status, palette))
                .child(div().child("·"))
                .child(div().child(doc.category.as_deref().unwrap_or("Unfiled").to_owned()))
                .child(div().child("·"))
                .child(div().child(format_bytes(doc.byte_len)))
                .child(div().child("·"))
                .child(div().child(doc.imported_at.get(..10).unwrap_or("—").to_owned()))
                .when_some(doc.project.as_ref(), |el, proj| {
                    el.child(div().child("·"))
                        .child(div().child(format!("in {proj}")))
                }),
        )
        .when_some(doc.suggestion_reason.as_ref(), |el, reason| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .mt(px(2.))
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(rgba((palette.amber & 0xffffff) | 0x18000000))
                    .text_xs()
                    .text_color(rgb(palette.amber))
                    .child(format!("Suggested: {reason}")),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{parse_page_ranges, workflow_label, Filter, Reader, Zoom};
    use crate::shell::DocumentSummary;

    #[test]
    fn filters_match_document_statuses() {
        let mut row = DocumentSummary {
            id: "id".into(),
            filename: "document.pdf".into(),
            media_type: "application/pdf".into(),
            byte_len: 1,
            imported_at: String::new(),
            status: "inbox".into(),
            category: None,
            project: None,
            extracted_title: None,
            extractor: None,
            extracted_chars: 0,
            suggested_category: None,
            suggestion_reason: None,
            data_class: "c2".into(),
            page_count: 0,
            parent_id: None,
            page_range: None,
            matched_pages: Vec::new(),
            workflow_stage: "analyze".into(),
        };
        assert!(Filter::Inbox.matches(&row));
        assert!(!Filter::Filed.matches(&row));
        assert!(Filter::All.matches(&row));
        assert!(Filter::NeedsReview.matches(&row));
        row.workflow_stage.clear();
        assert_eq!(workflow_label(&row.workflow_stage), "Needs review");
        assert!(Filter::NeedsReview.matches(&row));
        row.workflow_stage = "split".into();
        assert!(!Filter::NeedsReview.matches(&row));
        row.status = "filed".into();
        assert!(Filter::Filed.matches(&row));
        assert!(Filter::All.matches(&row));
        row.status = "archived".into();
        assert!(!Filter::All.matches(&row));
    }

    #[test]
    fn paging_stops_at_both_ends_instead_of_wrapping() {
        let mut reader = Reader {
            pages: 3,
            ..Reader::default()
        };
        assert!(!reader.next_page(-1), "page one has nothing before it");
        assert!(reader.next_page(1));
        assert_eq!(reader.page, 1);
        assert!(reader.next_page(1));
        assert_eq!(reader.page, 2);
        assert!(!reader.next_page(1), "the last page is the last page");
        assert_eq!(reader.page, 2);
    }

    #[test]
    fn a_document_with_no_pages_does_not_move() {
        let mut reader = Reader::default();
        assert!(!reader.next_page(1));
        assert_eq!(reader.page, 0);
    }

    #[test]
    fn the_trim_keeps_the_pages_near_the_one_on_screen() {
        let mut reader = Reader {
            pages: 40,
            page: 20,
            ..Reader::default()
        };
        let width = Zoom::Page.width();
        for page in 0..40 {
            reader.drawn.insert(
                (page, width),
                std::sync::Arc::new(gpui::RenderImage::new(vec![])),
            );
        }
        reader.trim();
        assert_eq!(reader.drawn.len(), super::CACHED_PAGES);
        // Page 20 and its neighbours survive; page 0 does not.
        assert!(reader.drawn.contains_key(&(20, width)));
        assert!(!reader.drawn.contains_key(&(0, width)));
    }

    #[test]
    fn a_batch_is_a_screenful_not_the_whole_library() {
        // The list draws `ROW_BATCH` rows and grows by that much per press, so a library of a few
        // thousand cannot put every row in one frame.
        assert_eq!(super::ROW_BATCH, 100);
        const { assert!(super::ROW_BATCH < 3101) };
    }

    #[test]
    fn a_drawn_page_is_asked_for_at_the_zoom_that_is_on_screen() {
        assert_ne!(Zoom::Page.width(), Zoom::Close.width());
        assert_eq!(Zoom::Page.other(), Zoom::Close);
        assert_eq!(Zoom::Close.other(), Zoom::Page);
    }

    #[test]
    fn zoom_labels_do_not_collide_with_close() {
        assert_ne!(Zoom::Page.label(), "Close");
        assert_ne!(Zoom::Close.label(), "Close");
        assert_eq!(Zoom::Page.label(), "Fit");
        assert_eq!(Zoom::Close.label(), "2×");
    }

    #[test]
    fn format_bytes_human_readable() {
        assert_eq!(super::format_bytes(500), "500 B");
        assert_eq!(super::format_bytes(1024), "1.0 KB");
        assert_eq!(super::format_bytes(138041), "134.8 KB");
        assert_eq!(super::format_bytes(1048576), "1.0 MB");
    }

    #[test]
    fn split_input_parses_inclusive_one_based_ranges() {
        let ranges = parse_page_ranges("1-3, 4-8").unwrap();
        assert_eq!(
            ranges
                .iter()
                .map(|range| (range.first, range.last))
                .collect::<Vec<_>>(),
            vec![(1, 3), (4, 8)]
        );
        assert!(parse_page_ranges("").is_err());
        assert!(parse_page_ranges("0-3").is_err());
        assert!(parse_page_ranges("4-2").is_err());
        assert!(parse_page_ranges("1..3").is_err());
    }
}
