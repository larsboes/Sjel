//! The single-day and mobile Calendar view, drawn from local store or mirror copy.
use std::sync::Arc;

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, MouseButton, Render,
    SharedString, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    Disableable, Sizable,
};
use sjel_gpui_core::calendar::{self, Draft};
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};

use crate::data::{self, Press, Repo};
use crate::theme::{tint, ThemeContext};
use crate::{Density, Palette};

use super::grid::CalendarDrag;
use super::types::{CalendarEvent, CalendarSyncStatus, Composer, DayViewMode, Origin};

#[derive(Clone)]
struct TouchEdit {
    id: String,
    resize: bool,
    start_y: gpui::Pixels,
    original: chrono::NaiveDateTime,
    target: chrono::NaiveDateTime,
}

/// The page's own state: which day, what answered it, and why nothing did.
pub struct Calendar {
    /// This device's own store, once the app has one.
    pub(crate) store: Option<Repo>,
    /// The Mac's read-only copy, when one is on disk.
    pub(crate) copy: Option<Arc<Mirror>>,
    /// Why the Mac's copy could not be opened, in the words the person needs.
    pub(crate) missing: Option<SharedString>,
    /// Which of the two answered the week on screen.
    pub(crate) origin: Origin,
    pub(crate) day: NaiveDate,
    pub(crate) week: Vec<NaiveDate>,
    /// The week's entries, all days at once: one read answers the day list and the chips' dots, and
    /// a second read could only disagree with the first.
    pub(crate) entries: Vec<CalendarEntry>,
    pub(crate) loaded: bool,
    pub(crate) loading: bool,
    /// A failed read, kept beside the rows that answered before it.
    pub(crate) error: Option<SharedString>,
    /// The open new-entry form, and its complaint when it cannot be saved.
    pub(crate) composer: Option<Composer>,
    pub(crate) form_error: Option<SharedString>,
    /// The entry whose Delete was asked for, waiting for the second tap.
    pub(crate) confirm_delete: Option<SharedString>,
    /// True while a write is in flight, so one tap cannot make two entries.
    pub(crate) saving: bool,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    pub(crate) note: Option<SharedString>,
    pub sync_status: Option<CalendarSyncStatus>,
    pub(crate) palette: Palette,
    pub(crate) palette_override: Option<Palette>,
    pub(crate) density: Density,
    pub view_mode: DayViewMode,
    selected: Option<String>,
    undo_time: Option<(CalendarEntry, CalendarEntry)>,
    show_availability: bool,
    drop_minute: Option<u16>,
    touch_edit: Option<TouchEdit>,
    clock: Option<gpui::Task<()>>,
    timeline_scroll: gpui::ScrollHandle,
    /// Bumped per read, so a slow answer for a week the person has left cannot replace the week
    /// they are looking at.
    pub(crate) generation: u64,
}

impl EventEmitter<CalendarEvent> for Calendar {}

impl Calendar {
    pub fn new(density: Density) -> Self {
        Self::at(Local::now().date_naive(), density)
    }

    /// The page opened on `day` rather than today. A screenshot or a touch check of a day that is
    /// not today needs this; `ios/run-sim` sets it from `SJEL_DAY`.
    pub fn at(day: NaiveDate, density: Density) -> Self {
        let timeline_scroll = gpui::ScrollHandle::new();
        timeline_scroll.set_offset(gpui::point(px(0.), px(-8. * 52.)));
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
            sync_status: None,
            palette: Palette::light(),
            palette_override: None,
            density,
            view_mode: DayViewMode::default(),
            selected: None,
            undo_time: None,
            show_availability: false,
            drop_minute: None,
            touch_edit: None,
            clock: None,
            timeline_scroll,
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
        self.palette_override = Some(palette);
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
            let sync_status = if let Some(store) = &store {
                store.calendar_sync_status().await.ok()
            } else {
                None
            };
            if let Some(store) = store {
                if store.has_calendar().await {
                    let answer = store.fetch_calendar(&from, &to).await;
                    return (Origin::Store, answer, sync_status);
                }
            }
            if let Some(copy) = copy {
                let answer = copy.calendar(&from, &to).await.map_err(ShellError::Store);
                return (Origin::Copy, answer, sync_status);
            }
            (
                Origin::Unread,
                Err(ShellError::Store("The calendar was not read.".to_owned())),
                sync_status,
            )
        });
        cx.spawn(async move |this, cx| {
            let (origin, answer, sync_status) = work.await.unwrap_or((
                Origin::Unread,
                Err(ShellError::Store("The calendar was not read.".to_owned())),
                None,
            ));
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.origin = origin;
                view.sync_status = sync_status;
                view.loading = false;
                match answer {
                    Ok(entries) => {
                        view.entries = entries;
                        view.loaded = true;
                        view.error = None;
                    }
                    Err(error) => {
                        view.error = Some(SharedString::from(error.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_day(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        let current_week = self.week.clone();
        self.selected = None;
        self.touch_edit = None;
        self.day = day;
        self.week = calendar::week_dates(day);
        if self.week != current_week {
            self.loaded = false;
            self.load(cx);
        }
        cx.notify();
    }

    fn step(&mut self, days: i64, cx: &mut Context<Self>) {
        let Some(next) = self.day.checked_add_signed(Duration::days(days)) else {
            return;
        };
        self.open_day(next, cx);
    }

    pub fn entries_of_day(&self) -> Vec<&CalendarEntry> {
        calendar::entries_on(&self.entries, self.day)
    }

    fn dotted(&self) -> Vec<bool> {
        self.week
            .iter()
            .map(|day| !calendar::entries_on(&self.entries, *day).is_empty())
            .collect()
    }

    fn source_line(&self) -> Option<String> {
        match self.origin {
            Origin::Store => {
                if let Some(status) = &self.sync_status {
                    Some(status.label())
                } else {
                    self.note.as_ref().map(|note| note.to_string())
                }
            }
            Origin::Copy => {
                let note = self
                    .note
                    .as_ref()
                    .map(|n| format!(" · {n}"))
                    .unwrap_or_default();
                Some(format!("Read-only copy from Mac{note}"))
            }
            Origin::Unread => None,
        }
    }

    fn open_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_composer_at(9, window, cx);
    }

    fn open_composer_at(&mut self, hour: u32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New entry"));
        let sub = cx.subscribe_in(&input, window, |view, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                view.save_composer(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let start = format!("{hour:02}:00");
        let end = format!("{:02}:00", (hour + 1) % 24);
        self.composer = Some(Composer {
            input,
            all_day: false,
            start,
            end,
            commitment: "planned",
            _subscription: sub,
        });
        self.form_error = None;
        cx.emit(CalendarEvent::Editing(true));
        cx.notify();
    }

    fn close_composer(&mut self, cx: &mut Context<Self>) {
        if self.composer.take().is_some() {
            self.form_error = None;
            cx.emit(CalendarEvent::Editing(false));
            cx.notify();
        }
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
            .map_or(entry.commitment.as_str(), |(_, label)| *label);
        let dimmed = entry.commitment == "possible";
        let height = if self.density.is_mobile() { 56. } else { 44. };
        let id = entry.id.clone();
        div()
            .id(SharedString::from(format!("calendar-row-{id}")))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.selected = Some(id.clone());
                cx.notify();
            }))
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

    fn change_time(
        &mut self,
        id: &str,
        day: NaiveDate,
        minute: u16,
        resize: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self.entries.iter().find(|entry| entry.id == id).cloned() else {
            return;
        };
        match calendar::timeline::reschedule(&before, day, minute, resize) {
            Ok(after) => self.write_time(before, after, false, window, cx),
            Err(error) => {
                self.error = Some(error.into());
                cx.notify();
            }
        }
    }

    fn write_time(
        &mut self,
        before: CalendarEntry,
        after: CalendarEntry,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving || !self.writable() {
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
            Press::quiet(""),
            move |repo| async move {
                let saved = repo.save_calendar_entry(after).await?;
                Ok((before, saved, undo))
            },
            |view, result, cx| {
                view.saving = false;
                if let Some((before, saved, undo)) = result {
                    if let Some(entry) = view.entries.iter_mut().find(|entry| entry.id == saved.id)
                    {
                        *entry = saved.clone();
                    }
                    view.undo_time = if undo { None } else { Some((before, saved)) };
                    cx.emit(CalendarEvent::Wrote);
                }
                cx.notify();
            },
        );
    }

    fn detail_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| Some(&entry.id) == self.selected.as_ref())
        else {
            return div().into_any_element();
        };
        let mut sheet = div()
            .id("calendar-event-sheet")
            .debug_selector(|| "calendar-event-sheet".into())
            .occlude()
            .max_h(px(320.))
            .overflow_y_scroll()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .p_4()
            .bg(rgba(palette.surface))
            .border_t_1()
            .border_color(rgba(palette.border))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(entry.title.clone())
                    .child(
                        Button::new("calendar-sheet-close")
                            .small()
                            .ghost()
                            .label("Done")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.selected = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(calendar::time_label(entry, self.day))
            .child(format!(
                "{} · {}",
                calendar::kind_label(&entry.kind),
                entry.commitment
            ));
        for text in [&entry.location, &entry.notes].into_iter().flatten() {
            sheet = sheet.child(text.clone());
        }
        sheet = sheet.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("From {}", entry.source)),
        );
        if self.writable() {
            sheet = sheet.child(self.row_actions(entry, cx));
        }
        if !entry.all_day && self.writable() {
            let mut actions = div().flex().flex_wrap().gap_2();
            for (label, delta, resize) in [
                ("15 min earlier", -15, false),
                ("15 min later", 15, false),
                ("Shorten 15 min", -15, true),
                ("Extend 15 min", 15, true),
            ] {
                let id = entry.id.clone();
                let time = if resize {
                    &entry.ends_at
                } else {
                    &entry.starts_at
                };
                if let Ok(time) = chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M:%S") {
                    if let Some(target) = time.checked_add_signed(Duration::minutes(delta)) {
                        actions = actions.child(
                            Button::new(SharedString::from(format!("calendar-sheet-{label}")))
                                .small()
                                .ghost()
                                .label(label)
                                .disabled(self.saving)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.change_time(
                                        &id,
                                        target.date(),
                                        (target.hour() * 60 + target.minute()) as u16,
                                        resize,
                                        window,
                                        cx,
                                    )
                                })),
                        );
                    }
                }
            }
            sheet = sheet.child(actions);
        }
        sheet.into_any_element()
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
            .when(!selected, |chip| {
                chip.bg(rgba(palette.surface))
                    .border_1()
                    .border_color(rgba(palette.border))
            })
            .hover(|chip| {
                if !selected {
                    chip.bg(rgba(palette.raised))
                } else {
                    chip
                }
            })
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

    fn render_timeline(&self, day: NaiveDate, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let hour_height = 52.;
        let all_day = self
            .entries_of_day()
            .into_iter()
            .filter(|entry| entry.all_day)
            .map(|entry| self.row(entry, day, cx))
            .collect::<Vec<_>>();
        let mut timeline = div()
            .id("mobile-calendar-timeline")
            .debug_selector(|| "mobile-calendar-timeline".into())
            .relative()
            .h(px(24. * hour_height))
            .on_drag_move(cx.listener(
                move |view, event: &gpui::DragMoveEvent<CalendarDrag>, _, cx| {
                    let y = f32::from(event.event.position.y - event.bounds.origin.y);
                    view.drop_minute =
                        Some(((y / hour_height * 60.).clamp(0., 1439.) as u16) / 15 * 15);
                    cx.notify();
                },
            ))
            .on_drop(cx.listener(move |view, drag: &CalendarDrag, window, cx| {
                if let Some(minute) = view.drop_minute.take() {
                    view.change_time(&drag.id, day, minute, drag.resize, window, cx);
                }
            }));
        for quarter in 0..96 {
            let minute = quarter * 15;
            timeline = timeline.child(
                div()
                    .id(("mobile-calendar-slot", quarter))
                    .absolute()
                    .top(px(minute as f32 / 60. * hour_height))
                    .left_0()
                    .right_0()
                    .h(px(hour_height / 4.))
                    .when(quarter % 4 == 0, |slot| {
                        slot.border_t_1().border_color(rgba(palette.border))
                    })
                    .when(quarter % 4 == 2, |slot| {
                        slot.border_t_1()
                            .border_color(rgba(tint(palette.border, 0x40)))
                    })
                    .hover(|slot| slot.bg(rgba(tint(palette.accent, 0x0c))))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .when(quarter % 4 == 0, |slot| {
                        slot.child(format!("{:02}:00", minute / 60))
                    })
                    .when(self.writable(), |slot| {
                        slot.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| {
                                view.open_composer_at(minute / 60, window, cx);
                                if let Some(composer) = &mut view.composer {
                                    composer.start =
                                        format!("{:02}:{:02}", minute / 60, minute % 60);
                                    let end = (minute + 60) % 1440;
                                    composer.end = format!("{:02}:{:02}", end / 60, end % 60);
                                }
                            }),
                        )
                    }),
            );
        }
        let mut events = div().absolute().left(px(48.)).right_0().top_0().h_full();
        for block in calendar::timeline::blocks(&self.entries, day) {
            let entry = &self.entries[block.entry_index];
            let id = entry.id.clone();
            let color = calendar::kind_color(&entry.kind);
            events = events.child(
                div()
                    .id(SharedString::from(format!(
                        "mobile-calendar-block-{}",
                        entry.id
                    )))
                    .debug_selector(|| "mobile-calendar-event".into())
                    .absolute()
                    .top(px(block.start as f32 / 60. * hour_height))
                    .left(gpui::relative(block.column as f32 / block.columns as f32))
                    .w(gpui::relative(1. / block.columns as f32))
                    .h(px((block.end - block.start) as f32 / 60. * hour_height))
                    .overflow_hidden()
                    .border_l_2()
                    .border_color(rgb(color))
                    .px_1()
                    .bg(rgba(crate::theme::tint(color, 0x28)))
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .child(entry.title.clone())
                    .child(calendar::time_label(entry, day))
                    .when_some(
                        self.touch_edit.as_ref().filter(|edit| edit.id == entry.id),
                        |block, edit| {
                            block.child(format!(
                                "{} {}",
                                if edit.resize { "End:" } else { "Move to:" },
                                edit.target.format("%a %H:%M")
                            ))
                        },
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected = Some(id.clone());
                        cx.notify();
                    }))
                    .when(self.writable(), |block| {
                        block
                            .child(self.touch_time_target(entry, hour_height, cx))
                            .on_drag(
                                CalendarDrag {
                                    id: entry.id.clone(),
                                    resize: false,
                                },
                                |drag, _, _, cx| cx.new(|_| drag.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("mobile-resize-{}", entry.id)))
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .h(px(12.))
                                    .on_drag(
                                        CalendarDrag {
                                            id: entry.id.clone(),
                                            resize: true,
                                        },
                                        |drag, _, _, cx| {
                                            cx.stop_propagation();
                                            cx.new(|_| drag.clone())
                                        },
                                    ),
                            )
                    }),
            );
        }
        timeline = timeline.child(events);
        if day == Local::now().date_naive() {
            let now = Local::now().time();
            let y = px((now.hour() * 60 + now.minute()) as f32 / 60. * hour_height);
            timeline = timeline.child(
                div()
                    .absolute()
                    .left(px(48.))
                    .right_0()
                    .top(y)
                    .h(px(2.))
                    .bg(rgb(palette.red))
                    .child(
                        div()
                            .absolute()
                            .top(px(-3.))
                            .left(px(-4.))
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(rgb(palette.red)),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .children(all_day)
            .child(timeline)
            .into_any_element()
    }

    fn touch_time_target(
        &self,
        entry: &CalendarEntry,
        hour_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        let entry = entry.clone();
        gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |bounds, hitbox, window, _| {
                window.on_mouse_event(move |event: &gpui::LongPressEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    let _ = entity.update(cx, |view, cx| match event.phase {
                        gpui::TouchPhase::Started
                            if hitbox.is_hovered(window)
                                && !window.default_prevented()
                                && view.writable()
                                && !view.saving =>
                        {
                            let resize = event.start_position.y >= bounds.bottom() - px(12.);
                            let timestamp = if resize {
                                &entry.ends_at
                            } else {
                                &entry.starts_at
                            };
                            let Ok(original) = chrono::NaiveDateTime::parse_from_str(
                                timestamp,
                                "%Y-%m-%dT%H:%M:%S",
                            ) else {
                                return;
                            };
                            view.touch_edit = Some(TouchEdit {
                                id: entry.id.clone(),
                                resize,
                                start_y: event.start_position.y,
                                original,
                                target: original,
                            });
                            window.capture_long_press(&cx.entity());
                            window.prevent_default();
                            cx.stop_propagation();
                            cx.notify();
                        }
                        gpui::TouchPhase::Moved | gpui::TouchPhase::Ended
                            if view
                                .touch_edit
                                .as_ref()
                                .is_some_and(|edit| edit.id == entry.id) =>
                        {
                            let edit = view.touch_edit.as_mut().expect("active touch edit");
                            let delta = (f32::from(event.position.y - edit.start_y) / hour_height
                                * 4.)
                                .round() as i64
                                * 15;
                            if let Some(target) =
                                edit.original.checked_add_signed(Duration::minutes(delta))
                            {
                                edit.target = target;
                            }
                            if event.phase == gpui::TouchPhase::Ended {
                                let edit = view.touch_edit.take().expect("active touch edit");
                                if edit.target != edit.original {
                                    view.change_time(
                                        &edit.id,
                                        edit.target.date(),
                                        (edit.target.hour() * 60 + edit.target.minute()) as u16,
                                        edit.resize,
                                        window,
                                        cx,
                                    );
                                }
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }
                        gpui::TouchPhase::Cancelled
                            if view
                                .touch_edit
                                .as_ref()
                                .is_some_and(|edit| edit.id == entry.id) =>
                        {
                            view.touch_edit = None;
                            cx.stop_propagation();
                            cx.notify();
                        }
                        _ => {}
                    });
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
}

impl Render for Calendar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clock.is_none() {
            self.clock = Some(super::minute_clock(cx));
        }
        if self.palette_override.is_none() {
            self.palette = ThemeContext::of(window, cx).palette;
        }
        let palette = self.palette;
        let day = self.day;
        let today = Local::now().date_naive();
        let dotted = self.dotted();
        let entries = self.entries_of_day();
        let count = entries.len();
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .map(|entry| self.row(entry, day, cx))
            .collect();
        let chips: Vec<AnyElement> = self
            .week
            .iter()
            .zip(dotted)
            .map(|(&date, dot)| self.chip(date, dot, cx))
            .collect();
        let source_line = self.source_line();
        let composing = self.composer.is_some();
        let composer = self.composer_card(window, cx);
        let error = self.error.clone();
        let note = self.note.clone();
        let missing = self.missing.clone();
        let to_today = cx.entity().clone();
        let empty_line = match (self.origin, self.loaded) {
            (Origin::Store, true) => Some("Nothing on the calendar for this day."),
            (Origin::Copy, true) => Some("No entries on this day in the Mac's copy."),
            _ => None,
        };
        let refresh = self.refresh_button(cx);
        let detail = self.detail_sheet(cx);
        let mut availability = div().flex().flex_wrap().gap_2();
        if self.show_availability && self.loaded && !self.loading && self.error.is_none() {
            for gap in calendar::timeline::gaps(
                &self.entries,
                day,
                calendar::timeline::Interval {
                    start: 480,
                    end: 1200,
                },
            ) {
                let start = format!("{:02}:{:02}", gap.start / 60, gap.start % 60);
                let end = format!("{:02}:{:02}", gap.end / 60, gap.end % 60);
                availability = availability.child(
                    Button::new(SharedString::from(format!("mobile-gap-{}", gap.start)))
                        .small()
                        .ghost()
                        .label(format!("{start}–{end} free"))
                        .disabled(!self.writable())
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_composer_at(u32::from(gap.start / 60), window, cx);
                            if let Some(composer) = &mut view.composer {
                                composer.start =
                                    format!("{:02}:{:02}", gap.start / 60, gap.start % 60);
                                let end = gap.end.min(gap.start + 60);
                                composer.end = format!("{:02}:{:02}", end / 60, end % 60);
                            }
                        })),
                );
            }
        }

        let mode_toggle = div()
            .id("calendar-mode-toggle")
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
            .child(match self.view_mode {
                DayViewMode::List => "Grid",
                DayViewMode::Grid => "List",
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.view_mode = match view.view_mode {
                        DayViewMode::List => DayViewMode::Grid,
                        DayViewMode::Grid => DayViewMode::List,
                    };
                    cx.notify();
                }),
            );

        div()
            .id("calendar")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            // Day title and the button that asks for newer data.
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap_2()
                    .when(self.density.is_mobile(), |header| header.flex_col().items_start())
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
                                let is_err = self
                                    .sync_status
                                    .as_ref()
                                    .is_some_and(|status| status.has_error());
                                let color = if is_err {
                                    palette.red
                                } else {
                                    palette.muted
                                };
                                column.child(
                                    div()
                                        .id("calendar-sync-status")
                                        .debug_selector(|| "calendar-sync-status".into())
                                        .text_xs()
                                        .text_color(rgb(color))
                                        .child(line),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .when(self.writable(), |actions| {
                                actions.child(
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
                            .when(self.undo_time.is_some(), |actions| {
                                actions.child(
                                    Button::new("mobile-calendar-undo")
                                        .small()
                                        .ghost()
                                        .label("Undo")
                                        .disabled(self.saving)
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            if let Some((before, after)) = view.undo_time.clone() {
                                                if let Some(mut current) = view.entries.iter().find(|entry| entry.id == after.id).cloned() {
                                                    if current.starts_at == after.starts_at && current.ends_at == after.ends_at {
                                                        current.starts_at = before.starts_at;
                                                        current.ends_at = before.ends_at;
                                                        view.write_time(after, current, true, window, cx);
                                                    } else {
                                                        view.error = Some("This event's time changed. Refresh before undoing.".into());
                                                        cx.notify();
                                                    }
                                                }
                                            }
                                        })),
                                )
                            })
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
            .child(
                Button::new("mobile-calendar-availability")
                    .small()
                    .ghost()
                    .label(if self.show_availability {
                        "Hide free time"
                    } else {
                        "Show free time"
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_availability = !view.show_availability;
                        cx.notify();
                    })),
            )
            .when(self.show_availability, |page| page.child(availability))
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
                page.child(
                    div()
                        .id("mobile-calendar-grid-scroll")
                        .flex_1()
                        .min_h_0()
                        .track_scroll(&self.timeline_scroll)
                        .overflow_y_scroll()
                        .child(self.render_timeline(day, cx)),
                )
            })
            .when(self.selected.is_some(), |page| page.child(detail))
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

    #[gpui::test]
    fn long_press_moves_resizes_and_cancels_without_a_mouse_drag(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_kit::init);
        let page = cx.new(|_| {
            let day = calendar::parse_day("2026-11-05").unwrap();
            let mut page = Calendar::at(day, Density::Mobile);
            let mut event = entry(
                "Review",
                "2026-11-05T09:00:00",
                "2026-11-05T10:00:00",
                false,
            );
            event.id = "calendar:fixture".into();
            page.entries = vec![event];
            page.store = Some(Repo::new(Err(ShellError::NotSetUp)));
            page.origin = Origin::Store;
            page.loaded = true;
            page.view_mode = DayViewMode::Grid;
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.simulate_resize(gpui::size(px(375.), px(812.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("mobile-calendar-event").unwrap();
        let start = bounds.origin + gpui::point(px(10.), px(5.));
        cx.simulate_mouse_move(start, None, gpui::Modifiers::none());
        cx.simulate_event(gpui::LongPressEvent {
            phase: gpui::TouchPhase::Started,
            start_position: start,
            position: start,
        });
        page.read_with(cx, |page, _| {
            assert!(page.touch_edit.as_ref().is_some_and(|edit| !edit.resize))
        });
        let moved = start + gpui::point(px(0.), px(26.));
        cx.simulate_event(gpui::LongPressEvent {
            phase: gpui::TouchPhase::Moved,
            start_position: start,
            position: moved,
        });
        page.read_with(cx, |page, _| {
            assert_eq!(
                page.touch_edit
                    .as_ref()
                    .unwrap()
                    .target
                    .format("%H:%M")
                    .to_string(),
                "09:30"
            )
        });
        cx.simulate_event(gpui::LongPressEvent {
            phase: gpui::TouchPhase::Cancelled,
            start_position: start,
            position: moved,
        });
        page.read_with(cx, |page, _| {
            assert!(page.touch_edit.is_none());
            assert!(!page.saving);
            assert!(page.error.is_none());
        });

        let start = gpui::point(bounds.origin.x + px(10.), bounds.bottom() - px(4.));
        cx.simulate_mouse_move(start, None, gpui::Modifiers::none());
        cx.simulate_event(gpui::LongPressEvent {
            phase: gpui::TouchPhase::Started,
            start_position: start,
            position: start,
        });
        page.read_with(cx, |page, _| {
            assert!(page.touch_edit.as_ref().is_some_and(|edit| edit.resize))
        });
        let moved = start - gpui::point(px(0.), px(78.));
        cx.simulate_event(gpui::LongPressEvent {
            phase: gpui::TouchPhase::Ended,
            start_position: start,
            position: moved,
        });
        page.read_with(cx, |page, _| {
            assert!(page.touch_edit.is_none());
            assert!(page
                .error
                .as_ref()
                .is_some_and(|error| error.contains("end must be after")));
            assert!(!page.saving);
        });
    }

    #[gpui::test]
    fn phone_grid_and_detail_sheet_fit_a_narrow_window(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| {
                let day = calendar::parse_day("2026-11-05").unwrap();
                let mut page = Calendar::at(day, Density::Mobile);
                let mut event = entry(
                    "A long event title that opens in full",
                    "2026-11-05T09:00:00",
                    "2026-11-05T10:00:00",
                    false,
                );
                event.id = "calendar:fixture".into();
                page.entries = vec![event];
                page.selected = Some("calendar:fixture".into());
                page.loaded = true;
                page.view_mode = DayViewMode::Grid;
                page
            });
            gpui_kit::component::Root::new(view, window, cx)
        });
        cx.simulate_resize(gpui::size(px(375.), px(812.)));
        cx.run_until_parked();
        let grid = cx
            .debug_bounds("mobile-calendar-timeline")
            .expect("grid rendered");
        assert!(grid.size.width <= px(351.));
        assert_eq!(grid.size.height, px(1248.));
        let sheet = cx
            .debug_bounds("calendar-event-sheet")
            .expect("detail rendered");
        assert!(sheet.size.width <= px(375.));
        assert!(sheet.size.height <= px(320.));
    }

    #[gpui::test]
    fn mobile_calendar_displays_sync_status_mark_and_errors(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_kit::init);
        let page = cx.new(|_| {
            let day = calendar::parse_day("2026-11-05").unwrap();
            let mut page = Calendar::at(day, Density::Mobile);
            page.origin = Origin::Store;
            page.loaded = true;
            page.sync_status = Some(CalendarSyncStatus {
                google_synced_at: None,
                google_error: None,
                icloud_synced_at: None,
                icloud_error: None,
                replica_synced_at: Some("2026-11-05T22:00:00Z".into()),
                replica_error: None,
            });
            page
        });
        let shown = page.clone();
        let (_, cx) =
            cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
        cx.simulate_resize(gpui::size(px(375.), px(812.)));
        cx.run_until_parked();
        let mark = cx
            .debug_bounds("calendar-sync-status")
            .expect("sync status rendered");
        assert!(mark.size.width > px(0.));

        page.update(cx, |page, cx| {
            page.sync_status = Some(CalendarSyncStatus {
                google_synced_at: None,
                google_error: None,
                icloud_synced_at: None,
                icloud_error: None,
                replica_synced_at: None,
                replica_error: Some("connection refused".into()),
            });
            cx.notify();
        });
        cx.run_until_parked();
        let mark = cx
            .debug_bounds("calendar-sync-status")
            .expect("sync error rendered");
        assert!(mark.size.width > px(0.));
    }
}
