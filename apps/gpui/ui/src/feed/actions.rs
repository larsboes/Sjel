use chrono::{Duration, NaiveDate};
use gpui::AppContext;
use gpui_kit::component::input::InputState;
use gpui_kit::{actions, App, Context, KeyBinding, Window};
use sjel_gpui_core::shell::{self, FeedEntry, FeedSurface, ShellError};

use crate::data::Remote;

use super::reading::{date_part, time_part};
use super::types::{CalendarEditForm, DataClassForm, Filter, Tab};
use super::Feed;

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

impl Feed {
    pub(crate) fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
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
        // The list draws only the rows on screen, so a keyboard move has to bring its row there.
        if self.tab == Tab::Reading && self.reading.is_none() {
            self.sync_reading();
            if let Some(ix) = self.reading_order.iter().position(|index| {
                self.rows
                    .value
                    .as_ref()
                    .and_then(|rows| rows.get(*index))
                    .map(|row| Some(row.id.as_str()) == self.cursor.as_deref())
                    .unwrap_or(false)
            }) {
                self.reading_list.scroll_to_reveal_item(ix);
            }
        }
        cx.notify();
    }

    /// The entry the keys act on: the open one in the reader, else the cursor's.
    pub(crate) fn target(&self) -> Option<String> {
        self.reading.clone().or_else(|| self.cursor.clone())
    }

    /// Writes `status` for `id`, showing it at once and putting the old one back if comms refuses.
    pub(crate) fn decide(&mut self, id: String, status: &'static str, cx: &mut Context<Self>) {
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
    pub(crate) fn set_local_status(&mut self, id: &str, status: &str) -> Option<&'static str> {
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
        self.rows_rev = self.rows_rev.wrapping_add(1);
        before
    }

    pub(crate) fn status_of(&self, id: &str) -> Option<&str> {
        self.entry(id).map(|e| e.status.as_str()).or_else(|| {
            let flags = self.review.value.as_ref()?;
            let flag = flags.iter().find(|f| f.feed_id == id)?;
            Some(flag.status.as_str())
        })
    }

    pub(crate) fn open(&mut self, id: String, cx: &mut Context<Self>) {
        self.content_source = None;
        self.calendar_edit = None;
        self.cloud_preview = Remote::default();
        self.cloud_preview_busy = false;
        self.cloud_approval_busy = false;
        self.cloud_preview_error = None;
        self.cloud_providers = Remote::new_loading();
        self.cloud_job_busy = false;
        self.cloud_job_error = None;
        self.data_class_form = None;
        self.data_class_busy = false;
        self.data_class_error = None;
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
            let (detail, content, providers) = futures::join!(
                shell.feed_detail(&id),
                shell.content_item_detail("feed", &id),
                shell.content_cloud_providers(),
            );
            (detail, content, providers)
        });
        cx.spawn(async move |this, cx| {
            let (detail, content, providers) = work.await.unwrap_or((
                Err(ShellError::Unreachable),
                Err(ShellError::Unreachable),
                Err(ShellError::Unreachable),
            ));
            let _ = this.update(cx, |view, cx| {
                if view.reading.as_deref() == Some(expected.as_str()) {
                    view.detail.settle(detail);
                    view.content_detail.settle(content);
                    view.cloud_providers.settle(providers);
                    cx.notify();
                }
            });
        })
        .detach();
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

    pub(crate) fn open_content_entry(
        &mut self,
        id: String,
        source: &'static str,
        cx: &mut Context<Self>,
    ) {
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
        self.cloud_providers = if matches!(source, "feed" | "mail") {
            Remote::new_loading()
        } else {
            Remote::default()
        };
        self.cloud_job_busy = false;
        self.cloud_job_error = None;
        self.data_class_form = None;
        self.data_class_busy = false;
        self.data_class_error = None;
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
            let providers = if matches!(source, "feed" | "mail") {
                shell.content_cloud_providers().await
            } else {
                Ok(Vec::new())
            };
            (content, exports, providers)
        });
        cx.spawn(async move |this, cx| {
            let (content, exports, providers) = work.await.unwrap_or((
                Err(ShellError::Unreachable),
                Err(ShellError::Unreachable),
                Err(ShellError::Unreachable),
            ));
            let _ = this.update(cx, |view, cx| {
                if view.content_source == Some(source)
                    && view.reading.as_deref() == Some(expected.as_str())
                {
                    view.content_detail.settle(content);
                    if source == "calendar" {
                        view.calendar_exports.settle(exports);
                    }
                    if matches!(source, "feed" | "mail") {
                        view.cloud_providers.settle(providers);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The entry in the feed list or the library, whichever holds it.
    pub(crate) fn entry(&self, id: &str) -> Option<&FeedEntry> {
        let rows = self.rows.value.iter().flatten();
        rows.chain(self.library.value.iter().flatten())
            .find(|e| e.id == id)
    }

    pub(crate) fn url_of(&self, id: &str) -> Option<String> {
        self.entry(id)
            .map(|e| e.url.clone())
            .or_else(|| self.detail.value.as_ref().map(|d| d.url.clone()))
            .or_else(|| self.content_detail.value.as_ref().map(|d| d.url.clone()))
    }

    pub(crate) fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(true, cx);
        }
    }

    pub(crate) fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(false, cx);
        }
    }

    pub(crate) fn on_keep(&mut self, _: &Keep, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            let next = match self.status_of(&id) {
                Some("keeper") => "new",
                _ => "keeper",
            };
            self.decide(id, next, cx);
        }
    }

    pub(crate) fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            let next = match self.status_of(&id) {
                Some("dismissed") => "new",
                _ => "dismissed",
            };
            self.decide(id, next, cx);
        }
    }

    pub(crate) fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            self.decide(id, "new", cx);
        }
    }

    pub(crate) fn on_read(&mut self, _: &Read, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_some() {
            return;
        }
        if let Some(id) = self.cursor.clone() {
            self.open(id, cx);
        }
    }

    pub(crate) fn on_open_original(
        &mut self,
        _: &OpenOriginal,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(url) = self.target().and_then(|id| self.url_of(&id)) {
            cx.open_url(&url);
        }
    }

    pub(crate) fn set_calendar_export(
        &mut self,
        id: String,
        opted_in: bool,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn preview_calendar_export(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn run_calendar_export(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn request_calendar_export(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn prepare_cloud_preview(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn approve_cloud_preview(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn close_cloud_preview(&mut self, cx: &mut Context<Self>) {
        if self.cloud_approval_busy {
            return;
        }
        self.cloud_preview = Remote::default();
        self.cloud_preview_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn queue_cloud_job(&mut self, provider_role: String, cx: &mut Context<Self>) {
        if self.cloud_job_busy || self.cloud_preview_busy || self.cloud_approval_busy {
            return;
        }
        let source = self.content_source.unwrap_or("feed");
        if !matches!(source, "feed" | "mail") {
            return;
        }
        let Some(item) = self.content_detail.value.as_ref() else {
            return;
        };
        let Some(hash) = item.cloud_processing.preview_hash.clone() else {
            return;
        };
        let id = item.id.clone();
        let expected_source = source;
        let expected_id = id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        self.cloud_job_busy = true;
        self.cloud_job_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        let generation = self.cloud_generation;
        cx.notify();

        let work = shell::run(async move {
            shell
                .content_cloud_queue(source, &id, &hash, &provider_role)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.cloud_generation != generation
                    || view.content_source.unwrap_or("feed") != expected_source
                    || view.reading.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                view.cloud_job_busy = false;
                match result {
                    Ok(state) => {
                        if let Some(item) = view.content_detail.value.as_mut() {
                            item.cloud_processing = state;
                        }
                    }
                    Err(error) => view.cloud_job_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn run_cloud_job(&mut self, cx: &mut Context<Self>) {
        if self.cloud_job_busy || self.cloud_preview_busy || self.cloud_approval_busy {
            return;
        }
        let source = self.content_source.unwrap_or("feed");
        let Some(item) = self.content_detail.value.as_ref() else {
            return;
        };
        let Some(job_id) = item.cloud_processing.job_id.clone() else {
            return;
        };
        let expected_source = source;
        let expected_id = item.id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        self.cloud_job_busy = true;
        self.cloud_job_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        let generation = self.cloud_generation;
        cx.notify();

        let id_for_detail = expected_id.clone();
        let work = shell::run(async move {
            let state = shell.content_cloud_run_job(&job_id).await?;
            let refreshed = shell.content_item_detail(source, &id_for_detail).await.ok();
            let providers = shell.content_cloud_providers().await.ok();
            Ok((state, refreshed, providers))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.cloud_generation != generation
                    || view.content_source.unwrap_or("feed") != expected_source
                    || view.reading.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                view.cloud_job_busy = false;
                match result {
                    Ok((state, refreshed, providers)) => {
                        if let Some(detail) = refreshed {
                            view.content_detail.settle(Ok(detail));
                        } else if let Some(item) = view.content_detail.value.as_mut() {
                            item.cloud_processing = state;
                        }
                        if let Some(providers) = providers {
                            view.cloud_providers.settle(Ok(providers));
                        }
                    }
                    Err(error) => view.cloud_job_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn select_data_class(
        &mut self,
        target: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.data_class_busy {
            return;
        }
        let Some(item) = self.content_detail.value.as_ref() else {
            return;
        };
        let current = item.data_class.value.as_str();
        if current == target {
            self.data_class_form = None;
            cx.notify();
            return;
        }
        if shell::data_class_lowering_needs_rationale(current, target) {
            let rationale = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Rationale required to lower the data class…")
            });
            self.data_class_form = Some(DataClassForm {
                target,
                rationale,
                error: None,
            });
            self.data_class_error = None;
            cx.notify();
        } else {
            self.data_class_form = None;
            self.submit_data_class(target, None, cx);
        }
    }

    pub(crate) fn confirm_data_class_lowering(&mut self, cx: &mut Context<Self>) {
        if self.data_class_busy {
            return;
        }
        let Some(form) = self.data_class_form.as_mut() else {
            return;
        };
        let rationale = form.rationale.read(cx).value().trim().to_owned();
        if rationale.is_empty() {
            form.error = Some("Lowering a data class requires a rationale.".to_owned());
            cx.notify();
            return;
        }
        let target = form.target;
        form.error = None;
        self.submit_data_class(target, Some(rationale), cx);
    }

    pub(crate) fn submit_data_class(
        &mut self,
        target: &'static str,
        rationale: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let source = self.content_source.unwrap_or("feed");
        if !matches!(source, "feed" | "mail") {
            return;
        }
        let Some(id) = self.reading.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.data_class_busy = true;
        self.data_class_error = None;
        self.cloud_generation = self.cloud_generation.wrapping_add(1);
        let generation = self.cloud_generation;
        cx.notify();

        let expected_source = source;
        let expected_id = id.clone();
        let work = shell::run(async move {
            shell
                .set_content_data_class(source, &id, target, rationale.as_deref())
                .await?;
            shell.content_item_detail(source, &id).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.cloud_generation != generation
                    || view.content_source.unwrap_or("feed") != expected_source
                    || view.reading.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                view.data_class_busy = false;
                match result {
                    Ok(detail) => {
                        view.content_detail.settle(Ok(detail));
                        view.data_class_form = None;
                        view.cloud_preview = Remote::default();
                    }
                    Err(error) => {
                        view.data_class_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_calendar_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn save_calendar_edit(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn on_back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
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
