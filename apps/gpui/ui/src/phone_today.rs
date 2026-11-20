//! Phone Today state shared by renderers.

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Local, NaiveDate};
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ProjectRecord};
use sjel_gpui_core::today::{self, Spot, Task, Today as DayView};
use tokio::sync::watch;

/// What the page asks its host to do. The host owns the network, the copy on disk and the sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodayEvent {
    /// The person tapped the headline or an entry: show that day in Calendar.
    OpenDay(NaiveDate),
    /// The person tapped a project: open it in Projects.
    OpenProject(String),
    /// The person tapped the gear, or the banner that says this phone is not paired.
    Settings,
    /// The person tapped the note: the soundscape seat of the right edge.
    Soundscape,
    /// The person tapped the list: the activity seat of the right edge.
    Activity,
    /// The person asked for newer data.
    Refresh,
    /// A task was written here and waits for the Mac, so the host may sync now.
    Wrote,
    /// The person tapped quick capture.
    Capture,
    /// The person tapped the People & Places shortcut.
    People,
    /// The person tapped the Documents shortcut.
    Documents,
    /// Opens all project hubs, including on an empty replica.
    Projects,
    /// Opens the phone's scan archive and room review.
    Interior,
    /// Opens the phone's Finance overview, transactions, and candidate review.
    Finance,
    /// Opens the phone's Inventory & Wishlist lookup and comparison view.
    Inventory,
}

/// Everything the screen draws, rebuilt whenever a source moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) view: Option<DayView>,
    pub(crate) headline: String,
    pub(crate) source: String,
    pub(crate) note: Option<String>,
    pub(crate) travel: Option<(String, bool)>,
    pub(crate) show_pair: bool,
    pub(crate) writable: bool,
    pub(crate) busy: Option<String>,
}

#[derive(Default)]
struct State {
    copy: Option<Arc<Mirror>>,
    missing: Option<String>,
    note: Option<String>,
    store: Option<Repo>,
    records: Vec<ProjectRecord>,
    entries: Vec<CalendarEntry>,
    error: Option<String>,
    pending: usize,
    paired: bool,
    frozen: Option<NaiveDate>,
    generation: u64,
    busy: Option<String>,
}

type Host = Box<dyn Fn(TodayEvent) + Send>;

struct Feed {
    state: Mutex<State>,
    snapshot: watch::Sender<Snapshot>,
    host: Mutex<Option<Host>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        state: Mutex::new(State::default()),
        snapshot: watch::channel(Snapshot::default()).0,
        host: Mutex::new(None),
    })
}

fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}

fn day_of(state: &State) -> NaiveDate {
    state.frozen.unwrap_or_else(|| Local::now().date_naive())
}

fn snapshot_of(state: &State) -> Snapshot {
    let day = day_of(state);
    let now = match state.frozen {
        Some(_) => None,
        None => Some(Local::now().format("%H:%M").to_string()),
    };
    let view = today::build(day, &state.entries, &state.records, now.as_deref());
    let source = match (&state.error, &state.copy, &state.missing) {
        (Some(error), _, _) => format!("The day could not be read: {error}"),
        (_, Some(copy), _) => format!(
            "Calendar from the Mac's copy, {}",
            copy.manifest().built_label()
        ),
        (_, None, Some(missing)) => missing.clone(),
        (_, None, None) => "Looking for the Mac's copy…".to_owned(),
    };
    let mut travel = Vec::new();
    if state.pending > 0 {
        travel.push(format!("{} waiting for the Mac", state.pending));
    }
    if !state.paired {
        travel.push("not connected, so changes stay here".to_owned());
    }
    Snapshot {
        headline: view.headline(),
        view: Some(view),
        source,
        note: state.note.clone(),
        travel: (!travel.is_empty()).then(|| (travel.join(" · "), state.pending > 0)),
        show_pair: !state.paired && state.copy.is_none(),
        writable: state.store.is_some(),
        busy: state.busy.clone(),
    }
}

/// Rebuilds the snapshot from `state` and hands it to the screen.
fn publish(state: &State) {
    feed().snapshot.send_replace(snapshot_of(state));
}

pub(crate) fn subscribe() -> watch::Receiver<Snapshot> {
    publish(&state());
    feed().snapshot.subscribe()
}

pub(crate) fn emit(event: TodayEvent) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(event);
    }
}

/// What the host does when the page asks for something it does not own.
pub fn set_host(host: impl Fn(TodayEvent) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}

/// Fixes the day on screen (`SJEL_DAY`), so a screenshot of another day reads no clock.
pub fn set_day(day: NaiveDate) {
    let mut state = state();
    state.frozen = Some(day);
    publish(&state);
}

/// Hands the page the Mac's copy, or the reason it is not there, and reads the day from it.
pub fn set_copy(copy: Option<Arc<Mirror>>, missing: Option<String>) {
    {
        let mut state = state();
        state.copy = copy;
        state.missing = missing;
        state.error = None;
        publish(&state);
    }
    load();
}

/// Hands the page the phone's replica, so a task can be ticked off here.
pub fn set_store(store: Option<Repo>) {
    let mut state = state();
    state.store = store;
    publish(&state);
}

/// The replica's records, pushed by the host after every read and every sync.
pub fn set_records(records: Vec<ProjectRecord>) {
    let mut state = state();
    state.records = records;
    publish(&state);
}

/// How many writes wait for the Mac, and whether this launch has reached it.
pub fn set_travel(pending: usize, paired: bool) {
    let mut state = state();
    state.pending = pending;
    state.paired = paired;
    publish(&state);
}

/// A sentence from the host about the data on screen.
pub fn set_note(note: Option<String>) {
    let mut state = state();
    state.note = note;
    publish(&state);
}

/// Reads the day's entries from the copy. A newer read makes an older answer stale.
pub fn load() {
    let (copy, day, generation) = {
        let mut state = state();
        let Some(copy) = state.copy.clone() else {
            return;
        };
        state.generation += 1;
        (copy, day_of(&state), state.generation)
    };
    let from = day.format("%Y-%m-%d").to_string();
    let to = today::step(day, 1).format("%Y-%m-%d").to_string();
    shell::spawn(async move {
        let answer = copy.calendar(&from, &to).await;
        let mut state = state();
        if state.generation != generation {
            return;
        }
        match answer {
            Ok(entries) => {
                state.entries = entries;
                state.error = None;
            }
            Err(error) => state.error = Some(error),
        }
        publish(&state);
    });
}

/// Ticks a task in the replica, where the next sync carries it to the Mac. The row moves at
/// once and moves back if the write fails.
pub(crate) fn toggle(path: String, done: bool) {
    let repo = {
        let mut state = state();
        let Some(repo) = state.store.clone() else {
            return;
        };
        if state.busy.is_some() {
            return;
        }
        let Some(record) = state.records.iter_mut().find(|r| r.path == path) else {
            return;
        };
        record.done = done;
        state.busy = Some(path.clone());
        publish(&state);
        repo
    };
    shell::spawn(async move {
        let answer = repo.set_task_done(&path, done).await;
        let landed = {
            let mut state = state();
            state.busy = None;
            let landed = match answer {
                Ok(task) => {
                    if let Some(record) = state.records.iter_mut().find(|r| r.path == task.id) {
                        record.done = task.done;
                        record.due = task.due.clone();
                        record.priority = task.priority;
                    }
                    true
                }
                Err(error) => {
                    if let Some(record) = state.records.iter_mut().find(|r| r.path == path) {
                        record.done = !done;
                    }
                    state.error = Some(format!("The task was not saved: {error}"));
                    false
                }
            };
            publish(&state);
            landed
        };
        if landed {
            emit(TodayEvent::Wrote);
        }
    });
}

pub(crate) fn due_label(task: &Task) -> Option<String> {
    if task.overdue {
        return Some("Overdue".to_owned());
    }
    task.due.as_deref().map(|due| {
        sjel_gpui_core::calendar::parse_day(due)
            .map(|day| day.format("%-d %b").to_string())
            .unwrap_or_else(|| due.to_owned())
    })
}

pub(crate) fn spot_label(spot: Spot) -> Option<&'static str> {
    match spot {
        Spot::Now => Some("Now"),
        Spot::Next => Some("Next"),
        Spot::Past | Spot::Later => None,
    }
}

pub(crate) fn rest_line(rest: usize) -> Option<String> {
    (rest > 0).then(|| format!("{rest} more in Projects"))
}

#[cfg(all(feature = "gpui", target_os = "ios"))]
mod view;
#[cfg(all(feature = "gpui", target_os = "ios"))]
pub use view::PhoneToday;

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) static TEST_STATE: Mutex<()> = Mutex::new(());

    #[cfg(all(feature = "nsh", not(target_os = "ios")))]
    fn record(path: &str, kind: &str, title: &str, done: bool) -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: path.to_owned(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            content: String::new(),
            done,
            due: None,
            priority: 1,
        }
    }

    fn reset(day: NaiveDate) {
        *state() = State {
            frozen: Some(day),
            ..State::default()
        };
    }

    #[test]
    fn the_source_line_and_travel_line_follow_the_host() {
        let _guard = TEST_STATE.lock().unwrap();
        reset(NaiveDate::from_ymd_opt(2026, 10, 24).unwrap());
        let snap = snapshot_of(&state());
        assert_eq!(snap.source, "Looking for the Mac's copy…");
        assert_eq!(
            snap.travel,
            Some(("not connected, so changes stay here".to_owned(), false))
        );
        assert!(snap.show_pair);
        assert!(!snap.writable);
        set_travel(4, false);
        let snap = feed().snapshot.borrow().clone();
        assert_eq!(
            snap.travel,
            Some((
                "4 waiting for the Mac · not connected, so changes stay here".to_owned(),
                true
            ))
        );
        set_copy(None, Some("No copy yet.".to_owned()));
        assert_eq!(feed().snapshot.borrow().source, "No copy yet.");
        assert_eq!(feed().snapshot.borrow().headline, "Saturday, 24 October");
    }

    #[test]
    fn a_frozen_day_reads_no_clock_so_the_rows_cannot_move() {
        let _guard = TEST_STATE.lock().unwrap();
        reset(NaiveDate::from_ymd_opt(2026, 10, 24).unwrap());
        let mut state = state();
        state.entries = vec![CalendarEntry {
            title: "Standup".to_owned(),
            starts_at: "2026-10-24T09:00:00".to_owned(),
            ends_at: "2026-10-24T09:15:00".to_owned(),
            all_day: false,
            commitment: "planned".to_owned(),
            id: "a".to_owned(),
            kind: "event".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }];
        let view = snapshot_of(&state).view.expect("a view");
        assert_eq!(view.entries[0].spot, Spot::Next);
        assert!(!view.entries.iter().any(|row| row.spot == Spot::Now));
    }

    #[cfg(all(feature = "nsh", not(target_os = "ios")))]
    #[test]
    fn the_screen_lists_tasks_and_reports_presses_to_the_host() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let _guard = TEST_STATE.lock().unwrap();
        reset(NaiveDate::from_ymd_opt(2026, 10, 24).unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        set_host(move |event| sink.lock().unwrap().push(event));
        set_records(vec![
            record(
                "Life/tasks/a.md",
                "task",
                "Paper aus der Bachelorarbeit",
                false,
            ),
            record("Life/tasks/b.md", "task", "Buy 8TB drive", false),
        ]);
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<crate::nsh_today::TodayScreen>())
        })
        .expect("the Today screen starts");
        app.wait_for("text:Saturday, 24 October").unwrap();
        app.wait_for("text:Buy 8TB drive").unwrap();
        app.wait_for("text:Nothing on the calendar today.").unwrap();
        app.press("text:People & Places").unwrap();
        app.press("text:Refresh").unwrap();
        set_note(Some("Copy is a day old.".to_owned()));
        app.wait_for("text:Copy is a day old.").unwrap();
        let events = seen.lock().unwrap().clone();
        assert_eq!(events, vec![TodayEvent::People, TodayEvent::Refresh]);
    }
}
