//! Native Feed: triage the comms feed and read one entry.
//!
//! Comms owns the data (`capabilities/comms/src/server/content.rs`); this page reads
//! `GET /comms/feed` and writes decisions only through `POST /comms/feed/:id/status`, the route
//! that records the ledger row with them. The keys are the dashboard's
//! (`dashboard/src/routes/feed/+page.svelte`, "Keyboard triage"), so the two surfaces agree.

use chrono::{Duration, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::shell::{
    self, CalendarGoogleExportOptIn, CalendarGoogleExportReport, CloudDerivativePreview,
    FeedContentDetail, FeedDetail, FeedEntry, FeedQualityFlag, FeedQualityRefresh, FeedSurface,
    ShellError,
};

use crate::clipper::Clips;
use crate::data::{Remote, Repo};
use crate::mail::Mail;
use crate::scout::ScoutingPage;
use crate::theme::Palette;
use sjel_gpui_ui::{filter_pill, tint};

actions!(
    feed,
    [
        /// Move to the next entry.
        Next,
        /// Move to the previous entry.
        Previous,
        /// Keep the entry, or take the keep back.
        Keep,
        /// Dismiss the entry.
        Dismiss,
        /// Undo a keep or dismiss.
        Undo,
        /// Read the entry.
        Read,
        /// Open the original in the browser.
        OpenOriginal,
        /// Back to the list.
        Back,
    ]
);

/// How far back the list reads. The dashboard's inbox reads 7 days by default; a native window
/// that stays open longer gets a month.
const DAYS: u32 = 30;

/// The library reads the whole archive, as the dashboard's library does.
const ARCHIVE_DAYS: u32 = 3650;

/// How many library rows draw before `Show more`, as in `crate::documents`.
const ROW_BATCH: usize = 200;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("Feed")),
        KeyBinding::new("down", Next, Some("Feed")),
        KeyBinding::new("k", Previous, Some("Feed")),
        KeyBinding::new("up", Previous, Some("Feed")),
        KeyBinding::new("s", Keep, Some("Feed")),
        KeyBinding::new("d", Dismiss, Some("Feed")),
        KeyBinding::new("u", Undo, Some("Feed")),
        KeyBinding::new("enter", Read, Some("Feed")),
        KeyBinding::new("o", Read, Some("Feed")),
        KeyBinding::new("shift-o", OpenOriginal, Some("Feed")),
        KeyBinding::new("escape", Back, Some("Feed")),
        KeyBinding::new("cmd-[", Back, Some("Feed")),
    ]);
}

pub use sjel_gpui_core::route::FeedTab as Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    New,
    Kept,
    Dismissed,
}

struct CalendarEditForm {
    id: String,
    title: Entity<InputState>,
    start_date: Entity<InputState>,
    end_date: Entity<InputState>,
    start_time: Entity<InputState>,
    end_time: Entity<InputState>,
    location: Entity<InputState>,
    notes: Entity<InputState>,
    all_day: bool,
    kind: String,
    commitment: String,
    saving: bool,
    error: Option<String>,
}

impl Filter {
    fn status(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Kept => "keeper",
            Self::Dismissed => "dismissed",
        }
    }
}

pub struct Feed {
    repo: Repo,
    pub tab: Tab,
    pub mail: Entity<Mail>,
    pub discover: Entity<ScoutingPage>,
    pub clips: Entity<Clips>,
    palette: Option<Palette>,
    pub rows: Remote<Vec<FeedEntry>>,
    pub filter: Filter,
    /// `None` shows both streams.
    pub stream: Option<&'static str>,
    pub cursor: Option<String>,
    /// The entry the reader shows; `None` shows the list.
    pub reading: Option<String>,
    content_source: Option<&'static str>,
    pub detail: Remote<FeedDetail>,
    pub content_detail: Remote<FeedContentDetail>,
    calendar_exports: Remote<Vec<CalendarGoogleExportOptIn>>,
    calendar_export_report: Remote<CalendarGoogleExportReport>,
    calendar_export_busy: bool,
    calendar_export_error: Option<ShellError>,
    calendar_export_confirm: bool,
    calendar_edit: Option<CalendarEditForm>,
    cloud_preview: Remote<CloudDerivativePreview>,
    cloud_preview_busy: bool,
    cloud_approval_busy: bool,
    cloud_preview_error: Option<ShellError>,
    cloud_preview_scroll: ScrollHandle,
    cloud_generation: u64,
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
    library_query: String,
    library_shown: usize,
    library_input: Option<(Entity<InputState>, Subscription)>,
    focus: FocusHandle,
    focus_next: bool,
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
            cloud_generation: 0,
            write_error: None,
            review: Remote::default(),
            review_refresh: None,
            refreshing: false,
            library: Remote::default(),
            library_status: None,
            library_kind: None,
            library_query: String::new(),
            library_shown: ROW_BATCH,
            library_input: None,
            focus: cx.focus_handle(),
            focus_next: true,
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

    /// Open a comms feed entry from a deep link, without a preceding list selection.
    pub fn open_entry(&mut self, id: String, cx: &mut Context<Self>) {
        self.tab = Tab::Reading;
        self.content_source = None;
        self.open(id, cx);
    }

    /// Open a mail item from its shared content URL, without loading the live mail list.
    pub fn open_mail_entry(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_content_entry(id, "mail", cx);
    }

    /// Open a calendar item from its shared content URL.
    pub fn open_calendar_entry(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_content_entry(id, "calendar", cx);
    }

    fn open_content_entry(&mut self, id: String, source: &'static str, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.tab = Tab::Reading;
        self.content_source = Some(source);
        self.cursor = Some(id.clone());
        self.reading = Some(id.clone());
        self.detail = Remote::default();
        self.content_detail = Remote::new_loading();
        self.calendar_exports = if source == "calendar" {
            Remote::new_loading()
        } else {
            Remote::default()
        };
        self.calendar_export_report = Remote::default();
        self.calendar_export_confirm = false;
        self.calendar_export_error = None;
        self.calendar_edit = None;
        self.cloud_preview = Remote::default();
        self.cloud_preview_busy = false;
        self.cloud_approval_busy = false;
        self.cloud_preview_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        self.focus_next = true;
        cx.notify();

        let expected = id.clone();
        let work = shell::run(async move {
            let content = shell.content_item_detail(source, &id).await;
            let exports = if source == "calendar" {
                shell.calendar_google_exports().await
            } else {
                Ok(Vec::new())
            };
            (content, exports)
        });
        cx.spawn(async move |this, cx| {
            let (content, exports) = work
                .await
                .unwrap_or((Err(ShellError::Unreachable), Err(ShellError::Unreachable)));
            let _ = this.update(cx, |view, cx| {
                if view.content_source == Some(source)
                    && view.reading.as_deref() == Some(expected.as_str())
                {
                    view.content_detail.settle(content);
                    if source == "calendar" {
                        view.calendar_exports.settle(exports);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
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
                cx.notify();
            });
        })
        .detach();
    }

    fn load_review(&mut self, cx: &mut Context<Self>) {
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
    fn refresh_review(&mut self, cx: &mut Context<Self>) {
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

    fn load_library(&mut self, cx: &mut Context<Self>) {
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
                cx.notify();
            });
        })
        .detach();
    }

    fn library_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.library_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search title, author or link…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.library_query = input.read(cx).value().trim().to_lowercase();
                view.library_shown = ROW_BATCH;
                cx.notify();
            }
        });
        self.library_input = Some((input.clone(), subscription));
        input
    }

    pub fn visible(&self) -> Vec<&FeedEntry> {
        in_view(self.rows.value.iter().flatten(), self.filter, self.stream)
    }

    fn count(&self, filter: Filter) -> usize {
        in_view(self.rows.value.iter().flatten(), filter, self.stream).len()
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.cursor = None;
        cx.notify();
    }

    fn set_stream(&mut self, stream: Option<&'static str>, cx: &mut Context<Self>) {
        self.stream = stream;
        self.cursor = None;
        cx.notify();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
        let at = self
            .cursor
            .as_ref()
            .and_then(|id| ids.iter().position(|i| i == id));
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(ids.len().saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.cursor = ids.get(next).cloned();
        cx.notify();
    }

    /// The entry the keys act on: the open one in the reader, else the cursor's.
    fn target(&self) -> Option<String> {
        self.reading.clone().or_else(|| self.cursor.clone())
    }

    /// Writes `status` for `id`, showing it at once and putting the old one back if comms refuses.
    fn decide(&mut self, id: String, status: &'static str, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let surface = if self.reading.is_some() {
            FeedSurface::Reader
        } else if self.tab == Tab::Library {
            FeedSurface::Library
        } else {
            FeedSurface::Inbox
        };
        // The cursor moves on to the row below, so a run of `d` presses works down the list.
        if self.reading.is_none() && self.cursor.as_deref() == Some(id.as_str()) {
            let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
            let at = ids.iter().position(|i| *i == id).unwrap_or(0);
            self.cursor = ids
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|i| ids.get(i)))
                .cloned();
        }
        let before = self.set_local_status(&id, status);
        self.write_error = None;
        cx.notify();
        let work = shell::run({
            let id = id.clone();
            async move { shell.set_feed_status(&id, status, surface).await }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if let Err(error) = result {
                    if let Some(before) = before {
                        view.set_local_status(&id, before);
                    }
                    view.write_error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sets the status in the list and the reader, and answers what it was.
    fn set_local_status(&mut self, id: &str, status: &str) -> Option<&'static str> {
        if let Some(detail) = self.detail.value.as_mut().filter(|d| d.id == id) {
            detail.status = status.to_owned();
        }
        for flag in self.review.value.iter_mut().flatten() {
            if flag.feed_id == id {
                flag.status = status.to_owned();
            }
        }
        let mut before = None;
        let rows = self.rows.value.iter_mut().flatten();
        for row in rows.chain(self.library.value.iter_mut().flatten()) {
            if row.id == id {
                before = before.or_else(|| {
                    [Filter::New, Filter::Kept, Filter::Dismissed]
                        .into_iter()
                        .map(Filter::status)
                        .find(|s| *s == row.status)
                });
                row.status = status.to_owned();
            }
        }
        before
    }

    fn status_of(&self, id: &str) -> Option<&str> {
        self.entry(id).map(|e| e.status.as_str()).or_else(|| {
            let flags = self.review.value.as_ref()?;
            let flag = flags.iter().find(|f| f.feed_id == id)?;
            Some(flag.status.as_str())
        })
    }

    fn open(&mut self, id: String, cx: &mut Context<Self>) {
        self.content_source = None;
        self.calendar_edit = None;
        self.cloud_preview = Remote::default();
        self.cloud_preview_busy = false;
        self.cloud_approval_busy = false;
        self.cloud_preview_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        self.calendar_exports = Remote::default();
        self.calendar_export_report = Remote::default();
        self.calendar_export_confirm = false;
        self.calendar_export_error = None;
        let Ok(shell) = self.repo.shell() else { return };
        let first = self.status_of(&id) == Some("new");
        self.cursor = Some(id.clone());
        self.reading = Some(id.clone());
        self.detail = Remote::new_loading();
        self.content_detail = Remote::new_loading();
        cx.notify();
        let expected = id.clone();
        let work = shell::run(async move {
            // Telemetry for the ranking model; a failed write never stops the reader.
            let _ = shell.record_feed_open(&id, first).await;
            let (detail, content) = futures::join!(
                shell.feed_detail(&id),
                shell.content_item_detail("feed", &id),
            );
            (detail, content)
        });
        cx.spawn(async move |this, cx| {
            let (detail, content) = work
                .await
                .unwrap_or((Err(ShellError::Unreachable), Err(ShellError::Unreachable)));
            let _ = this.update(cx, |view, cx| {
                if view.reading.as_deref() == Some(expected.as_str()) {
                    view.detail.settle(detail);
                    view.content_detail.settle(content);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The entry in the feed list or the library, whichever holds it.
    fn entry(&self, id: &str) -> Option<&FeedEntry> {
        let rows = self.rows.value.iter().flatten();
        rows.chain(self.library.value.iter().flatten())
            .find(|e| e.id == id)
    }

    fn url_of(&self, id: &str) -> Option<String> {
        self.entry(id)
            .map(|e| e.url.clone())
            .or_else(|| self.detail.value.as_ref().map(|d| d.url.clone()))
            .or_else(|| self.content_detail.value.as_ref().map(|d| d.url.clone()))
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(true, cx);
        }
    }

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(false, cx);
        }
    }

    fn on_keep(&mut self, _: &Keep, _: &mut Window, cx: &mut Context<Self>) {
        if self.content_source.is_some() {
            return;
        }
        if let Some(id) = self.target() {
            let next = if self.status_of(&id) == Some("keeper") {
                "new"
            } else {
                "keeper"
            };
            self.decide(id, next, cx);
        }
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.content_source.is_some() {
            return;
        }
        if let Some(id) = self.target() {
            self.decide(id, "dismissed", cx);
        }
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.content_source.is_some() {
            return;
        }
        if let Some(id) = self.target() {
            self.decide(id, "new", cx);
        }
    }

    fn on_read(&mut self, _: &Read, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            if let Some(id) = self.cursor.clone() {
                self.open(id, cx);
            }
        }
    }

    fn on_open_original(&mut self, _: &OpenOriginal, _: &mut Window, cx: &mut Context<Self>) {
        if self.content_source == Some("calendar") {
            cx.open_url("sjel://calendar");
            return;
        }
        if let Some(url) = self.target().and_then(|id| self.url_of(&id)) {
            cx.open_url(&url);
        }
    }

    fn set_calendar_export(&mut self, id: String, opted_in: bool, cx: &mut Context<Self>) {
        if self.calendar_export_busy || self.content_source != Some("calendar") {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.calendar_export_busy = true;
        self.calendar_export_error = None;
        self.calendar_export_report = Remote::default();
        self.calendar_export_confirm = false;
        cx.notify();

        let request_id = id.clone();
        let work = shell::run(async move {
            if opted_in {
                shell.calendar_google_opt_in(&request_id).await.map(Some)
            } else {
                shell
                    .calendar_google_opt_out(&request_id)
                    .await
                    .map(|()| None)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.calendar_export_busy = false;
                match result {
                    Ok(Some(entry)) => {
                        if let Some(entries) = view.calendar_exports.value.as_mut() {
                            entries.retain(|current| current.entry_id != id);
                            entries.push(entry);
                        }
                    }
                    Ok(None) => {
                        if let Some(entries) = view.calendar_exports.value.as_mut() {
                            entries.retain(|entry| entry.entry_id != id);
                        }
                    }
                    Err(error) => view.calendar_export_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn preview_calendar_export(&mut self, cx: &mut Context<Self>) {
        if self.calendar_export_busy
            || self.content_source != Some("calendar")
            || self
                .calendar_exports
                .value
                .as_ref()
                .is_none_or(Vec::is_empty)
        {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.calendar_export_busy = true;
        self.calendar_export_error = None;
        self.calendar_export_report = Remote::new_loading();
        self.calendar_export_confirm = false;
        cx.notify();

        let work = shell::run(async move { shell.calendar_google_export(true).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.calendar_export_busy = false;
                view.calendar_export_report.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn run_calendar_export(&mut self, cx: &mut Context<Self>) {
        let has_preview = self
            .calendar_export_report
            .value
            .as_ref()
            .is_some_and(|report| report.dry_run && !report.pushed.is_empty());
        if self.calendar_export_busy
            || !has_preview
            || !self.calendar_export_confirm
            || self.content_source != Some("calendar")
        {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.calendar_export_busy = true;
        self.calendar_export_error = None;
        self.calendar_export_confirm = false;
        let work = shell::run(async move {
            let report = shell.calendar_google_export(false).await;
            let exports = if report.is_ok() {
                Some(shell.calendar_google_exports().await)
            } else {
                None
            };
            (report, exports)
        });
        cx.spawn(async move |this, cx| {
            let (report, exports) = work.await.unwrap_or((Err(ShellError::Unreachable), None));
            let _ = this.update(cx, |view, cx| {
                view.calendar_export_busy = false;
                view.calendar_export_report.settle(report);
                if let Some(exports) = exports {
                    view.calendar_exports.settle(exports);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn request_calendar_export(&mut self, cx: &mut Context<Self>) {
        let has_preview = self
            .calendar_export_report
            .value
            .as_ref()
            .is_some_and(|report| report.dry_run && !report.pushed.is_empty());
        if !self.calendar_export_busy && has_preview && self.content_source == Some("calendar") {
            self.calendar_export_confirm = true;
            cx.notify();
        }
    }

    fn prepare_cloud_preview(&mut self, cx: &mut Context<Self>) {
        let source = self.content_source.unwrap_or("feed");
        if !matches!(source, "feed" | "mail") {
            return;
        }
        let Some(id) = self.reading.clone() else {
            return;
        };
        if self.cloud_preview_busy || self.cloud_approval_busy {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.cloud_preview_busy = true;
        self.cloud_preview_error = None;
        self.cloud_preview = Remote::new_loading();
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        let generation = self.cloud_generation;
        cx.notify();

        let expected_source = source;
        let expected_id = id.clone();
        let work = shell::run(async move { shell.content_cloud_preview(source, &id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.cloud_generation != generation
                    || view.content_source.unwrap_or("feed") != expected_source
                    || view.reading.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                view.cloud_preview_busy = false;
                view.cloud_preview.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn approve_cloud_preview(&mut self, cx: &mut Context<Self>) {
        if self.cloud_preview_busy || self.cloud_approval_busy {
            return;
        }
        let Some(preview) = self.cloud_preview.value.as_ref() else {
            return;
        };
        let source = preview.source.clone();
        let id = preview.id.clone();
        let hash = preview.preview_hash.clone();
        let expected_source = source.clone();
        let expected_id = id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        self.cloud_approval_busy = true;
        self.cloud_preview_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        let generation = self.cloud_generation;
        cx.notify();

        let work =
            shell::run(async move { shell.content_cloud_approve(&source, &id, &hash).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.cloud_generation != generation {
                    return;
                }
                view.cloud_approval_busy = false;
                match result {
                    Ok(state)
                        if view.content_source.unwrap_or("feed") == expected_source
                            && view.reading.as_deref() == Some(expected_id.as_str()) =>
                    {
                        if let Some(item) = view.content_detail.value.as_mut() {
                            item.cloud_processing = state;
                        }
                        view.cloud_preview = Remote::default();
                    }
                    Ok(_) => {}
                    Err(error) => view.cloud_preview_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_cloud_preview(&mut self, cx: &mut Context<Self>) {
        if self.cloud_approval_busy {
            return;
        }
        self.cloud_preview = Remote::default();
        self.cloud_preview_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        cx.notify();
    }

    fn open_calendar_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.content_detail.value.as_ref() else {
            return;
        };
        let Some(calendar) = item.calendar.as_ref() else {
            return;
        };
        let make_input = |value: String,
                          placeholder: &'static str,
                          window: &mut Window,
                          cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            })
        };
        let start_date = date_part(&calendar.starts_at);
        let raw_end_date = date_part(&calendar.ends_at);
        let end_date = if calendar.all_day {
            NaiveDate::parse_from_str(&raw_end_date, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.checked_sub_signed(Duration::days(1)))
                .map(|date| date.format("%Y-%m-%d").to_string())
                .unwrap_or(raw_end_date)
        } else {
            raw_end_date
        };
        let value = |value: Option<&str>| value.unwrap_or_default().to_owned();
        self.calendar_edit = Some(CalendarEditForm {
            id: item.id.clone(),
            title: make_input(item.title.clone().unwrap_or_default(), "Title", window, cx),
            start_date: make_input(start_date, "Start date · YYYY-MM-DD", window, cx),
            end_date: make_input(end_date, "End date · YYYY-MM-DD", window, cx),
            start_time: make_input(time_part(&calendar.starts_at), "Start · HH:MM", window, cx),
            end_time: make_input(time_part(&calendar.ends_at), "End · HH:MM", window, cx),
            location: make_input(value(calendar.location.as_deref()), "Location", window, cx),
            notes: make_input(value(item.content.as_deref()), "Notes", window, cx),
            all_day: calendar.all_day,
            kind: item.kind.clone(),
            commitment: calendar.commitment.clone(),
            saving: false,
            error: None,
        });
        cx.notify();
    }

    fn save_calendar_edit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.calendar_edit.as_mut() else {
            return;
        };
        if form.saving {
            return;
        }
        let title = form.title.read(cx).value().to_string();
        let start_date = form.start_date.read(cx).value().to_string();
        let end_date = form.end_date.read(cx).value().to_string();
        let start_time = form.start_time.read(cx).value().to_string();
        let end_time = form.end_time.read(cx).value().to_string();
        let location = form.location.read(cx).value().to_string();
        let notes = form.notes.read(cx).value().to_string();
        let patch =
            sjel_gpui_core::calendar::entry_edit_patch(&sjel_gpui_core::calendar::EntryEdit {
                title: &title,
                kind: &form.kind,
                commitment: &form.commitment,
                all_day: form.all_day,
                start_date: &start_date,
                end_date: &end_date,
                start_time: &start_time,
                end_time: &end_time,
                location: &location,
                notes: &notes,
            });
        let patch = match patch {
            Ok(patch) => patch,
            Err(error) => {
                form.error = Some(error.to_owned());
                cx.notify();
                return;
            }
        };
        let id = form.id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        form.saving = true;
        form.error = None;
        let request_id = id.clone();
        let work = shell::run(async move { shell.update_calendar_entry(&request_id, patch).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| match result {
                Ok(_) => view.open_content_entry(id, "calendar", cx),
                Err(error) => {
                    if let Some(form) = view.calendar_edit.as_mut() {
                        form.saving = false;
                        form.error = Some(error.to_string());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn on_back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.content_source {
            self.reading = None;
            self.cursor = None;
            self.content_source = None;
            if source == "mail" {
                self.tab = Tab::Mail;
                self.mail.update(cx, |mail, cx| mail.activate(cx));
            }
            cx.notify();
            return;
        }
        // Escape closes the reader first, then clears the cursor.
        if self.reading.take().is_some() || self.cursor.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Feed {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }
        let palette = self.palette.unwrap_or_else(Palette::dark);
        // The reader draws over Reading, Review and Library, and Escape returns to the tab that
        // opened it. Only then, or on the Reading list, do the Feed keys act: elsewhere they would
        // move a cursor nobody sees.
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
                .child(div().flex_1().min_h_0().child(self.clips.clone()))
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

impl Feed {
    fn list(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let pill = |label: &'static str, filter: Filter, cx: &mut Context<Self>| {
            filter_pill(
                label,
                self.count(filter),
                self.filter == filter,
                palette,
                cx.listener(move |view, _, _, cx| view.set_filter(filter, cx)),
            )
        };
        let stream_button =
            |label: &'static str, stream: Option<&'static str>, cx: &mut Context<Self>| {
                Button::new(SharedString::from(format!("feed-stream-{label}")))
                    .small()
                    .label(label)
                    .when(self.stream == stream, |b| b.primary())
                    .when(self.stream != stream, |b| b.ghost())
                    .on_click(cx.listener(move |view, _, _, cx| view.set_stream(stream, cx)))
            };
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(pill("New", Filter::New, cx))
            .child(pill("Kept", Filter::Kept, cx))
            .child(pill("Dismissed", Filter::Dismissed, cx))
            .child(div().flex_1())
            .child(stream_button("All", None, cx))
            .child(stream_button("News", Some("news"), cx))
            .child(stream_button("Media", Some("media"), cx));

        let mut list = div().flex().flex_col().gap_1();
        if self.rows.value.is_none() && self.rows.loading {
            list = list.child(muted("Loading the feed…", palette));
        }
        if let Some(error) = &self.rows.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        let visible = self.visible();
        if visible.is_empty() && !self.rows.loading && self.rows.error.is_none() {
            list = list.child(muted(
                match self.filter {
                    Filter::New => "Nothing new. The inbox is clear.",
                    Filter::Kept => "Nothing kept in the last 30 days.",
                    Filter::Dismissed => "Nothing dismissed in the last 30 days.",
                },
                palette,
            ));
        }
        let mut day = None;
        for (i, entry) in visible.into_iter().enumerate() {
            if day != Some(entry.day.as_str()) {
                day = Some(entry.day.as_str());
                list = list.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(entry.day.clone()),
                );
            }
            let id = entry.id.clone();
            let chosen = self.cursor.as_deref() == Some(entry.id.as_str());
            let preview = entry
                .summary
                .as_deref()
                .or(entry.digest_preview.as_deref())
                .unwrap_or_default()
                .trim()
                .to_owned();
            let score = entry.score();
            let score_badge = (score > 0.).then(|| {
                let color = if score >= 0.8 {
                    palette.green
                } else if score >= 0.5 {
                    palette.amber
                } else {
                    palette.muted
                };
                div()
                    .px_1p5()
                    .py(px(0.5))
                    .rounded(px(3.))
                    .bg(rgba(tint(color, 0x18)))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(color))
                    .child(format!("{:.0}%", score * 100.))
            });
            let keep_id = entry.id.clone();
            let dismiss_id = entry.id.clone();
            let quick_triage = (entry.status == "new").then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id(("feed-quick-keep", i))
                            .px_2()
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(rgba(tint(palette.green, 0x18)))
                            .hover(move |s| s.bg(rgba(tint(palette.green, 0x33))))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.green))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.decide(keep_id.clone(), "keeper", cx);
                            }))
                            .child("Keep"),
                    )
                    .child(
                        div()
                            .id(("feed-quick-dismiss", i))
                            .px_2()
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(rgba(tint(palette.red, 0x18)))
                            .hover(move |s| s.bg(rgba(tint(palette.red, 0x33))))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.red))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.decide(dismiss_id.clone(), "dismissed", cx);
                            }))
                            .child("Dismiss"),
                    )
            });
            list = list.child(
                div()
                    .id(("feed-row", i))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .when(chosen, |el| el.bg(rgba(palette.raised)))
                    .hover(move |style| style.bg(rgba(palette.raised)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title(entry.title.as_deref(), &entry.url)),
                            )
                            .children(score_badge)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(meta(entry)),
                            )
                            .children(quick_triage),
                    )
                    .when(!preview.is_empty(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .line_clamp(2)
                                .child(preview),
                        )
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_row(cx))
            .child(muted(
                "j/k move · s keep · d dismiss · u undo · o read",
                palette,
            ))
            .child(toolbar)
            .child(list)
            .into_any_element()
    }

    /// The page title and the native Feed task switch.
    fn title_row(&self, cx: &mut Context<Self>) -> AnyElement {
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
            .into_any_element()
    }

    fn reader(&self, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(source) = self.content_source {
            return self.source_reader(source, palette, window, cx);
        }
        let back = Button::new("feed-back")
            .small()
            .ghost()
            .label(match self.tab {
                Tab::Review => "← Review",
                Tab::Library => "← Library",
                _ => "← Feed",
            })
            .on_click(cx.listener(|view, _, window, cx| view.on_back(&Back, window, cx)));
        let Some(detail) = &self.detail.value else {
            let state = match &self.detail.error {
                Some(error) => div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
                None => muted("Opening…", palette),
            };
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().child(back))
                .child(state)
                .into_any_element();
        };
        let kept = detail.status == "keeper";
        let actions = div()
            .flex()
            .gap_2()
            .child(back)
            .child(div().flex_1())
            .child(
                Button::new("feed-keep")
                    .small()
                    .label(if kept { "Kept · S" } else { "Keep · S" })
                    .when(kept, |b| b.primary())
                    .on_click(cx.listener(|view, _, window, cx| view.on_keep(&Keep, window, cx))),
            )
            .child(
                Button::new("feed-dismiss")
                    .small()
                    .label("Dismiss · D")
                    .on_click(
                        cx.listener(|view, _, window, cx| view.on_dismiss(&Dismiss, window, cx)),
                    ),
            )
            .child(
                Button::new("feed-original")
                    .small()
                    .label("Open original · ⇧O")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.on_open_original(&OpenOriginal, window, cx)
                    })),
            );
        let why = detail
            .evaluation
            .as_ref()
            .map(|e| e.explanation.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| detail.relevance.first().map(|r| r.rationale.clone()));
        let text = |label: &'static str, body: &Option<String>| {
            body.as_ref()
                .filter(|t| !t.trim().is_empty())
                .map(|body| section(label, body.clone(), palette))
        };
        let content_detail = self.content_detail.value.as_ref();
        let digest = content_detail
            .and_then(|content| content.digest.as_ref())
            .and_then(|digest| digest.text.as_deref())
            .filter(|text| !text.trim().is_empty())
            .filter(|text| detail.summary.as_deref() != Some(*text))
            .filter(|text| detail.transcript.as_deref() != Some(*text));
        let source_content = content_detail
            .and_then(|content| content.content.as_deref())
            .filter(|text| !text.trim().is_empty())
            .filter(|text| detail.transcript.as_deref() != Some(*text));
        let mut reader = div()
            .flex()
            .flex_col()
            .gap_4()
            .max_w(px(760.))
            .child(actions)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title(detail.title.as_deref(), &detail.url)),
            )
            .child(muted(
                &[
                    Some(detail.kind.as_str()),
                    detail.author.as_deref(),
                    Some(detail.day.as_str()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                palette,
            ))
            .when_some(why, |el, why| el.child(section("WHY THIS", why, palette)))
            .children(text("SUMMARY", &detail.summary))
            .when_some(digest, |el, digest| {
                el.child(section("LOCAL DIGEST", digest.to_owned(), palette))
            })
            .children(text("TRANSCRIPT", &detail.transcript))
            .when_some(source_content, |el, body| {
                let label = content_detail
                    .map(|content| content.content_label.as_str())
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or("SOURCE CONTENT");
                el.child(section(label, body.to_owned(), palette))
            })
            .when(
                detail.summary.is_none() && detail.transcript.is_none(),
                |el| {
                    el.child(muted(
                        match detail.content_status.as_str() {
                            "none" => "Comms holds no text for this entry. Open the original.",
                            _ => "No summary yet. Open the original.",
                        },
                        palette,
                    ))
                },
            );
        if let Some(content) = content_detail {
            reader = reader.child(self.cloud_processing_panel(content, palette, cx));
        }
        reader.into_any_element()
    }

    fn source_reader(
        &self,
        source: &str,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mail = source == "mail";
        let back = Button::new("source-content-back")
            .small()
            .ghost()
            .label(if mail { "← Mail" } else { "← Feed" })
            .on_click(cx.listener(|view, _, window, cx| view.on_back(&Back, window, cx)));
        let Some(item) = self.content_detail.value.as_ref() else {
            let state = match &self.content_detail.error {
                Some(error) => div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
                None => muted("Opening source item…", palette),
            };
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(back)
                .child(state)
                .into_any_element();
        };

        let open = Button::new("source-content-original")
            .small()
            .label(if mail {
                "Open in Gmail ↗"
            } else {
                "Open in Calendar ↗"
            })
            .on_click(
                cx.listener(|view, _, window, cx| view.on_open_original(&OpenOriginal, window, cx)),
            );
        let edit = (!mail).then(|| {
            Button::new("calendar-edit")
                .small()
                .label(if self.calendar_edit.is_some() {
                    "Editing…"
                } else {
                    "Edit"
                })
                .disabled(self.calendar_edit.as_ref().is_some_and(|form| form.saving))
                .on_click(cx.listener(|view, _, window, cx| {
                    if view.calendar_edit.is_some() {
                        view.calendar_edit = None;
                        cx.notify();
                    } else {
                        view.open_calendar_edit(window, cx);
                    }
                }))
        });
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(back)
            .child(div().flex_1())
            .children(edit)
            .child(open);
        let mut reader = div()
            .flex()
            .flex_col()
            .gap_4()
            .max_w(px(760.))
            .child(actions)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title(item.title.as_deref(), &item.url)),
            );

        if let Some(editor) = self.calendar_edit_panel(palette, window, cx) {
            reader = reader.child(editor);
        }

        let metadata = [
            Some(item.kind.as_str()),
            item.author.as_deref(),
            (!item.day.is_empty()).then_some(item.day.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        if !metadata.is_empty() {
            reader = reader.child(muted(&metadata, palette));
        }
        if let Some(calendar) = item.calendar.as_ref() {
            let when = if calendar.all_day {
                format!(
                    "All day · {} through the day before {}",
                    calendar.starts_at, calendar.ends_at
                )
            } else {
                format!("{} – {}", calendar.starts_at, calendar.ends_at)
            };
            reader = reader.child(section("WHEN", when, palette));
            if let Some(location) = calendar
                .location
                .as_deref()
                .filter(|location| !location.trim().is_empty())
            {
                reader = reader.child(section("LOCATION", location.to_owned(), palette));
            }
            if !calendar.commitment.is_empty() {
                reader = reader.child(section("COMMITMENT", calendar.commitment.clone(), palette));
            }
            let export_blocked = if calendar.entry_source == "google" {
                Some("Google entries cannot be exported back to Google.")
            } else if calendar.rhythm_id.is_some() {
                Some("Rhythm instances cannot be exported individually.")
            } else {
                None
            };
            let exported = self
                .calendar_exports
                .value
                .as_ref()
                .is_some_and(|entries| entries.iter().any(|entry| entry.entry_id == item.id));
            let export_id = item.id.clone();
            let export_button = Button::new("calendar-google-export-opt-in")
                .small()
                .when(exported, |button| button.ghost())
                .when(!exported, |button| button.primary())
                .label(if self.calendar_export_busy {
                    "Saving…"
                } else if exported {
                    "Remove Google export opt-in"
                } else {
                    "Allow Google export"
                })
                .disabled(
                    self.calendar_export_busy
                        || export_blocked.is_some()
                        || self.calendar_exports.value.is_none(),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.set_calendar_export(export_id.clone(), !exported, cx)
                }));
            let can_preview = self
                .calendar_exports
                .value
                .as_ref()
                .is_some_and(|entries| !entries.is_empty());
            let preview_button = Button::new("calendar-google-export-preview")
                .small()
                .ghost()
                .label(if self.calendar_export_report.loading {
                    "Building preview…"
                } else {
                    "Preview approved exports"
                })
                .disabled(self.calendar_export_busy || !can_preview)
                .on_click(cx.listener(|view, _, _, cx| view.preview_calendar_export(cx)));
            reader = reader.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(export_button)
                    .child(preview_button),
            );
            if let Some(report) = self.calendar_export_report.value.as_ref() {
                reader = reader.child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .bg(rgba(palette.raised))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(if report.dry_run {
                            format!("{} approved entries would be sent", report.pushed.len())
                        } else {
                            format!("Export finished: {} created, {} updated", report.inserted, report.patched)
                        }))
                        .children(report.pushed.iter().map(|entry| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} · {}", entry.title, entry.operation))
                        }))
                        .children(report.skipped.iter().map(|entry| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .child(format!("Skipped · {}", entry.reason))
                        }))
                        .when(report.dry_run && report.pushed.is_empty(), |panel| {
                            panel.child(muted("No approved entries are ready to send.", palette))
                        })
                        .when(
                            report.dry_run
                                && !self.calendar_export_confirm
                                && !report.pushed.is_empty(),
                            |panel| {
                            panel.child(
                                Button::new("calendar-google-export-run")
                                    .small()
                                    .primary()
                                    .label(if self.calendar_export_busy {
                                        "Sending…".to_owned()
                                    } else {
                                        format!("Send {} approved entries to Google", report.pushed.len())
                                    })
                                    .disabled(self.calendar_export_busy)
                                    .on_click(cx.listener(|view, _, _, cx| view.request_calendar_export(cx))),
                            )
                        })
                        .when(
                            report.dry_run
                                && self.calendar_export_confirm
                                && !report.pushed.is_empty(),
                            |panel| {
                                panel.child(muted(
                                    "This sends only the entries individually approved for Google.",
                                    palette,
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            Button::new("calendar-google-export-cancel")
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.calendar_export_confirm = false;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("calendar-google-export-confirm")
                                                .small()
                                                .primary()
                                                .label(format!("Confirm send · {} entries", report.pushed.len()))
                                                .disabled(self.calendar_export_busy)
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.run_calendar_export(cx)
                                                })),
                                        ),
                                )
                            },
                        ),
                );
            }
            if let Some(reason) = export_blocked {
                reader = reader.child(muted(reason, palette));
            }
            if let Some(error) = self
                .calendar_export_error
                .as_ref()
                .or(self.calendar_exports.error.as_ref())
                .or(self.calendar_export_report.error.as_ref())
            {
                reader = reader.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Google Calendar action failed: {error}")),
                );
            }
        }
        if let Some(summary) = item
            .summary
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            reader = reader.child(section("SUMMARY", summary.to_owned(), palette));
        }
        if let Some(digest) = item
            .digest
            .as_ref()
            .and_then(|digest| digest.text.as_deref())
            .filter(|text| !text.trim().is_empty())
        {
            reader = reader.child(section("LOCAL DIGEST", digest.to_owned(), palette));
        }
        if matches!(source, "feed" | "mail") {
            reader = reader.child(self.cloud_processing_panel(item, palette, cx));
        }
        if let Some(content) = item
            .content
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            let label = if item.content_label.trim().is_empty() {
                if mail {
                    "MAIL PREVIEW"
                } else {
                    "NOTES"
                }
            } else {
                item.content_label.as_str()
            };
            reader = reader.child(section(label, content.to_owned(), palette));
        } else if item.content_status == "none" {
            reader = reader.child(muted(
                if mail {
                    "Sjel holds no mail text for this item. Open it in Gmail."
                } else {
                    "This calendar entry has no notes."
                },
                palette,
            ));
        }
        reader.into_any_element()
    }

    fn cloud_processing_panel(
        &self,
        item: &FeedContentDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = match item.cloud_processing.status.as_str() {
            "staged" => "Staged locally",
            "stale" => "Review again",
            _ => "Not prepared",
        };
        let preview_button = Button::new("feed-cloud-preview")
            .small()
            .label(if self.cloud_preview_busy {
                "Preparing locally…"
            } else if item.cloud_processing.status == "staged" {
                "Review a fresh copy"
            } else {
                "Preview cloud-ready copy"
            })
            .disabled(self.cloud_preview_busy || self.cloud_approval_busy)
            .on_click(cx.listener(|view, _, _, cx| view.prepare_cloud_preview(cx)));
        let mut panel = div()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.raised))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Cloud processing"),
                    )
                    .child(muted(status, palette)),
            )
            .child(muted(
                "Build a bounded copy locally and inspect it before staging approval.",
                palette,
            ))
            .child(preview_button);

        if let Some(error) = self
            .cloud_preview_error
            .as_ref()
            .or(self.cloud_preview.error.as_ref())
        {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(format!("Cloud preview failed: {error}")),
            );
        }

        if let Some(preview) = self.cloud_preview.value.as_ref() {
            let can_approve = preview.approval_required && preview.provider_calls == 0;
            let mut review = div()
                .mt_2()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(palette.border))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Review the cloud-ready copy"),
                )
                .child(muted(
                    &format!(
                        "{} source → {} derivative · {} redactions · {} · {}",
                        preview.original_data_class,
                        preview.derivative_data_class,
                        preview.redaction_count,
                        if preview.truncated {
                            "bounded at 16,000 characters"
                        } else {
                            "complete bounded document"
                        },
                        preview.entity_detection
                    ),
                    palette,
                ));
            if let Some(receipt) = preview.redaction_receipt.as_deref() {
                review = review.child(section("REDACTION RECEIPT", receipt.to_owned(), palette));
            }
            if !preview.redactions.is_empty() {
                review = review
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Detected locally"),
                    )
                    .children(preview.redactions.iter().map(|finding| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} {} × {}",
                                finding.marker, finding.entity_type, finding.count
                            ))
                    }));
            }
            review = review.child(
                div()
                    .id("feed-cloud-preview-document")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .track_scroll(&self.cloud_preview_scroll)
                    .vertical_scrollbar(&self.cloud_preview_scroll)
                    .p_2()
                    .rounded(px(4.))
                    .bg(rgba(palette.surface))
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(18.))
                            .child(preview.document.clone()),
                    ),
            );
            if !preview.limitations.is_empty() {
                review = review
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Review limits"),
                    )
                    .children(preview.limitations.iter().map(|limitation| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("• {limitation}"))
                    }));
            }
            review = review
                .child(muted(
                    &format!("This stores the exact reviewed copy locally. Provider calls: {}.", preview.provider_calls),
                    palette,
                ))
                .when(!can_approve, |el| {
                    el.child(muted(
                        "This preview cannot be approved because it does not meet the local approval contract.",
                        palette,
                    ))
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("feed-cloud-preview-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(self.cloud_approval_busy)
                                .on_click(cx.listener(|view, _, _, cx| view.close_cloud_preview(cx))),
                        )
                        .child(
                            Button::new("feed-cloud-preview-approve")
                                .small()
                                .primary()
                                .label(if self.cloud_approval_busy {
                                    "Staging…"
                                } else {
                                    "Approve cloud-ready copy"
                                })
                                .disabled(!can_approve || self.cloud_approval_busy)
                                .on_click(cx.listener(|view, _, _, cx| view.approve_cloud_preview(cx))),
                        ),
                );
            panel = panel.child(review);
        } else if self.cloud_preview.loading {
            panel = panel.child(muted("Building a local preview…", palette));
        }
        panel.into_any_element()
    }

    fn calendar_edit_panel(
        &self,
        palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.calendar_edit.as_ref()?;
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Input::new(&form.title).small())
            .child(
                Button::new("calendar-edit-all-day")
                    .xsmall()
                    .label(if form.all_day { "All day" } else { "Timed" })
                    .when(form.all_day, |button| button.primary())
                    .when(!form.all_day, |button| button.ghost())
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(form) = view.calendar_edit.as_mut() {
                            form.all_day = !form.all_day;
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&form.start_date).small()))
                    .child(div().flex_1().child(Input::new(&form.end_date).small())),
            );
        if !form.all_day {
            fields = fields.child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&form.start_time).small()))
                    .child(div().flex_1().child(Input::new(&form.end_time).small())),
            );
        }
        fields =
            fields
                .child(Input::new(&form.location).small())
                .child(Input::new(&form.notes).small())
                .child(div().text_xs().text_color(rgb(palette.muted)).child("KIND"))
                .child(
                    div().flex().flex_wrap().gap_1().children(
                        sjel_gpui_core::calendar::KINDS
                            .iter()
                            .map(|(key, label, _)| {
                                let selected = form.kind == *key;
                                Button::new(SharedString::from(format!("feed-calendar-kind-{key}")))
                                    .xsmall()
                                    .label(*label)
                                    .when(selected, |button| button.primary())
                                    .when(!selected, |button| button.ghost())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(form) = view.calendar_edit.as_mut() {
                                            form.kind = (*key).to_owned();
                                        }
                                        cx.notify();
                                    }))
                            }),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("COMMITMENT"),
                )
                .child(
                    div().flex().gap_1().children(
                        sjel_gpui_core::calendar::COMMITMENTS
                            .iter()
                            .map(|(key, label)| {
                                let selected = form.commitment == *key;
                                Button::new(SharedString::from(format!(
                                    "feed-calendar-commitment-{key}"
                                )))
                                .xsmall()
                                .label(*label)
                                .when(selected, |button| button.primary())
                                .when(!selected, |button| button.ghost())
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        if let Some(form) = view.calendar_edit.as_mut() {
                                            form.commitment = (*key).to_owned();
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
                );
        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        Some(
            div()
                .p_3()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.raised))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Edit calendar entry"),
                )
                .child(fields)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("feed-calendar-cancel-edit")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(form.saving)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.calendar_edit = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("feed-calendar-save-edit")
                                .small()
                                .primary()
                                .label(if form.saving {
                                    "Saving…"
                                } else {
                                    "Save changes"
                                })
                                .disabled(form.saving)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.save_calendar_edit(cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

impl Feed {
    /// Flags suggest a look; they never change a decision or start enrichment.
    fn review_list(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let items = flagged(self.review.value.iter().flatten());
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(match items.len() {
                        1 => "1 entry to inspect".to_owned(),
                        n => format!("{n} entries to inspect"),
                    }),
            )
            .child(
                Button::new("feed-review-refresh")
                    .small()
                    .label(if self.refreshing {
                        "Computing…"
                    } else {
                        "Recompute flags"
                    })
                    .disabled(self.refreshing)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh_review(cx))),
            );
        let mut list = div().flex().flex_col().gap_3();
        if let Some(r) = &self.review_refresh {
            list = list.child(muted(
                &format!(
                    "Checked {}; flagged {} entries with {} signals; provider calls {}.",
                    r.reviewed, r.flagged_items, r.flag_count, r.provider_calls
                ),
                palette,
            ));
        }
        if let Some(error) = &self.review.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        } else if self.review.value.is_none() && self.review.loading {
            list = list.child(muted("Loading stored flags…", palette));
        } else if items.is_empty() && !self.review.loading {
            list = list.child(muted(
                "No stored flags. Recompute, or the evidence is inside the thresholds.",
                palette,
            ));
        }
        for (i, flags) in items.into_iter().enumerate() {
            let first = flags[0];
            let id = first.feed_id.clone();
            let url = first.url.clone();
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(8.))
                .bg(rgba(palette.raised))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id(("feed-review-title", i))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(rgb(palette.accent)))
                                .on_click(
                                    cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)),
                                )
                                .child(title(first.title.as_deref(), &first.url)),
                        )
                        .child(
                            Button::new(("feed-review-original", i))
                                .small()
                                .ghost()
                                .label("Original ↗")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                )
                .child(muted(
                    &format!(
                        "{} · content {} · {}",
                        first.status,
                        first.content_status,
                        first
                            .derived_at
                            .get(..16)
                            .unwrap_or(&first.derived_at)
                            .replace('T', " ")
                    ),
                    palette,
                ));
            for flag in flags {
                card = card.child(
                    div()
                        .flex()
                        .gap_3()
                        .pt_2()
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .child(signal_label(&flag.signal)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_sm().child(flag.reason.clone()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(flag.evidence.clone()),
                                ),
                        ),
                );
            }
            list = list.child(card);
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_row(cx))
            .child(head)
            .child(list)
            .into_any_element()
    }
}

impl Feed {
    /// The archive as one table, newest first. A row opens the reader; its buttons decide.
    fn library_list(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = self.library_field(window, cx);
        let all: Vec<&FeedEntry> = self.library.value.iter().flatten().collect();
        let kind = self.library_kind.as_deref();
        let query = self.library_query.as_str();
        let count = |status| in_library(all.iter().copied(), status, kind, query).len();
        let pill = |label: &'static str, status: Option<Filter>, cx: &mut Context<Self>| {
            filter_pill(
                label,
                count(status),
                self.library_status == status,
                palette,
                cx.listener(move |view, _, _, cx| {
                    view.library_status = status;
                    view.library_shown = ROW_BATCH;
                    cx.notify();
                }),
            )
        };
        let kind_button = |label: String, kind: Option<String>, cx: &mut Context<Self>| {
            let active = self.library_kind == kind;
            Button::new(SharedString::from(format!("feed-kind-{label}")))
                .small()
                .label(label)
                .when(active, |b| b.primary())
                .when(!active, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.library_kind = kind.clone();
                    view.library_shown = ROW_BATCH;
                    cx.notify();
                }))
        };
        let mut kind_row = div().flex().flex_wrap().gap_1().child(kind_button(
            format!("All {}", all.len()),
            None,
            cx,
        ));
        for (kind, n) in kinds(&all) {
            kind_row = kind_row.child(kind_button(format!("{kind} {n}"), Some(kind), cx));
        }
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(pill("All", None, cx))
            .child(pill("New", Some(Filter::New), cx))
            .child(pill("Kept", Some(Filter::Kept), cx))
            .child(pill("Dismissed", Some(Filter::Dismissed), cx))
            .child(div().flex_1())
            .child(div().w(px(280.)).child(Input::new(&input).small()));

        let visible = in_library(all.iter().copied(), self.library_status, kind, query);
        let mut list = div().flex().flex_col();
        if let Some(error) = &self.library.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        } else if self.library.value.is_none() && self.library.loading {
            list = list.child(muted("Loading the archive…", palette));
        } else if visible.is_empty() && !self.library.loading {
            list = list.child(muted("No entries match these filters.", palette));
        }
        let hidden = visible.len().saturating_sub(self.library_shown);
        for (i, entry) in visible.into_iter().take(self.library_shown).enumerate() {
            list = list.child(library_row(i, entry, palette, cx));
        }
        if hidden > 0 {
            list = list.child(
                div().pt_2().child(
                    Button::new("feed-library-more")
                        .small()
                        .ghost()
                        .label(format!("Show more ({hidden} left)"))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.library_shown += ROW_BATCH;
                            cx.notify();
                        })),
                ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_row(cx))
            .child(toolbar)
            .child(kind_row)
            .child(list)
            .into_any_element()
    }
}

fn library_row(
    i: usize,
    entry: &FeedEntry,
    palette: Palette,
    cx: &mut Context<Feed>,
) -> AnyElement {
    let cell = |width: f32, text: String| {
        div()
            .w(px(width))
            .flex_none()
            .truncate()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(text)
    };
    // Author and lens give way first, so a narrow window still shows the title.
    let shrinking = |width: f32, text: String| cell(width, text).flex_shrink(1.).min_w_0();
    let action = |label: &'static str, status: &'static str, cx: &mut Context<Feed>| {
        let id = entry.id.clone();
        Button::new(SharedString::from(format!("feed-library-{label}-{i}")))
            .small()
            .ghost()
            .label(label)
            .on_click(cx.listener(move |view, _, _, cx| view.decide(id.clone(), status, cx)))
    };
    let actions = match entry.status.as_str() {
        "new" => div()
            .flex()
            .child(action("Keep", "keeper", cx))
            .child(action("Dismiss", "dismissed", cx)),
        _ => div().flex().child(action("Restore", "new", cx)),
    };
    let score = entry.score();
    let lens = entry
        .relevance
        .as_ref()
        .map_or("Unassigned".to_owned(), |r| r.profile_label.clone());
    let status = match entry.status.as_str() {
        "keeper" => "Kept",
        "dismissed" => "Dismissed",
        _ => "New",
    };
    let id = entry.id.clone();
    div()
        .id(("feed-library-row", i))
        .flex()
        .items_center()
        .gap_3()
        .px_2()
        .py_1()
        .rounded(px(6.))
        .when(entry.status == "dismissed", |el| el.opacity(0.5))
        .hover(move |style| style.bg(rgba(palette.raised)))
        .cursor_pointer()
        .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)))
        .child(cell(80., entry.kind.clone()))
        .child(
            div()
                .flex_1()
                .min_w(px(180.))
                .truncate()
                .text_sm()
                .child(title(entry.title.as_deref(), &entry.url)),
        )
        .child(shrinking(120., entry.author.clone().unwrap_or_default()))
        .child(shrinking(110., lens))
        .child(cell(
            40.,
            if score > 0.0 {
                format!("{:.0}%", score * 100.0)
            } else {
                String::new()
            },
        ))
        .child(cell(70., status.to_owned()))
        .child(cell(80., entry.day.clone()))
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .flex()
                .justify_end()
                .child(actions),
        )
        .into_any_element()
}

/// The library's rows for one status (`None`: all), kind and lowercase query, newest day first
/// and best score first within a day.
fn in_library<'a>(
    rows: impl Iterator<Item = &'a FeedEntry>,
    status: Option<Filter>,
    kind: Option<&str>,
    query: &str,
) -> Vec<&'a FeedEntry> {
    let mut rows: Vec<&FeedEntry> = rows
        .filter(|e| status.is_none_or(|s| e.status == s.status()))
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .filter(|e| {
            query.is_empty()
                || [
                    e.title.as_deref(),
                    e.author.as_deref(),
                    Some(e.url.as_str()),
                ]
                .into_iter()
                .flatten()
                .any(|text| text.to_lowercase().contains(query))
        })
        .collect();
    rows.sort_by(|a, b| b.day.cmp(&a.day).then(b.score().total_cmp(&a.score())));
    rows
}

/// Each kind in the archive with its count, most common first.
fn kinds(rows: &[&FeedEntry]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for row in rows {
        match counts.iter_mut().find(|(kind, _)| *kind == row.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((row.kind.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}

/// The queue's rows grouped by entry, in the order comms returned the entries.
fn flagged<'a>(flags: impl Iterator<Item = &'a FeedQualityFlag>) -> Vec<Vec<&'a FeedQualityFlag>> {
    let mut items: Vec<Vec<&FeedQualityFlag>> = Vec::new();
    for flag in flags {
        match items
            .iter_mut()
            .find(|item| item[0].feed_id == flag.feed_id)
        {
            Some(item) => item.push(flag),
            None => items.push(vec![flag]),
        }
    }
    items
}

/// The dashboard's names for comms' signal keys (`capabilities/comms/src/quality.rs`).
fn signal_label(signal: &str) -> String {
    match signal {
        "content_status" => "Content",
        "extraction_path" => "Extraction",
        "retention" => "Retention",
        "boilerplate_leakage" => "Boilerplate",
        "summary_attempts" => "Summary retries",
        "ranking_basis" => "Ranking basis",
        other => other,
    }
    .to_owned()
}

/// The entries `filter` and `stream` show, in the order the list draws them: newest day first,
/// best score first within a day.
fn in_view<'a>(
    rows: impl Iterator<Item = &'a FeedEntry>,
    filter: Filter,
    stream: Option<&str>,
) -> Vec<&'a FeedEntry> {
    let mut rows: Vec<&FeedEntry> = rows
        .filter(|e| e.status == filter.status())
        .filter(|e| stream.is_none_or(|s| e.stream == s))
        .collect();
    rows.sort_by(|a, b| b.day.cmp(&a.day).then(b.score().total_cmp(&a.score())));
    rows
}

fn title(title: Option<&str>, url: &str) -> String {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(url)
        .to_owned()
}

fn meta(entry: &FeedEntry) -> String {
    let score = entry.score();
    let mut parts = vec![entry.kind.clone()];
    if let Some(author) = entry.author.as_deref().filter(|a| !a.is_empty()) {
        parts.push(author.to_owned());
    }
    if score > 0.0 {
        parts.push(format!("{:.0}%", score * 100.0));
    }
    parts.join(" · ")
}

fn date_part(value: &str) -> String {
    value.get(..10).unwrap_or_default().to_owned()
}

fn time_part(value: &str) -> String {
    value.get(11..16).unwrap_or("09:00").to_owned()
}

fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}

fn section(label: &str, body: String, palette: Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(sjel_gpui_ui::render_markdown(
            &body,
            palette,
            sjel_gpui_ui::Density::Desktop,
        ))
}

#[cfg(test)]
mod tests {
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
