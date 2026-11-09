//! Today, the phone's landing page, drawn by NSH (`docs/plans/gpui.md` §11).
//!
//! The host owns the Mac's copy, the replica and the sync. It hands each of them over with the
//! setters here, as it did to the GPUI page. This module reads the day from the copy, reduces it
//! with `sjel_gpui_core::today::build`, and writes a ticked task into the replica. What the page
//! cannot do itself, such as opening Calendar or syncing, goes back to the host as a
//! [`TodayEvent`].

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Local, NaiveDate};
use nsh_base::{button, checkbox};
use nsh_core::prelude::*;
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ProjectRecord};
use sjel_gpui_core::today::{self, Entry, Project, Spot, Task, Today as DayView};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

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
}

/// Everything the screen draws, rebuilt whenever a source moves.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    view: Option<DayView>,
    headline: String,
    source: String,
    note: Option<String>,
    travel: Option<(String, bool)>,
    show_pair: bool,
    writable: bool,
    busy: Option<String>,
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
    snapshot: Mutex<Snapshot>,
    tx: Mutex<Option<UnboundedSender<Snapshot>>>,
    host: Mutex<Option<Host>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        state: Mutex::new(State::default()),
        snapshot: Mutex::new(Snapshot::default()),
        tx: Mutex::new(None),
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
    let snap = snapshot_of(state);
    *feed().snapshot.lock().unwrap_or_else(|e| e.into_inner()) = snap.clone();
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(snap);
    }
}

fn emit(event: TodayEvent) {
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
fn toggle(path: String, done: bool) {
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

fn due_label(task: &Task) -> Option<String> {
    if task.overdue {
        return Some("Overdue".to_owned());
    }
    task.due.as_deref().map(|due| {
        sjel_gpui_core::calendar::parse_day(due)
            .map(|day| day.format("%-d %b").to_string())
            .unwrap_or_else(|| due.to_owned())
    })
}

fn spot_label(spot: Spot) -> Option<&'static str> {
    match spot {
        Spot::Now => Some("Now"),
        Spot::Next => Some("Next"),
        Spot::Past | Spot::Later => None,
    }
}

fn rest_line(rest: usize) -> Option<String> {
    (rest > 0).then(|| format!("{rest} more in Projects"))
}

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn heading(label: &'static str) -> impl IntoElement {
    text(label)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || muted())
}

fn card() -> View {
    view()
        .rounded_lg()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
}

fn pill(label: &'static str, event: TodayEvent) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .rounded_full()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .accessibility_label(label)
        .on_press(move || emit(event.clone()))
        .child(text(label).text_sm())
}

fn entry_row(entry: Entry, day: NaiveDate) -> impl IntoElement {
    let mark = spot_label(entry.spot);
    button()
        .px_4()
        .py_3()
        .gap_1()
        .on_press(move || emit(TodayEvent::OpenDay(day)))
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(
                    text(entry.title.clone())
                        .text_base()
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(mark.map(|m| text(m).text_xs().font_weight(FontWeight::SEMIBOLD))),
        )
        .child(
            text(format!("{} · {}", entry.when, entry.where_))
                .text_xs()
                .text_color(move || muted()),
        )
}

fn task_row(task: Task, writable: bool) -> impl IntoElement {
    let path = task.path.clone();
    let due = due_label(&task);
    let overdue = task.overdue;
    let second = (!task.project.is_empty()).then(|| task.project.clone());
    view()
        .flex_row()
        .items_center()
        .px_4()
        .py_3()
        .gap_3()
        .child(
            checkbox()
                .checked(false)
                .indicator_class("w-6 h-6 rounded-md border-2 border-border disabled:opacity-40")
                .disabled(!writable)
                .accessibility_label(format!("Done: {}", task.title))
                .hit_slop(EdgeInsets::all(px(10.0)))
                .on_change(move |done: &bool| toggle(path.clone(), *done)),
        )
        .child(
            view()
                .flex_1()
                .gap_1()
                .child(text(task.title.clone()).text_base())
                .child(second.map(|s| text(s).text_xs().text_color(move || muted()))),
        )
        .child(due.map(move |label| {
            text(label).text_xs().text_color(move || {
                if overdue {
                    nsh_base::theme().destructive
                } else {
                    muted()
                }
            })
        }))
        .child(text(format!("P{}", task.priority)).text_xs())
}

fn project_row(project: Project) -> impl IntoElement {
    let name = project.name.clone();
    button()
        .px_4()
        .py_3()
        .gap_1()
        .on_press(move || emit(TodayEvent::OpenProject(name.clone())))
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(text(project.title.clone()).text_base())
                .child(text("→").text_color(move || muted())),
        )
        .child(text(project.line()).text_xs().text_color(move || muted()))
}

fn section<T: IntoElement>(
    label: &'static str,
    empty: &'static str,
    rows: Vec<T>,
    extra: Option<String>,
) -> impl IntoElement {
    let body = if rows.is_empty() {
        card()
            .px_4()
            .py_3()
            .child(text(empty).text_sm().text_color(move || muted()))
    } else {
        let mut list = card();
        for row in rows {
            list = list.child(row);
        }
        list
    };
    view()
        .gap_2()
        .child(heading(label))
        .child(body)
        .child(extra.map(|line| text(line).text_xs().text_color(move || muted())))
}

fn body(snap: Snapshot) -> impl IntoElement {
    let Some(view_model) = snap.view.clone() else {
        return view().child(text("Reading today…").text_sm());
    };
    let day = view_model.day;
    let entries: Vec<_> = view_model
        .entries
        .iter()
        .cloned()
        .map(|e| entry_row(e, day))
        .collect();
    let tasks: Vec<_> = view_model
        .focus
        .iter()
        .cloned()
        .map(|t| task_row(t, snap.writable))
        .collect();
    let projects: Vec<_> = view_model
        .projects
        .iter()
        .cloned()
        .map(project_row)
        .collect();
    view()
        .gap_5()
        .child(section(
            "THE DAY",
            "Nothing on the calendar today.",
            entries,
            None,
        ))
        .child(
            view()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(heading("UP NEXT"))
                        .child(
                            button()
                                .px_2()
                                .py_1()
                                .on_press(|| emit(TodayEvent::Refresh))
                                .child(text("Refresh").text_sm()),
                        ),
                )
                .child(section(
                    "",
                    "Nothing left to do.",
                    tasks,
                    rest_line(view_model.rest),
                )),
        )
        .child(section("PROJECTS", "No projects yet.", projects, None))
}

fn header(snap: Signal<Snapshot>) -> impl IntoElement {
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .justify_between()
                .items_start()
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(heading("TODAY"))
                        .child(
                            button()
                                .on_press(move || {
                                    if let Some(v) = snap.with(|s| s.view.clone()) {
                                        emit(TodayEvent::OpenDay(v.day));
                                    }
                                })
                                .child(
                                    text(move || snap.with(|s| s.headline.clone()))
                                        .text_2xl()
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            text(move || {
                                snap.with(|s| {
                                    s.view.as_ref().map(DayView::summary).unwrap_or_default()
                                })
                            })
                            .text_sm()
                            .text_color(move || muted()),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(pill("♪", TodayEvent::Soundscape))
                        .child(pill("≡", TodayEvent::Activity))
                        .child(pill("⚙", TodayEvent::Settings)),
                ),
        )
        .child(
            text(move || snap.with(|s| s.source.clone()))
                .text_xs()
                .text_color(move || muted()),
        )
        .child(dynamic(move || {
            snap.with(|s| s.note.clone())
                .map(|note| text(note).text_xs().into_element())
        }))
        .child(dynamic(move || {
            snap.with(|s| s.travel.clone()).map(|(line, waiting)| {
                text(line)
                    .text_xs()
                    .text_color(move || {
                        if waiting {
                            nsh_base::theme().destructive
                        } else {
                            muted()
                        }
                    })
                    .into_element()
            })
        }))
        .child(dynamic(move || {
            snap.with(|s| s.show_pair).then(|| {
                button()
                    .py_3()
                    .rounded_lg()
                    .items_center()
                    .bg(move || nsh_base::theme().primary)
                    .on_press(|| emit(TodayEvent::Settings))
                    .child(
                        text("Pair this phone with your Mac")
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || nsh_base::theme().primary_foreground),
                    )
                    .into_element()
            })
        }))
}

fn capture_bar() -> impl IntoElement {
    let shortcut = |label: &'static str, event: TodayEvent| {
        button()
            .flex_1()
            .py_3()
            .items_center()
            .rounded_lg()
            .border(px(1.0))
            .border_color(move || nsh_base::theme().border)
            .on_press(move || emit(event.clone()))
            .child(text(label).text_sm().font_weight(FontWeight::MEDIUM))
    };
    view()
        .gap_2()
        .child(
            button()
                .flex_row()
                .justify_between()
                .items_center()
                .px_4()
                .py_3()
                .rounded_lg()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().border)
                .on_press(|| emit(TodayEvent::Capture))
                .child(
                    text("✎ Capture note, link or task…")
                        .text_sm()
                        .text_color(move || muted()),
                )
                .child(text("+ New").text_xs().font_weight(FontWeight::MEDIUM)),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(shortcut("People & Places", TodayEvent::People))
                .child(shortcut("Documents", TodayEvent::Documents)),
        )
}

#[component]
pub fn TodayScreen() -> impl IntoElement {
    let initial = {
        let state = state();
        let snap = snapshot_of(&state);
        *feed().snapshot.lock().unwrap_or_else(|e| e.into_inner()) = snap.clone();
        snap
    };
    let snap = signal(initial);
    let (tx, mut rx) = unbounded_channel();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(next) = rx.recv().await {
            snap.set(next);
        }
    });
    scroll().flex_1().child(
        view()
            .px_4()
            .pt_4()
            .pb_6()
            .gap_4()
            .child(header(snap))
            .child(capture_bar())
            .child(dynamic(move || {
                snap.with(|s| s.busy.clone())
                    .map(|_| text("Saving…").text_xs().into_element())
            }))
            .child(dynamic(move || body(snap.get()).into_element())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) static TEST_STATE: Mutex<()> = Mutex::new(());

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
        let snap = feed().snapshot.lock().unwrap().clone();
        assert_eq!(
            snap.travel,
            Some((
                "4 waiting for the Mac · not connected, so changes stay here".to_owned(),
                true
            ))
        );
        set_copy(None, Some("No copy yet.".to_owned()));
        assert_eq!(feed().snapshot.lock().unwrap().source, "No copy yet.");
        assert_eq!(
            feed().snapshot.lock().unwrap().headline,
            "Saturday, 24 October"
        );
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
            ..EngineConfig::new(Root::of::<TodayScreen>())
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
