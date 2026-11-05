//! Native Feed: triage the comms feed and read one entry.
//!
//! Comms owns the data (`capabilities/comms/src/server/content.rs`); this page reads
//! `GET /comms/feed` and writes decisions only through `POST /comms/feed/:id/status`, the route
//! that records the ledger row with them. The keys are the dashboard's
//! (`dashboard/src/routes/feed/+page.svelte`, "Keyboard triage"), so the two surfaces agree.

pub mod actions;
pub mod library;
pub mod reading;
pub mod review;
pub mod types;

pub use actions::*;
pub use types::*;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, point, prelude::*, px, rgb, AnyElement, Context, Entity, FocusHandle, FontWeight,
    ListAlignment, ListOffset, ListState, ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::shell::{
    self, CalendarGoogleExportOptIn, CalendarGoogleExportReport, CloudDerivativePreview,
    CloudProviderOption, FeedContentDetail, FeedDetail, FeedEntry, FeedQualityFlag,
    FeedQualityRefresh, ShellError,
};

use crate::clipper::Clips;
use crate::data::{Remote, Repo};
use crate::mail::Mail;
use crate::scout::ScoutingPage;
use crate::theme::Palette;

use self::library::{feed_order, library_matches, list_anchor, restore_anchor};
use self::reading::in_view;

pub struct Feed {
    pub(crate) repo: Repo,
    pub tab: Tab,
    pub mail: Entity<Mail>,
    pub discover: Entity<ScoutingPage>,
    pub clips: Entity<Clips>,
    pub(crate) palette: Option<Palette>,
    pub rows: Remote<Vec<FeedEntry>>,
    pub filter: Filter,
    /// `None` shows both streams.
    pub stream: Option<&'static str>,
    pub cursor: Option<String>,
    /// The entry the reader shows; `None` shows the list.
    pub reading: Option<String>,
    pub(crate) content_source: Option<&'static str>,
    pub detail: Remote<FeedDetail>,
    pub content_detail: Remote<FeedContentDetail>,
    pub(crate) calendar_exports: Remote<Vec<CalendarGoogleExportOptIn>>,
    pub(crate) calendar_export_report: Remote<CalendarGoogleExportReport>,
    pub(crate) calendar_export_busy: bool,
    pub(crate) calendar_export_error: Option<ShellError>,
    pub(crate) calendar_export_confirm: bool,
    pub(crate) calendar_edit: Option<CalendarEditForm>,
    pub(crate) cloud_preview: Remote<CloudDerivativePreview>,
    pub(crate) cloud_preview_busy: bool,
    pub(crate) cloud_approval_busy: bool,
    pub(crate) cloud_preview_error: Option<ShellError>,
    pub(crate) cloud_preview_scroll: ScrollHandle,
    pub(crate) cloud_providers: Remote<Vec<CloudProviderOption>>,
    pub(crate) cloud_job_busy: bool,
    pub(crate) cloud_job_error: Option<ShellError>,
    pub(crate) data_class_form: Option<DataClassForm>,
    pub(crate) data_class_busy: bool,
    pub(crate) data_class_error: Option<ShellError>,
    pub(crate) cloud_generation: u64,
    /// The last write that failed, shown above the list until the next one.
    pub write_error: Option<ShellError>,
    /// The stored quality flags, one row per signal.
    pub review: Remote<Vec<FeedQualityFlag>>,
    /// What the last explicit refresh checked; `None` until one ran this session.
    pub review_refresh: Option<FeedQualityRefresh>,
    pub refreshing: bool,
    /// Every entry comms holds, dismissed included.
    pub library: Remote<Vec<FeedEntry>>,
    /// `None` shows every status.
    pub library_status: Option<Filter>,
    pub library_kind: Option<String>,
    pub(crate) library_query: String,
    pub(crate) library_input: Option<(Entity<InputState>, Subscription)>,
    /// The archive's scroller, and its order: the Library tab draws only the rows on screen.
    pub(crate) library_list: ListState,
    pub(crate) library_order: Vec<usize>,
    pub(crate) library_counts: [usize; 4],
    pub(crate) library_kinds: Vec<(String, usize)>,
    pub(crate) library_key: Option<(u64, usize)>,
    /// Counts a change to the archive's three filters, which the entries cannot report.
    pub(crate) library_rev: u64,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_next: bool,
    /// The scroller every body but the Reading list uses, so a tab change keeps one scroll
    /// position per page style rather than none at all.
    pub(crate) tab_scroll: ScrollHandle,
    /// The Reading list's scroller: it draws only the rows the window shows.
    pub(crate) reading_list: ListState,
    /// Indices into `rows` in draw order, rebuilt when the data, the filter or a status moves.
    pub(crate) reading_order: Vec<usize>,
    pub(crate) reading_counts: [usize; 3],
    pub(crate) reading_key: Option<ReadingKey>,
    /// Counts in-place status changes, which the entries themselves cannot report.
    pub(crate) rows_rev: u64,
}

impl Feed {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            mail: cx.new(|cx| Mail::new(repo.clone(), cx)),
            discover: cx.new(|_| ScoutingPage::new(repo.clone())),
            clips: cx.new(|_| Clips::new()),
            repo,
            tab: Tab::Reading,
            palette: None,
            rows: Remote::default(),
            filter: Filter::New,
            stream: None,
            cursor: None,
            reading: None,
            content_source: None,
            detail: Remote::default(),
            content_detail: Remote::default(),
            calendar_exports: Remote::default(),
            calendar_export_report: Remote::default(),
            calendar_export_busy: false,
            calendar_export_error: None,
            calendar_export_confirm: false,
            calendar_edit: None,
            cloud_preview: Remote::default(),
            cloud_preview_busy: false,
            cloud_approval_busy: false,
            cloud_preview_error: None,
            cloud_preview_scroll: ScrollHandle::new(),
            cloud_providers: Remote::default(),
            cloud_job_busy: false,
            cloud_job_error: None,
            data_class_form: None,
            data_class_busy: false,
            data_class_error: None,
            cloud_generation: 0,
            write_error: None,
            review: Remote::default(),
            review_refresh: None,
            refreshing: false,
            library: Remote::default(),
            library_status: None,
            library_kind: None,
            library_query: String::new(),
            library_input: None,
            library_list: ListState::new(0, ListAlignment::Top, px(600.)),
            library_order: Vec::new(),
            library_counts: [0; 4],
            library_kinds: Vec::new(),
            library_key: None,
            library_rev: 0,
            focus: cx.focus_handle(),
            focus_next: true,
            reading_list: ListState::new(0, ListAlignment::Top, px(600.)),
            tab_scroll: ScrollHandle::new(),
            reading_order: Vec::new(),
            reading_counts: [0; 3],
            reading_key: None,
            rows_rev: 0,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
        self.mail
            .update(cx, |mail, cx| mail.set_palette(palette, cx));
        self.discover
            .update(cx, |page, cx| page.set_palette(palette, cx));
        self.clips
            .update(cx, |page, cx| page.set_palette(palette, cx));
    }

    /// Called when the page opens: read the open tab again and give it the keys.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Reading => {
                self.focus_next = true;
                self.load(cx);
            }
            Tab::Review => self.load_review(cx),
            Tab::Library => self.load_library(cx),
            Tab::Mail => self.mail.update(cx, |mail, cx| mail.activate(cx)),
            Tab::Discover => self.discover.update(cx, |page, cx| page.load(cx)),
            Tab::Clips => self.clips.update(cx, |page, cx| page.load(cx)),
        }
    }

    /// Reads the open tab again (⌘R).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Reading => self.load(cx),
            Tab::Review => self.load_review(cx),
            Tab::Library => self.load_library(cx),
            Tab::Mail => self.mail.update(cx, |mail, cx| mail.load(cx)),
            Tab::Discover => self.discover.update(cx, |page, cx| page.load(cx)),
            Tab::Clips => self.clips.update(cx, |page, cx| page.load(cx)),
        }
    }

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            if tab == Tab::Discover {
                self.reading = None;
                self.cursor = None;
            }
            self.activate(cx);
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
        cx.notify();
        let work = shell::run(async move { shell.feed(DAYS).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.rows.settle(result);
                view.rows_rev = view.rows_rev.wrapping_add(1);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_review(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.review.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.review.loading = true;
        cx.notify();
        let work = shell::run(async move { shell.feed_quality().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.review.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks comms to recompute the flags, then reads the queue again.
    pub fn refresh_review(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.review.error = None;
        cx.notify();
        let work = shell::run(async move {
            let refresh = shell.refresh_feed_quality().await?;
            Ok((refresh, shell.feed_quality().await?))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.refreshing = false;
                match result {
                    Ok((refresh, flags)) => {
                        view.review_refresh = Some(refresh);
                        view.review.settle(Ok(flags));
                    }
                    Err(error) => view.review.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_library(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.library.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.library.loading = true;
        cx.notify();
        let work = shell::run(async move { shell.feed(ARCHIVE_DAYS).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.library.settle(result);
                view.library_rev = view.library_rev.wrapping_add(1);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn library_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.library_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search title, author or link…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.library_query = input.read(cx).value().trim().to_lowercase();
                view.library_rev = view.library_rev.wrapping_add(1);
                cx.notify();
            }
        });
        self.library_input = Some((input.clone(), subscription));
        input
    }

    pub fn visible(&self) -> Vec<&FeedEntry> {
        in_view(self.rows.value.iter().flatten(), self.filter, self.stream)
    }

    pub(crate) fn count(&self, filter: Filter) -> usize {
        match filter {
            Filter::New => self.reading_counts[0],
            Filter::Kept => self.reading_counts[1],
            Filter::Dismissed => self.reading_counts[2],
        }
    }

    /// The Reading order, rebuilt only when the entries, the filter or a status moved.
    pub(crate) fn sync_reading(&mut self) {
        let key: ReadingKey = (
            self.rows_rev,
            self.rows.value.as_ref().map_or(0, Vec::len),
            self.filter,
            self.stream,
        );
        if self.reading_key == Some(key) {
            return;
        }
        let same_view = self
            .reading_key
            .is_some_and(|seen| (seen.2, seen.3) == (self.filter, self.stream));
        let anchor = same_view.then(|| {
            list_anchor(
                &self.reading_list,
                &self.reading_order,
                self.rows.value.as_ref(),
            )
        });
        let mut counts = [0; 3];
        let mut order: Vec<usize> = Vec::new();
        if let Some(rows) = self.rows.value.as_ref() {
            for (index, row) in rows.iter().enumerate() {
                if !self.stream.is_none_or(|stream| row.stream == stream) {
                    continue;
                }
                match row.status.as_str() {
                    "new" => counts[0] += 1,
                    "keeper" => counts[1] += 1,
                    "dismissed" => counts[2] += 1,
                    _ => {}
                }
                if row.status == self.filter.status() {
                    order.push(index);
                }
            }
            order.sort_by(|left, right| feed_order(&rows[*left], &rows[*right]));
        }
        self.reading_counts = counts;
        self.reading_key = Some(key);
        self.reading_list.reset(order.len());
        restore_anchor(
            &self.reading_list,
            anchor.flatten(),
            &order,
            self.rows.value.as_ref(),
        );
        self.reading_order = order;
    }

    /// The archive's order, rebuilt only when its entries or one of its three filters move.
    pub(crate) fn sync_library(&mut self) {
        let key = (
            self.library_rev,
            self.library.value.as_ref().map_or(0, Vec::len),
        );
        if self.library_key == Some(key) {
            return;
        }
        let anchor = self.library_key.is_some().then(|| {
            list_anchor(
                &self.library_list,
                &self.library_order,
                self.library.value.as_ref(),
            )
        });
        let kind = self.library_kind.clone();
        let query = self.library_query.clone();
        let mut counts = [0; 4];
        let mut order: Vec<usize> = Vec::new();
        let mut kind_counts: Vec<(String, usize)> = Vec::new();
        if let Some(rows) = self.library.value.as_ref() {
            for (index, row) in rows.iter().enumerate() {
                match kind_counts.iter_mut().find(|(k, _)| *k == row.kind) {
                    Some((_, n)) => *n += 1,
                    None => kind_counts.push((row.kind.clone(), 1)),
                }
                if !library_matches(row, None, kind.as_deref(), &query) {
                    continue;
                }
                counts[0] += 1;
                match row.status.as_str() {
                    "new" => counts[1] += 1,
                    "keeper" => counts[2] += 1,
                    "dismissed" => counts[3] += 1,
                    _ => {}
                }
                if self
                    .library_status
                    .is_none_or(|wanted| row.status == wanted.status())
                {
                    order.push(index);
                }
            }
            order.sort_by(|left, right| feed_order(&rows[*left], &rows[*right]));
        }
        kind_counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.library_counts = counts;
        self.library_kinds = kind_counts;
        self.library_key = Some(key);
        self.library_list.reset(order.len());
        restore_anchor(
            &self.library_list,
            anchor.flatten(),
            &order,
            self.library.value.as_ref(),
        );
        self.library_order = order;
    }

    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let target = if self.reading.is_some() {
            None
        } else if self.tab == Tab::Reading {
            self.sync_reading();
            Some((self.reading_list.clone(), self.reading_order.len()))
        } else if self.tab == Tab::Library {
            self.sync_library();
            Some((self.library_list.clone(), self.library_order.len()))
        } else {
            None
        };
        if let Some((state, rows)) = target {
            let item_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
            state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        } else {
            let max = self.tab_scroll.max_offset().y;
            self.tab_scroll.set_offset(point(px(0.), -max * fraction));
        }
        cx.notify();
    }

    pub(crate) fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.cursor = None;
        cx.notify();
    }

    pub(crate) fn set_stream(&mut self, stream: Option<&'static str>, cx: &mut Context<Self>) {
        self.stream = stream;
        self.cursor = None;
        cx.notify();
    }

    /// The page title and the native Feed task switch.
    pub(crate) fn title_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |label: &'static str, tab: Tab, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("feed-tab-{label}")))
                .small()
                .label(label)
                .when(self.tab == tab, |b| b.primary())
                .when(self.tab != tab, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| view.set_tab(tab, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Feed"),
            )
            .child(tab("Reading", Tab::Reading, cx))
            .child(tab("Review", Tab::Review, cx))
            .child(tab("Library", Tab::Library, cx))
            .child(tab("Discover", Tab::Discover, cx))
            .child(tab("Mail", Tab::Mail, cx))
            .child(tab("Clips", Tab::Clips, cx))
            .child(div().flex_1())
            .child(
                Button::new("feed-quick-clip")
                    .small()
                    .label("+ Quick Clip")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.tab = Tab::Clips;
                        view.reading = None;
                        view.clips.update(cx, |clips, cx| {
                            clips.open_composer(window, cx);
                        });
                        view.activate(cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

impl gpui_kit::Render for Feed {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let reading = self.tab == Tab::Reading
            || (!matches!(self.tab, Tab::Mail | Tab::Clips) && self.reading.is_some());
        let body = if self.tab == Tab::Mail {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.title_row(cx))
                .child(self.mail.clone())
                .into_any_element()
        } else if self.tab == Tab::Clips {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.title_row(cx))
                .child(self.clips.clone())
                .into_any_element()
        } else if self.tab == Tab::Discover {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.title_row(cx))
                .child(div().flex_1().min_h_0().child(self.discover.clone()))
                .into_any_element()
        } else if self.reading.is_some() {
            self.reader(palette, window, cx)
        } else if self.tab == Tab::Review {
            self.review_list(palette, cx)
        } else if self.tab == Tab::Library {
            self.library_list(palette, window, cx)
        } else {
            self.list(palette, cx)
        };
        let own_list = self.reading.is_none() && matches!(self.tab, Tab::Reading | Tab::Library);
        let body = if own_list {
            body
        } else {
            let scroll = self.tab_scroll.clone();
            div()
                .id("feed-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .vertical_scrollbar(&scroll)
                .child(body)
                .into_any_element()
        };
        div()
            .when(reading, |el| el.key_context("Feed"))
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_keep))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_read))
            .on_action(cx.listener(Self::on_open_original))
            .on_action(cx.listener(Self::on_back))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .when_some(self.write_error.as_ref(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Not saved: {error}")),
                )
            })
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use self::library::{in_library, kinds};
    use self::review::flagged;
    use super::*;

    fn entry(id: char, day: &str, status: &str, score: f64) -> FeedEntry {
        FeedEntry {
            id: id.to_string().repeat(64),
            stream: "news".into(),
            kind: "article".into(),
            title: Some(id.to_string()),
            url: format!("https://example.org/{id}"),
            author: None,
            summary: None,
            digest_preview: None,
            day: day.into(),
            status: status.into(),
            relevance: None,
            evaluation: Some(shell::FeedEvaluation {
                overall_score: score,
                explanation: String::new(),
            }),
        }
    }

    /// The list draws one item per row the filter shows, and follows the filter without a re-read.
    #[gpui::test]
    fn the_reading_list_holds_one_item_per_row(cx: &mut TestAppContext) {
        let repo = Repo::new(shell::Shell::new("http://127.0.0.1:1").map(std::sync::Arc::new));
        let feed = cx.new(|cx| Feed::new(repo, cx));
        feed.update(cx, |feed, cx| {
            feed.rows.value = Some(vec![
                entry('a', "2026-10-20", "new", 0.9),
                entry('b', "2026-10-21", "new", 0.1),
                entry('c', "2026-10-21", "keeper", 0.8),
            ]);
            feed.sync_reading();
            assert_eq!(feed.reading_order.len(), 2);
            assert_eq!(feed.reading_list.item_count(), 2);
            assert_eq!(feed.reading_counts, [2, 1, 0]);
            feed.filter = Filter::Kept;
            feed.sync_reading();
            assert_eq!(feed.reading_order.len(), 1);
            assert_eq!(feed.reading_list.item_count(), 1);
            assert_eq!(feed.reading_counts, [2, 1, 0]);
            cx.notify();
        });
    }

    #[test]
    fn newest_day_first_then_best_score() {
        let rows = [
            entry('a', "2026-10-20", "new", 0.9),
            entry('b', "2026-10-21", "new", 0.1),
            entry('c', "2026-10-21", "new", 0.8),
            entry('d', "2026-10-21", "keeper", 1.0),
        ];
        let order = |stream| -> String {
            in_view(rows.iter(), Filter::New, stream)
                .iter()
                .map(|e| e.title.clone().unwrap())
                .collect()
        };
        assert_eq!(order(None), "cba");
        assert_eq!(order(Some("media")), "");
    }

    #[test]
    fn library_filters_by_status_kind_and_query() {
        let mut rows = [
            entry('a', "2026-10-20", "new", 0.9),
            entry('b', "2026-10-21", "dismissed", 0.1),
            entry('c', "2026-10-21", "keeper", 0.8),
        ];
        rows[2].kind = "github".into();
        rows[2].author = Some("Ferris".into());
        let order = |status, kind, query| -> String {
            in_library(rows.iter(), status, kind, query)
                .iter()
                .map(|e| e.title.clone().unwrap())
                .collect()
        };
        assert_eq!(order(None, None, ""), "cba");
        assert_eq!(order(Some(Filter::Dismissed), None, ""), "b");
        assert_eq!(order(None, Some("github"), ""), "c");
        assert_eq!(order(None, None, "ferris"), "c");
        assert_eq!(order(None, None, "example.org/a"), "a");
        let refs: Vec<&FeedEntry> = rows.iter().collect();
        assert_eq!(kinds(&refs), [("article".into(), 2), ("github".into(), 1)]);
    }

    #[test]
    fn flags_group_by_entry_in_arrival_order() {
        let flag = |id: &str, signal: &str| FeedQualityFlag {
            feed_id: id.into(),
            title: None,
            url: String::new(),
            status: "new".into(),
            content_status: "thin".into(),
            signal: signal.into(),
            reason: String::new(),
            evidence: String::new(),
            derived_at: String::new(),
        };
        let rows = [
            flag("b", "retention"),
            flag("a", "content_status"),
            flag("b", "ranking_basis"),
        ];
        let grouped: Vec<(String, usize)> = flagged(rows.iter())
            .iter()
            .map(|item| (item[0].feed_id.clone(), item.len()))
            .collect();
        assert_eq!(grouped, [("b".into(), 2), ("a".into(), 1)]);
    }
}
