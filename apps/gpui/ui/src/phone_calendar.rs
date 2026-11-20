//! Calendar replica, cached reader, and phone write commands.

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Local, NaiveDate};
use sjel_gpui_core::calendar::{self, timeline, CalendarSyncStatus, Draft};
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ShellError};
use tokio::sync::watch;

/// Where the week on screen was read from, which is also what the screen may do with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Origin {
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
pub(crate) struct Snapshot {
    pub(crate) day: NaiveDate,
    pub(crate) week: Vec<NaiveDate>,
    pub(crate) entries: Vec<CalendarEntry>,
    pub(crate) origin: Origin,
    pub(crate) loaded: bool,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) sync_error: bool,
    pub(crate) saving: bool,
    /// The last time change, so the sheet can put it back.
    pub(crate) undo: Option<(CalendarEntry, CalendarEntry)>,
    /// Bumped after each saved entry, so the screen closes its form.
    pub(crate) saved: u64,
}

impl Snapshot {
    pub(crate) fn writable(&self, has_store: bool) -> bool {
        self.loaded && self.origin == Origin::Store && has_store
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
    tx: watch::Sender<Snapshot>,
    host: Mutex<Option<Host>>,
}

pub(crate) fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let day = today();
        let snap = Snapshot {
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
        };
        Feed {
            state: Mutex::new(State {
                store: None,
                copy: None,
                missing: None,
                note: None,
                sync_status: None,
                generation: 0,
                snap: snap.clone(),
            }),
            tx: watch::channel(snap).0,
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
    feed().tx.send_replace(state.snap.clone());
}

pub(crate) fn emit(event: CalendarEvent) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(event);
    }
}

pub(crate) fn has_store() -> bool {
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
        if state.snap.saving || !state.snap.loaded || state.snap.origin != Origin::Store {
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

pub(crate) fn save_entry(draft: Draft) -> Result<(), String> {
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

pub(crate) fn delete_entry(id: String) {
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
pub(crate) fn change_time(before: CalendarEntry, after: CalendarEntry, undo: bool) {
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

pub(crate) fn nudge(entry: &CalendarEntry, minutes: i64, resize: bool) -> Result<(), String> {
    let after = timeline::shift_minutes(entry, minutes, resize).map_err(str::to_owned)?;
    change_time(entry.clone(), after, false);
    Ok(())
}

pub(crate) fn subscribe() -> watch::Receiver<Snapshot> {
    feed().tx.subscribe()
}

#[cfg(all(test, feature = "nsh", not(target_os = "ios")))]
pub(crate) fn test_snapshot(update: impl FnOnce(&mut Snapshot)) {
    let mut state = state();
    update(&mut state.snap);
    publish(&mut state);
}
