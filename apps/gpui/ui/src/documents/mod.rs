//! Native Documents library and reader for the shared GPUI surface.
//!
//! The list is metadata only. Opening a record reads its locally extracted text and its
//! hash-verified stored PDF, and draws that record's pages through [`PdfRenderer`]. Proposal
//! reviews and regenerations go through the Documents API; filing remains a separate explicit
//! `PATCH /api/documents/{id}` action.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, Context, Entity, ExternalPaths,
    FontWeight, PathPromptOptions, Subscription, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::shell::{
    self, DocumentDetail, DocumentSummary, DocumentUpload, DuplicateCandidate,
    DuplicateCandidateKind, DuplicateCandidatePage, ShellError,
};

use crate::data::{Remote, Repo};
use crate::primitives::{
    column_width, cycle_column_width, filter_pill, resizable_table_header, tabular_cell,
    virtual_list, SortOrder, SortableColumn,
};
use crate::theme::{tint, Density, Palette};

mod duplicates;
mod proposals;
pub mod viewer;

use proposals::{proposal_panel, workflow_panel};
pub use viewer::{register_renderer, set_default_pdf_renderer, PdfError, PdfRenderer};
use viewer::{search_extracted_pages, PageSearchHit, Reader, Zoom, DEFAULT_RENDERER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Inbox,
    Filed,
    All,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocSort {
    #[default]
    Added,
    Name,
    Category,
    Pages,
    Status,
}

impl DocSort {
    fn default_order(self) -> SortOrder {
        match self {
            Self::Added | Self::Pages => SortOrder::Descending,
            Self::Name | Self::Category | Self::Status => SortOrder::Ascending,
        }
    }

    fn button_label(self, order: SortOrder) -> String {
        match (self, order) {
            (Self::Added, SortOrder::Descending) => "Newest".to_owned(),
            (Self::Added, SortOrder::Ascending) => "Oldest".to_owned(),
            (Self::Name, SortOrder::Ascending) => "Name A–Z".to_owned(),
            (Self::Name, SortOrder::Descending) => "Name Z–A".to_owned(),
            (Self::Category, o) => format!("Category{}", o.glyph()),
            (Self::Pages, o) => format!("Pages{}", o.glyph()),
            (Self::Status, o) => format!("Status{}", o.glyph()),
        }
    }
}

fn compare_documents(
    left: &DocumentSummary,
    right: &DocumentSummary,
    sort: DocSort,
    order: SortOrder,
) -> std::cmp::Ordering {
    let category = |row: &DocumentSummary| {
        row.category
            .as_deref()
            .or(row.suggested_category.as_deref())
            .unwrap_or("")
            .to_lowercase()
    };
    let primary = match sort {
        DocSort::Added => left.imported_at.cmp(&right.imported_at),
        DocSort::Name => left
            .filename
            .to_lowercase()
            .cmp(&right.filename.to_lowercase()),
        DocSort::Category => category(left).cmp(&category(right)),
        DocSort::Pages => left.page_count.cmp(&right.page_count),
        DocSort::Status => left.status.cmp(&right.status),
    };
    let primary = if order.is_ascending() {
        primary
    } else {
        primary.reverse()
    };
    primary
        .then_with(|| right.imported_at.cmp(&left.imported_at))
        .then_with(|| {
            left.filename
                .to_lowercase()
                .cmp(&right.filename.to_lowercase())
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Grouping {
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

/// One row of the list: where it comes from, and the group header it opens when it is first in one.
pub(crate) struct DocRow {
    /// Index into `rows.value`.
    index: usize,
    /// The group label and its size, when a header belongs above this row.
    group: Option<(String, usize)>,
}

/// Whether one record answers the search box. The caller owns `search_applied`, which skips it.
fn document_matches(row: &DocumentSummary, query: &str) -> bool {
    row.filename.to_lowercase().contains(query)
        || row
            .extracted_title
            .as_deref()
            .is_some_and(|title| title.to_lowercase().contains(query))
        || row
            .category
            .as_deref()
            .is_some_and(|category| category.to_lowercase().contains(query))
        || row
            .project
            .as_deref()
            .is_some_and(|project| project.to_lowercase().contains(query))
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

pub struct Documents {
    pub(super) repo: Repo,
    pub(super) palette: Option<Palette>,
    pub(super) density: Density,
    pub rows: Remote<Vec<DocumentSummary>>,
    pub detail: Remote<DocumentDetail>,
    pub filter: Filter,
    pub search_query: String,
    pub(super) grouping: Grouping,
    pub(super) category_filter: Option<String>,
    pub(super) search_revision: u64,
    pub(super) search_applied: bool,
    pub(super) search_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) sort: (DocSort, SortOrder),
    pub(super) col_widths: std::collections::HashMap<&'static str, f32>,
    pub(super) selected: Option<String>,
    /// The list's own scroller: it draws only the rows the window shows.
    pub(super) list: gpui::ListState,
    /// The rows in draw order, each with the group header that opens its group.
    pub(super) order: Vec<DocRow>,
    pub(super) counts: [usize; 5],
    pub(super) order_key: Option<(u64, usize, usize)>,
    /// Counts a change to the filters and the sort, which the records cannot report.
    pub(super) order_rev: u64,
    /// Extracted text is its own scroll area when reading text mode is on.
    pub(super) text_scroll: gpui::ScrollHandle,
    pub(super) thumb_scroll: gpui::ScrollHandle,
    pub(super) page_search_query: String,
    pub(super) page_search_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) reader: Reader,
    pub(super) renderer: Option<Arc<dyn PdfRenderer>>,
    pub(super) renderer_error: Option<String>,
    pub(super) duplicate_mode: bool,
    pub(super) duplicate_kind: Option<DuplicateCandidateKind>,
    pub(super) duplicate_offset: usize,
    pub(super) duplicate_page: Remote<DuplicateCandidatePage>,
    pub(super) duplicate_selection: Option<DuplicateCandidate>,
    pub(super) compare_readers: [Reader; 2],
    pub(super) preferred_id: Option<String>,
    pub(super) split_ranges: String,
    pub(super) duplicate_error: Option<String>,
    pub(super) split_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) proposal_busy: Option<String>,
    pub(super) proposal_error: Option<String>,
    pub(super) proposal_notice: Option<String>,
    pub(super) show_proposal_history: bool,
    pub(super) workflow_busy: Option<String>,
    pub(super) workflow_error: Option<String>,
    pub(super) uploading: usize,
    pub(super) upload_notice: Option<(bool, String)>,
    pub(super) page_offset: usize,
    pub(super) total_filtered: usize,
    pub(super) filing_compare_id: Option<String>,
    pub(super) filing_compare_detail: Remote<DocumentDetail>,
    pub(super) filing_compare_reader: Reader,
    pub(super) show_compare_picker: bool,
}

impl Documents {
    pub fn new(repo: Repo) -> Self {
        let renderer = match DEFAULT_RENDERER.get() {
            Some(factory) => factory(),
            None => crate::pdf_kit::default_renderer(),
        };
        Self::with_renderer(repo, renderer)
    }

    pub fn with_renderer(repo: Repo, renderer: Result<Arc<dyn PdfRenderer>, String>) -> Self {
        let (renderer, renderer_error) = match renderer {
            Ok(renderer) => (Some(renderer), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            repo,
            palette: None,
            density: Density::Desktop,
            rows: Remote::default(),
            detail: Remote::default(),
            filter: Filter::Inbox,
            search_query: String::new(),
            grouping: Grouping::None,
            category_filter: None,
            search_revision: 0,
            search_applied: false,
            search_input: None,
            sort: (DocSort::Added, SortOrder::Descending),
            col_widths: Default::default(),
            selected: None,
            list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.)),
            order: Vec::new(),
            counts: [0; 5],
            order_key: None,
            order_rev: 0,
            text_scroll: gpui::ScrollHandle::new(),
            thumb_scroll: gpui::ScrollHandle::new(),
            page_search_query: String::new(),
            page_search_input: None,
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
            uploading: 0,
            upload_notice: None,
            page_offset: 0,
            total_filtered: 0,
            filing_compare_id: None,
            filing_compare_detail: Remote::default(),
            filing_compare_reader: Reader::default(),
            show_compare_picker: false,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density != density {
            self.density = density;
            cx.notify();
        }
    }

    pub fn set_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.repo = repo;
        self.load(cx);
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

    pub(super) fn prompt_import(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import documents".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.import_paths(paths, cx);
            });
        })
        .detach();
    }

    /// Read dropped or selected files locally and upload their raw bytes to the Documents API.
    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.upload_notice = Some((true, error.to_string()));
                cx.notify();
                return;
            }
        };
        let batch_len = paths.len();
        self.uploading = self.uploading.saturating_add(batch_len);
        self.upload_notice = None;
        cx.notify();

        let work = shell::run(async move {
            let mut results: Vec<Result<(String, DocumentUpload), String>> =
                Vec::with_capacity(paths.len());
            for path in paths {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned);
                let Some(filename) = filename else {
                    results.push(Err("Skipped a path without a valid filename.".into()));
                    continue;
                };
                let read_path = path.clone();
                let bytes =
                    match tokio::task::spawn_blocking(move || std::fs::read(&read_path)).await {
                        Ok(Ok(bytes)) => bytes,
                        Ok(Err(err)) => {
                            results.push(Err(format!("{filename}: {err}")));
                            continue;
                        }
                        Err(_) => {
                            results.push(Err(format!("{filename}: could not read file.")));
                            continue;
                        }
                    };
                match shell.upload_document(&filename, bytes).await {
                    Ok(upload) => results.push(Ok((filename, upload))),
                    Err(err) => results.push(Err(format!("{filename}: {err}"))),
                }
            }
            results
        });

        cx.spawn(async move |this, cx| {
            let results = work
                .await
                .unwrap_or_else(|| vec![Err(ShellError::Unreachable.to_string())]);
            let _ = this.update(cx, |view, cx| {
                view.uploading = view.uploading.saturating_sub(batch_len);
                let mut created = 0usize;
                let mut existing = 0usize;
                let mut last_id: Option<String> = None;
                let mut last_filename: Option<String> = None;
                let mut errors: Vec<String> = Vec::new();
                for item in results {
                    match item {
                        Ok((filename, upload)) => {
                            if upload.created {
                                created += 1;
                            } else {
                                existing += 1;
                            }
                            last_id = Some(upload.document.id.clone());
                            last_filename = Some(filename);
                            let rows = view.rows.value.get_or_insert_with(Vec::new);
                            upsert_uploaded_document(rows, upload.document);
                        }
                        Err(err) => errors.push(err),
                    }
                }
                if created + existing > 0 {
                    view.order_rev = view.order_rev.wrapping_add(1);
                }
                view.upload_notice = Some(format_upload_summary(
                    created,
                    existing,
                    last_filename.as_deref(),
                    &errors,
                ));
                if created + existing == 1 {
                    if let Some(id) = last_id {
                        view.open(id, cx);
                    }
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
            self.page_search_query.clear();
            self.page_search_input = None;
            self.proposal_busy = None;
            self.workflow_busy = None;
            self.workflow_error = None;
            self.proposal_error = None;
            self.proposal_notice = None;
            self.show_proposal_history = false;
            cx.notify();
        }
    }

    /// Step through the rows the list draws by `by` (+1 next, -1 prev).
    ///
    /// It walks the drawn order, so a category filter, a grouping or a search narrows what a key
    /// steps through as well as what is on screen. Nothing is filtered here: the order the frame
    /// already built is the order the keyboard moves through.
    pub fn select_relative(&mut self, by: isize, cx: &mut Context<Self>) {
        self.sync_order();
        if self.order.is_empty() {
            return;
        }
        let current = self.selected.as_deref().and_then(|selected| {
            let rows = self.rows.value.as_ref()?;
            self.order
                .iter()
                .position(|entry| rows.get(entry.index).is_some_and(|row| row.id == selected))
        });
        let next = match current {
            Some(at) => (at as isize + by).clamp(0, self.order.len() as isize - 1) as usize,
            None => 0,
        };
        let next_id = self
            .rows
            .value
            .as_ref()
            .and_then(|rows| self.order.get(next).and_then(|entry| rows.get(entry.index)))
            .map(|row| row.id.clone());
        if let Some(next_id) = next_id {
            self.open(next_id, cx);
        }
    }

    /// Open the current document original in the default host application (e.g. Preview).
    pub fn open_externally(&mut self) {
        let (Some(renderer), Some(bytes), Some(detail)) = (
            self.renderer.as_ref(),
            &self.reader.bytes,
            self.detail.value.as_ref(),
        ) else {
            return;
        };
        if detail.document.parent_id.is_some() {
            return;
        }
        renderer.open_external(&detail.document.filename, bytes.as_slice());
    }

    /// Open one record: its metadata and text first, then the original for the page view.
    fn open(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.selected = Some(id.clone());
        self.detail = Remote::new_loading();
        self.reader.clear();
        self.page_search_query.clear();
        self.page_search_input = None;
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

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let rows = self.order.len();
        let item_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
        self.list.scroll_to(gpui::ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        cx.notify();
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
                    view.page_offset = 0;
                    view.order_rev = view.order_rev.wrapping_add(1);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => view.search_documents(cx),
                _ => {}
            });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn page_search_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.page_search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in document…"));
        let subscription =
            cx.subscribe_in(&input, window, |view, input, event, _, cx| match event {
                InputEvent::Change => {
                    view.page_search_query = input.read(cx).value().trim().to_owned();
                    if let Some(first_hit) = view.page_search_hits().first() {
                        view.go_to_page_index(first_hit.page_index, cx);
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    let hits = view.page_search_hits();
                    if !hits.is_empty() {
                        let next_ix = hits
                            .iter()
                            .find(|hit| hit.page_index > view.reader.page)
                            .or_else(|| hits.first())
                            .map(|hit| hit.page_index);
                        if let Some(target) = next_ix {
                            view.go_to_page_index(target, cx);
                        }
                    }
                }
                _ => {}
            });
        self.page_search_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn page_search_hits(&self) -> Vec<PageSearchHit> {
        let Some(detail) = self.detail.value.as_ref() else {
            return Vec::new();
        };
        search_extracted_pages(
            &detail.pages,
            self.reader.first_page,
            self.reader.last_page,
            &self.page_search_query,
        )
    }

    pub(super) fn matched_page_numbers(&self) -> Vec<u32> {
        let local_hits: Vec<u32> = self
            .page_search_hits()
            .into_iter()
            .map(|hit| hit.page_number)
            .collect();
        if !local_hits.is_empty() {
            return local_hits;
        }
        self.selected
            .as_deref()
            .and_then(|id| {
                self.rows
                    .value
                    .as_ref()
                    .and_then(|rows| rows.iter().find(|r| r.id == id))
            })
            .map(|row| row.matched_pages.clone())
            .unwrap_or_default()
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.page_offset = 0;
        self.order_rev = self.order_rev.wrapping_add(1);
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
                    view.order_rev = view.order_rev.wrapping_add(1);
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

    fn cycle_grouping(&mut self, cx: &mut Context<Self>) {
        self.grouping = self.grouping.next();
        self.page_offset = 0;
        self.order_rev = self.order_rev.wrapping_add(1);
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
        self.page_offset = 0;
        self.order_rev = self.order_rev.wrapping_add(1);
        cx.notify();
    }

    /// The list's order, rebuilt only when the records, a filter, the grouping or the sort move.
    ///
    /// Group headers and their sizes are computed here, once per rebuild, rather than compared
    /// row by row on every frame.
    fn sync_order(&mut self) {
        let key = (
            self.order_rev,
            self.rows.value.as_ref().map_or(0, Vec::len),
            self.page_offset,
        );
        if self.order_key == Some(key) {
            return;
        }
        let mut picked: Vec<usize> = Vec::new();
        let mut summary_counts = [0; 5];
        if let Some(rows) = self.rows.value.as_ref() {
            summary_counts[0] = rows.len();
            for (index, row) in rows.iter().enumerate() {
                if row.status == "inbox" {
                    summary_counts[1] += 1;
                } else if row.status == "filed" {
                    summary_counts[2] += 1;
                }
                if row.status != "archived" {
                    summary_counts[3] += 1;
                }
                if Filter::NeedsReview.matches(row) {
                    summary_counts[4] += 1;
                }
                if self.filter.matches(row)
                    && self
                        .category_filter
                        .as_deref()
                        .is_none_or(|category| row.category.as_deref().unwrap_or("") == category)
                    && (self.search_applied
                        || self.search_query.is_empty()
                        || document_matches(row, &self.search_query))
                {
                    picked.push(index);
                }
            }
            let (sort_col, sort_order) = self.sort;
            match self.grouping {
                Grouping::None => {
                    if (sort_col, sort_order) != (DocSort::Added, SortOrder::Descending) {
                        picked.sort_by(|left, right| {
                            compare_documents(&rows[*left], &rows[*right], sort_col, sort_order)
                        });
                    }
                }
                grouping => picked.sort_by(|left, right| {
                    grouping_key(&rows[*left], grouping)
                        .cmp(&grouping_key(&rows[*right], grouping))
                        .then_with(|| {
                            compare_documents(&rows[*left], &rows[*right], sort_col, sort_order)
                        })
                }),
            }
        }
        self.counts = summary_counts;
        self.total_filtered = picked.len();
        let page_start = if self.total_filtered > 500 {
            (self.page_offset / 500) * 500
        } else {
            0
        };
        let page_end = (page_start + 500).min(self.total_filtered);
        let visible_picked = if self.total_filtered > 500 {
            &picked[page_start..page_end]
        } else {
            &picked[..]
        };

        let mut counts: Vec<(String, usize)> = Vec::new();
        if let Some(rows) = self
            .rows
            .value
            .as_ref()
            .filter(|_| self.grouping != Grouping::None)
        {
            for index in visible_picked {
                let label = grouping_key(&rows[*index], self.grouping);
                match counts.iter_mut().find(|(seen, _)| *seen == label) {
                    Some((_, size)) => *size += 1,
                    None => counts.push((label, 1)),
                }
            }
        }
        let mut order: Vec<DocRow> = Vec::with_capacity(visible_picked.len());
        let mut previous: Option<String> = None;
        for index in visible_picked {
            let index = *index;
            let group = self
                .rows
                .value
                .as_ref()
                .filter(|_| self.grouping != Grouping::None)
                .map(|rows| grouping_key(&rows[index], self.grouping))
                .filter(|label| previous.as_deref() != Some(label.as_str()))
                .map(|label| {
                    let size = counts
                        .iter()
                        .find(|(seen, _)| *seen == label)
                        .map_or(0, |(_, size)| *size);
                    previous = Some(label.clone());
                    (label, size)
                });
            order.push(DocRow { index, group });
        }
        self.order_key = Some(key);
        self.list.reset(order.len());
        self.order = order;
    }

    pub fn prev_page(&mut self, cx: &mut Context<Self>) {
        if self.page_offset >= 500 {
            self.page_offset = self.page_offset.saturating_sub(500);
            self.order_rev = self.order_rev.wrapping_add(1);
            cx.notify();
        }
    }

    pub fn next_page(&mut self, cx: &mut Context<Self>) {
        if self.page_offset + 500 < self.total_filtered {
            self.page_offset += 500;
            self.order_rev = self.order_rev.wrapping_add(1);
            cx.notify();
        }
    }

    pub(super) fn file_document_as(
        &mut self,
        id: String,
        category: String,
        cx: &mut Context<Self>,
    ) {
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
                    if let Some(detail) = &mut view.filing_compare_detail.value {
                        if detail.document.id == updated.id {
                            detail.document = updated.clone();
                        }
                    }
                    if let Some(rows) = &mut view.rows.value {
                        if let Some(row) = rows.iter_mut().find(|row| row.id == updated.id) {
                            *row = updated;
                            view.order_rev = view.order_rev.wrapping_add(1);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn adopt_compare_category(&mut self, cx: &mut Context<Self>) {
        let Some(compare_cat) = self
            .filing_compare_detail
            .value
            .as_ref()
            .and_then(|d| d.document.category.clone())
        else {
            return;
        };
        let Some(primary_id) = self.selected.clone() else {
            return;
        };
        self.file_document_as(primary_id, compare_cat, cx);
    }

    fn col_width(&self, id: &'static str, default: f32) -> f32 {
        column_width(&self.col_widths, id, default)
    }

    fn resize_column(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let default = match id {
            "category" => Some(110.),
            "pages" => Some(64.),
            "status" => Some(76.),
            "added" => Some(110.),
            _ => None,
        };
        if let Some(default) = default {
            cycle_column_width(&mut self.col_widths, id, default);
            cx.notify();
        }
    }

    fn set_sort(&mut self, column: DocSort, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            (current, order) if current == column => (column, order.flip()),
            _ => (column, column.default_order()),
        };
        self.page_offset = 0;
        self.order_rev = self.order_rev.wrapping_add(1);
        cx.notify();
    }

    /// One list row: the group header when it opens one, then the record itself.
    fn document_row(
        &self,
        ix: usize,
        narrow: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(entry) = self.order.get(ix) else {
            return div().into_any_element();
        };
        let Some(row) = self
            .rows
            .value
            .as_ref()
            .and_then(|rows| rows.get(entry.index))
        else {
            return div().into_any_element();
        };
        let mut item = div().w_full().flex().flex_col();
        if let Some((group, count)) = &entry.group {
            item = item.child(
                div()
                    .w_full()
                    .pt_3()
                    .pb_1()
                    .px(self.density.pad_x())
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(format!("{group} · {count}")),
            );
        }
        let id = row.id.clone();
        let chosen = self.selected.as_deref() == Some(row.id.as_str());
        let category = row
            .category
            .as_deref()
            .or(row.suggested_category.as_deref())
            .unwrap_or("—");
        let date = row.imported_at.get(..10).unwrap_or("—").to_owned();
        let pages_label = if row.page_count > 0 {
            format!("{}p", row.page_count)
        } else {
            "—".to_owned()
        };
        let category_w = self.col_width("category", 110.);
        let pages_w = self.col_width("pages", 64.);
        let status_w = self.col_width("status", 76.);
        let added_w = self.col_width("added", 110.);
        let workflow = workflow_label(&row.workflow_stage);
        let primary_title = row
            .extracted_title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(&row.filename)
            .to_owned();
        let show_filename_sub = primary_title != row.filename;
        item.child(
            div()
                .id(("document-row", ix))
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .min_h(self.density.row_height())
                .px(self.density.pad_x())
                .py_1p5()
                .rounded(px(6.))
                .border_b_1()
                .border_color(rgba(tint(palette.border, 0x15)))
                .when(chosen, |el| {
                    el.bg(rgba(tint(palette.accent, 0x18)))
                        .border_l_2()
                        .border_color(rgb(palette.accent))
                })
                .when(!chosen, |el| {
                    el.hover(|style| style.bg(rgba(tint(palette.raised, 0x22))))
                })
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
                        .gap_0p5()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if chosen {
                                    rgb(palette.accent)
                                } else {
                                    rgb(palette.text)
                                })
                                .child(primary_title),
                        )
                        .when(show_filename_sub, |column| {
                            column.child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(row.filename.clone()),
                            )
                        })
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
                                    .text_color(rgb(palette.accent))
                                    .child(format!("Matches on page {pages}")),
                            )
                        }),
                )
                .when(!narrow, |cells| {
                    cells.child(
                        div()
                            .w(px(category_w))
                            .flex_none()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(category.to_owned()),
                    )
                })
                .when(!narrow, |cells| {
                    cells.child(tabular_cell(pages_label, px(pages_w), palette.muted))
                })
                .child(
                    div()
                        .w(px(status_w))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_start()
                        .child(status_chip(&row.status, palette)),
                )
                .when(!narrow, |cells| {
                    cells.child(tabular_cell(date, px(added_w), palette.muted))
                }),
        )
        .into_any_element()
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
        self.sync_order();
        let [total, inbox, filed, active, next] = self.counts;
        let selected = self.selected.clone();
        let mobile = self.density == Density::Mobile;
        // With a document open the list keeps a column beside the reader on desktop; on mobile
        // the reader takes the whole screen.
        let narrow = selected.is_some() || mobile;
        let category_label = match self.category_filter.as_deref() {
            None => "Category: all".to_owned(),
            Some("") => "Category: unfiled".to_owned(),
            Some(category) => format!("Category: {category}"),
        };

        let mut table = div().flex().flex_col().gap_1().flex_1().min_h_0();
        let search = self.search_field(window, cx);
        let page_search = self.page_search_field(window, cx);
        let split_input = self.split_range_field(window, cx);
        let uploading = self.uploading;
        table = table.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .mb_2()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
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
                                .ghost()
                                .label(category_label)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.cycle_category_filter(cx)),
                                ),
                        )
                        .child(
                            Button::new("documents-grouping")
                                .small()
                                .ghost()
                                .label(self.grouping.label())
                                .on_click(cx.listener(|view, _, _, cx| view.cycle_grouping(cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("documents-import")
                                .small()
                                .primary()
                                .label(if uploading > 0 {
                                    format!("Importing ({uploading})…")
                                } else {
                                    "+ Import…".to_owned()
                                })
                                .disabled(uploading > 0)
                                .on_click(cx.listener(Self::prompt_import)),
                        )
                        .when(!narrow, |actions| {
                            actions.child(
                                Button::new("documents-duplicate-review")
                                    .small()
                                    .ghost()
                                    .label("Review duplicates")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.set_duplicate_mode(true, cx)
                                    })),
                            )
                        }),
                ),
        );
        let (sort_col, sort_order) = self.sort;
        table = table.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .mb_2p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(!narrow, |row| {
                            row.child(div().w(px(240.)).child(Input::new(&search).small()))
                                .child(
                                    Button::new("documents-search-text")
                                        .small()
                                        .ghost()
                                        .label("Search")
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.search_documents(cx)),
                                        ),
                                )
                        })
                        .when(narrow, |row| {
                            row.child(div().w(px(160.)).child(Input::new(&search).small()))
                                .child(
                                    Button::new("documents-search-text-narrow")
                                        .small()
                                        .ghost()
                                        .label("Search")
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.search_documents(cx)),
                                        ),
                                )
                        })
                        .child(
                            Button::new("documents-sort")
                                .small()
                                .ghost()
                                .label(sort_col.button_label(sort_order))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let next = match view.sort {
                                        (DocSort::Added, SortOrder::Descending) => {
                                            (DocSort::Name, SortOrder::Ascending)
                                        }
                                        (DocSort::Name, SortOrder::Ascending) => {
                                            (DocSort::Pages, SortOrder::Descending)
                                        }
                                        _ => (DocSort::Added, SortOrder::Descending),
                                    };
                                    view.sort = next;
                                    view.order_rev = view.order_rev.wrapping_add(1);
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .when(self.total_filtered > 500, |bar| {
                            let start = (self.page_offset / 500) * 500;
                            let display_start = start + 1;
                            let end = (start + 500).min(self.total_filtered);
                            let total = self.total_filtered;
                            bar.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        Button::new("documents-page-prev")
                                            .xsmall()
                                            .ghost()
                                            .label("‹ Prev 500")
                                            .disabled(start == 0)
                                            .on_click(
                                                cx.listener(|view, _, _, cx| view.prev_page(cx)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(format!("{display_start}–{end} of {total}")),
                                    )
                                    .child(
                                        Button::new("documents-page-next")
                                            .xsmall()
                                            .ghost()
                                            .label("Next 500 ›")
                                            .disabled(end >= total)
                                            .on_click(
                                                cx.listener(|view, _, _, cx| view.next_page(cx)),
                                            ),
                                    ),
                            )
                        })
                        .when(!narrow, |bar| {
                            bar.child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("{total} documents · local only")),
                            )
                        }),
                ),
        );
        let sort_state = |column: DocSort| (sort_col == column).then_some(sort_order);
        let mut header_cols =
            vec![SortableColumn::flex("name", "DOCUMENT").sorted(sort_state(DocSort::Name))];
        if !narrow {
            header_cols.push(
                SortableColumn::fixed("category", "CATEGORY", px(self.col_width("category", 110.)))
                    .sorted(sort_state(DocSort::Category)),
            );
            header_cols.push(
                SortableColumn::fixed("pages", "PAGES", px(self.col_width("pages", 64.)))
                    .right_align()
                    .sorted(sort_state(DocSort::Pages)),
            );
        }
        header_cols.push(
            SortableColumn::fixed("status", "STATUS", px(self.col_width("status", 76.)))
                .sorted(sort_state(DocSort::Status)),
        );
        if !narrow {
            header_cols.push(
                SortableColumn::fixed("added", "ADDED", px(self.col_width("added", 110.)))
                    .right_align()
                    .sorted(sort_state(DocSort::Added)),
            );
        }
        let sort_entity = cx.entity();
        let resize_entity = cx.entity();
        table = table.child(resizable_table_header(
            "documents",
            header_cols,
            self.density,
            palette,
            move |col_id, _, cx| {
                let column = match col_id {
                    "name" => DocSort::Name,
                    "category" => DocSort::Category,
                    "pages" => DocSort::Pages,
                    "status" => DocSort::Status,
                    "added" => DocSort::Added,
                    _ => return,
                };
                sort_entity.update(cx, |view, cx| view.set_sort(column, cx));
            },
            move |col_id, _, cx| {
                resize_entity.update(cx, |view, cx| view.resize_column(col_id, cx));
            },
        ));
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
        if self.order.is_empty() && !self.rows.loading {
            table = table.child(div().py_5().text_sm().text_color(rgb(palette.muted)).child(
                if total == 0 {
                    "No documents yet."
                } else {
                    "No documents in this view."
                },
            ));
        }
        if !self.order.is_empty() {
            // The library holds every record; only the rows in the window are built.
            let entity = cx.entity();
            table = table.child(virtual_list(
                "documents-list",
                &self.list,
                move |ix, _window, cx| {
                    entity.update(cx, |documents, cx| {
                        documents.document_row(ix, narrow, palette, cx)
                    })
                },
            ));
        }

        // With a record open the list keeps a column and the reader takes the rest; with nothing
        // open the list is the page.
        let reader: Option<AnyElement> = selected.as_ref().map(|_| {
            let page = self.reader.page;
            let pages = self.reader.pages;
            let reading_text = self.reader.reading_text;
            let show_thumbnails = self.reader.show_thumbnails;
            let zoom = self.reader.zoom;
            let page_hits = self.page_search_hits();
            let matched_pages = self.matched_page_numbers();
            let has_extracted_pages = self
                .detail
                .value
                .as_ref()
                .is_some_and(|detail| !detail.pages.is_empty());
            let suggestion = self
                .detail
                .value
                .as_ref()
                .and_then(|detail| detail.document.suggested_category.clone());
            let can_open_externally = !mobile
                && self
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
                                .label(if mobile {
                                    "← Documents"
                                } else {
                                    "Close ✕ (Esc)"
                                })
                                .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
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
                            Button::new("document-zoom-out")
                                .small()
                                .label("−")
                                .disabled(zoom == Zoom::Page)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.zoom_out(cx);
                                })),
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
                            Button::new("document-zoom-in")
                                .small()
                                .label("+")
                                .disabled(zoom == Zoom::Close)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.zoom_in(cx);
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
                        .when(!reading_text && pages > 1, |bar| {
                            bar.child(
                                Button::new("document-toggle-thumbnails")
                                    .small()
                                    .label(if show_thumbnails {
                                        "Hide thumbs"
                                    } else {
                                        "Thumbnails"
                                    })
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.toggle_thumbnails(cx);
                                    })),
                            )
                        })
                        .when(has_extracted_pages, |bar| {
                            bar.child(div().w(px(160.)).child(Input::new(&page_search).small()))
                        })
                        .when(can_open_externally, |bar| {
                            bar.child(
                                Button::new("document-open-external")
                                    .small()
                                    .label("Open in Preview")
                                    .on_click(cx.listener(|view, _, _, _| view.open_externally())),
                            )
                        })
                        .when(!mobile, |bar| {
                            bar.child(
                                Button::new("document-compare-btn")
                                    .small()
                                    .label(if self.filing_compare_id.is_some() {
                                        "Close compare ✕"
                                    } else {
                                        "Compare ⧉"
                                    })
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        if view.filing_compare_id.is_some() {
                                            view.close_compare(cx);
                                        } else {
                                            view.show_compare_picker = !view.show_compare_picker;
                                            cx.notify();
                                        }
                                    })),
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
                .when(
                    self.show_compare_picker && self.filing_compare_id.is_none(),
                    |el| {
                        let mut picker = div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .p_2()
                            .rounded(px(6.))
                            .bg(rgba(palette.surface))
                            .border_1()
                            .border_color(rgba(palette.border))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.muted))
                                    .child("Compare with:"),
                            );
                        let other_docs: Vec<_> = self
                            .rows
                            .value
                            .as_deref()
                            .unwrap_or_default()
                            .iter()
                            .filter(|r| Some(&r.id) != self.selected.as_ref())
                            .take(8)
                            .collect();
                        for other in other_docs {
                            let other_id = other.id.clone();
                            let other_title =
                                other.extracted_title.as_deref().unwrap_or(&other.filename);
                            let title_trunc = if other_title.len() > 24 {
                                format!("{}…", &other_title[..24])
                            } else {
                                other_title.to_owned()
                            };
                            picker = picker.child(
                                Button::new(format!("compare-pick-{}", other.id))
                                    .xsmall()
                                    .ghost()
                                    .label(title_trunc)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.open_compare_document(other_id.clone(), cx);
                                    })),
                            );
                        }
                        el.child(picker)
                    },
                )
                .when(self.filing_compare_id.is_some(), |el| {
                    let compare_title = self
                        .filing_compare_detail
                        .value
                        .as_ref()
                        .map(|d| {
                            d.document
                                .extracted_title
                                .clone()
                                .unwrap_or_else(|| d.document.filename.clone())
                        })
                        .unwrap_or_else(|| "Comparison Document".to_owned());
                    let compare_cat = self
                        .filing_compare_detail
                        .value
                        .as_ref()
                        .and_then(|d| d.document.category.clone());
                    let primary_cat = self
                        .detail
                        .value
                        .as_ref()
                        .and_then(|d| d.document.category.clone());
                    let can_adopt = compare_cat.is_some() && compare_cat != primary_cat;
                    let adopt_label = format!(
                        "Adopt category '{}' →",
                        compare_cat.as_deref().unwrap_or("")
                    );

                    let compare_bar = div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .p_2()
                        .rounded(px(6.))
                        .bg(rgba(tint(palette.accent, 0x14)))
                        .border_1()
                        .border_color(rgba(palette.accent))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("Side-by-Side: ↔ {compare_title}"))
                                .when_some(compare_cat, |el, cat| el.child(format!("({cat})"))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(can_adopt, |el| {
                                    el.child(
                                        Button::new("adopt-compare-cat")
                                            .xsmall()
                                            .primary()
                                            .label(adopt_label)
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.adopt_compare_category(cx)
                                            })),
                                    )
                                })
                                .child(
                                    Button::new("exit-compare")
                                        .xsmall()
                                        .ghost()
                                        .label("Close ✕")
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.close_compare(cx)),
                                        ),
                                ),
                        );

                    let compare_page_label = if self.filing_compare_reader.pages == 0 {
                        "Page —".to_owned()
                    } else {
                        format!(
                            "Page {} of {}",
                            self.filing_compare_reader.page + 1,
                            self.filing_compare_reader.pages
                        )
                    };

                    let compare_controls = div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Button::new("cmp-page-prev").xsmall().label("‹").on_click(
                            cx.listener(|view, _, _, cx| view.go_to_compare_page(-1, cx)),
                        ))
                        .child(
                            Button::new("cmp-page-next").xsmall().label("›").on_click(
                                cx.listener(|view, _, _, cx| view.go_to_compare_page(1, cx)),
                            ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(compare_page_label),
                        )
                        .child(
                            Button::new("cmp-zoom")
                                .xsmall()
                                .label(self.filing_compare_reader.zoom.label())
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.toggle_compare_zoom(cx)),
                                ),
                        )
                        .child(
                            Button::new("cmp-text")
                                .xsmall()
                                .label(if self.filing_compare_reader.reading_text {
                                    "Pages"
                                } else {
                                    "Text"
                                })
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.toggle_compare_text(cx)),
                                ),
                        )
                        .when(self.filing_compare_reader.pages > 1, |el| {
                            el.child(Button::new("cmp-thumbs").xsmall().label("Thumbs").on_click(
                                cx.listener(|view, _, _, cx| view.toggle_compare_thumbnails(cx)),
                            ))
                        });

                    let right_canvas = if self.filing_compare_reader.reading_text {
                        let text = self
                            .filing_compare_detail
                            .value
                            .as_ref()
                            .and_then(|d| d.text.clone())
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| "No extracted text.".into());
                        div()
                            .id("compare-text-panel")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.text_scroll)
                            .vertical_scrollbar(&self.text_scroll)
                            .p_3()
                            .rounded(px(6.))
                            .bg(rgba(palette.surface))
                            .child(crate::doc_view::render_markdown(
                                &text,
                                palette,
                                self.density,
                            ))
                            .into_any_element()
                    } else {
                        self.compare_page_element(palette, cx)
                    };

                    let dual_pane = div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(self.page_element(&matched_pages, palette, cx)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(compare_controls)
                                .child(right_canvas),
                        );

                    el.child(compare_bar).child(dual_pane)
                })
                .when(self.filing_compare_id.is_none(), |el| {
                    el.when(!self.page_search_query.trim().is_empty(), |el| {
                        el.child(page_search_hits_strip(
                            &self.page_search_query,
                            &page_hits,
                            page,
                            palette,
                            cx,
                        ))
                    })
                    .when(!reading_text && self.reader.pages > 1, |el| {
                        el.child(page_jump_strip(self, &matched_pages, palette, cx))
                    })
                    .when(reading_text, |el| el.child(text_panel(self, palette)))
                    .when(!reading_text, |el| {
                        el.child(self.page_element(&matched_pages, palette, cx))
                    })
                })
                .into_any_element()
        });

        let show_table = !(mobile && reader.is_some());
        let upload_notice = self.upload_notice.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .rounded(px(6.))
            .on_drop(cx.listener(|view, paths: &ExternalPaths, _, cx| {
                view.import_paths(paths.paths().to_vec(), cx);
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style
                    .bg(rgba(tint(palette.accent, 0x10)))
                    .border_1()
                    .border_color(rgba(palette.accent))
            })
            .when_some(upload_notice, |el, (is_error, message)| {
                let tone = if is_error { palette.red } else { palette.green };
                el.child(
                    div()
                        .id("documents-upload-notice")
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .rounded(px(5.))
                        .bg(rgba(tint(tone, 0x18)))
                        .border_1()
                        .border_color(rgba(tint(tone, 0x40)))
                        .text_xs()
                        .text_color(rgb(tone))
                        .child(div().flex_1().truncate().child(message))
                        .child(
                            Button::new("documents-upload-notice-dismiss")
                                .xsmall()
                                .ghost()
                                .label("Dismiss")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.upload_notice = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .when(show_table, |el| {
                        el.child(
                            div()
                                .when(reader.is_some(), |el| el.w(px(380.)).min_w(px(320.)))
                                .when(reader.is_none(), |el| el.flex_1())
                                .flex()
                                .flex_col()
                                // A scroll area's min-content width is the table's whole width; without
                                // this the column refuses to shrink and pushes the reader off screen.
                                .min_w_0()
                                .min_h_0()
                                .id("documents-pane")
                                .overflow_hidden()
                                .child(
                                    div()
                                        .id("documents-rows")
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .child(table.w_full()),
                                ),
                        )
                    })
                    .when_some(reader, |el, reader| el.child(reader)),
            )
    }
}

pub(crate) fn upsert_uploaded_document(rows: &mut Vec<DocumentSummary>, uploaded: DocumentSummary) {
    if let Some(existing) = rows.iter_mut().find(|row| row.id == uploaded.id) {
        *existing = uploaded;
    } else {
        rows.insert(0, uploaded);
    }
}

pub(crate) fn format_upload_summary(
    created: usize,
    existing: usize,
    last_filename: Option<&str>,
    errors: &[String],
) -> (bool, String) {
    let total = created + existing;
    if total == 0 {
        return (
            true,
            if errors.is_empty() {
                "No files were imported.".to_owned()
            } else {
                errors.join(" · ")
            },
        );
    }
    if errors.is_empty() {
        if total == 1 {
            let name = last_filename.unwrap_or("document");
            if created == 1 {
                return (false, format!("Imported {name}."));
            }
            return (false, format!("{name} is already in Documents."));
        }
        return (
            false,
            format!("Imported {total} documents ({created} new, {existing} existing)."),
        );
    }
    (
        true,
        format!(
            "Imported {total} ({created} new, {existing} existing); {} failed: {}",
            errors.len(),
            errors.join(" · ")
        ),
    )
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
        .py_0p5()
        .rounded_full()
        .bg(rgba(tint(color, 0x18)))
        .border_1()
        .border_color(rgba(tint(color, 0x30)))
        .text_color(rgb(color))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(text.to_owned())
        .into_any_element()
}

/// In-document text search match strip showing matched page chips and a snippet preview.
fn page_search_hits_strip(
    query: &str,
    hits: &[PageSearchHit],
    current_page: usize,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    if hits.is_empty() {
        return div()
            .id("document-page-search-empty")
            .px_2p5()
            .py_1()
            .rounded(px(5.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(format!("No pages match \"{query}\"."))
            .into_any_element();
    }
    let active_snippet = hits
        .iter()
        .find(|hit| hit.page_index == current_page)
        .or_else(|| hits.first())
        .map(|hit| (hit.page_number, hit.snippet.clone()));

    let mut chips = div().flex().flex_wrap().items_center().gap_1();
    for hit in hits {
        let page_ix = hit.page_index;
        let page_num = hit.page_number;
        let active = page_ix == current_page;
        chips = chips.child(
            div()
                .id(("doc-search-hit", page_ix))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .cursor_pointer()
                .bg(rgba(if active {
                    tint(palette.amber, 0x30)
                } else {
                    tint(palette.amber, 0x16)
                }))
                .text_color(rgb(palette.amber))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.go_to_page_index(page_ix, cx);
                }))
                .child(format!("p.{page_num}")),
        );
    }

    div()
        .id("document-page-search-strip")
        .flex()
        .flex_col()
        .gap_1()
        .px_2p5()
        .py_1p5()
        .rounded(px(5.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} matching {}:",
                    hits.len(),
                    if hits.len() == 1 { "page" } else { "pages" }
                ))
                .child(chips),
        )
        .when_some(active_snippet, |col, (page_num, snippet)| {
            col.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .truncate()
                    .child(format!("p.{page_num}: {snippet}")),
            )
        })
        .into_any_element()
}

/// Quick page jump pills (first, nearby, last, and search-matched pages) for multi-page PDFs.
fn page_jump_strip(
    view: &Documents,
    matched: &[u32],
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let jumps = view.reader.jump_pages();
    let current = view.reader.page;

    let mut strip = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(rgb(palette.muted));

    for page_ix in jumps {
        let active = page_ix == current;
        let page_num = u32::try_from(page_ix).unwrap_or(0).saturating_add(1);
        let is_match = matched.contains(&page_num);
        strip = strip.child(
            div()
                .id(("doc-jump-page", page_ix))
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .cursor_pointer()
                .bg(rgba(if active {
                    tint(palette.accent, 0x24)
                } else if is_match {
                    tint(palette.amber, 0x20)
                } else {
                    palette.raised
                }))
                .text_color(rgb(if active {
                    palette.accent
                } else if is_match {
                    palette.amber
                } else {
                    palette.muted
                }))
                .font_weight(if active || is_match {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.go_to_page_index(page_ix, cx);
                }))
                .child(format!("p.{page_num}")),
        );
    }
    strip.into_any_element()
}

/// The locally extracted text, rendered through the selectable Markdown [`TextView`].
fn text_panel(view: &Documents, palette: Palette) -> AnyElement {
    let text = view
        .detail
        .value
        .as_ref()
        .and_then(|detail| detail.text.clone())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "No text extraction is available for this file.".into());
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
        .child(crate::doc_view::render_markdown(
            &text,
            palette,
            view.density,
        ))
        .into_any_element()
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
                    .bg(rgba(tint(palette.amber, 0x18)))
                    .text_xs()
                    .text_color(rgb(palette.amber))
                    .child(format!("Suggested: {reason}")),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;
    use gpui::TestAppContext;

    use super::duplicates::parse_page_ranges;
    use super::{workflow_label, DocSort, Documents, Filter, Reader, SortOrder, Zoom};
    use crate::data::Repo;
    use sjel_gpui_core::shell::{DocumentSummary, ShellError};

    fn summary(id: &str, category: Option<&str>, filename: &str) -> DocumentSummary {
        DocumentSummary {
            id: id.into(),
            sha256: String::new(),
            filename: filename.into(),
            media_type: "application/pdf".into(),
            byte_len: 1,
            imported_at: "2026-11-01".into(),
            status: "inbox".into(),
            category: category.map(str::to_owned),
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
        }
    }

    /// The list's order is what the keyboard steps through, so a filter narrows both.
    #[gpui::test]
    fn the_order_follows_the_filters(cx: &mut TestAppContext) {
        let page = cx.new(|_| Documents::new(Repo::new(Err(ShellError::NotSetUp))));
        page.update(cx, |page, cx| {
            let mut a = summary("a", Some("Tax"), "b-tax.pdf");
            a.page_count = 4;
            let mut b = summary("b", Some("Home"), "a-home.pdf");
            b.page_count = 12;
            let mut c = summary("c", None, "c-unfiled.pdf");
            c.page_count = 1;
            page.rows.value = Some(vec![a, b, c]);
            page.sync_order();
            assert_eq!(page.order.len(), 3);
            assert_eq!(page.list.item_count(), 3);

            page.set_sort(DocSort::Name, cx);
            page.sync_order();
            let ids: Vec<&str> = page
                .order
                .iter()
                .map(|row| page.rows.value.as_ref().unwrap()[row.index].id.as_str())
                .collect();
            assert_eq!(ids, ["b", "a", "c"]);

            page.set_sort(DocSort::Pages, cx);
            page.sync_order();
            let ids: Vec<&str> = page
                .order
                .iter()
                .map(|row| page.rows.value.as_ref().unwrap()[row.index].id.as_str())
                .collect();
            assert_eq!(ids, ["b", "a", "c"]);
            assert_eq!(page.sort, (DocSort::Pages, SortOrder::Descending));

            page.resize_column("category", cx);
            assert_eq!(page.col_width("category", 110.), 149.);

            page.category_filter = Some("Tax".to_owned());
            page.order_rev = page.order_rev.wrapping_add(1);
            page.sync_order();
            assert_eq!(page.order.len(), 1);
            assert_eq!(page.list.item_count(), 1);

            page.category_filter = None;
            page.search_query = "home".to_owned();
            page.order_rev = page.order_rev.wrapping_add(1);
            page.sync_order();
            assert_eq!(page.order.len(), 1);
            assert_eq!(page.list.item_count(), 1);
            cx.notify();
        });
    }

    #[test]
    fn filters_match_document_statuses() {
        let mut row = DocumentSummary {
            id: "id".into(),
            sha256: String::new(),
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
        assert!(reader.set_page(0));
        assert_eq!(reader.page, 0);
        assert!(!reader.set_page(0));
        assert!(reader.set_page(99));
        assert_eq!(reader.page, 2);
        assert_eq!(reader.jump_pages(), vec![0, 1, 2]);
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
        assert_eq!(reader.drawn.len(), super::viewer::CACHED_PAGES);
        // Page 20 and its neighbours survive; page 0 does not.
        assert!(reader.drawn.contains_key(&(20, width)));
        assert!(!reader.drawn.contains_key(&(0, width)));
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

    #[test]
    fn in_pdf_search_finds_matching_pages_and_respects_page_bounds() {
        use sjel_gpui_core::shell::DocumentExtractedPage;
        let pages = vec![
            DocumentExtractedPage {
                page_number: 1,
                text: "Cover page for the 2026 annual report.".into(),
                engine: "pdftotext".into(),
                failure: None,
            },
            DocumentExtractedPage {
                page_number: 2,
                text: "Invoice summary: total amount due is 1,240.00 EUR.".into(),
                engine: "pdftotext".into(),
                failure: None,
            },
            DocumentExtractedPage {
                page_number: 3,
                text: "Appendix and payment instructions for the invoice.".into(),
                engine: "pdftotext".into(),
                failure: None,
            },
        ];
        let hits = super::viewer::search_extracted_pages(&pages, 0, None, "INVOICE");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].page_index, 1);
        assert_eq!(hits[0].page_number, 2);
        assert!(hits[0].snippet.contains("Invoice summary"));
        assert_eq!(hits[1].page_index, 2);
        assert_eq!(hits[1].page_number, 3);

        // Child split range [0..=1] excludes page 3 (index 2).
        let bounded = super::viewer::search_extracted_pages(&pages, 0, Some(1), "invoice");
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].page_number, 2);
    }

    #[test]
    fn thumbnail_cache_trims_furthest_pages_from_current() {
        let mut reader = Reader {
            pages: 50,
            page: 25,
            show_thumbnails: true,
            ..Reader::default()
        };
        for page in 0..50 {
            reader
                .thumbnails
                .insert(page, std::sync::Arc::new(gpui::RenderImage::new(vec![])));
        }
        reader.trim_thumbnails();
        assert_eq!(reader.thumbnails.len(), super::viewer::CACHED_THUMBS);
        assert!(reader.thumbnails.contains_key(&25));
        assert!(!reader.thumbnails.contains_key(&0));
    }

    #[test]
    fn upsert_uploaded_document_prepends_new_and_updates_existing_in_place() {
        let mut rows = vec![
            summary("a", Some("Tax"), "a.pdf"),
            summary("b", None, "b.pdf"),
        ];
        super::upsert_uploaded_document(&mut rows, summary("c", None, "c.pdf"));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );

        let mut updated_a = summary("a", Some("FiledTax"), "a-renamed.pdf");
        updated_a.status = "filed".into();
        super::upsert_uploaded_document(&mut rows, updated_a);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
        assert_eq!(rows[1].filename, "a-renamed.pdf");
        assert_eq!(rows[1].status, "filed");
    }

    #[test]
    fn format_upload_summary_distinguishes_created_existing_and_errors() {
        assert_eq!(
            super::format_upload_summary(1, 0, Some("invoice.pdf"), &[]),
            (false, "Imported invoice.pdf.".into())
        );
        assert_eq!(
            super::format_upload_summary(0, 1, Some("invoice.pdf"), &[]),
            (false, "invoice.pdf is already in Documents.".into())
        );
        assert_eq!(
            super::format_upload_summary(2, 1, Some("last.pdf"), &[]),
            (false, "Imported 3 documents (2 new, 1 existing).".into())
        );
        assert_eq!(
            super::format_upload_summary(0, 0, None, &["bad.pdf: refused".into()]),
            (true, "bad.pdf: refused".into())
        );
        assert_eq!(
            super::format_upload_summary(1, 0, Some("ok.pdf"), &["bad.pdf: refused".into()]),
            (
                true,
                "Imported 1 (1 new, 0 existing); 1 failed: bad.pdf: refused".into()
            )
        );
    }

    #[gpui::test]
    fn cursor_paging_windows_and_steps_500_records(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|_cx| {
            let mut docs = Documents::new(Repo::new(Err(ShellError::NotSetUp)));
            let mut rows = Vec::new();
            for i in 0..1200 {
                rows.push(summary(
                    &format!("doc_{i}"),
                    Some("Test"),
                    &format!("file_{i}.pdf"),
                ));
            }
            docs.rows.value = Some(rows);
            docs.sync_order();
            docs
        });

        page.update(cx, |docs, cx| {
            assert_eq!(docs.total_filtered, 1200);
            assert_eq!(docs.order.len(), 500);
            assert_eq!(docs.page_offset, 0);

            docs.next_page(cx);
            docs.sync_order();
            assert_eq!(docs.page_offset, 500);
            assert_eq!(docs.order.len(), 500);

            docs.next_page(cx);
            docs.sync_order();
            assert_eq!(docs.page_offset, 1000);
            assert_eq!(docs.order.len(), 200);

            docs.prev_page(cx);
            docs.sync_order();
            assert_eq!(docs.page_offset, 500);
            assert_eq!(docs.order.len(), 500);
        });
    }

    #[gpui::test]
    fn filing_compare_manages_state_and_side_by_side_readers(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|_cx| {
            let mut docs = Documents::new(Repo::new(Err(ShellError::NotSetUp)));
            docs.selected = Some("doc_1".into());
            docs
        });

        page.update(cx, |docs, cx| {
            assert!(docs.filing_compare_id.is_none());
            docs.open_compare_document("doc_2".into(), cx);
            assert_eq!(docs.filing_compare_id.as_deref(), Some("doc_2"));
            assert_eq!(docs.filing_compare_reader.id.as_deref(), Some("doc_2"));

            docs.close_compare(cx);
            assert!(docs.filing_compare_id.is_none());
            assert!(docs.filing_compare_reader.id.is_none());
        });
    }
}
