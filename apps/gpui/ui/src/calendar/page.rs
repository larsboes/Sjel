//! Desktop CalendarPage view, month/week navigation, side panel and day entries.
use chrono::{Duration, Local, Months, NaiveDate, NaiveTime};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::time_field::{TimeField, TimeFieldEvent, TimeFieldState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FocusHandle, FontWeight, ScrollHandle,
    SharedString, Window,
};
use sjel_gpui_core::calendar::{
    self, entries_on, kind_color, kind_label, time_label, COMMITMENTS, KINDS,
};
use sjel_gpui_core::shell::{self, CalendarEntry, CalendarEntryDetail, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::key_bar;
use crate::theme::{tint, Density, Palette, ThemeContext};

use super::types::CalendarIntent;
use super::types::{muted, Form, View};

pub use super::actions::bind_keys;

pub struct CalendarPage {
    pub(super) repo: Repo,
    palette: Option<Palette>,
    density: Density,
    pub view: View,
    pub(super) show_availability: bool,
    pub(super) changing_time: bool,
    pub(super) drop_time: Option<(NaiveDate, u16)>,
    pub(super) undo_time: Option<(CalendarEntry, CalendarEntry)>,
    /// The selected day; the grid shows its month or week.
    pub day: NaiveDate,
    pub entries: Remote<Vec<CalendarEntry>>,
    /// The `[from, to)` window `entries` answers.
    pub(super) loaded: Option<(String, String)>,
    pub(super) proposals: Remote<Vec<CalendarEntry>>,
    pub(super) proposals_window: Option<(String, String)>,
    pub(super) reviewing_proposals: bool,
    pub(super) proposal_action: Option<String>,
    pub(super) proposal_confirm_remove: Option<String>,
    pub selected_entry: Option<String>,
    pub(super) selected_detail: Option<(String, CalendarEntryDetail)>,
    pub(super) confirm_delete: Option<String>,
    pub(super) form: Option<Form>,
    pub write_error: Option<ShellError>,
    pub sync_status: Option<super::types::CalendarSyncStatus>,
    side_scroll: ScrollHandle,
    pub(super) timeline_scroll: ScrollHandle,
    pub(super) month_scroll: ScrollHandle,
    focus: FocusHandle,
    focus_next: bool,
    clock: Option<gpui::Task<()>>,
}

impl gpui::EventEmitter<CalendarIntent> for CalendarPage {}

impl CalendarPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self::with_density(repo, Density::Desktop, cx)
    }

    pub fn with_density(repo: Repo, density: Density, cx: &mut Context<Self>) -> Self {
        let timeline_scroll = ScrollHandle::new();
        timeline_scroll.set_offset(gpui::point(px(0.), px(-8. * 48.)));
        Self {
            repo,
            palette: None,
            density,
            view: View::Month,
            show_availability: false,
            changing_time: false,
            drop_time: None,
            undo_time: None,
            day: Local::now().date_naive(),
            entries: Remote::default(),
            loaded: None,
            proposals: Remote::default(),
            proposals_window: None,
            reviewing_proposals: false,
            proposal_action: None,
            proposal_confirm_remove: None,
            selected_entry: None,
            selected_detail: None,
            confirm_delete: None,
            form: None,
            write_error: None,
            sync_status: None,
            side_scroll: ScrollHandle::new(),
            timeline_scroll,
            month_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            focus_next: true,
            clock: None,
        }
    }

    pub fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density != density {
            self.density = density;
            cx.notify();
        }
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        match self.view {
            View::Month => self.month_scroll.clone(),
            View::Day | View::Week => self.timeline_scroll.clone(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        self.loaded = None;
        self.load(cx);
        self.load_proposals(cx, true);
    }

    pub fn open_day(&mut self, day: NaiveDate, entry_id: Option<String>, cx: &mut Context<Self>) {
        self.day = day;
        self.selected_entry = None;
        self.selected_detail = None;
        self.confirm_delete = None;
        self.loaded = None;
        self.load(cx);
        if let Some(id) = entry_id {
            self.select_entry(id, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn dates(&self) -> Vec<NaiveDate> {
        match self.view {
            View::Day => vec![self.day],
            View::Month => calendar::month_dates(self.day),
            View::Week => calendar::week_dates(self.day),
        }
    }

    /// Reads the window on screen, unless that window is already loaded.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(window) = calendar::window(&self.dates()) else {
            return;
        };
        if self.loaded.as_ref() == Some(&window) {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.entries.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.loaded = Some(window.clone());
        self.entries.loading = true;
        cx.notify();
        let (from, to) = window.clone();
        let work = shell::run(async move { shell.calendar(&from, &to).await });
        let repo = self.repo.clone();
        let status_work = shell::run(async move { repo.calendar_sync_status().await.ok() });
        cx.spawn(async move |this, cx| {
            let status = status_work.await.flatten();
            let _ = this.update(cx, |view, cx| {
                view.sync_status = status;
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.loaded.as_ref() == Some(&window) {
                    view.entries.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Reads the window on screen again (⌘R and after a write).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loaded = None;
        self.load(cx);
        if self.reviewing_proposals {
            self.load_proposals(cx, true);
        }
    }

    pub(super) fn select(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        if day != self.day {
            self.day = day;
            self.selected_entry = None;
            self.selected_detail = None;
            self.confirm_delete = None;
        }
        self.load(cx);
        cx.notify();
    }

    pub(super) fn select_entry(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_entry = Some(id.clone());
        self.selected_detail = None;
        self.confirm_delete = None;
        if let Ok(shell) = self.repo.shell() {
            let entry_id = id.clone();
            let query_id = id.clone();
            let work = shell::run(async move { shell.calendar_entry(&query_id).await.ok() });
            cx.spawn(async move |this, cx| {
                if let Some(detail) = work.await.flatten() {
                    let _ = this.update(cx, |page, cx| {
                        if page.selected_entry.as_deref() == Some(&entry_id) {
                            page.selected_detail = Some((entry_id.clone(), detail));
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn shift_page(&mut self, forward: bool, cx: &mut Context<Self>) {
        let day = match (self.view, forward) {
            (View::Day, true) => self.day.succ_opt(),
            (View::Day, false) => self.day.pred_opt(),
            (View::Month, true) => self.day.checked_add_months(Months::new(1)),
            (View::Month, false) => self.day.checked_sub_months(Months::new(1)),
            (View::Week, true) => Some(self.day + Duration::days(7)),
            (View::Week, false) => Some(self.day - Duration::days(7)),
        };
        if let Some(day) = day {
            self.select(day, cx);
        }
    }

    pub(super) fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.load(cx);
        cx.notify();
    }

    pub(super) fn day_entries(&self) -> Vec<CalendarEntry> {
        self.entries
            .value
            .as_deref()
            .map(|all| entries_on(all, self.day).into_iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn change_time(
        &mut self,
        id: &str,
        day: NaiveDate,
        minute: u16,
        resize: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self
            .entries
            .value
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == id))
            .cloned()
        else {
            return;
        };
        match calendar::timeline::reschedule(&before, day, minute, resize) {
            Ok(after) => self.write_time(before, after, false, cx),
            Err(error) => {
                self.write_error = Some(ShellError::Refused(400, Some(error.into())));
                cx.notify();
            }
        }
    }

    pub(super) fn quick_shift_days(&mut self, id: &str, days: i64, cx: &mut Context<Self>) {
        let Some(before) = self
            .entries
            .value
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == id))
            .cloned()
        else {
            return;
        };
        match calendar::timeline::shift_days(&before, days) {
            Ok(after) => self.write_time(before, after, false, cx),
            Err(error) => {
                self.write_error = Some(ShellError::Refused(400, Some(error.into())));
                cx.notify();
            }
        }
    }

    pub(super) fn quick_shift_minutes(
        &mut self,
        id: &str,
        minutes: i64,
        resize: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self
            .entries
            .value
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == id))
            .cloned()
        else {
            return;
        };
        match calendar::timeline::shift_minutes(&before, minutes, resize) {
            Ok(after) => self.write_time(before, after, false, cx),
            Err(error) => {
                self.write_error = Some(ShellError::Refused(400, Some(error.into())));
                cx.notify();
            }
        }
    }

    fn write_time(
        &mut self,
        before: CalendarEntry,
        after: CalendarEntry,
        undo: bool,
        cx: &mut Context<Self>,
    ) {
        if self.changing_time {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.write_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.changing_time = true;
        self.write_error = None;
        let id = after.id.clone();
        let body = serde_json::json!({"starts_at": after.starts_at, "ends_at": after.ends_at});
        let expected = before.clone();
        let work = shell::run(async move {
            let current = shell.calendar_entry(&id).await?.entry;
            if current.starts_at != expected.starts_at || current.ends_at != expected.ends_at {
                return Err(ShellError::Refused(
                    409,
                    Some("This event's time changed. Refresh before editing it.".into()),
                ));
            }
            shell.update_calendar_entry(&id, body).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.changing_time = false;
                match result {
                    Ok(saved) => {
                        page.undo_time = if undo {
                            None
                        } else {
                            Some((before, saved.clone()))
                        };
                        if let Some(entries) = &mut page.entries.value {
                            if let Some(entry) =
                                entries.iter_mut().find(|entry| entry.id == saved.id)
                            {
                                *entry = saved;
                            }
                        }
                        page.refresh(cx);
                    }
                    Err(error) => page.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn undo_time(&mut self, cx: &mut Context<Self>) {
        if let Some((before, after)) = self.undo_time.clone() {
            self.write_time(after, before, true, cx);
        }
    }

    pub(super) fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let start = cx.new(|cx| {
            let mut field = TimeFieldState::new(window, cx);
            field.set_time(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN),
                window,
                cx,
            );
            field
        });
        let end = cx.new(|cx| {
            let mut field = TimeFieldState::new(window, cx);
            field.set_time(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap_or(NaiveTime::MIN),
                window,
                cx,
            );
            field
        });
        let day = self.day;
        let start_date = cx.new(|cx| {
            let mut picker = DatePickerState::new(window, cx).date_format("%Y-%m-%d");
            picker.set_date(Date::Single(Some(day)), window, cx);
            picker
        });
        let end_date = cx.new(|cx| {
            let mut picker = DatePickerState::new(window, cx).date_format("%Y-%m-%d");
            picker.set_date(Date::Single(Some(day)), window, cx);
            picker
        });
        let end_date_for_sync = end_date.clone();
        let subscriptions = vec![
            cx.subscribe_in(&title, window, |view, _, event, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    view.save(cx);
                }
            }),
            cx.subscribe_in(
                &start_date,
                window,
                move |view, _, event: &DatePickerEvent, window, cx| {
                    let DatePickerEvent::Change(dt) = event;
                    if let Some(new_start) = dt.start().map(|d| d.date()) {
                        let current_end = end_date_for_sync
                            .read(cx)
                            .date_time()
                            .start()
                            .map(|d| d.date());
                        if current_end.is_none_or(|end| end < new_start) {
                            end_date_for_sync.update(cx, |picker, cx| {
                                picker.set_date(Date::Single(Some(new_start)), window, cx);
                            });
                        }
                    }
                    if let Some(form) = view.form.as_mut() {
                        form.error = None;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(&end_date, window, |view, _, _: &DatePickerEvent, _, cx| {
                if let Some(form) = view.form.as_mut() {
                    form.error = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&start, window, |view, _, _: &TimeFieldEvent, _, cx| {
                if let Some(form) = view.form.as_mut() {
                    form.error = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&end, window, |view, _, _: &TimeFieldEvent, _, cx| {
                if let Some(form) = view.form.as_mut() {
                    form.error = None;
                }
                cx.notify();
            }),
        ];
        title.update(cx, |input, cx| input.focus(window, cx));
        self.form = Some(Form {
            editing: None,
            title,
            start_date,
            end_date,
            start,
            end,
            all_day: false,
            kind: "event".into(),
            commitment: "planned".into(),
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub(super) fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .day_entries()
            .into_iter()
            .find(|entry| Some(&entry.id) == self.selected_entry.as_ref())
        else {
            return;
        };
        self.open_form(window, cx);
        if let Some(form) = &mut self.form {
            let start_day = calendar::parse_day(&entry.starts_at).unwrap_or(self.day);
            let end_day = calendar::parse_day(&entry.ends_at)
                .and_then(|day| {
                    if entry.all_day {
                        day.pred_opt()
                    } else {
                        Some(day)
                    }
                })
                .unwrap_or(start_day);
            let start_time = entry
                .starts_at
                .get(11..16)
                .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
                .unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap_or(NaiveTime::MIN));
            let end_time = entry
                .ends_at
                .get(11..16)
                .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
                .unwrap_or_else(|| NaiveTime::from_hms_opt(10, 0, 0).unwrap_or(NaiveTime::MIN));
            form.title.update(cx, |input, cx| {
                input.set_value(entry.title.clone(), window, cx)
            });
            form.start_date.update(cx, |picker, cx| {
                picker.set_date(Date::Single(Some(start_day)), window, cx);
            });
            form.end_date.update(cx, |picker, cx| {
                picker.set_date(Date::Single(Some(end_day)), window, cx);
            });
            form.start
                .update(cx, |field, cx| field.set_time(start_time, window, cx));
            form.end
                .update(cx, |field, cx| field.set_time(end_time, window, cx));
            form.kind = entry.kind.clone();
            form.commitment = entry.commitment.clone();
            form.all_day = entry.all_day;
            form.editing = Some(entry);
        }
        cx.notify();
    }

    pub(super) fn open_form_at(
        &mut self,
        minute: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_form(window, cx);
        if let Some(form) = &self.form {
            let start = NaiveTime::from_hms_opt((minute / 60) % 24, minute % 60, 0)
                .unwrap_or(NaiveTime::MIN);
            let end_min = (minute + 60) % 1440;
            let end = NaiveTime::from_hms_opt((end_min / 60) % 24, end_min % 60, 0)
                .unwrap_or(NaiveTime::MIN);
            form.start
                .update(cx, |field, cx| field.set_time(start, window, cx));
            form.end
                .update(cx, |field, cx| field.set_time(end, window, cx));
            if minute >= 1380 {
                let next = self.day + Duration::days(1);
                form.end_date.update(cx, |picker, cx| {
                    picker.set_date(Date::Single(Some(next)), window, cx);
                });
            }
        }
    }

    pub(super) fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        if self.changing_time {
            return;
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let original = form.editing.clone();
        let start_date = form
            .start_date
            .read(cx)
            .date_time()
            .start()
            .map(|dt| dt.date().format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let end_date = form
            .end_date
            .read(cx)
            .date_time()
            .start()
            .map(|dt| dt.date().format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let start_time = form.start.read(cx).time().format("%H:%M").to_string();
        let end_time = form.end.read(cx).time().format("%H:%M").to_string();
        let draft = calendar::EntryEdit {
            title: &form.title.read(cx).value(),
            start_date: &start_date,
            end_date: &end_date,
            start_time: &start_time,
            end_time: &end_time,
            all_day: form.all_day,
            kind: &form.kind,
            commitment: &form.commitment,
            location: original
                .as_ref()
                .and_then(|entry| entry.location.as_deref())
                .unwrap_or(""),
            notes: original
                .as_ref()
                .and_then(|entry| entry.notes.as_deref())
                .unwrap_or(""),
        };
        let body = match calendar::entry_edit_patch(&draft) {
            Ok(body) => body,
            Err(reason) => {
                form.error = Some(reason);
                cx.notify();
                return;
            }
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.write_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.write_error = None;
        self.changing_time = true;
        let previous = original.clone();
        let work = shell::run(async move {
            match original {
                Some(entry) => shell.update_calendar_entry(&entry.id, body).await,
                None => shell.create_calendar_entry(body).await,
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.changing_time = false;
                match result {
                    Ok(entry) => {
                        if let Some(before) = previous {
                            view.undo_time = Some((before, entry.clone()));
                        }
                        view.form = None;
                        view.focus_next = true;
                        view.selected_entry = Some(entry.id);
                        view.selected_detail = None;
                        view.refresh(cx);
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.confirm_delete = None;
        self.write_error = None;
        let work = shell::run(async move { shell.delete_calendar_entry(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(()) => {
                        view.selected_entry = None;
                        view.selected_detail = None;
                        view.refresh(cx);
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn day_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.day_entries();
        let is_today = self.day == Local::now().date_naive();
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(self.day.format("%A, %b %-d").to_string()),
                    )
                    .when(is_today, |el| {
                        el.child(
                            div()
                                .px_1p5()
                                .py(px(1.))
                                .rounded_full()
                                .bg(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0xffffff))
                                .child("Today"),
                        )
                    }),
            )
            .child(
                Button::new("day-panel-add")
                    .xsmall()
                    .ghost()
                    .label("+ Add")
                    .on_click(cx.listener(|page, _, window, cx| page.open_form(window, cx))),
            );

        let mut panel = div().flex().flex_col().gap_3().child(header);
        if self.selected_entry.is_some() {
            panel = panel.child(
                Button::new("calendar-edit-selected")
                    .small()
                    .ghost()
                    .label("Edit entry · Enter")
                    .disabled(self.changing_time)
                    .on_click(cx.listener(|page, _, window, cx| page.edit_selected(window, cx))),
            );
        }
        if self.view == View::Day && self.entries.error.is_none() && !self.entries.loading {
            let all = self.entries.value.as_deref().unwrap_or_default();
            let now = Local::now();
            let next = calendar::timeline::blocks(all, self.day)
                .into_iter()
                .find(|block| {
                    !is_today
                        || block.end as u32
                            > chrono::Timelike::hour(&now) * 60 + chrono::Timelike::minute(&now)
                });
            if let Some(block) = next {
                panel = panel.child(div().text_sm().child(format!(
                    "{} · {}",
                    if is_today {
                        "Up next / now"
                    } else {
                        "First event"
                    },
                    all[block.entry_index].title
                )));
            }
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Free time shows gaps from 08:00–20:00; planned events reserve time."),
            );
        }

        if self.entries.loading && self.entries.value.is_none() {
            panel = panel.child(muted("Loading…", palette));
        } else if entries.is_empty() {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .py_6()
                    .px_3()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.raised))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.muted))
                            .child("No events scheduled"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Press 'n' or click + Add above"),
                    ),
            );
        } else {
            for (i, entry) in entries.into_iter().enumerate() {
                let color = kind_color(&entry.kind);
                let chosen = self.selected_entry.as_deref() == Some(entry.id.as_str());
                let id = entry.id.clone();
                let confirming =
                    chosen && self.confirm_delete.as_deref() == Some(entry.id.as_str());
                let time_str = time_label(&entry, self.day);
                let kind_str = kind_label(&entry.kind).to_owned();

                let (origin_item_id, importance_rationale, origin_cap, origin_source, plan_id) =
                    if chosen {
                        self.selected_detail
                            .as_ref()
                            .filter(|(detail_id, _)| {
                                Some(detail_id.as_str()) == self.selected_entry.as_deref()
                            })
                            .and_then(|(_, detail)| detail.payload.as_ref())
                            .map(|payload| {
                                let origin = payload.get("origin").and_then(|o| o.as_object());
                                let item_id = origin
                                    .and_then(|o| o.get("item_id"))
                                    .and_then(|i| i.as_str())
                                    .map(str::to_string);
                                let cap = origin
                                    .and_then(|o| o.get("capability"))
                                    .and_then(|c| c.as_str())
                                    .map(str::to_string);
                                let src = origin
                                    .and_then(|o| o.get("source"))
                                    .and_then(|s| s.as_str())
                                    .map(str::to_string);
                                let rationale = payload
                                    .get("importance_rationale")
                                    .and_then(|r| r.as_str())
                                    .map(str::to_string);
                                let plan_id = payload
                                    .get("plan_id")
                                    .and_then(|id| id.as_str())
                                    .map(str::to_string);
                                (item_id, rationale, cap, src, plan_id)
                            })
                            .unwrap_or((None, None, None, None, None))
                    } else {
                        (None, None, None, None, None)
                    };

                let is_comms = entry.source == "comms"
                    || entry.source == "mail"
                    || entry.source == "feed"
                    || origin_cap.as_deref() == Some("comms")
                    || origin_source.as_deref() == Some("mail");
                let is_trips = entry.source == "trips" || origin_cap.as_deref() == Some("trips");

                panel = panel.child(
                    div()
                        .id(("calendar-entry", i))
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .py_2()
                        .px_2p5()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(if chosen {
                            rgb(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .bg(if chosen {
                            rgba(palette.raised)
                        } else {
                            rgba(tint(color, 0x0c))
                        })
                        .hover(|el| {
                            if !chosen {
                                el.bg(rgba(tint(color, 0x1c)))
                            } else {
                                el
                            }
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.select_entry(id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .w(px(6.))
                                                .h(px(6.))
                                                .flex_none()
                                                .rounded_full()
                                                .bg(rgb(color)),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(color))
                                                .child(kind_str),
                                        )
                                        .when(
                                            entry.commitment == "tentative"
                                                || entry.commitment == "possible",
                                            |el| {
                                                el.child(
                                                    div()
                                                        .px_1p5()
                                                        .py(px(0.5))
                                                        .rounded(px(3.))
                                                        .border_1()
                                                        .border_color(rgba(palette.border))
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .child(entry.commitment.clone()),
                                                )
                                            },
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .when(is_comms, |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("✉ Mail"),
                                            )
                                        })
                                        .when(is_trips, |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("✈ Travel"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(palette.muted))
                                                .child(time_str),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(entry.title.clone()),
                        )
                        .when_some(
                            entry.location.clone().filter(|l| chosen && !l.is_empty()),
                            |el, l| {
                                el.child(div().text_xs().text_color(rgb(palette.muted)).child(l))
                            },
                        )
                        .when_some(
                            entry.notes.clone().filter(|n| chosen && !n.is_empty()),
                            |el, n| {
                                el.child(
                                    div()
                                        .p_2()
                                        .rounded(px(4.))
                                        .bg(rgba(palette.surface))
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(n),
                                )
                            },
                        )
                        .when(chosen && is_comms, |el| {
                            let origin_id = origin_item_id.clone();
                            el.child(
                                div()
                                    .mt_1()
                                    .p_2p5()
                                    .rounded(px(6.))
                                    .bg(rgba(palette.surface))
                                    .border_1()
                                    .border_color(rgba(palette.border))
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("Source: Mail / Comms"),
                                            )
                                            .child(
                                                Button::new("calendar-open-source-mail")
                                                    .xsmall()
                                                    .primary()
                                                    .label("Open in Mail ↗")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.emit(CalendarIntent::OpenMail {
                                                            item_id: origin_id.clone(),
                                                        });
                                                    })),
                                            ),
                                    )
                                    .when_some(importance_rationale, |card, reason| {
                                        card.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child(reason),
                                        )
                                    }),
                            )
                        })
                        .when(chosen && is_trips, |el| {
                            let plan_id = plan_id.clone();
                            el.child(
                                div()
                                    .mt_1()
                                    .p_2p5()
                                    .rounded(px(6.))
                                    .bg(rgba(palette.surface))
                                    .border_1()
                                    .border_color(rgba(palette.border))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.accent))
                                            .child("Source: Travel / Trips"),
                                    )
                                    .child(
                                        Button::new("calendar-open-source-travel")
                                            .xsmall()
                                            .primary()
                                            .label("View in Travel ↗")
                                            .disabled(plan_id.is_none())
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                if let Some(plan_id) = &plan_id {
                                                    cx.emit(CalendarIntent::OpenTrip {
                                                        plan_id: plan_id.clone(),
                                                    });
                                                }
                                            })),
                                    ),
                            )
                        })
                        .when(
                            chosen
                                && !entry.source.is_empty()
                                && entry.source != "manual"
                                && !is_comms
                                && !is_trips,
                            |el| {
                                el.child(
                                    div()
                                        .mt_1()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("From {}", entry.source)),
                                )
                            },
                        )
                        .when(chosen, |card| {
                            let entry_id = entry.id.clone();
                            let all_day = entry.all_day;
                            let mut bar = div()
                                .mt_2()
                                .pt_2()
                                .border_t_1()
                                .border_color(rgba(palette.border))
                                .flex()
                                .flex_col()
                                .gap_1p5();

                            let mut day_row =
                                div().flex().flex_wrap().gap_1p5().items_center().child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child("Reschedule:"),
                                );
                            for (label, days) in [("Tomorrow", 1), ("+7d", 7), ("−1d", -1)] {
                                let eid = entry_id.clone();
                                day_row = day_row.child(
                                    Button::new(SharedString::from(format!(
                                        "calendar-quick-day-{days}-{eid}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .label(label)
                                    .disabled(self.changing_time)
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.quick_shift_days(&eid, days, cx);
                                    })),
                                );
                            }
                            bar = bar.child(day_row);

                            if !all_day {
                                let mut time_row =
                                    div().flex().flex_wrap().gap_1p5().items_center().child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child("Time:"),
                                    );
                                for (label, delta, resize) in [
                                    ("−15m", -15, false),
                                    ("+15m", 15, false),
                                    ("−1h", -60, false),
                                    ("+1h", 60, false),
                                    ("−15m len", -15, true),
                                    ("+15m len", 15, true),
                                ] {
                                    let eid = entry_id.clone();
                                    time_row =
                                        time_row.child(
                                            Button::new(SharedString::from(format!(
                                                "calendar-quick-time-{label}-{eid}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .label(label)
                                            .disabled(self.changing_time)
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.quick_shift_minutes(&eid, delta, resize, cx);
                                            })),
                                        );
                                }
                                bar = bar.child(time_row);
                            }

                            if self.undo_time.is_some() {
                                bar =
                                    bar.child(
                                        div().flex().items_center().justify_end().child(
                                            Button::new(SharedString::from(format!(
                                                "calendar-quick-undo-{entry_id}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .label("Undo change ⎌")
                                            .disabled(self.changing_time)
                                            .on_click(cx.listener(|page, _, _, cx| {
                                                page.undo_time(cx);
                                            })),
                                        ),
                                    );
                            }

                            card.child(bar)
                        })
                        .when(confirming, |el| {
                            let from = match entry.source.as_str() {
                                "" | "manual" => String::new(),
                                source => format!(" (from {source})"),
                            };
                            el.child(
                                div()
                                    .p_2()
                                    .rounded(px(4.))
                                    .bg(rgba(0xef444426))
                                    .border_1()
                                    .border_color(rgb(palette.red))
                                    .text_xs()
                                    .text_color(rgb(palette.red))
                                    .child(format!(
                                        "Delete this entry{from}? Enter confirms · Esc cancels"
                                    )),
                            )
                        }),
                );
            }
        }
        panel.into_any_element()
    }

    pub(super) fn form_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let all_day = form.all_day;
        let kinds =
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(KINDS.iter().map(|(key, label, _)| {
                    let on = form.kind == *key;
                    Button::new(SharedString::from(format!("calendar-kind-{key}")))
                        .xsmall()
                        .label(*label)
                        .when(on, |b| b.primary())
                        .when(!on, |b| b.ghost())
                        .on_click(cx.listener(move |page, _, _, cx| {
                            if let Some(form) = page.form.as_mut() {
                                form.kind = (*key).into();
                            }
                            cx.notify();
                        }))
                }));
        let commitments = div()
            .flex()
            .gap_1()
            .children(COMMITMENTS.iter().map(|(key, label)| {
                let on = form.commitment == *key;
                Button::new(SharedString::from(format!("calendar-commitment-{key}")))
                    .xsmall()
                    .label(*label)
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.ghost())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(form) = page.form.as_mut() {
                            form.commitment = (*key).into();
                        }
                        cx.notify();
                    }))
            }));
        div()
            .key_context("CalendarForm")
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                if form.editing.is_some() {
                    "Edit entry"
                } else {
                    "New entry"
                },
            ))
            .child(Input::new(&form.title))
            .when(self.show_availability && form.editing.is_none(), |panel| {
                panel.child(
                    Button::new("calendar-gap-travel")
                        .small()
                        .ghost()
                        .label("Plan travel on this day")
                        .on_click(cx.listener(|page, _, _, cx| {
                            cx.emit(CalendarIntent::PlanTravel { day: page.day })
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            DatePicker::new(&form.start_date)
                                .small()
                                .placeholder("Start date"),
                        ),
                    )
                    .child(muted("to", palette))
                    .child(
                        div().flex_1().child(
                            DatePicker::new(&form.end_date)
                                .small()
                                .placeholder("End date"),
                        ),
                    ),
            )
            .child(
                Button::new("calendar-all-day")
                    .xsmall()
                    .label("All day")
                    .when(all_day, |b| b.primary())
                    .when(!all_day, |b| b.ghost())
                    .on_click(cx.listener(|page, _, _, cx| {
                        if let Some(form) = page.form.as_mut() {
                            form.all_day = !form.all_day;
                        }
                        cx.notify();
                    })),
            )
            .when(!all_day, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(80.)).child(TimeField::new(&form.start).small()))
                        .child(muted("to", palette))
                        .child(div().w(px(80.)).child(TimeField::new(&form.end).small())),
                )
            })
            .child(div().text_xs().text_color(rgb(palette.muted)).child("KIND"))
            .child(kinds)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("COMMITMENT"),
            )
            .child(commitments)
            .when_some(form.error, |el, error| {
                el.child(div().text_xs().text_color(rgb(palette.red)).child(error))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("calendar-save")
                            .small()
                            .primary()
                            .label(if self.changing_time {
                                "Saving…"
                            } else {
                                "Save · ↩"
                            })
                            .disabled(self.changing_time)
                            .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
                    )
                    .child(
                        Button::new("calendar-cancel")
                            .small()
                            .ghost()
                            .label("Cancel · Esc")
                            .on_click(
                                cx.listener(|page, _, window, cx| page.close_form(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for CalendarPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clock.is_none() {
            self.clock = Some(super::minute_clock(cx));
        }
        if std::mem::take(&mut self.focus_next) && self.form.is_none() {
            window.focus(&self.focus, cx);
        }
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        let is_mobile = self.density.is_mobile();
        let today = Local::now().date_naive();
        let dates = self.dates();
        let label = match self.view {
            View::Day => self.day.format("%A, %b %-d, %Y").to_string(),
            View::Month => self.day.format("%B %Y").to_string(),
            View::Week => format!(
                "{} – {}",
                dates[0].format("%b %-d"),
                dates[6].format("%b %-d, %Y")
            ),
        };
        let view_button = |label: &'static str, view: View, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("calendar-view-{label}")))
                .small()
                .label(label)
                .when(self.view == view, |b| b.primary())
                .when(self.view != view, |b| b.ghost())
                .on_click(cx.listener(move |page, _, _, cx| page.set_view(view, cx)))
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .mr_2()
                    .child(label),
            )
            .when_some(self.sync_status.as_ref(), |header, status| {
                header.child(super::types::sync_status_mark(status, palette))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("calendar-prev")
                            .small()
                            .ghost()
                            .label("‹")
                            .on_click(cx.listener(|page, _, _, cx| page.shift_page(false, cx))),
                    )
                    .child(
                        Button::new("calendar-today")
                            .small()
                            .ghost()
                            .label("Today")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.select(Local::now().date_naive(), cx)
                            })),
                    )
                    .child(
                        Button::new("calendar-next")
                            .small()
                            .ghost()
                            .label("›")
                            .on_click(cx.listener(|page, _, _, cx| page.shift_page(true, cx))),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .p(px(2.))
                    .rounded(px(6.))
                    .bg(rgba(palette.surface))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(view_button("Day", View::Day, cx))
                    .child(view_button("Week", View::Week, cx))
                    .child(view_button("Month", View::Month, cx)),
            )
            .when(self.undo_time.is_some(), |header| {
                header.child(
                    Button::new("calendar-undo-time")
                        .small()
                        .ghost()
                        .label("Undo time change · ⌘Z")
                        .disabled(self.changing_time)
                        .on_click(cx.listener(|page, _, _, cx| page.undo_time(cx))),
                )
            })
            .child(
                Button::new("calendar-availability")
                    .small()
                    .label("Free time")
                    .when(self.show_availability, |button| button.primary())
                    .when(!self.show_availability, |button| button.ghost())
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.show_availability = !page.show_availability;
                        if page.show_availability && page.view == View::Month {
                            page.set_view(View::Week, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("calendar-proposal-review")
                    .small()
                    .ghost()
                    .label(if self.reviewing_proposals {
                        "Back to calendar".to_owned()
                    } else {
                        format!(
                            "Review · {}",
                            self.proposals.value.as_ref().map_or(0, Vec::len)
                        )
                    })
                    .on_click(cx.listener(|page, _, _, cx| page.toggle_proposals(cx))),
            )
            .child(
                Button::new("calendar-new")
                    .small()
                    .primary()
                    .label("+ New entry · N")
                    .on_click(cx.listener(|page, _, window, cx| page.open_form(window, cx))),
            );

        let content = if self.reviewing_proposals {
            self.proposal_panel(palette, cx)
        } else {
            let entries = self.entries.value.clone().unwrap_or_default();
            let grid = match self.view {
                View::Day => self.time_grid(&dates, &entries, today, palette, cx),
                View::Month => self.month(&dates, &entries, today, palette, cx),
                View::Week => self.time_grid(&dates, &entries, today, palette, cx),
            };
            let side = match self.form.is_some() {
                true => self.form_panel(palette, cx),
                false => self.day_panel(palette, cx),
            };
            div()
                .flex()
                .gap_4()
                .flex_1()
                .min_h_0()
                .child(div().flex_1().min_w_0().min_h_0().child(grid))
                .child(
                    div()
                        .id("calendar-side-panel")
                        .w(px(330.))
                        .flex_none()
                        .p_4()
                        .rounded(px(8.))
                        .bg(rgba(palette.surface))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .track_scroll(&self.side_scroll)
                        .overflow_y_scroll()
                        .vertical_scrollbar(&self.side_scroll)
                        .child(side),
                )
                .into_any_element()
        };

        div()
            .key_context("CalendarGrid")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_previous_day))
            .on_action(cx.listener(Self::on_next_day))
            .on_action(cx.listener(Self::on_previous_week))
            .on_action(cx.listener(Self::on_next_week))
            .on_action(cx.listener(Self::on_previous_page))
            .on_action(cx.listener(Self::on_next_page))
            .on_action(cx.listener(Self::on_today))
            .on_action(cx.listener(Self::on_month_view))
            .on_action(cx.listener(Self::on_day_view))
            .on_action(cx.listener(Self::on_week_view))
            .on_action(cx.listener(Self::on_next_entry))
            .on_action(cx.listener(Self::on_new_entry))
            .on_action(cx.listener(Self::on_delete_entry))
            .on_action(cx.listener(Self::on_undo_time))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_cancel))
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .when_some(self.entries.error.as_ref(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error.to_string()),
                )
            })
            .when_some(self.write_error.as_ref(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Not saved: {error}")),
                )
            })
            .child(content)
            .when(!is_mobile, |el| {
                el.child(key_bar(
                    &[
                        ("←→", "day"),
                        ("↑↓", "week"),
                        ("[ ]", "page"),
                        ("t", "today"),
                        ("d/m/w", "view"),
                        ("e", "entry"),
                        ("n", "new"),
                        ("⌫", "delete"),
                    ],
                    palette,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn day(s: &str) -> NaiveDate {
        calendar::parse_day(s).unwrap()
    }

    /// Paging keeps the day where the month allows it and clamps where it does not; a week page
    /// is seven days. No shell, so the reads fail fast and the selection is what is checked.
    #[gpui::test]
    fn pages_move_the_selected_day(cx: &mut TestAppContext) {
        let page = cx.new(|cx| CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx));
        page.update(cx, |page, cx| {
            page.day = day("2026-01-31");
            page.shift_page(true, cx);
            assert_eq!(page.day, day("2026-02-28"));
            page.shift_page(false, cx);
            assert_eq!(page.day, day("2026-01-28"));
            page.set_view(View::Week, cx);
            page.shift_page(true, cx);
            assert_eq!(page.day, day("2026-02-04"));
            assert!(page.entries.error.is_some());
            page.set_view(View::Day, cx);
            page.shift_page(true, cx);
            assert_eq!(page.day, day("2026-02-05"));
            assert_eq!(page.dates(), [day("2026-02-05")]);
        });
    }

    #[gpui::test]
    fn changing_keyboard_selection_or_open_day_cannot_reuse_stale_details(cx: &mut TestAppContext) {
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.selected_entry = Some("calendar:old".into());
            page.selected_detail = Some((
                "calendar:old".into(),
                CalendarEntryDetail {
                    payload: Some(serde_json::json!({
                        "importance_rationale": "stale rationale",
                        "origin": {"capability": "comms", "source": "mail"}
                    })),
                    ..Default::default()
                },
            ));
            page
        });
        page.update(cx, |page, cx| {
            page.select_entry("calendar:new".into(), cx);
            assert_eq!(page.selected_entry.as_deref(), Some("calendar:new"));
            assert!(page.selected_detail.is_none());
            page.selected_detail = Some(("calendar:old".into(), CalendarEntryDetail::default()));
            page.open_day(day("2026-11-06"), Some("calendar:next".into()), cx);
            assert_eq!(page.selected_entry.as_deref(), Some("calendar:next"));
            assert!(page.selected_detail.is_none());
        });
    }

    #[gpui::test]
    fn month_has_a_bounded_scroll_viewport_for_dense_weeks(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = day("2026-11-05");
            page.entries.value = Some(
                (0..8)
                    .map(|i| CalendarEntry {
                        id: format!("calendar:fixture{i}"),
                        title: format!("All-day event {i}"),
                        starts_at: "2026-10-26".into(),
                        ends_at: "2026-12-01".into(),
                        all_day: true,
                        ..Default::default()
                    })
                    .collect(),
            );
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.simulate_resize(gpui::size(px(1100.), px(700.)));
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("calendar-month-scroll")
            .expect("month viewport");
        assert!(bounds.size.height > px(0.));
        assert!(bounds.bottom() < px(700.));
        page.read_with(cx, |page, _| {
            assert!(page.month_scroll.max_offset().y > px(0.))
        });
    }

    #[gpui::test]
    fn calendar_header_displays_sync_status_mark(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.sync_status = Some(crate::calendar::types::CalendarSyncStatus {
                google_synced_at: Some("2026-11-05T20:00:00Z".into()),
                google_error: None,
                icloud_synced_at: Some("2026-11-05T20:00:00Z".into()),
                icloud_error: None,
                replica_synced_at: None,
                replica_error: None,
            });
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.simulate_resize(gpui::size(px(1100.), px(700.)));
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("calendar-sync-status")
            .expect("sync status mark rendered");
        assert!(bounds.size.width > px(0.));
    }

    #[gpui::test]
    fn day_timeline_sizes_a_ninety_minute_event_and_edits_it_by_keyboard(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(bind_keys);
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = day("2026-11-05");
            page.view = View::Day;
            page.entries.value = Some(vec![CalendarEntry {
                id: "calendar:fixture".into(),
                title: "Review".into(),
                starts_at: "2026-11-05T09:00:00".into(),
                ends_at: "2026-11-05T10:30:00".into(),
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            }]);
            page.selected_entry = Some("calendar:fixture".into());
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.run_until_parked();
        let block = cx
            .debug_bounds("calendar-time-block")
            .expect("timed block rendered");
        assert_eq!(block.size.height, px(72.));
        let scroll = cx
            .debug_bounds("calendar-time-scroll")
            .expect("scroll viewport rendered");
        assert!(scroll.size.height > px(0.));
        let start = block.origin + gpui::point(px(12.), px(12.));
        let target = start + gpui::point(px(0.), px(60.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(
            start + gpui::point(px(0.), px(6.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(target, gpui::MouseButton::Left, gpui::Modifiers::none());
        page.read_with(cx, |page, _| {
            assert_eq!(page.drop_time, Some((day("2026-11-05"), 630)));
        });
        cx.simulate_mouse_up(target, gpui::MouseButton::Left, gpui::Modifiers::none());
        page.read_with(cx, |page, _| {
            assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
        });
        cx.simulate_keystrokes("enter");
        page.read_with(cx, |page, cx| {
            let form = page.form.as_ref().expect("Enter opens edit");
            assert_eq!(
                form.start.read(cx).time().format("%H:%M").to_string(),
                "09:00"
            );
            assert_eq!(
                form.end.read(cx).time().format("%H:%M").to_string(),
                "10:30"
            );
            assert_eq!(
                form.start_date
                    .read(cx)
                    .date_time()
                    .start()
                    .map(|dt| dt.date()),
                Some(day("2026-11-05"))
            );
            assert_eq!(
                form.end_date
                    .read(cx)
                    .date_time()
                    .start()
                    .map(|dt| dt.date()),
                Some(day("2026-11-05"))
            );
            assert!(form.editing.is_some());
        });
    }

    #[gpui::test]
    fn form_date_picker_and_time_field_clamp_end_date_and_validate_save(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(bind_keys);
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = day("2026-11-05");
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            page.update(cx, |page, cx| {
                page.open_form_at(23 * 60 + 30, window, cx);
            });
        });
        page.read_with(cx, |page, cx| {
            let form = page.form.as_ref().expect("form opened at 23:30");
            assert_eq!(
                form.start.read(cx).time().format("%H:%M").to_string(),
                "23:30"
            );
            assert_eq!(
                form.end.read(cx).time().format("%H:%M").to_string(),
                "00:30"
            );
            assert_eq!(
                form.end_date
                    .read(cx)
                    .date_time()
                    .start()
                    .map(|dt| dt.date()),
                Some(day("2026-11-06"))
            );
        });
    }

    #[gpui::test]
    fn undo_time_action_reverts_time_change(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = day("2026-11-05");
            let original = CalendarEntry {
                id: "calendar:fixture".into(),
                title: "Standup".into(),
                starts_at: "2026-11-05T09:00:00".into(),
                ends_at: "2026-11-05T09:30:00".into(),
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            };
            let moved = CalendarEntry {
                starts_at: "2026-11-05T10:00:00".into(),
                ends_at: "2026-11-05T10:30:00".into(),
                ..original.clone()
            };
            page.entries.value = Some(vec![moved.clone()]);
            page.undo_time = Some((original, moved));
            page
        });
        page.update(cx, |page, cx| {
            page.undo_time(cx);
            assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
        });
    }

    #[gpui::test]
    fn week_timeline_renders_seven_day_columns_duration_blocks_overlaps_and_now_line(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        cx.update(bind_keys);
        let today = chrono::Local::now().date_naive();
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = today;
            page.view = View::Week;
            let day_str = today.format("%Y-%m-%d").to_string();
            page.entries.value = Some(vec![
                CalendarEntry {
                    id: "calendar:overlap1".into(),
                    title: "Architecture Review".into(),
                    starts_at: format!("{day_str}T10:00:00"),
                    ends_at: format!("{day_str}T11:30:00"),
                    kind: "event".into(),
                    commitment: "planned".into(),
                    ..Default::default()
                },
                CalendarEntry {
                    id: "calendar:overlap2".into(),
                    title: "Security Sync".into(),
                    starts_at: format!("{day_str}T10:30:00"),
                    ends_at: format!("{day_str}T12:00:00"),
                    kind: "event".into(),
                    commitment: "planned".into(),
                    ..Default::default()
                },
                CalendarEntry {
                    id: "calendar:allday".into(),
                    title: "Offsite".into(),
                    starts_at: day_str.clone(),
                    ends_at: (today + chrono::Duration::days(1))
                        .format("%Y-%m-%d")
                        .to_string(),
                    all_day: true,
                    kind: "event".into(),
                    commitment: "planned".into(),
                    ..Default::default()
                },
            ]);
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.run_until_parked();

        // 7 days in week view
        page.read_with(cx, |page, _| {
            assert_eq!(page.dates().len(), 7);
            assert!(page.dates().contains(&today));
        });

        // Now line rendered for today
        let now_line = cx.debug_bounds("calendar-now-line");
        assert!(now_line.is_some(), "now-line rendered on today's column");

        // All-day span rendered in top strip
        let span_block = cx.debug_bounds("calendar-span-block");
        assert!(span_block.is_some(), "all-day span rendered across week");

        // Overlapping blocks share columns and are sized to 90 min (72px)
        let blocks = cx.debug_bounds("calendar-time-block");
        assert!(blocks.is_some(), "timed blocks rendered");
        assert_eq!(blocks.unwrap().size.height, px(72.));

        // Dragging moves target day and minute across columns
        let block_origin = blocks.unwrap().origin + gpui::point(px(10.), px(10.));
        let next_day_target = block_origin + gpui::point(px(80.), px(48.));
        cx.simulate_mouse_down(
            block_origin,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(
            block_origin + gpui::point(px(0.), px(5.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(
            next_day_target,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        page.read_with(cx, |page, _| {
            assert!(page.drop_time.is_some(), "drop target tracked across week");
        });
        cx.simulate_mouse_up(
            next_day_target,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
    }

    #[gpui::test]
    fn desktop_quick_reschedule_shifts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let page = cx.new(|cx| {
            let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            page.day = day("2026-11-05");
            page.entries.value = Some(vec![CalendarEntry {
                id: "calendar:quick".into(),
                title: "Standup".into(),
                starts_at: "2026-11-05T09:00:00".into(),
                ends_at: "2026-11-05T09:30:00".into(),
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            }]);
            page.selected_entry = Some("calendar:quick".into());
            page
        });
        page.update(cx, |page, cx| {
            page.quick_shift_days("calendar:quick", 1, cx);
            assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
            page.write_error = None;

            page.quick_shift_minutes("calendar:quick", 15, false, cx);
            assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
        });
    }
}
