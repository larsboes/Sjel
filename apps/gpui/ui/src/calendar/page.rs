//! The Calendar page: a month grid or a week of day columns over the calendar capability, the
//! selected day's entries beside it, and a form for a new entry.
//!
//! Grid and entry rules live in `sjel_gpui_core::calendar`; mail-backed event proposals are
//! reviewed here before they become planned entries.

use chrono::{Datelike, Duration, Local, Months, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, SharedString, Subscription, Window,
};
use sjel_gpui_core::calendar::{
    self, entries_on, kind_color, kind_label, time_label, Draft, COMMITMENTS, KINDS,
};
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

actions!(
    calendar,
    [
        /// Select the previous day.
        PreviousDay,
        /// Select the next day.
        NextDay,
        /// Select the same day a week earlier.
        PreviousWeek,
        /// Select the same day a week later.
        NextWeek,
        /// Show the previous month or week.
        PreviousPage,
        /// Show the next month or week.
        NextPage,
        /// Select today.
        Today,
        /// Show the month grid.
        MonthView,
        /// Show the week.
        WeekView,
        /// Select the next entry of the day.
        NextEntry,
        /// Add an entry on the selected day.
        NewEntry,
        /// Delete the selected entry, after confirming.
        DeleteEntry,
        /// Confirm the deletion that asked.
        Confirm,
        /// Close the form or the question.
        Cancel,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", PreviousDay, Some("CalendarGrid")),
        KeyBinding::new("h", PreviousDay, Some("CalendarGrid")),
        KeyBinding::new("right", NextDay, Some("CalendarGrid")),
        KeyBinding::new("l", NextDay, Some("CalendarGrid")),
        KeyBinding::new("up", PreviousWeek, Some("CalendarGrid")),
        KeyBinding::new("k", PreviousWeek, Some("CalendarGrid")),
        KeyBinding::new("down", NextWeek, Some("CalendarGrid")),
        KeyBinding::new("j", NextWeek, Some("CalendarGrid")),
        KeyBinding::new("[", PreviousPage, Some("CalendarGrid")),
        KeyBinding::new("]", NextPage, Some("CalendarGrid")),
        KeyBinding::new("t", Today, Some("CalendarGrid")),
        KeyBinding::new("m", MonthView, Some("CalendarGrid")),
        KeyBinding::new("w", WeekView, Some("CalendarGrid")),
        KeyBinding::new("e", NextEntry, Some("CalendarGrid")),
        KeyBinding::new("n", NewEntry, Some("CalendarGrid")),
        KeyBinding::new("backspace", DeleteEntry, Some("CalendarGrid")),
        KeyBinding::new("enter", Confirm, Some("CalendarGrid")),
        KeyBinding::new("escape", Cancel, Some("CalendarGrid")),
        KeyBinding::new("escape", Cancel, Some("CalendarForm")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Month,
    Week,
}

/// The new-entry form's fields. The text fields are inputs; the rest are buttons.
struct Form {
    title: Entity<InputState>,
    start: Entity<InputState>,
    end: Entity<InputState>,
    all_day: bool,
    kind: &'static str,
    commitment: &'static str,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

pub struct CalendarPage {
    repo: Repo,
    palette: Option<Palette>,
    pub view: View,
    /// The selected day; the grid shows its month or week.
    pub day: NaiveDate,
    pub entries: Remote<Vec<CalendarEntry>>,
    /// The `[from, to)` window `entries` answers.
    loaded: Option<(String, String)>,
    proposals: Remote<Vec<CalendarEntry>>,
    proposals_window: Option<(String, String)>,
    reviewing_proposals: bool,
    proposal_action: Option<String>,
    proposal_confirm_remove: Option<String>,
    pub selected_entry: Option<String>,
    confirm_delete: Option<String>,
    form: Option<Form>,
    pub write_error: Option<ShellError>,
    focus: FocusHandle,
    focus_next: bool,
}

impl CalendarPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            view: View::Month,
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
            focus: cx.focus_handle(),
            focus_next: true,
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

    fn dates(&self) -> Vec<NaiveDate> {
        match self.view {
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
                // A later navigation asked for another window; this answer is stale.
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

    fn load_proposals(&mut self, cx: &mut Context<Self>, force: bool) {
        if self.proposals.loading {
            return;
        }
        let from = Local::now().date_naive();
        let to = from + Duration::days(90);
        let window = (
            from.format("%Y-%m-%d").to_string(),
            to.format("%Y-%m-%d").to_string(),
        );
        if !force && self.proposals_window.as_ref() == Some(&window) {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.proposals.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.proposals_window = Some(window.clone());
        self.proposals.loading = true;
        let (from, to) = window.clone();
        let work = shell::run(async move { shell.calendar_proposals(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.proposals_window.as_ref() == Some(&window) {
                    view.proposals.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn review_proposal(&mut self, id: String, remove: bool, cx: &mut Context<Self>) {
        if remove && self.proposal_confirm_remove.as_deref() != Some(id.as_str()) {
            self.proposal_confirm_remove = Some(id);
            cx.notify();
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_action = Some(id.clone());
        self.proposal_confirm_remove = None;
        self.write_error = None;
        let entry_id = id.clone();
        let work = shell::run(async move {
            if remove {
                shell.delete_calendar_entry(&entry_id).await.map(|()| None)
            } else {
                shell
                    .update_calendar_entry(
                        &entry_id,
                        serde_json::json!({ "kind": "event", "commitment": "planned" }),
                    )
                    .await
                    .map(Some)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.proposal_action = None;
                match result {
                    Ok(_) => {
                        if let Some(proposals) = view.proposals.value.as_mut() {
                            proposals.retain(|entry| entry.id != id);
                        }
                        view.proposals_window = None;
                        view.load_proposals(cx, true);
                        view.refresh(cx);
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_proposals(&mut self, cx: &mut Context<Self>) {
        self.reviewing_proposals = !self.reviewing_proposals;
        self.proposal_confirm_remove = None;
        if self.reviewing_proposals {
            self.load_proposals(cx, false);
        }
        cx.notify();
    }

    fn proposal_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let proposals = self.proposals.value.as_deref().unwrap_or_default();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Detected event proposals"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Mail event dates appear here as possible entries. Add a proposal to put it on your Calendar."),
            );

        if let Some(error) = &self.proposals.error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        if self.proposals.loading && self.proposals.value.is_none() {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Loading proposals…"),
                )
                .into_any_element();
        }
        if proposals.is_empty() && self.proposals.error.is_none() {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Nothing pending in the next 90 days."),
                )
                .into_any_element();
        }

        for entry in proposals {
            let id = entry.id.clone();
            let remove_id = id.clone();
            let confirming = self.proposal_confirm_remove.as_deref() == Some(id.as_str());
            let busy = self.proposal_action.as_deref() == Some(id.as_str());
            let when = if entry.all_day {
                format!(
                    "{} · all day",
                    entry.starts_at.get(..10).unwrap_or(&entry.starts_at)
                )
            } else {
                format!(
                    "{}–{}",
                    entry.starts_at.get(..16).unwrap_or(&entry.starts_at),
                    entry.ends_at.get(11..16).unwrap_or("")
                )
            };
            panel =
                panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_3()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(entry.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{}{} · {}",
                                    when,
                                    entry
                                        .location
                                        .as_ref()
                                        .map(|place| format!(" · {place}"))
                                        .unwrap_or_default(),
                                    entry.source
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new(SharedString::from(format!("proposal-add-{id}")))
                                        .small()
                                        .label(if busy { "Adding…" } else { "Add to Calendar" })
                                        .disabled(self.proposal_action.is_some())
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            page.review_proposal(id.clone(), false, cx)
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "proposal-remove-{remove_id}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label(if confirming {
                                        "Confirm remove"
                                    } else {
                                        "Remove"
                                    })
                                    .disabled(self.proposal_action.is_some())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.review_proposal(remove_id.clone(), true, cx)
                                    })),
                                ),
                        ),
                );
        }
        panel.into_any_element()
    }

    fn select(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        if day != self.day {
            self.day = day;
            self.selected_entry = None;
            self.confirm_delete = None;
        }
        self.load(cx);
        cx.notify();
    }

    fn shift_page(&mut self, forward: bool, cx: &mut Context<Self>) {
        let day = match (self.view, forward) {
            (View::Month, true) => self.day.checked_add_months(Months::new(1)),
            (View::Month, false) => self.day.checked_sub_months(Months::new(1)),
            (View::Week, true) => Some(self.day + Duration::days(7)),
            (View::Week, false) => Some(self.day - Duration::days(7)),
        };
        if let Some(day) = day {
            self.select(day, cx);
        }
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.load(cx);
        cx.notify();
    }

    fn day_entries(&self) -> Vec<CalendarEntry> {
        self.entries
            .value
            .as_deref()
            .map(|all| entries_on(all, self.day).into_iter().cloned().collect())
            .unwrap_or_default()
    }

    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = |placeholder: &'static str,
                     value: &'static str,
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
        // Enter in any field saves, the way the dashboard's form submits.
        let subscriptions = [&title, &start, &end]
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
            title,
            start,
            end,
            all_day: false,
            kind: "event",
            commitment: "planned",
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let draft = Draft {
            title: form.title.read(cx).value().to_string(),
            day: self.day,
            start: form.start.read(cx).value().to_string(),
            end: form.end.read(cx).value().to_string(),
            all_day: form.all_day,
            kind: form.kind.to_owned(),
            commitment: form.commitment.to_owned(),
        };
        let body = match draft.body() {
            Ok(body) => body,
            Err(reason) => {
                form.error = Some(reason);
                cx.notify();
                return;
            }
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.write_error = None;
        let work = shell::run(async move { shell.create_calendar_entry(body).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(entry) => {
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

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
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

    fn on_previous_day(&mut self, _: &PreviousDay, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day - Duration::days(1), cx);
    }
    fn on_next_day(&mut self, _: &NextDay, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day + Duration::days(1), cx);
    }
    fn on_previous_week(&mut self, _: &PreviousWeek, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day - Duration::days(7), cx);
    }
    fn on_next_week(&mut self, _: &NextWeek, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day + Duration::days(7), cx);
    }
    fn on_previous_page(&mut self, _: &PreviousPage, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_page(false, cx);
    }
    fn on_next_page(&mut self, _: &NextPage, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_page(true, cx);
    }
    fn on_today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        self.select(Local::now().date_naive(), cx);
    }
    fn on_month_view(&mut self, _: &MonthView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Month, cx);
    }
    fn on_week_view(&mut self, _: &WeekView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Week, cx);
    }

    fn on_next_entry(&mut self, _: &NextEntry, _: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.day_entries().into_iter().map(|e| e.id).collect();
        let at = self
            .selected_entry
            .as_ref()
            .and_then(|id| ids.iter().position(|i| i == id));
        self.selected_entry = match at {
            Some(i) => ids.get((i + 1) % ids.len()).cloned(),
            None => ids.first().cloned(),
        };
        self.confirm_delete = None;
        cx.notify();
    }

    fn on_new_entry(&mut self, _: &NewEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.open_form(window, cx);
    }

    fn on_delete_entry(&mut self, _: &DeleteEntry, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_delete = self.selected_entry.clone();
        cx.notify();
    }

    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.confirm_delete.take() {
            self.delete(id, cx);
        }
    }

    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            self.close_form(window, cx);
        } else if self.confirm_delete.take().is_none() {
            self.selected_entry = None;
        }
        cx.notify();
    }
}

impl Render for CalendarPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) && self.form.is_none() {
            window.focus(&self.focus, cx);
        }
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let today = Local::now().date_naive();
        let dates = self.dates();
        let label = match self.view {
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
                    .label("‹")
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
                    .label("›")
                    .on_click(cx.listener(|page, _, _, cx| page.shift_page(true, cx))),
            )
            .child(div().flex_1())
            .child(view_button("Month", View::Month, cx))
            .child(view_button("Week", View::Week, cx))
            .child(
                Button::new("calendar-proposal-review")
                    .small()
                    .ghost()
                    .label(if self.reviewing_proposals {
                        "Back to calendar".to_owned()
                    } else {
                        format!(
                            "Review detected events · {}",
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
                View::Month => self.month(&dates, &entries, today, palette, cx),
                View::Week => self.week(&dates, &entries, today, palette, cx),
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
                        .overflow_y_scroll()
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_1()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(shortcut_hint("←→", "day", palette))
                    .child(shortcut_hint("↑↓", "week", palette))
                    .child(shortcut_hint("[ ]", "page", palette))
                    .child(shortcut_hint("t", "today", palette))
                    .child(shortcut_hint("m/w", "view", palette))
                    .child(shortcut_hint("e", "entry", palette))
                    .child(shortcut_hint("n", "new", palette))
                    .child(shortcut_hint("⌫", "delete", palette)),
            )
    }
}

impl CalendarPage {
    fn month(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let weekdays = div().flex().gap_1().mb_1().children(
            ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"].map(|d| {
                div()
                    .flex_1()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(d)
            }),
        );
        let month = self.day.month();
        let rows = dates.chunks(7).map(|week| {
            div().flex().gap_1().children(week.iter().map(|&day| {
                let on = entries_on(entries, day);
                let more = on.len().saturating_sub(3);
                let selected = day == self.day;
                let is_today = day == today;
                let in_current_month = day.month() == month;
                div()
                    .id(SharedString::from(format!("calendar-day-{day}")))
                    .flex_1()
                    .min_w_0()
                    .min_h(px(104.))
                    .p(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if selected {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(if selected {
                        rgba(palette.raised)
                    } else if !in_current_month {
                        rgba((palette.surface & 0xffffff) | 0x44000000)
                    } else {
                        rgba(palette.surface)
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |page, _, _, cx| page.select(day, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb_1()
                            .child(
                                div()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(is_today, |el| el.bg(rgb(palette.accent)))
                                    .text_xs()
                                    .font_weight(if is_today {
                                        FontWeight::BOLD
                                    } else if in_current_month {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(rgb(if is_today {
                                        0xffffff
                                    } else if in_current_month {
                                        palette.text
                                    } else {
                                        palette.muted
                                    }))
                                    .child(day.day().to_string()),
                            )
                            .when(day.day() == 1, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(if in_current_month {
                                            palette.text
                                        } else {
                                            palette.muted
                                        }))
                                        .child(day.format("%b").to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(on.into_iter().take(3).map(|e| chip(e, palette))),
                    )
                    .when(more > 0, |el| {
                        el.child(
                            div()
                                .pt(px(2.))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.muted))
                                .child(format!("+{more} more")),
                        )
                    })
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(weekdays)
            .children(rows)
            .into_any_element()
    }

    fn week(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .gap_1()
            .children(dates.iter().map(|&day| {
                let selected = day == self.day;
                let is_today = day == today;
                let on = entries_on(entries, day);
                div()
                    .id(SharedString::from(format!("calendar-col-{day}")))
                    .flex_1()
                    .min_w_0()
                    .min_h(px(360.))
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if selected {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(if selected {
                        rgba(palette.raised)
                    } else {
                        rgba(palette.surface)
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |page, _, _, cx| page.select(day, cx)))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_1()
                            .border_b_1()
                            .border_color(rgba(palette.border))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child(day.format("%a").to_string().to_uppercase()),
                            )
                            .child(
                                div()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(is_today, |el| el.bg(rgb(palette.accent)))
                                    .text_xs()
                                    .font_weight(if is_today {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if is_today { 0xffffff } else { palette.text }))
                                    .child(day.day().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .children(on.into_iter().map(|e| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(time_label(e, day)),
                                    )
                                    .child(chip(e, palette))
                            })),
                    )
            }))
            .into_any_element()
    }

    fn day_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.day_entries();
        let is_today = self.day == Local::now().date_naive();
        let header = div()
            .flex()
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
                        .gap_2()
                        .p_3()
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
                            rgba((palette.surface & 0xffffff) | 0x88000000)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.selected_entry = Some(id.clone());
                            page.confirm_delete = None;
                            cx.notify();
                        }))
                        // Top row: kind pill and time
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
                                        .child(
                                            div()
                                                .px_1()
                                                .py(px(1.))
                                                .rounded(px(3.))
                                                .bg(rgba((color & 0xffffff) | 0x2e000000))
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(color))
                                                .child(kind_str),
                                        )
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
                        // Title
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(entry.title.clone()),
                        )
                        // Location
                        .when_some(entry.location.clone().filter(|l| !l.is_empty()), |el, l| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("📍 {l}")),
                            )
                        })
                        // Notes
                        .when_some(entry.notes.clone().filter(|n| !n.is_empty()), |el, n| {
                            el.child(
                                div()
                                    .p_2()
                                    .rounded(px(4.))
                                    .bg(rgba(palette.surface))
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(n),
                            )
                        })
                        // Non-manual source
                        .when(!entry.source.is_empty() && entry.source != "manual", |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("From {}", entry.source)),
                            )
                        })
                        // Confirm deletion alert
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

    fn form_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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
                                form.kind = key;
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
                            form.commitment = key;
                        }
                        cx.notify();
                    }))
            }));
        div()
            .key_context("CalendarForm")
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("New entry · {}", self.day.format("%a %b %-d"))),
            )
            .child(Input::new(&form.title))
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
                            .label("Save · ↩")
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

/// One entry as a coloured badge pill and its title.
fn chip(entry: &CalendarEntry, palette: Palette) -> gpui_kit::Div {
    let color = kind_color(&entry.kind);
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .px_1()
        .py(px(1.5))
        .rounded(px(4.))
        .bg(rgba((color & 0xffffff) | 0x22000000))
        .child(
            div()
                .w(px(5.))
                .h(px(5.))
                .flex_none()
                .rounded_full()
                .bg(rgb(color)),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if entry.commitment == "possible" {
                    palette.muted
                } else {
                    palette.text
                }))
                .child(entry.title.clone()),
        )
}

fn shortcut_hint(key: &'static str, desc: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1()
                .py(px(1.))
                .rounded(px(3.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().child(desc))
        .into_any_element()
}

fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
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
        });
    }
}
