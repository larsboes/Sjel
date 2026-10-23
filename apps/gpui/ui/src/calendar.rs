//! A day of the calendar, drawn from whatever copy of the data this client can read.
//!
//! The Mac's page (`apps/gpui/src/calendar_view.rs`) draws a month for a machine that is here. This
//! one is the phone's: one day at a time, seven day chips around it, and no editor, because what it
//! reads is the Mac's read-only copy ([`sjel_gpui_core::mirror`]) and a copy takes no writes.
//!
//! The page knows nothing about the network. It is handed a [`Mirror`] by its host, says when that
//! copy was built, and emits [`CalendarEvent::Refresh`] when the person asks for a newer one; the
//! host owns the fetch and hands the result back. That is the split the Projects page keeps with
//! its replica, and it lets either client draw this page.
//!
//! A day an entry covers is a day it is shown on, including the middle days of a trip
//! ([`sjel_gpui_core::calendar::covers_day`]) — the rule the Mac's month grid already keeps.

use std::sync::Arc;

use chrono::{Datelike, Duration, Local, NaiveDate};
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, MouseButton, Render,
    SharedString, Window,
};
use sjel_gpui_core::calendar;
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::CalendarEntry;

use crate::{Density, Palette};

/// What the page asks its host to do. The host owns the network and the copy on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarEvent {
    /// The person asked for a newer copy of the Mac's data.
    Refresh,
}

/// The page's own state: which day, what the copy answered, and why it could not.
pub struct Calendar {
    mirror: Option<Arc<Mirror>>,
    /// Why there is no copy to read, in the words the person needs. `None` when nothing failed.
    missing: Option<SharedString>,
    day: NaiveDate,
    week: Vec<NaiveDate>,
    /// The copy's entries for the week on screen, all days at once: one read answers the day list
    /// and the chips' dots, and a second read could only disagree with the first.
    entries: Vec<CalendarEntry>,
    loaded: bool,
    loading: bool,
    /// A failed read, kept beside the rows that answered before it.
    error: Option<SharedString>,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    note: Option<SharedString>,
    palette: Palette,
    density: Density,
    /// Bumped per read, so a slow answer for a week the person has left cannot replace the week
    /// they are looking at.
    generation: u64,
}

impl EventEmitter<CalendarEvent> for Calendar {}

impl Calendar {
    pub fn new(density: Density) -> Self {
        Self::at(Local::now().date_naive(), density)
    }

    /// The page opened on `day` rather than today. A screenshot or a touch check of a day that is
    /// not today needs this; `ios/run-sim` sets it from `SJEL_DAY`.
    pub fn at(day: NaiveDate, density: Density) -> Self {
        Self {
            mirror: None,
            missing: None,
            day,
            week: calendar::week_dates(day),
            entries: Vec::new(),
            loaded: false,
            loading: false,
            error: None,
            note: None,
            palette: Palette::light(),
            density,
            generation: 0,
        }
    }

    /// Hands the page a copy to read, or the reason the Mac has not sent one.
    ///
    /// A newer copy replaces the old one without the page forgetting which day is open, so a
    /// refresh that lands while it is on screen moves the entries, not the person.
    pub fn set_mirror(
        &mut self,
        mirror: Option<Arc<Mirror>>,
        missing: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mirror = mirror;
        self.missing = missing.map(SharedString::from);
        self.loaded = false;
        self.error = None;
        self.load(cx);
    }

    /// A sentence about this copy from the host — most often why a newer one could not be fetched.
    /// A failed fetch does not take away the rows already on screen: they are still true, only
    /// older, and the copy line above says how old.
    pub fn set_note(&mut self, note: Option<String>, cx: &mut Context<Self>) {
        self.note = note.map(SharedString::from);
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette {
            self.palette = palette;
            cx.notify();
        }
    }

    /// True once the copy has answered this week, so a host can tell a blank page from a loading one.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Reads the week on screen from the copy.
    ///
    /// Nothing happens without one: a phone that has not reached its Mac reads nothing, and says so
    /// through [`Self::set_mirror`]'s `missing` instead of drawing zeroes that look like an answer.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(mirror) = self.mirror.clone() else {
            return;
        };
        let Some((from, to)) = calendar::window(&self.week) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let work = sjel_gpui_core::shell::run(async move { mirror.calendar(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let answer = work
                .await
                .unwrap_or_else(|| Err("The copy was not read.".to_owned()));
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                view.loaded = true;
                match answer {
                    Ok(entries) => {
                        view.entries = entries;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(SharedString::from(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens another day, reading the week again only when the day is outside the week on screen.
    pub fn open_day(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        let week = calendar::week_dates(day);
        let moved_week = week != self.week;
        self.day = day;
        if moved_week {
            self.week = week;
            self.load(cx);
        }
        cx.notify();
    }

    fn step(&mut self, days: i64, cx: &mut Context<Self>) {
        let day = self.day + Duration::days(days);
        self.open_day(day, cx);
    }

    /// The day's entries, all-day first, then by start ([`calendar::entries_on`]).
    pub fn entries_of_day(&self) -> Vec<&CalendarEntry> {
        calendar::entries_on(&self.entries, self.day)
    }

    /// Days in the week that hold at least one entry, for the chips' dots.
    fn dotted(&self) -> Vec<bool> {
        self.week
            .iter()
            .map(|day| self.entries.iter().any(|e| calendar::covers_day(e, *day)))
            .collect()
    }

    /// The copy's age, in the words the page shows under its title.
    fn copy_line(&self) -> Option<String> {
        let mirror = self.mirror.as_ref()?;
        Some(format!(
            "Copy from {} · read-only",
            mirror.manifest().built_label()
        ))
    }

    fn row(&self, entry: &CalendarEntry, day: NaiveDate) -> AnyElement {
        let palette = self.palette;
        let rail = calendar::kind_color(&entry.kind);
        let time = calendar::time_label(entry, day);
        let kind = calendar::kind_label(&entry.kind);
        // A commitment is the difference between "I could" and "I must", so it is said in words and
        // not only in a shade: a phone is read in daylight, and not everyone sees colour.
        let commitment = calendar::COMMITMENTS
            .iter()
            .find(|(key, _)| *key == entry.commitment)
            .map_or("Possible", |(_, label)| *label);
        let dimmed = entry.commitment == "possible";
        let height = if self.density.is_mobile() { 56. } else { 44. };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .min_h(px(height))
            .py(px(6.))
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .w(px(3.))
                    .h(px(height - 20.))
                    .rounded_full()
                    .bg(rgb(rail)),
            )
            .child(
                div()
                    .w(px(94.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .whitespace_nowrap()
                            .text_color(rgb(if dimmed { palette.muted } else { palette.text }))
                            .child(time),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_nowrap()
                            .truncate()
                            .text_color(rgb(palette.muted))
                            .child(kind.to_owned()),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            // One line per row: a phone list is scanned, and a three-line title
                            // pushes the time a thumb is aiming for off the row it belongs to.
                            .whitespace_nowrap()
                            .truncate()
                            .when(dimmed, |text| text.text_color(rgb(palette.muted)))
                            .child(entry.title.clone()),
                    )
                    .when_some(entry.location.clone(), |column, location| {
                        column.child(
                            div()
                                .text_xs()
                                .whitespace_nowrap()
                                .truncate()
                                .text_color(rgb(palette.muted))
                                .child(location),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .whitespace_nowrap()
                    .text_color(rgb(if entry.commitment == "committed" {
                        palette.text
                    } else {
                        palette.muted
                    }))
                    .child(commitment.to_owned()),
            )
            .into_any_element()
    }

    fn chip(&self, day: NaiveDate, dotted: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let selected = day == self.day;
        let today = day == Local::now().date_naive();
        let initial =
            ["M", "T", "W", "T", "F", "S", "S"][day.weekday().num_days_from_monday() as usize];
        div()
            .id(SharedString::from(format!(
                "calendar-day-{}",
                day.format("%Y-%m-%d")
            )))
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .min_h(px(if self.density.is_mobile() { 56. } else { 44. }))
            .rounded(px(10.))
            .when(selected, |chip| chip.bg(rgb(palette.accent)))
            .when(!selected, |chip| chip.bg(rgba(palette.surface)))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if selected { 0xffffff } else { palette.muted }))
                    .child(initial.to_owned()),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(if today || selected {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(if selected {
                        0xffffff
                    } else if today {
                        palette.accent
                    } else {
                        palette.text
                    }))
                    .child(day.day().to_string()),
            )
            .child(
                div()
                    .h(px(4.))
                    .w(px(4.))
                    .rounded_full()
                    .when(dotted, |dot| {
                        dot.bg(rgb(if selected { 0xffffff } else { palette.muted }))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| view.open_day(day, cx)),
            )
            .into_any_element()
    }

    /// The pill that asks the host for a newer copy.
    fn refresh_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        div()
            .id("calendar-refresh")
            .flex_none()
            .px_3()
            .py(px(6.))
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(rgb(palette.text))
            .child(if self.loading {
                "Fetching…"
            } else {
                "Refresh"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(CalendarEvent::Refresh)),
            )
            .into_any_element()
    }
}

impl Render for Calendar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let day = self.day;
        let today = Local::now().date_naive();
        let dotted = self.dotted();
        let rows: Vec<AnyElement> = self
            .entries_of_day()
            .into_iter()
            .map(|entry| self.row(entry, day))
            .collect();
        let count = rows.len();
        let chips: Vec<AnyElement> = self
            .week
            .clone()
            .into_iter()
            .zip(dotted)
            .map(|(date, dotted)| self.chip(date, dotted, cx))
            .collect();
        let copy_line = self.copy_line();
        let error = self.error.clone();
        let note = self.note.clone();
        let missing = self.missing.clone();
        let loading = self.loading;
        let empty_line = match (missing.is_some(), loading, count) {
            (true, _, _) => None,
            // Rows that answered before a refresh stay on screen while the new week is read.
            (false, _, 1..) => None,
            (false, true, 0) => Some("Reading the copy…".to_owned()),
            (false, false, _) if self.loaded => Some("Nothing on this day in the copy.".to_owned()),
            (false, false, _) => Some("Not read yet.".to_owned()),
        };
        let refresh = self.refresh_button(cx);
        let to_today = cx.entity().clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_3()
            // Title row: the day, and what it was read from.
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(day.format("%A, %-d %B").to_string()),
                            )
                            .when_some(copy_line, |column, line| {
                                column.child(
                                    div().text_xs().text_color(rgb(palette.muted)).child(line),
                                )
                            }),
                    )
                    .child(refresh),
            )
            // The week around it, with a dot on every day that holds something.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(step_button(
                        "calendar-prev",
                        "‹",
                        palette,
                        cx,
                        |view, cx| view.step(-1, cx),
                    ))
                    .child(div().flex_1().flex().gap_1().children(chips))
                    .child(step_button(
                        "calendar-next",
                        "›",
                        palette,
                        cx,
                        |view, cx| view.step(1, cx),
                    )),
            )
            .when(day != today, |page| {
                page.child(
                    div()
                        .id("calendar-today")
                        .text_xs()
                        .text_color(rgb(palette.accent))
                        .child("Back to today")
                        .on_mouse_down(MouseButton::Left, move |_, _, app: &mut gpui::App| {
                            let day = Local::now().date_naive();
                            to_today.update(app, |view, cx| view.open_day(day, cx));
                        }),
                )
            })
            .when_some(error, |page, error| {
                page.child(div().text_xs().text_color(rgb(palette.red)).child(error))
            })
            .when_some(note, |page, note| {
                page.child(div().text_xs().text_color(rgb(palette.muted)).child(note))
            })
            .when_some(missing, |page, missing| {
                page.child(
                    div()
                        .rounded(px(12.))
                        .bg(rgba(palette.surface))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("The calendar reads a copy of your Mac’s data"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(missing),
                        ),
                )
            })
            .when_some(empty_line, |page, line| {
                page.child(div().text_sm().text_color(rgb(palette.muted)).child(line))
            })
            .when(count > 0, |page| {
                page.child(
                    div()
                        .rounded(px(12.))
                        .bg(rgba(palette.surface))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .children(rows),
                )
            })
            .when(count > 0, |page| {
                page.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(match count {
                            1 => "1 entry".to_owned(),
                            other => format!("{other} entries"),
                        }),
                )
            })
    }
}

/// A chevron that moves the day on by one, with the 44-point target a finger needs.
fn step_button(
    id: &'static str,
    glyph: &'static str,
    palette: Palette,
    cx: &mut Context<Calendar>,
    step: impl Fn(&mut Calendar, &mut Context<Calendar>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .w(px(36.))
        .h(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .text_color(rgb(palette.muted))
        .child(glyph)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, _, _, cx| step(view, cx)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, starts_at: &str, ends_at: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts_at.to_owned(),
            ends_at: ends_at.to_owned(),
            all_day,
            kind: "event".to_owned(),
            commitment: "planned".to_owned(),
            ..CalendarEntry::default()
        }
    }

    #[test]
    fn the_week_on_screen_starts_on_monday_and_holds_the_day_it_opened_on() {
        let day = calendar::parse_day("2026-10-23").expect("a date");
        let page = Calendar::at(day, Density::Mobile);
        assert_eq!(
            page.week.first().copied(),
            calendar::parse_day("2026-10-19")
        );
        assert_eq!(page.week.last().copied(), calendar::parse_day("2026-10-25"));
        assert!(page.week.contains(&day));
    }

    #[test]
    fn a_trip_is_shown_on_every_day_it_covers_and_dots_those_days() {
        let day = calendar::parse_day("2026-10-09").expect("a date");
        let mut page = Calendar::at(day, Density::Mobile);
        page.entries = vec![
            entry("Berlin-Urlaub", "2026-10-07", "2026-10-14", true),
            entry("Lunch", "2026-10-09T12:00:00", "2026-10-09T15:30:00", false),
            entry("Later", "2026-10-24", "2026-10-25", true),
        ];
        let titles: Vec<&str> = page
            .entries_of_day()
            .iter()
            .map(|e| e.title.as_str())
            .collect();
        // All-day first, then by start: the trip is the whole day, lunch sits inside it.
        assert_eq!(titles, ["Berlin-Urlaub", "Lunch"]);
        // The week runs Monday 5 October and the trip starts on the 7th, so the first two days
        // carry no dot and every day the trip covers does.
        assert_eq!(
            page.dotted(),
            vec![false, false, true, true, true, true, true]
        );
    }

    #[test]
    fn a_stale_commitment_is_named_rather_than_guessed() {
        // The page says the commitment in words, so an unknown one must not read as "Possible".
        let day = calendar::parse_day("2026-10-09").expect("a date");
        let mut page = Calendar::at(day, Density::Mobile);
        let mut odd = entry("Odd", "2026-10-09", "2026-10-10", true);
        odd.commitment = "unheard-of".to_owned();
        page.entries = vec![odd];
        let row = format!("{:?}", page.entries_of_day());
        assert!(row.contains("unheard-of"), "{row}");
    }
}
