//! Actions, keybindings, and navigation handling for Calendar.
use chrono::{Duration, Local};
use gpui_kit::{actions, App, Context, KeyBinding, Window};

use super::page::CalendarPage;
use super::types::View;

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
        /// Focus one day's timeline.
        DayView,
        /// Show the week.
        WeekView,
        /// Select the next entry of the day.
        NextEntry,
        /// Add an entry on the selected day.
        NewEntry,
        /// Delete the selected entry, after confirming.
        DeleteEntry,
        /// Undo the last moved or resized event.
        UndoTime,
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
        KeyBinding::new("d", DayView, Some("CalendarGrid")),
        KeyBinding::new("w", WeekView, Some("CalendarGrid")),
        KeyBinding::new("e", NextEntry, Some("CalendarGrid")),
        KeyBinding::new("n", NewEntry, Some("CalendarGrid")),
        KeyBinding::new("backspace", DeleteEntry, Some("CalendarGrid")),
        KeyBinding::new("cmd-z", UndoTime, Some("CalendarGrid")),
        KeyBinding::new("u", UndoTime, Some("CalendarGrid")),
        KeyBinding::new("enter", Confirm, Some("CalendarGrid")),
        KeyBinding::new("escape", Cancel, Some("CalendarGrid")),
        KeyBinding::new("escape", Cancel, Some("CalendarForm")),
    ]);
}

impl CalendarPage {
    pub(super) fn on_day_view(&mut self, _: &DayView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Day, cx);
    }

    pub(super) fn on_previous_day(
        &mut self,
        _: &PreviousDay,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(self.day - Duration::days(1), cx);
    }

    pub(super) fn on_next_day(&mut self, _: &NextDay, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day + Duration::days(1), cx);
    }

    pub(super) fn on_previous_week(
        &mut self,
        _: &PreviousWeek,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(self.day - Duration::days(7), cx);
    }

    pub(super) fn on_next_week(&mut self, _: &NextWeek, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day + Duration::days(7), cx);
    }

    pub(super) fn on_previous_page(
        &mut self,
        _: &PreviousPage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shift_page(false, cx);
    }

    pub(super) fn on_next_page(&mut self, _: &NextPage, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_page(true, cx);
    }

    pub(super) fn on_today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        self.select(Local::now().date_naive(), cx);
    }

    pub(super) fn on_month_view(&mut self, _: &MonthView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Month, cx);
    }

    pub(super) fn on_week_view(&mut self, _: &WeekView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(View::Week, cx);
    }

    pub(super) fn on_next_entry(&mut self, _: &NextEntry, _: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.day_entries().into_iter().map(|e| e.id).collect();
        let at = self
            .selected_entry
            .as_ref()
            .and_then(|id| ids.iter().position(|i| i == id));
        let next = match at {
            Some(i) => ids.get((i + 1) % ids.len()).cloned(),
            None => ids.first().cloned(),
        };
        if let Some(id) = next {
            self.select_entry(id, cx);
        } else {
            self.selected_entry = None;
            self.selected_detail = None;
            self.confirm_delete = None;
            cx.notify();
        }
    }

    pub(super) fn on_new_entry(
        &mut self,
        _: &NewEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_form(window, cx);
    }

    pub(super) fn on_delete_entry(
        &mut self,
        _: &DeleteEntry,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_delete = self.selected_entry.clone();
        cx.notify();
    }

    pub(super) fn on_undo_time(&mut self, _: &UndoTime, _: &mut Window, cx: &mut Context<Self>) {
        if self.undo_time.is_some() && !self.changing_time {
            self.undo_time(cx);
        }
    }

    pub(super) fn on_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.confirm_delete.take() {
            self.delete(id, cx);
        } else if self.selected_entry.is_some() {
            self.edit_selected(window, cx);
        }
    }

    pub(super) fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            self.close_form(window, cx);
        } else if self.confirm_delete.take().is_none() {
            self.selected_entry = None;
        }
        cx.notify();
    }
}
