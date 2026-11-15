//! Ely views with Sjel's selection, capability intents and free intervals.
use super::{adapter, page::CalendarPage};
use crate::ely::calendar::{CalendarDays, CalendarMonthView, Event, FreeInterval, When};
use chrono::NaiveDate;
use gpui::{div, prelude::*, AnyElement, Context, SharedString};
use gpui_kit::component::scroll::ScrollableElement;
use sjel_gpui_core::{calendar::timeline, shell::CalendarEntry};

impl CalendarPage {
    pub(super) fn events(&self, entries: &[CalendarEntry]) -> (Vec<Event>, Vec<String>) {
        let mut events = Vec::with_capacity(entries.len());
        let mut errors = Vec::new();
        let mut ids = std::collections::HashSet::with_capacity(entries.len());
        for entry in entries {
            if !ids.insert(&entry.id) {
                errors.push(format!("Duplicate calendar entry: {}", entry.id));
                continue;
            }
            match adapter::event(entry, &self.zone, self.selected_entry.as_deref()) {
                Ok(event) => events.push(event),
                Err(error) => errors.push(format!("{}: {error}", entry.title)),
            }
        }
        (events, errors)
    }

    pub(super) fn activate_event(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .entries
            .value
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.id == id))
        {
            if !sjel_gpui_core::calendar::covers_day(entry, self.day) {
                let day = self
                    .dates()
                    .into_iter()
                    .find(|day| sjel_gpui_core::calendar::covers_day(entry, *day));
                if let Some(day) = day {
                    self.select(day, cx);
                }
            }
        }
        self.select_entry(id.to_owned(), cx);
    }

    fn days<const DAYS: usize>(
        &self,
        entries: &[CalendarEntry],
        dates: &[NaiveDate],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (events, errors) = self.events(entries);
        let free = if self.show_availability
            && self.entries.error.is_none()
            && self.loaded.is_some()
            && !self.entries.loading
        {
            dates
                .iter()
                .flat_map(|day| {
                    timeline::gaps(
                        entries,
                        *day,
                        timeline::Interval {
                            start: 480,
                            end: 1200,
                        },
                    )
                    .into_iter()
                    .map(|gap| FreeInterval {
                        day: adapter::date(*day),
                        start: gap.start,
                        end: gap.end,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let (pick, event, make, slot, moved, resized) = (
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(errors.into_iter().map(|error| div().child(error)))
            .child(
                div().flex_1().min_h_0().child(
                    CalendarDays::<DAYS>::new(
                        SharedString::from(format!("calendar-days-{DAYS}")),
                        adapter::date(self.day),
                        events,
                    )
                    .header(false)
                    .zone(self.zone.clone())
                    .track_scroll(self.timeline_scroll.clone())
                    .free_intervals(free)
                    .on_select(move |day, _, cx| {
                        let _ = pick.update(cx, |page, cx| page.select(adapter::day(day), cx));
                    })
                    .on_event(move |id, _, cx| {
                        let _ = event.update(cx, |page, cx| page.activate_event(id, cx));
                    })
                    .on_create(move |start, end, _, cx| {
                        let _ = make.update(cx, |page, cx| {
                            page.select(adapter::day(start.to_zoned(page.zone.clone()).date()), cx);
                            page.open_draft(When::Timed { start, end }, None, cx);
                        });
                    })
                    .on_slot(move |start, end, _, cx| {
                        let _ = slot.update(cx, |page, cx| {
                            page.select(adapter::day(start.to_zoned(page.zone.clone()).date()), cx);
                            page.open_draft(When::Timed { start, end }, None, cx);
                        });
                    })
                    .on_move(move |id, start, end, _, cx| {
                        let _ =
                            moved.update(cx, |page, cx| page.commit_time(id, Some(start), end, cx));
                    })
                    .on_resize(move |id, end, _, cx| {
                        let _ = resized.update(cx, |page, cx| page.commit_time(id, None, end, cx));
                    }),
                ),
            )
            .into_any_element()
    }

    pub(super) fn time_grid(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if dates.len() == 1 {
            self.days::<1>(entries, dates, cx)
        } else {
            self.days::<7>(entries, dates, cx)
        }
    }

    pub(super) fn month(&self, entries: &[CalendarEntry], cx: &mut Context<Self>) -> AnyElement {
        let (events, errors) = self.events(entries);
        let (pick, open, event) = (
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
        );
        div()
            .id("calendar-month-scroll")
            .debug_selector(|| "calendar-month-scroll".into())
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.month_scroll)
            .vertical_scrollbar(&self.month_scroll)
            .children(errors.into_iter().map(|error| div().child(error)))
            .child(
                CalendarMonthView::new("calendar-month", adapter::date(self.day), events)
                    .zone(self.zone.clone())
                    .selected(adapter::date(self.day))
                    .keyboard(false)
                    .header(false)
                    .on_select(move |day, _, cx| {
                        let _ = pick.update(cx, |page, cx| page.select(adapter::day(day), cx));
                    })
                    .on_day(move |day, _, cx| {
                        let _ = open.update(cx, |page, cx| {
                            page.set_view(super::types::View::Day, cx);
                            page.open_day(adapter::day(day), None, cx);
                        });
                    })
                    .on_event(move |id, _, cx| {
                        let _ = event.update(cx, |page, cx| page.activate_event(id, cx));
                    }),
            )
            .into_any_element()
    }
}
