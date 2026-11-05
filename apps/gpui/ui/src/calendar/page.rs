//! Desktop CalendarPage view, month/week navigation, side panel and day entries.
use chrono::{Duration, Local, Months, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FocusHandle, FontWeight, ScrollHandle,
    SharedString, Window,
};
use sjel_gpui_core::calendar::{
    self, entries_on, kind_color, kind_label, time_label, COMMITMENTS, KINDS,
};
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::{key_bar, tone_chip};
use crate::theme::{Density, Palette, ThemeContext};

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
    pub(super) confirm_delete: Option<String>,
    pub(super) form: Option<Form>,
    pub write_error: Option<ShellError>,
    side_scroll: ScrollHandle,
    pub(super) timeline_scroll: ScrollHandle,
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
            confirm_delete: None,
            form: None,
            write_error: None,
            side_scroll: ScrollHandle::new(),
            timeline_scroll,
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
        self.side_scroll.clone()
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
        self.selected_entry = entry_id;
        self.loaded = None;
        self.load(cx);
        cx.notify();
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
            self.confirm_delete = None;
        }
        self.load(cx);
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
        let field = |placeholder: &'static str,
                     value: &str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            })
        };
        let title = field("Title", "", window, cx);
        let start = field("09:00", "09:00", window, cx);
        let end = field("10:00", "10:00", window, cx);
        let date = self.day.format("%Y-%m-%d").to_string();
        let start_date = field("Start date", &date, window, cx);
        let end_date = field("End date", &date, window, cx);
        let subscriptions = [&title, &start, &end, &start_date, &end_date]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |view, _, event, _, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        view.save(cx);
                    }
                })
            })
            .collect();
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
            let end_date = calendar::parse_day(&entry.ends_at).and_then(|day| {
                if entry.all_day {
                    day.pred_opt()
                } else {
                    Some(day)
                }
            });
            let fields = [
                (&form.title, entry.title.clone()),
                (
                    &form.start_date,
                    entry.starts_at.get(..10).unwrap_or("").to_owned(),
                ),
                (
                    &form.end_date,
                    end_date
                        .map(|day| day.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                ),
                (
                    &form.start,
                    entry.starts_at.get(11..16).unwrap_or("09:00").to_owned(),
                ),
                (
                    &form.end,
                    entry.ends_at.get(11..16).unwrap_or("10:00").to_owned(),
                ),
            ];
            for (input, value) in fields {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
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
            let start = format!("{:02}:{:02}", minute / 60, minute % 60);
            let end = (minute + 60) % 1440;
            let end = format!("{:02}:{:02}", end / 60, end % 60);
            form.start
                .update(cx, |input, cx| input.set_value(start, window, cx));
            form.end
                .update(cx, |input, cx| input.set_value(end, window, cx));
            if minute >= 1380 {
                let next = (self.day + Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string();
                form.end_date
                    .update(cx, |input, cx| input.set_value(next, window, cx));
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
        let draft = calendar::EntryEdit {
            title: &form.title.read(cx).value(),
            start_date: &form.start_date.read(cx).value(),
            end_date: &form.end_date.read(cx).value(),
            start_time: &form.start.read(cx).value(),
            end_time: &form.end.read(cx).value(),
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
                                .px_1()
                                .py(px(1.))
                                .rounded(px(3.))
                                .bg(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xffffff))
                                .child("TODAY"),
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

                panel = panel.child(
                    div()
                        .id(("calendar-entry", i))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_2()
                        .px_1()
                        .border_b_1()
                        .border_color(if chosen {
                            rgb(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .bg(if chosen {
                            rgba(palette.raised)
                        } else {
                            rgba(0x00000000)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.selected_entry = Some(id.clone());
                            page.confirm_delete = None;
                            cx.notify();
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
                                        .gap_1()
                                        .when(chosen, |el| el.child(tone_chip(kind_str, color)))
                                        .when(
                                            entry.commitment == "tentative"
                                                || entry.commitment == "possible",
                                            |el| {
                                                el.child(
                                                    div()
                                                        .px_1()
                                                        .py(px(1.))
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
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child(time_str),
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
                        .when(
                            chosen && !entry.source.is_empty() && entry.source != "manual",
                            |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("From {}", entry.source)),
                                )
                            },
                        )
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
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&form.start_date).small()))
                    .child(muted("to", palette))
                    .child(div().flex_1().child(Input::new(&form.end_date).small())),
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
                        .child(div().w(px(80.)).child(Input::new(&form.start).small()))
                        .child(muted("to", palette))
                        .child(div().w(px(80.)).child(Input::new(&form.end).small())),
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
            .child(
                Button::new("calendar-prev")
                    .small()
                    .ghost()
                    .label("Previous")
                    .on_click(cx.listener(|page, _, _, cx| page.shift_page(false, cx))),
            )
            .child(
                Button::new("calendar-today")
                    .small()
                    .ghost()
                    .label("Today")
                    .on_click(
                        cx.listener(|page, _, _, cx| page.select(Local::now().date_naive(), cx)),
                    ),
            )
            .child(
                Button::new("calendar-next")
                    .small()
                    .ghost()
                    .label("Next")
                    .on_click(cx.listener(|page, _, _, cx| page.shift_page(true, cx))),
            )
            .child(div().flex_1())
            .child(view_button("Day", View::Day, cx))
            .child(view_button("Month", View::Month, cx))
            .child(view_button("Week", View::Week, cx))
            .when(self.undo_time.is_some(), |header| {
                header.child(
                    Button::new("calendar-undo-time")
                        .small()
                        .ghost()
                        .label("Undo time change")
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
                    .label("New entry · N")
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
                .child(div().flex_1().min_w_0().child(grid))
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
            assert_eq!(form.start.read(cx).value().as_str(), "09:00");
            assert_eq!(form.end.read(cx).value().as_str(), "10:30");
            assert!(form.editing.is_some());
        });
    }
}
