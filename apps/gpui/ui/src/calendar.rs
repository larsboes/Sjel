//! A day of the calendar, drawn from wherever this device keeps it.
//!
//! The Mac's page (`apps/gpui/src/calendar_view.rs`) draws a month for a machine that is here. This
//! one is the phone's: one day at a time, seven day chips around it, and the two writes a day needs
//! — an entry made, an entry deleted.
//!
//! Two sources, one page, and the page picks per read. This device's own store
//! ([`sjel_gpui_core::data::Repo`]: the phone's replica, or the Mac's database) answers once its
//! change feed has been read ([`Repo::has_calendar`]), and takes writes into that store, queueing
//! them for the next sync. Until then the Mac's read-only copy ([`sjel_gpui_core::mirror`]) draws
//! the day; the page says which one it is drawing and offers no editor over a copy, because a copy
//! takes no writes.
//!
//! The page knows nothing about the network. Its host hands it the store and the copy, and it emits
//! [`CalendarEvent::Refresh`] when the person asks for newer data and [`CalendarEvent::Wrote`] when
//! an entry landed; the host owns the fetch and the sync.
//!
//! A day an entry covers is a day it is shown on, including the middle days of a trip
//! ([`sjel_gpui_core::calendar::covers_day`]) — the rule the Mac's month grid already keeps.

use std::sync::Arc;

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, EventEmitter, MouseButton, Render,
    SharedString, Subscription, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use sjel_gpui_core::calendar::{self, Draft};
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};

use crate::data::{self, Press, Repo};
use crate::{Density, Palette};

/// What the page asks its host to do. The host owns the network, the copy on disk, and the sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarEvent {
    /// The person asked for newer data: a sync into the store, or a newer copy when there is none.
    Refresh,
    /// An entry was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
    /// The new-entry form opened (`true`) or closed, so the host can show or hide the keyboard.
    Editing(bool),
}

/// Where the week on screen was read from, which is also what the page may do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// This device's own store: the phone's replica, or the Mac's database. Writes land here.
    Store,
    /// The Mac's read-only copy, because the store holds no calendar yet.
    Copy,
    /// Neither answered.
    Unread,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DayViewMode {
    #[default]
    List,
    Grid,
}

/// The new-entry form: a title, times, and commitment chips.
struct Composer {
    input: Entity<InputState>,
    all_day: bool,
    start: String,
    end: String,
    commitment: &'static str,
    /// The listener that saves on Enter, held for as long as the form is open.
    _subscription: Subscription,
}

/// The page's own state: which day, what answered it, and why nothing did.
pub struct Calendar {
    /// This device's own store, once the app has one.
    store: Option<Repo>,
    /// The Mac's read-only copy, when one is on disk.
    copy: Option<Arc<Mirror>>,
    /// Why the Mac's copy could not be opened, in the words the person needs.
    missing: Option<SharedString>,
    /// Which of the two answered the week on screen.
    origin: Origin,
    day: NaiveDate,
    week: Vec<NaiveDate>,
    /// The week's entries, all days at once: one read answers the day list and the chips' dots, and
    /// a second read could only disagree with the first.
    entries: Vec<CalendarEntry>,
    loaded: bool,
    loading: bool,
    /// A failed read, kept beside the rows that answered before it.
    error: Option<SharedString>,
    /// The open new-entry form, and its complaint when it cannot be saved.
    composer: Option<Composer>,
    form_error: Option<SharedString>,
    /// The entry whose Delete was asked for, waiting for the second tap.
    confirm_delete: Option<SharedString>,
    /// True while a write is in flight, so one tap cannot make two entries.
    saving: bool,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    note: Option<SharedString>,
    palette: Palette,
    density: Density,
    pub view_mode: DayViewMode,
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
            store: None,
            copy: None,
            missing: None,
            origin: Origin::Unread,
            day,
            week: calendar::week_dates(day),
            entries: Vec::new(),
            loaded: false,
            loading: false,
            error: None,
            composer: None,
            form_error: None,
            confirm_delete: None,
            saving: false,
            note: None,
            palette: Palette::light(),
            density,
            view_mode: DayViewMode::default(),
            generation: 0,
        }
    }

    /// Hands the page this device's own store once it opens.
    ///
    /// The store wins over the copy from the moment its change feed has been read, which the read
    /// itself decides ([`Self::load`]): a replica that has never pulled holds no entries, and
    /// drawing its empty month over a readable copy would hide a day that is there.
    pub fn set_store(&mut self, store: Option<Repo>, cx: &mut Context<Self>) {
        self.store = store;
        self.loaded = false;
        self.load(cx);
    }

    /// Hands the page the Mac's copy, or the reason it is not there.
    ///
    /// A newer copy replaces the old one without the page forgetting which day is open, so a
    /// refresh that lands while it is on screen moves the entries, not the person.
    pub fn set_copy(
        &mut self,
        copy: Option<Arc<Mirror>>,
        missing: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.copy = copy;
        self.missing = missing.map(SharedString::from);
        self.loaded = false;
        self.error = None;
        self.load(cx);
    }

    /// A sentence about the data on screen from the host — most often why a newer copy could not be
    /// fetched. A failed fetch does not take away the rows already there: they are still true, only
    /// older, and the line above says how old.
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

    /// True once something has answered this week, so a host can tell a blank page from a loading
    /// one.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// True when the day on screen came from this device's own store, so entries can be written.
    pub fn writable(&self) -> bool {
        self.origin == Origin::Store && self.store.is_some()
    }

    /// Reads the week on screen from the store, or from the Mac's copy until the store can answer.
    ///
    /// Nothing happens without either: a phone that has not reached its Mac and has no copy reads
    /// nothing, and says so through [`Self::set_copy`]'s `missing` instead of drawing zeroes that
    /// look like an answer.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() && self.copy.is_none() {
            return;
        }
        let Some((from, to)) = calendar::window(&self.week) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let store = self.store.clone();
        let copy = self.copy.clone();
        let work = shell::run(async move {
            if let Some(store) = store {
                if store.has_calendar().await {
                    let answer = store.fetch_calendar(&from, &to).await;
                    return (Origin::Store, answer);
                }
            }
            if let Some(copy) = copy {
                let answer = copy.calendar(&from, &to).await.map_err(ShellError::Store);
                return (Origin::Copy, answer);
            }
            (
                Origin::Unread,
                Err(ShellError::Store("The calendar was not read.".to_owned())),
            )
        });
        cx.spawn(async move |this, cx| {
            let (origin, answer) = work.await.unwrap_or((
                Origin::Unread,
                Err(ShellError::Store("The calendar was not read.".to_owned())),
            ));
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                view.loaded = true;
                view.origin = origin;
                match answer {
                    Ok(entries) => {
                        view.entries = entries;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(SharedString::from(error.to_string())),
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

    /// The line under the title: where this week came from, and whether it can be changed here.
    fn source_line(&self) -> Option<String> {
        match self.origin {
            Origin::Store => Some("Your store · Google & iCloud synced".to_owned()),
            Origin::Copy => self.copy.as_ref().map(|copy| {
                format!(
                    "Copy from {} · Google & iCloud synced",
                    copy.manifest().built_label()
                )
            }),
            Origin::Unread => None,
        }
    }

    /// Opens the new-entry form on the day on screen, with the keyboard asked for.
    fn open_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_composer_at(9, window, cx);
    }

    fn open_composer_at(&mut self, hour: u32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable() || self.composer.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("What is it?"));
        let subscription = cx.subscribe_in(&input, window, |view, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                view.save_composer(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let start = format!("{hour:02}:00");
        let end = format!("{:02}:00", (hour + 1).min(23));
        self.composer = Some(Composer {
            input,
            all_day: false,
            start,
            end,
            commitment: "planned",
            _subscription: subscription,
        });
        self.form_error = None;
        cx.emit(CalendarEvent::Editing(true));
        cx.notify();
    }

    fn close_composer(&mut self, cx: &mut Context<Self>) {
        if self.composer.take().is_some() {
            self.form_error = None;
            cx.emit(CalendarEvent::Editing(false));
        }
        cx.notify();
    }

    /// Writes the form's entry into this device's store, where the next sync carries it to the Mac.
    fn save_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(composer) = self.composer.as_ref() else {
            return;
        };
        let Some(store) = self.store.clone() else {
            return;
        };
        let draft = Draft {
            title: composer.input.read(cx).value().to_string(),
            day: self.day,
            start: composer.start.clone(),
            end: composer.end.clone(),
            all_day: composer.all_day,
            kind: "event".to_owned(),
            commitment: composer.commitment.to_owned(),
        };
        let entry = match draft.entry(String::new()) {
            Ok(entry) => entry,
            Err(why) => {
                self.form_error = Some(SharedString::from(why));
                cx.notify();
                return;
            }
        };
        self.saving = true;
        data::write_repo(
            cx,
            window,
            self,
            &store,
            Press::quiet("Entry saved."),
            move |repo| async move { repo.save_calendar_entry(entry).await },
            |this, saved, cx| {
                this.saving = false;
                if saved.is_some() {
                    this.composer = None;
                    this.confirm_delete = None;
                    this.load(cx);
                    cx.emit(CalendarEvent::Wrote);
                    cx.emit(CalendarEvent::Editing(false));
                }
                cx.notify();
            },
        );
    }

    /// Asks for a second tap before an entry goes: a list is tapped through, and one tap that
    /// deletes what a thumb brushed is not a rule a phone should keep.
    fn ask_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        self.confirm_delete = Some(SharedString::from(id.to_owned()));
        cx.notify();
    }

    /// Deletes the entry whose Delete was confirmed, here and — queued — on the Mac.
    fn delete_entry(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.saving = true;
        data::write_repo(
            cx,
            window,
            self,
            &store,
            Press::quiet("Entry deleted."),
            move |repo| async move { repo.delete_calendar_entry(&id).await },
            |this, done, cx| {
                this.saving = false;
                this.confirm_delete = None;
                if done.is_some() {
                    this.load(cx);
                    cx.emit(CalendarEvent::Wrote);
                }
                cx.notify();
            },
        );
    }

    fn row(&self, entry: &CalendarEntry, day: NaiveDate, cx: &mut Context<Self>) -> AnyElement {
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
            .when(self.writable(), |row| {
                row.child(self.row_actions(entry, cx))
            })
            .into_any_element()
    }

    /// What can be done to one row: the ask, and the second tap that deletes.
    ///
    /// Nothing renders when the day came from the Mac's copy: a copy takes no writes, and a button
    /// that cannot work is worse than no button.
    fn row_actions(&self, entry: &CalendarEntry, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let confirming = self.confirm_delete.as_deref() == Some(entry.id.as_str());
        if confirming {
            let id = entry.id.clone();
            return div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id(SharedString::from(format!("calendar-delete-{}", entry.id)))
                        .px_2()
                        .py(px(6.))
                        .rounded(px(6.))
                        .bg(rgb(palette.red))
                        .text_xs()
                        .text_color(rgb(0xffffff))
                        .child("Delete")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| {
                                view.delete_entry(id.clone(), window, cx)
                            }),
                        ),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("calendar-keep-{}", entry.id)))
                        .px_2()
                        .py(px(6.))
                        .rounded(px(6.))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Keep")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.confirm_delete = None;
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element();
        }
        let id = entry.id.clone();
        div()
            .id(SharedString::from(format!("calendar-more-{}", entry.id)))
            .flex_none()
            .px_2()
            .py(px(6.))
            .rounded(px(6.))
            .text_xs()
            .text_color(rgb(palette.muted))
            .child("⋯")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| view.ask_delete(&id, cx)),
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

    /// The new-entry form: one field, two choices that fit a thumb, and the two buttons that end
    /// it. Only drawn from a writable store ([`Self::writable`]).
    fn composer_card(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let Some(composer) = self.composer.as_ref() else {
            return div().into_any_element();
        };
        let commitment_chips: Vec<AnyElement> = calendar::COMMITMENTS
            .iter()
            .map(|(key, label)| {
                let key: &'static str = key;
                let on = composer.commitment == key;
                div()
                    .id(SharedString::from(format!(
                        "calendar-form-commitment-{key}"
                    )))
                    .flex_1()
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .when(on, |chip| chip.bg(rgb(palette.accent)))
                    .when(!on, |chip| chip.bg(rgba(palette.border)))
                    .text_xs()
                    .text_color(rgb(if on { 0xffffff } else { palette.text }))
                    .child(*label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            if let Some(composer) = view.composer.as_mut() {
                                composer.commitment = key;
                            }
                            cx.notify();
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        let all_day = composer.all_day;
        let time_range: SharedString = format!("{}–{}", composer.start, composer.end).into();
        let when_button = |label: SharedString, day: bool| {
            let on = all_day == day;
            div()
                .id(SharedString::from(format!(
                    "calendar-form-{}",
                    if day { "all-day" } else { "timed" }
                )))
                .flex_1()
                .py(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .when(on, |pill| pill.bg(rgb(palette.text)))
                .when(!on, |pill| pill.bg(rgba(palette.border)))
                .text_xs()
                .text_color(rgb(if on { 0xffffff } else { palette.text }))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        if let Some(composer) = view.composer.as_mut() {
                            composer.all_day = day;
                        }
                        cx.notify();
                    }),
                )
                .into_any_element()
        };
        let saving = self.saving;
        let form_error = self.form_error.clone();

        div()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(Input::new(&composer.input).small())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(when_button("All day".into(), true))
                    .child(when_button(time_range, false)),
            )
            .child(div().flex().gap_2().children(commitment_chips))
            .when_some(form_error, |card, why| {
                card.child(div().text_xs().text_color(rgb(palette.red)).child(why))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("calendar-form-save")
                            .flex_1()
                            .py(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(rgb(if saving {
                                palette.muted
                            } else {
                                palette.accent
                            }))
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(0xffffff))
                            .child(if saving { "Saving…" } else { "Save" })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, window, cx| view.save_composer(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("calendar-form-cancel")
                            .flex_none()
                            .px_4()
                            .py(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("Cancel")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.close_composer(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_time_grid(&self, day: NaiveDate, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let today = Local::now().date_naive();
        let is_today = day == today;
        let now = Local::now().time();
        let now_minutes = now.hour() * 60 + now.minute();
        let entries = self.entries_of_day();
        let hour_height = 44.0;

        let mut grid = div()
            .relative()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden();

        for hour in 0..24 {
            let hour_start = hour * 60;
            let hour_end = (hour + 1) * 60;
            let hour_entries: Vec<&CalendarEntry> = entries
                .iter()
                .filter(|e| {
                    if e.all_day {
                        return false;
                    }
                    if let Some((start_h, start_m)) = parse_time(&e.starts_at) {
                        let mins = start_h * 60 + start_m;
                        mins >= hour_start && mins < hour_end
                    } else {
                        false
                    }
                })
                .copied()
                .collect();

            let writable = self.writable();
            let mut slot = div()
                .flex()
                .items_start()
                .min_h(px(hour_height))
                .border_t_1()
                .border_color(rgba(palette.border))
                .when(writable, |slot| {
                    slot.cursor_pointer().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, window, cx| {
                            view.open_composer_at(hour, window, cx);
                        }),
                    )
                })
                .child(
                    div()
                        .w(px(52.))
                        .flex_none()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(format!("{hour:02}:00")),
                );

            let mut events_col = div().flex_1().flex().flex_col().gap_1().py_1().pr_2();
            for entry in hour_entries {
                let rail = calendar::kind_color(&entry.kind);
                let time = calendar::time_label(entry, day);
                events_col = events_col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .p_1()
                        .rounded(px(4.))
                        .bg(rgba(palette.raised))
                        .child(div().w(px(3.)).h(px(18.)).rounded_full().bg(rgb(rail)))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(entry.title.clone()),
                        )
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(time)),
                );
            }
            slot = slot.child(events_col);
            grid = grid.child(slot);
        }

        if is_today {
            let now_offset = (now_minutes as f32 / 60.0) * hour_height;
            grid = grid.child(
                div()
                    .absolute()
                    .top(px(now_offset))
                    .left(px(52.))
                    .right_0()
                    .h(px(2.))
                    .bg(rgb(palette.red))
                    .child(
                        div()
                            .absolute()
                            .left(px(-4.))
                            .top(px(-3.))
                            .size(px(8.))
                            .rounded_full()
                            .bg(rgb(palette.red)),
                    ),
            );
        }

        grid.into_any_element()
    }
}

fn parse_time(text: &str) -> Option<(u32, u32)> {
    let time_part = text.split('T').nth(1).unwrap_or(text);
    let mut parts = time_part.split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    Some((h, m))
}

impl Render for Calendar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let day = self.day;
        let today = Local::now().date_naive();
        let dotted = self.dotted();
        let rows: Vec<AnyElement> = self
            .entries_of_day()
            .into_iter()
            .map(|entry| self.row(entry, day, cx))
            .collect();
        let count = rows.len();
        let chips: Vec<AnyElement> = self
            .week
            .clone()
            .into_iter()
            .zip(dotted)
            .map(|(date, dotted)| self.chip(date, dotted, cx))
            .collect();
        let source_line = self.source_line();
        let error = self.error.clone();
        let note = self.note.clone();
        let missing = self.missing.clone();
        let loading = self.loading;
        let writable = self.writable();
        let composing = self.composer.is_some();
        let empty_line = match (missing.is_some(), loading, count) {
            (true, _, _) => None,
            // Rows that answered before a refresh stay on screen while the new week is read.
            (false, _, 1..) => None,
            (false, true, 0) => Some("Reading the calendar…".to_owned()),
            (false, false, _) if !self.loaded => Some("Not read yet.".to_owned()),
            (false, false, _) if self.origin == Origin::Copy => {
                Some("Nothing on this day in the copy.".to_owned())
            }
            (false, false, _) => Some("Nothing on this day.".to_owned()),
        };
        let refresh = self.refresh_button(cx);
        let composer = self.composer_card(window, cx);
        let to_today = cx.entity().clone();
        let mode_toggle = div()
            .flex()
            .items_center()
            .rounded(px(6.))
            .bg(rgba(palette.border))
            .p(px(2.))
            .child(
                div()
                    .id("view-mode-list")
                    .cursor_pointer()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .when(self.view_mode == DayViewMode::List, |b| {
                        b.bg(rgb(palette.accent)).text_color(rgb(0xffffff))
                    })
                    .when(self.view_mode != DayViewMode::List, |b| {
                        b.text_color(rgb(palette.muted))
                    })
                    .text_xs()
                    .child("List")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.view_mode = DayViewMode::List;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("view-mode-grid")
                    .cursor_pointer()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .when(self.view_mode == DayViewMode::Grid, |b| {
                        b.bg(rgb(palette.accent)).text_color(rgb(0xffffff))
                    })
                    .when(self.view_mode != DayViewMode::Grid, |b| {
                        b.text_color(rgb(palette.muted))
                    })
                    .text_xs()
                    .child("24h Grid")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.view_mode = DayViewMode::Grid;
                            cx.notify();
                        }),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_3()
            // Title row: the day, what it was read from, and the one write a day needs.
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
                            .when_some(source_line, |column, line| {
                                column.child(
                                    div().text_xs().text_color(rgb(palette.muted)).child(line),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(writable && !composing, |row| {
                                row.child(
                                    div()
                                        .id("calendar-new")
                                        .flex_none()
                                        .px_3()
                                        .py(px(6.))
                                        .rounded(px(8.))
                                        .bg(rgb(palette.accent))
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(rgb(0xffffff))
                                        .child("New")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|view, _, window, cx| {
                                                view.open_composer(window, cx)
                                            }),
                                        ),
                                )
                            })
                            .child(mode_toggle)
                            .child(refresh),
                    ),
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
            .when(composing, |page| page.child(composer))
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
                                .child("The calendar is not on this phone yet"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(missing),
                        ),
                )
            })
            .when(self.view_mode == DayViewMode::List, |page| {
                page.when_some(empty_line, |page, line| {
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
            })
            .when(self.view_mode == DayViewMode::Grid, |page| {
                page.child(self.render_time_grid(day, cx))
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
    fn a_page_that_has_read_nothing_offers_no_editor() {
        let day = calendar::parse_day("2026-10-24").expect("a date");
        let page = Calendar::at(day, Density::Mobile);
        assert_eq!(page.origin, Origin::Unread);
        assert!(
            !page.writable(),
            "an editor over nothing would drop what it was given"
        );
        assert!(
            page.source_line().is_none(),
            "and there is no source to name yet"
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
