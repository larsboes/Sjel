//! Desktop CalendarPage view, month/week navigation, side panel and day entries.
use super::adapter;
use crate::ely::calendar::jiff::{tz::TimeZone, Timestamp};
use chrono::{Duration, Local, Months, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, Context, FocusHandle, FontWeight, ScrollHandle, SharedString,
    Window,
};
use sjel_gpui_core::calendar::{self, entries_on};
use sjel_gpui_core::shell::{self, CalendarEntry, CalendarEntryDetail, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::key_bar;
use crate::theme::{Density, Palette, ThemeContext};

use super::types::CalendarIntent;
use super::types::{Form, View};

pub use super::actions::bind_keys;

pub struct CalendarPage {
    pub(super) repo: Repo,
    palette: Option<Palette>,
    density: Density,
    pub view: View,
    pub(super) show_availability: bool,
    pub(super) changing_time: bool,
    pub(super) zone: TimeZone,
    pub(super) form_session: u64,
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
    pub(super) focus: FocusHandle,
    pub(super) focus_next: bool,
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
            zone: TimeZone::system(),
            form_session: 0,
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

    pub(super) fn commit_time(
        &mut self,
        id: &str,
        start: Option<Timestamp>,
        end: Timestamp,
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
        let mut after = before.clone();
        if let Some(start) = start {
            after.starts_at = adapter::local(start, &self.zone);
        }
        after.ends_at = adapter::local(end, &self.zone);
        self.write_time(before, after, false, cx);
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
                View::Day => self.time_grid(&dates, &entries, cx),
                View::Month => self.month(&entries, cx),
                View::Week => self.time_grid(&dates, &entries, cx),
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
            .key_context(if self.form.is_some() {
                "CalendarEditing"
            } else {
                "CalendarGrid"
            })
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
#[path = "tests.rs"]
mod tests;
