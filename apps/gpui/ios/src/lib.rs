//! Phone application state and bounded event routing, independent of GPUI.
use serde::Deserialize;
use sjel_gpui_core::{
    data::{Board, Repo},
    device::{self, DeviceClient, Node, Signer},
    interior::{RoomPlanLiveUpdate, RoomPlanLocalRecord, RoomPlanSource},
    mirror::{self, Mirror},
    notify as reminders,
    route::{self, FeedTab, Page, Target},
    shell::{self, NodeHealth},
    sync::{self, Report},
    terminal::Host,
};
use sjel_gpui_ui::{
    nsh_calendar::{self, CalendarEvent},
    nsh_edge::{self, Seat},
    nsh_today::{self, TodayEvent},
    AccentColor, AppearanceSettings, BackgroundTone, NotifyPrefs, SoundscapePrefs, TerminalFont,
    ThemeMode, WindowOpacity, TERMINAL_FONT_SIZES,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
mod audio;
mod events;
mod host;
mod identity;
mod launch;
mod nav;
mod notify;
mod nsh;
mod pairing;
mod runtime;
mod settings;
use host::*;
use launch::*;
use pairing::*;
const HEALTH: &str = "/api/sjel-status/health";
const JOIN_POLL: Duration = Duration::from_secs(2);
#[cfg(target_os = "ios")]
const LOCAL_NETWORK_DENIED: &str =
    "Sjel may not use the local network. Allow it in Settings, then reopen the app.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Today,
    Projects,
    Calendar,
    People,
    Interior,
    Inbox,
    Travel,
    Settings,
    Mail,
    Documents,
}
enum Event {
    Status(String),
    /// What the last Bonjour browse found. Empty is a normal answer, not an error. Only an iOS
    /// host browses (`sjel_ios_nodes`), so the macOS build of this crate never constructs it.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Found(Vec<Found>),
    /// The person tapped Join on one of them.
    Join(Found),
    /// A pairing code was scanned with the camera (`host/PairScanner.swift`), carrying what the
    /// Mac's Setup page drew: address, certificate pin and one-time code in one string.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Scanned(String),
    /// A claim or a join ended without a pairing, and why. Frees the Scan and Join buttons.
    PairFailed(String),
    /// The camera could not be opened, and why, in the words the person needs.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    ScanFailed(String),
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    RoomplanResult(String),
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    RoomplanLive(RoomPlanLiveUpdate),
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    RoomplanUpload(String),
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    RoomplanArchive(Vec<RoomPlanLocalRecord>),
    Ready(Node, NodeHealth),
    /// The phone's replica after it was opened, read or written: its rows and queued count.
    Board(Board),
    /// The Projects page wrote the replica: the queued count moved and a sync may start.
    Wrote,
    /// The replica did not open, and why. Nothing typed can be kept until it does.
    StoreFailed(String),
    /// The Mac's read-only copy after an open or a fetch: the copy, or why there is none.
    Copy(Result<Arc<Mirror>, String>),
    /// A sync finished: what it moved, or why it could not run, plus the replica after it.
    Synced(Result<Report, String>, Board),
    /// Reminders were rebuilt from the replica: what to hold, and what the cap left out.
    Reminded(reminders::Plan),
    FareAlertsSeen(Vec<String>),
    /// What the notification centre holds, in the system's own count (`host/Notifications.swift`).
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    NotifyState(notify::State),
    /// A reminder was tapped: the `calendar:<day>` link it carried.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    NotifyOpen(String),
    /// The app came back to the foreground. A phone in a pocket moves through days without the app
    /// running, so this is the moment to re-read the replica and re-schedule.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Foreground,
    /// The app went to the background. The next terminal open asks for Face ID again.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Background,
    /// The answer to the Face ID ask before a terminal opens (`host/App.swift`).
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Unlocked(bool),
    /// A pinch on the page: its scale since it began, and whether it ended (`host/App.swift`).
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Pinch(f64, bool),
    /// Host or NSH tab navigation request.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    SelectTab(Tab),
    /// Host or NSH edge seat opening request.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    OpenEdge(Seat),
    /// What the NSH Today screen asked for (`sjel_gpui_ui::nsh_today`).
    Today(TodayEvent),
    /// An NSH screen asked for a newer copy of the Mac's data.
    RefreshCopy,
    /// A refresh or a write on the NSH Calendar screen (`sjel_gpui_ui::nsh_calendar`).
    Calendar(CalendarEvent),
    Map(sjel_gpui_ui::nsh_map::MapEvent),
    Interior(sjel_gpui_ui::nsh_interior::InteriorEvent),
    Retry,
    Zoom(i32),
    /// A press on the NSH Settings screen (`sjel_gpui_ui::nsh_settings`).
    Settings(sjel_gpui_ui::nsh_settings::SettingsAction),
}

#[derive(Clone)]
struct EventSender(tokio::sync::mpsc::Sender<Event>);
impl EventSender {
    fn post(&self, event: Event) {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(event)) = self.0.try_send(event) {
            let sender = self.0.clone();
            shell::spawn(async move {
                let _ = sender.send(event).await;
            });
        }
    }
}
struct AppState {
    status: String,
    synced: Option<String>,
    rows: Vec<(String, bool)>,
    found: Vec<Found>,
    joining: bool,
    board: Option<Board>,
    copy: Option<Arc<Mirror>>,
    copying: bool,
    client: Option<Arc<DeviceClient>>,
    syncing: bool,
    audio_started: bool,
    notify_prefs: NotifyPrefs,
    notify_state: notify::State,
    notify_plan: reminders::Plan,
    roomplan_status: String,
    repo: Option<Repo>,
    appearance: AppearanceSettings,
    terminal_unlocked: bool,
    pinch_base: f64,
    events: EventSender,
}

/// Starts the application once, before native browse and notification callbacks arrive.
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::channel(128);
        let events = EventSender(tx);
        let _ = EVENTS.set(events.clone());
        let mut prefs = NotifyPrefs::load();
        if notify::launch_enabled() && !prefs.enabled {
            prefs.enabled = true;
            notify::authorize();
        }
        let state = AppState {
            status: "Opening this phone's records…".into(),
            synced: None,
            rows: Vec::new(),
            found: Vec::new(),
            joining: false,
            board: None,
            copy: None,
            copying: false,
            client: None,
            syncing: false,
            audio_started: false,
            notify_prefs: prefs,
            notify_state: notify::State::default(),
            notify_plan: reminders::Plan::default(),
            roomplan_status: "Room scans stay on this iPhone until they are synced.".into(),
            repo: None,
            appearance: AppearanceSettings::load(),
            terminal_unlocked: false,
            pinch_base: 1.,
            events: events.clone(),
        };
        nsh_calendar::set_host(|event| announce(Event::Calendar(event)));
        nsh_calendar::open_day(launch_day());
        sjel_gpui_ui::nsh_map::set_host(|event| announce(Event::Map(event)));
        sjel_gpui_ui::nsh_projects::set_retry(|| announce(Event::Retry));
        sjel_gpui_ui::nsh_interior::set_host(|event| announce(Event::Interior(event)));
        sjel_gpui_ui::nsh_terminal::set_unlock(unlock);
        sjel_gpui_ui::nsh_terminal::set_zoom(|steps| announce(Event::Zoom(steps)));
        nsh_edge::set_host(|seat| announce(Event::OpenEdge(seat)));
        nsh_today::set_host(|event| announce(Event::Today(event)));
        sjel_gpui_ui::nsh_settings::set_host(|action| announce(Event::Settings(action)));
        sjel_gpui_ui::nsh_people::set_refresh(|| announce(Event::RefreshCopy));
        sjel_gpui_ui::nsh_capture::set_wrote(|| announce(Event::Wrote));
        if let Some(day) = frozen_day() {
            nsh_today::set_day(day);
        }
        sjel_gpui_ui::nsh_people::set_register(launch_register());
        if let Some(name) = launch_person() {
            sjel_gpui_ui::nsh_people::open_named(&name);
        }
        let (control, visual) = audio::parts();
        sjel_gpui_ui::nsh_soundscape::set_parts(control, visual, SoundscapePrefs::load());
        let initial = launch_tab();
        let sidebar = sjel_gpui_ui::SidebarPrefs::load();
        let initial_edge = if launch_edge_off() {
            None
        } else {
            launch_edge().or_else(|| {
                sidebar
                    .open
                    .then(|| Seat::from_key(&sidebar.seat))
                    .flatten()
            })
        };
        let startup = events.clone();
        shell::spawn(async move {
            let mut state = state;
            state.select_tab(initial);
            if std::env::var("SJEL_TAB").as_deref() == Ok("map") {
                sjel_gpui_ui::nsh_travel::open(sjel_gpui_core::travel::TravelRoute::Map);
            }
            if let Some(seat) = initial_edge {
                nsh_edge::open(seat);
            }
            state.publish();
            while let Some(event) = rx.recv().await {
                state.handle(event);
                state.publish();
            }
        });
        let pair_events = startup.clone();
        shell::spawn(async move {
            if let Err(error) = load(
                std::env::var("SJEL_PAIR").ok(),
                std::env::var("SJEL_JOIN").ok(),
                &pair_events,
            )
            .await
            {
                pair_events.post(Event::Status(error));
            }
        });
        let store_events = startup.clone();
        shell::spawn(async move {
            let result = match Board::default_path() {
                Some(path) => Board::open(path).await.map_err(|e| e.to_string()),
                None => Err("No application storage directory is available.".into()),
            };
            store_events.post(match result {
                Ok(board) => Event::Board(board),
                Err(error) => {
                    Event::StoreFailed(format!("Notes and tasks cannot be saved: {error}"))
                }
            });
        });
        shell::spawn(async move {
            let result = tokio::task::spawn_blocking(open_copy)
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            startup.post(Event::Copy(result));
        });
        load_roomplan_status();
        load_roomplan_archive();
        notify::ready();
    });
}

impl AppState {
    fn publish(&self) {
        sjel_gpui_ui::nsh_settings::set_state(self.settings_state());
        let pending = self.board.as_ref().map_or(0, Board::pending);
        sjel_gpui_ui::nsh_projects::set_sync(pending, self.synced.as_deref());
        let mut reminders = Vec::new();
        match notify::disabled_reason() {
            Some(reason) => reminders.push(reason.to_owned()),
            None if !self.notify_prefs.enabled => reminders.push("Reminders are off".into()),
            None => {
                reminders.push(self.notify_state.authorization.phrase().into());
                for entry in self.notify_plan.reminders.iter().take(3) {
                    reminders.push(reminders::when(
                        entry.at,
                        &entry.title,
                        chrono::Local::now(),
                    ));
                }
            }
        }
        nsh_edge::set_activity(nsh_edge::Activity {
            connection: vec![
                self.status.clone(),
                self.synced
                    .clone()
                    .unwrap_or_else(|| "Nothing has moved this launch.".into()),
                format!("{pending} queued for the Mac"),
            ],
            reminders,
        });
        sjel_gpui_ui::nsh_interior::set_status(self.roomplan_status.clone());
        sjel_gpui_ui::nsh_terminal::set_appearance(self.appearance);
        #[cfg(target_os = "ios")]
        nsh::apply_appearance(&self.appearance);
    }
}
