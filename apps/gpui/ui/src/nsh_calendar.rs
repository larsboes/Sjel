//! The phone's Calendar, drawn by NSH (`docs/plans/gpui.md` §11): one day of a week, as a list or
//! a 24-hour grid, with the gaps in it, a form for a new entry and a sheet per entry.
//!
//! The week is read from this device's store once its change feed has been read, and from the
//! Mac's read-only copy until then. Only a week the store answered can be written: an entry
//! written here goes to the Mac with the next sync. The GPUI page moved an entry by a long press
//! on the grid; here the sheet moves it by quarter hours, which needs no gesture a list lacks.

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Duration, Local, NaiveDate, Timelike};
use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::calendar::{self, timeline, CalendarSyncStatus, Draft};
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Where the week on screen was read from, which is also what the screen may do with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Origin {
    Store,
    Copy,
    #[default]
    Unread,
}

/// What the screen asks its host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarEvent {
    /// A newer copy, or a sync into the store.
    Refresh,
    /// An entry was written here and waits for the Mac.
    Wrote,
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    day: NaiveDate,
    week: Vec<NaiveDate>,
    entries: Vec<CalendarEntry>,
    origin: Origin,
    loaded: bool,
    loading: bool,
    error: Option<String>,
    source: Option<String>,
    sync_error: bool,
    saving: bool,
    /// The last time change, so the sheet can put it back.
    undo: Option<(CalendarEntry, CalendarEntry)>,
    /// Bumped after each saved entry, so the screen closes its form.
    saved: u64,
}

impl Snapshot {
    fn writable(&self, has_store: bool) -> bool {
        self.origin == Origin::Store && has_store
    }
}

struct State {
    store: Option<Repo>,
    copy: Option<Arc<Mirror>>,
    missing: Option<String>,
    note: Option<String>,
    sync_status: Option<CalendarSyncStatus>,
    generation: u64,
    snap: Snapshot,
}

type Host = Box<dyn Fn(CalendarEvent) + Send>;

struct Feed {
    state: Mutex<State>,
    tx: Mutex<Option<UnboundedSender<Snapshot>>>,
    host: Mutex<Option<Host>>,
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let day = today();
        Feed {
            state: Mutex::new(State {
                store: None,
                copy: None,
                missing: None,
                note: None,
                sync_status: None,
                generation: 0,
                snap: Snapshot {
                    day,
                    week: calendar::week_dates(day),
                    entries: Vec::new(),
                    origin: Origin::Unread,
                    loaded: false,
                    loading: false,
                    error: None,
                    source: None,
                    sync_error: false,
                    saving: false,
                    undo: None,
                    saved: 0,
                },
            }),
            tx: Mutex::new(None),
            host: Mutex::new(None),
        }
    })
}

fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The line under the title: who answered the week, or why nobody did.
fn source_line(state: &State) -> Option<String> {
    match state.snap.origin {
        Origin::Store => state
            .sync_status
            .as_ref()
            .map(CalendarSyncStatus::label)
            .or_else(|| state.note.clone()),
        Origin::Copy => {
            let note = state
                .note
                .as_ref()
                .map(|n| format!(" · {n}"))
                .unwrap_or_default();
            Some(format!("Read-only copy from Mac{note}"))
        }
        Origin::Unread => state.missing.clone(),
    }
}

fn publish(state: &mut State) {
    state.snap.source = source_line(state);
    state.snap.sync_error = state
        .sync_status
        .as_ref()
        .is_some_and(CalendarSyncStatus::has_error);
    let snap = state.snap.clone();
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(snap);
    }
}

fn emit(event: CalendarEvent) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(event);
    }
}

fn has_store() -> bool {
    state().store.is_some()
}

/// What the host does with a refresh or a write.
pub fn set_host(host: impl Fn(CalendarEvent) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}

/// Hands the screen this device's store. It wins over the copy once its change feed was read.
pub fn set_store(store: Option<Repo>) {
    {
        let mut state = state();
        state.store = store;
        state.snap.loaded = false;
        publish(&mut state);
    }
    load();
}

/// Hands the screen the Mac's copy, or the reason it is not there.
pub fn set_copy(copy: Option<Arc<Mirror>>, missing: Option<String>) {
    {
        let mut state = state();
        state.copy = copy;
        state.missing = missing;
        state.snap.error = None;
        publish(&mut state);
    }
    load();
}

/// A sentence from the host about the data on screen.
pub fn set_note(note: Option<String>) {
    let mut state = state();
    state.note = note;
    publish(&mut state);
}

/// Opens `day`, reading its week when the week changes.
pub fn open_day(day: NaiveDate) {
    let reload = {
        let mut state = state();
        let week = calendar::week_dates(day);
        let reload = week != state.snap.week;
        state.snap.day = day;
        state.snap.week = week;
        if reload {
            state.snap.loaded = false;
        }
        publish(&mut state);
        reload
    };
    if reload {
        load();
    }
}

/// Reads the week on screen: from the store when it holds a calendar, else from the copy.
pub fn load() {
    let (store, copy, window, generation) = {
        let mut state = state();
        if state.store.is_none() && state.copy.is_none() {
            return;
        }
        let Some(window) = calendar::window(&state.snap.week) else {
            return;
        };
        state.generation += 1;
        state.snap.loading = true;
        publish(&mut state);
        (
            state.store.clone(),
            state.copy.clone(),
            window,
            state.generation,
        )
    };
    shell::spawn(async move {
        let (from, to) = window;
        let sync_status = match &store {
            Some(store) => store.calendar_sync_status().await.ok(),
            None => None,
        };
        let mut answer = (
            Origin::Unread,
            Err(ShellError::Store("The calendar was not read.".to_owned())),
        );
        if let Some(store) = &store {
            if store.has_calendar().await {
                answer = (Origin::Store, store.fetch_calendar(&from, &to).await);
            }
        }
        if answer.0 == Origin::Unread {
            if let Some(copy) = copy {
                let read = copy.calendar(&from, &to).await.map_err(ShellError::Store);
                answer = (Origin::Copy, read);
            }
        }
        let mut state = state();
        if state.generation != generation {
            return;
        }
        state.sync_status = sync_status;
        state.snap.origin = answer.0;
        state.snap.loading = false;
        match answer.1 {
            Ok(entries) => {
                state.snap.entries = entries;
                state.snap.loaded = true;
                state.snap.error = None;
            }
            Err(error) => state.snap.error = Some(error.to_string()),
        }
        publish(&mut state);
    });
}

/// Runs one write against the store, then reloads the week and tells the host.
fn write(
    work: impl FnOnce(
            Repo,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Option<(CalendarEntry, CalendarEntry)>, ShellError>,
                    > + Send,
            >,
        > + Send
        + 'static,
    closes_form: bool,
) {
    let store = {
        let mut state = state();
        let Some(store) = state.store.clone() else {
            return;
        };
        if state.snap.saving || state.snap.origin != Origin::Store {
            return;
        }
        state.snap.saving = true;
        publish(&mut state);
        store
    };
    shell::spawn(async move {
        let answer = work(store).await;
        let landed = answer.is_ok();
        {
            let mut state = state();
            state.snap.saving = false;
            match answer {
                Ok(undo) => {
                    state.snap.undo = undo;
                    if closes_form {
                        state.snap.saved += 1;
                    }
                }
                Err(error) => state.snap.error = Some(error.to_string()),
            }
            publish(&mut state);
        }
        if landed {
            load();
            emit(CalendarEvent::Wrote);
        }
    });
}

fn save_entry(draft: Draft) -> Result<(), String> {
    let entry = draft.entry(String::new())?;
    write(
        move |repo| {
            Box::pin(async move {
                repo.save_calendar_entry(entry).await?;
                Ok(None)
            })
        },
        true,
    );
    Ok(())
}

fn delete_entry(id: String) {
    write(
        move |repo| {
            Box::pin(async move {
                repo.delete_calendar_entry(&id).await?;
                Ok(None)
            })
        },
        false,
    );
}

/// Writes `after` in place of `before`. An undo writes `before` back and offers no undo of its own.
fn change_time(before: CalendarEntry, after: CalendarEntry, undo: bool) {
    write(
        move |repo| {
            Box::pin(async move {
                let saved = repo.save_calendar_entry(after).await?;
                Ok((!undo).then_some((before, saved)))
            })
        },
        false,
    );
}

fn nudge(entry: &CalendarEntry, minutes: i64, resize: bool) -> Result<(), String> {
    let after = timeline::shift_minutes(entry, minutes, resize).map_err(str::to_owned)?;
    change_time(entry.clone(), after, false);
    Ok(())
}

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

const HOUR_PX: f32 = 52.0;

/// The day's new-entry form, while it is open.
#[derive(Clone, Debug, PartialEq)]
struct Form {
    start: String,
    end: String,
    all_day: bool,
    commitment: &'static str,
}

fn form_at(start_minute: u16, end_minute: u16) -> Form {
    let clock = |m: u16| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
    Form {
        start: clock(start_minute),
        end: clock(end_minute),
        all_day: false,
        commitment: "planned",
    }
}

#[derive(Clone, Copy)]
struct Ui {
    snap: Signal<Snapshot>,
    grid: Signal<bool>,
    free: Signal<bool>,
    selected: Signal<Option<String>>,
    confirm_delete: Signal<bool>,
    form: Signal<Option<Form>>,
    title: Signal<String>,
    form_error: Signal<Option<String>>,
    note: Signal<Option<String>>,
}

impl Ui {
    fn writable(&self) -> bool {
        self.snap.with(|s| s.writable(has_store()))
    }

    fn open_form(&self, form: Form) {
        if self.writable() {
            self.title.set(String::new());
            self.form_error.set(None);
            self.selected.set(None);
            self.form.set(Some(form));
        }
    }
}

fn chip(label: impl Into<String>, on: bool, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(move || {
            if on {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().muted
            }
        })
        .on_press(press)
        .child(
            text(label.into())
                .text_xs()
                .number_of_lines(1u32)
                .text_color(move || {
                    if on {
                        nsh_base::theme().primary_foreground
                    } else {
                        nsh_base::theme().foreground
                    }
                }),
        )
}

fn week_strip(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let mut strip = view().flex_row().items_center().gap_1().child(
        button()
            .px_2()
            .py_2()
            .on_press(move || open_day(snap_day(ui) - Duration::days(7)))
            .child(text("‹").text_base()),
    );
    for day in snap.week.clone() {
        let on = day == snap.day;
        let dotted = !calendar::entries_on(&snap.entries, day).is_empty();
        strip = strip.child(
            button()
                .flex_1()
                .py_2()
                .items_center()
                .rounded_lg()
                .bg(move || {
                    if on {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().background
                    }
                })
                .on_press(move || {
                    ui.selected.set(None);
                    open_day(day);
                })
                .child(
                    text(calendar_weekday(day))
                        .text_xs()
                        .text_color(move || weekday_color(on)),
                )
                .child(
                    text(day.format("%-d").to_string())
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || weekday_color(on)),
                )
                .child(
                    text(if dotted { "•" } else { " " })
                        .text_xs()
                        .text_color(move || weekday_color(on)),
                ),
        );
    }
    strip.child(
        button()
            .px_2()
            .py_2()
            .on_press(move || open_day(snap_day(ui) + Duration::days(7)))
            .child(text("›").text_base()),
    )
}

fn snap_day(ui: Ui) -> NaiveDate {
    ui.snap.with(|s| s.day)
}

fn calendar_weekday(day: NaiveDate) -> String {
    day.format("%a").to_string().chars().take(1).collect()
}

fn weekday_color(on: bool) -> AdaptiveColor {
    if on {
        nsh_base::theme().primary_foreground
    } else {
        nsh_base::theme().foreground
    }
}

fn entry_button(entry: &CalendarEntry, day: NaiveDate, ui: Ui) -> impl IntoElement {
    let id = entry.id.clone();
    let commitment = entry.commitment.clone();
    let place = entry
        .location
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| calendar::kind_label(&entry.kind).to_owned());
    button()
        .px_4()
        .py_3()
        .gap_1()
        .border_b(px(1.0))
        .border_b_color(move || nsh_base::theme().border)
        .on_press(move || {
            ui.form.set(None);
            ui.confirm_delete.set(false);
            ui.selected.set(Some(id.clone()));
        })
        .child(text(entry.title.clone()).text_base())
        .child(
            text(format!(
                "{} · {place} · {commitment}",
                calendar::time_label(entry, day)
            ))
            .text_xs()
            .text_color(move || muted()),
        )
}

fn list(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let day = snap.day;
    let entries = calendar::entries_on(&snap.entries, day);
    let mut out = view();
    if entries.is_empty() {
        let line = match (snap.origin, snap.loaded) {
            (_, false) if snap.loading => "Reading the day…",
            (Origin::Copy, true) => "No entries on this day in the Mac's copy.",
            (Origin::Store, true) => "Nothing on this day.",
            _ => "",
        };
        return out.child(
            text(line)
                .text_sm()
                .px_4()
                .py_3()
                .text_color(move || muted()),
        );
    }
    for entry in entries {
        out = out.child(entry_button(entry, day, ui));
    }
    out
}

/// The 24-hour grid: hour rows a tap opens the form on, and the day's timed entries placed by
/// `timeline::blocks`, side by side where they overlap.
fn grid(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let day = snap.day;
    let mut canvas = view().relative().h(px(HOUR_PX * 24.0));
    for hour in 0..24u16 {
        canvas = canvas.child(
            button()
                .absolute()
                .top(px(HOUR_PX * f32::from(hour)))
                .left(px(0.0))
                .right(px(0.0))
                .h(px(HOUR_PX))
                .border_b(px(1.0))
                .border_b_color(move || nsh_base::theme().border)
                .on_press(move || ui.open_form(form_at(hour * 60, hour * 60 + 60)))
                .child(
                    text(format!("{hour:02}"))
                        .text_xs()
                        .text_color(move || muted()),
                ),
        );
    }
    let lane_left = 40.0;
    for block in timeline::blocks(&snap.entries, day) {
        let Some(entry) = snap.entries.get(block.entry_index).cloned() else {
            continue;
        };
        let id = entry.id.clone();
        let width = 1.0 / block.columns.max(1) as f32;
        let top = HOUR_PX * f32::from(block.start) / 60.0;
        let height = (HOUR_PX * f32::from(block.end.saturating_sub(block.start)) / 60.0).max(20.0);
        canvas = canvas.child(
            view()
                .absolute()
                .top(px(top))
                .left(px(lane_left))
                .right(px(4.0))
                .h(px(height))
                .child(
                    button()
                        .absolute()
                        .top(px(0.0))
                        .h(px(height))
                        .left(relative(width * block.column as f32))
                        .w(relative(width))
                        .px_2()
                        .rounded_md()
                        .bg(move || nsh_base::theme().primary)
                        .on_press(move || {
                            ui.form.set(None);
                            ui.confirm_delete.set(false);
                            ui.selected.set(Some(id.clone()));
                        })
                        .child(
                            text(entry.title.clone())
                                .text_xs()
                                .number_of_lines(2u32)
                                .text_color(move || nsh_base::theme().primary_foreground),
                        ),
                ),
        );
    }
    if day == today() {
        let now = Local::now();
        let minute = now.hour() * 60 + now.minute();
        canvas = canvas.child(
            view()
                .absolute()
                .top(px(HOUR_PX * minute as f32 / 60.0))
                .left(px(lane_left))
                .right(px(0.0))
                .h(px(2.0))
                .bg(move || nsh_base::theme().destructive),
        );
    }
    let all_day: Vec<_> = calendar::entries_on(&snap.entries, day)
        .into_iter()
        .filter(|e| e.all_day)
        .cloned()
        .collect();
    let mut top = view();
    for entry in &all_day {
        top = top.child(entry_button(entry, day, ui));
    }
    view().child(top).child(canvas)
}

fn free_time(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let writable = ui.writable();
    let mut row = view().flex_row().flex_wrap().gap_2().px_4();
    if !(snap.loaded && !snap.loading && snap.error.is_none()) {
        return row;
    }
    let gaps = timeline::gaps(
        &snap.entries,
        snap.day,
        timeline::Interval {
            start: 480,
            end: 1200,
        },
    );
    if gaps.is_empty() {
        return row.child(text("No free time between 08:00 and 20:00.").text_sm());
    }
    for gap in gaps {
        let label = format!(
            "{:02}:{:02}–{:02}:{:02} free",
            gap.start / 60,
            gap.start % 60,
            gap.end / 60,
            gap.end % 60
        );
        row = row.child(chip(label, false, move || {
            if writable {
                ui.open_form(form_at(gap.start, gap.end.min(gap.start + 60)));
            }
        }));
    }
    row
}

fn form_card(ui: Ui, form: Form) -> impl IntoElement {
    let mut commitments = view().flex_row().gap_2();
    for (key, label) in calendar::COMMITMENTS {
        let on = form.commitment == key;
        commitments = commitments.child(chip(label, on, move || {
            ui.form.update(|f| {
                if let Some(f) = f {
                    f.commitment = key;
                }
            })
        }));
    }
    let timed = !form.all_day;
    let times = form.clone();
    view()
        .mx_4()
        .p_4()
        .gap_3()
        .rounded_lg()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
        .child(
            input()
                .value(move || ui.title.get())
                .on_change(move |value: &str| ui.title.set(value.to_owned()))
                .on_submit(move |_: &str| submit(ui))
                .placeholder("New entry")
                .px_3()
                .py_2()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().input)
                .rounded_lg(),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(chip("Timed", timed, move || set_all_day(ui, false)))
                .child(chip("All day", !timed, move || set_all_day(ui, true)))
                .child(
                    text(if timed {
                        format!("{}–{}", times.start, times.end)
                    } else {
                        "all day".to_owned()
                    })
                    .text_sm()
                    .text_color(move || muted()),
                ),
        )
        .child(commitments)
        .child(dynamic(move || {
            ui.form_error.get().map(|e| {
                text(e)
                    .text_xs()
                    .text_color(move || nsh_base::theme().destructive)
                    .into_element()
            })
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(chip("Save", true, move || submit(ui)))
                .child(chip("Cancel", false, move || ui.form.set(None))),
        )
}

fn set_all_day(ui: Ui, all_day: bool) {
    ui.form.update(|f| {
        if let Some(f) = f {
            f.all_day = all_day;
        }
    });
}

fn submit(ui: Ui) {
    let Some(form) = ui.form.get() else {
        return;
    };
    let draft = Draft {
        title: ui.title.get(),
        day: snap_day(ui),
        start: form.start,
        end: form.end,
        all_day: form.all_day,
        kind: "event".to_owned(),
        commitment: form.commitment.to_owned(),
    };
    if let Err(why) = save_entry(draft) {
        ui.form_error.set(Some(why));
    }
}

/// The sheet for one entry: what it is, quarter-hour moves, undo and a two-tap delete.
fn sheet(ui: Ui, entry: CalendarEntry) -> impl IntoElement {
    let snap = ui.snap.get();
    let writable = ui.writable();
    let id = entry.id.clone();
    let day = snap.day;
    let mut card = view()
        .mx_4()
        .p_4()
        .gap_3()
        .rounded_lg()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(
                    text(entry.title.clone())
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(chip("Close", false, move || ui.selected.set(None))),
        )
        .child(
            text(format!(
                "{} · {} · {}",
                calendar::time_label(&entry, day),
                calendar::kind_label(&entry.kind),
                entry.commitment
            ))
            .text_sm()
            .text_color(move || muted()),
        )
        .child(
            entry
                .location
                .clone()
                .filter(|l| !l.is_empty())
                .map(|l| text(l).text_sm()),
        )
        .child(
            entry
                .notes
                .clone()
                .filter(|n| !n.is_empty())
                .map(|n| text(n).text_sm()),
        );
    if !writable {
        return card.child(
            text("Read-only: this day comes from the Mac's copy.")
                .text_xs()
                .text_color(move || muted()),
        );
    }
    if !entry.all_day {
        let moves: [(&'static str, i64, bool); 4] = [
            ("−15 min", -15, false),
            ("+15 min", 15, false),
            ("Shorter", -15, true),
            ("Longer", 15, true),
        ];
        let mut row = view().flex_row().gap_2();
        for (label, minutes, resize) in moves {
            let entry = entry.clone();
            row = row.child(chip(label, false, move || {
                if let Err(why) = nudge(&entry, minutes, resize) {
                    ui.note.set(Some(why));
                }
            }));
        }
        card = card.child(row);
    }
    if let Some((before, after)) = snap.undo.clone().filter(|(_, after)| after.id == id) {
        card = card.child(chip("Undo time change", false, move || {
            change_time(after.clone(), before.clone(), true)
        }));
    }
    let delete_id = id.clone();
    card.child(dynamic(move || {
        if ui.confirm_delete.get() {
            let delete_id = delete_id.clone();
            view()
                .flex_row()
                .gap_2()
                .child(chip("Delete", true, move || {
                    delete_entry(delete_id.clone());
                    ui.selected.set(None);
                    ui.confirm_delete.set(false);
                }))
                .child(chip("Keep", false, move || ui.confirm_delete.set(false)))
                .into_element()
        } else {
            chip("Delete…", false, move || ui.confirm_delete.set(true)).into_element()
        }
    }))
}

#[component]
pub fn CalendarScreen() -> impl IntoElement {
    let initial = state().snap.clone();
    let seen = initial.saved;
    let ui = Ui {
        snap: signal(initial),
        grid: signal(false),
        free: signal(false),
        selected: signal(None::<String>),
        confirm_delete: signal(false),
        form: signal(None::<Form>),
        title: signal(String::new()),
        form_error: signal(None::<String>),
        note: signal(None::<String>),
    };
    let last_saved = signal(seen);
    let (tx, mut rx) = unbounded_channel::<Snapshot>();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(next) = rx.recv().await {
            if next.saved != last_saved.get() {
                last_saved.set(next.saved);
                ui.form.set(None);
            }
            ui.snap.set(next);
        }
    });
    load();
    let header = view()
        .px_4()
        .pt_4()
        .pb_2()
        .gap_2()
        .child(
            view()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    text(move || ui.snap.with(|s| s.day.format("%A, %-d %B").to_string()))
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(chip("Today", false, || open_day(today()))),
        )
        .child(dynamic(move || {
            ui.snap.with(|s| s.source.clone()).map(|line| {
                let error = ui.snap.with(|s| s.sync_error);
                text(line)
                    .text_xs()
                    .text_color(move || {
                        if error {
                            nsh_base::theme().destructive
                        } else {
                            muted()
                        }
                    })
                    .into_element()
            })
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(dynamic(move || {
                    chip(
                        if ui.grid.get() { "List" } else { "Grid" },
                        false,
                        move || ui.grid.update(|g| *g = !*g),
                    )
                    .into_element()
                }))
                .child(chip("Refresh", false, || emit(CalendarEvent::Refresh)))
                .child(dynamic(move || {
                    let on = ui.free.get();
                    chip(
                        if on {
                            "Hide free time"
                        } else {
                            "Show free time"
                        },
                        on,
                        move || ui.free.update(|f| *f = !*f),
                    )
                    .into_element()
                }))
                .child(dynamic(move || {
                    ui.writable().then(|| {
                        chip("+ New", true, move || ui.open_form(form_at(540, 600))).into_element()
                    })
                })),
        )
        .child(dynamic(move || week_strip(ui).into_element()))
        .child(dynamic(move || {
            let error = ui.snap.with(|s| s.error.clone()).or_else(|| ui.note.get());
            error.map(|e| {
                text(e)
                    .text_xs()
                    .text_color(move || nsh_base::theme().destructive)
                    .into_element()
            })
        }));
    view().flex_1().child(header).child(
        scroll().flex_1().child(
            view()
                .gap_3()
                .pb_6()
                .child(dynamic(move || {
                    ui.free.get().then(|| free_time(ui).into_element())
                }))
                .child(dynamic(move || {
                    ui.form.get().map(|form| form_card(ui, form).into_element())
                }))
                .child(dynamic(move || {
                    let id = ui.selected.get()?;
                    let entry = ui
                        .snap
                        .with(|s| s.entries.iter().find(|e| e.id == id).cloned())?;
                    Some(sheet(ui, entry).into_element())
                }))
                .child(dynamic(move || {
                    if ui.grid.get() {
                        grid(ui).into_element()
                    } else {
                        list(ui).into_element()
                    }
                })),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, title: &str, starts: &str, ends: &str) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts.to_owned(),
            ends_at: ends.to_owned(),
            all_day: false,
            commitment: "planned".to_owned(),
            id: id.to_owned(),
            kind: "event".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }
    }

    #[test]
    fn a_week_from_the_copy_is_read_only_and_says_so() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let mut snap = state().snap.clone();
        snap.day = day;
        snap.origin = Origin::Copy;
        assert!(!snap.writable(true), "the copy is never written");
        snap.origin = Origin::Store;
        assert!(snap.writable(true));
        assert!(!snap.writable(false));
        assert_eq!(form_at(480, 540).start, "08:00");
        assert_eq!(form_at(1410, 1500).end, "01:00");
    }

    #[test]
    fn the_screen_draws_a_day_opens_a_sheet_and_switches_to_the_grid() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        {
            let mut state = state();
            state.snap.day = day;
            state.snap.week = calendar::week_dates(day);
            state.snap.origin = Origin::Copy;
            state.snap.loaded = true;
            state.snap.entries = vec![entry(
                "a",
                "Dentist",
                "2026-11-10T09:00:00",
                "2026-11-10T09:30:00",
            )];
            publish(&mut state);
        }
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<CalendarScreen>())
        })
        .expect("the Calendar screen starts");
        app.wait_for("text:Tuesday, 10 November").unwrap();
        app.wait_for("text:Read-only copy from Mac").unwrap();
        app.press("text:Dentist").unwrap();
        app.wait_for("text:Read-only: this day comes from the Mac's copy.")
            .unwrap();
        app.press("text:Close").unwrap();
        app.press("text:Grid").unwrap();
        app.wait_for("text:List").unwrap();
        app.press("text:Show free time").unwrap();
        app.wait_for("text:08:00–09:00 free").unwrap();
    }
}
