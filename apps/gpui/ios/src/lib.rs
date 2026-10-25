//! The iOS app's GPUI content. UIKit owns the window, the navigation bar and the safe areas
//! (`host/App.swift`); GPUI draws inside them (`../README.md`, "GPUI for every client").
//!
//! The app is a paired device (`capabilities/devices/README.md`): it claims the one-time code
//! from the Mac's pairing QR code, then signs every request with its Keychain key
//! (`host/DeviceIdentity.swift`) over TLS pinned to the Mac's LAN listener.
//!
//! The Mac is found over Bonjour: `host/Discovery.swift` browses `_sjel._tcp`, whose TXT record
//! carries the host, port and certificate fingerprint, and hands every result to `sjel_ios_nodes`.
//! A scan needs no service resolution, so a found record is the whole address. The person taps
//! Join, asks to join, and allows the device in the dashboard's Devices panel.
//!
//! Two launch-environment paths stay as fallbacks for a host that cannot browse: `SJEL_JOIN=<mac
//! name> <port> <fingerprint>` asks to join (`ios/run-sim --join`), and `SJEL_PAIR` takes the QR
//! code's text and claims its one-time code.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use serde::Deserialize;

use gpui::{div, prelude::*, px, rgb, AnyElement, Context, MouseButton, SharedString, Window};
use sjel_gpui_core::agent::{Event as AgentEvent, Session as AgentSession, Turn as AgentTurn};
use sjel_gpui_core::data::Board;
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::device::{self, DeviceClient, Node, Signer};
use sjel_gpui_core::interior::{RoomPlanLiveUpdate, RoomPlanLocalRecord, RoomPlanSource};
use sjel_gpui_core::mirror::{self, Mirror};
use sjel_gpui_core::notify as reminders;
use sjel_gpui_core::shell::{self, NodeHealth};
use sjel_gpui_core::sync::{self, Report};
use sjel_gpui_core::terminal::Host;
use sjel_gpui_ui::calendar::{Calendar, CalendarEvent};
use sjel_gpui_ui::inbox::{Inbox, InboxEvent};
use sjel_gpui_ui::interior::RoomPlanHistory;
use sjel_gpui_ui::people::{People, PeopleEvent};
use sjel_gpui_ui::projects::{Projects, ProjectsEvent};
use sjel_gpui_ui::sidebar::{sidebar_frame, sidebar_scrim, Seat, Sidebar};
use sjel_gpui_ui::soundscape::Soundscape;
use sjel_gpui_ui::terminal::{TerminalEvent, TerminalView};
use sjel_gpui_ui::today::{Today, TodayEvent};
use sjel_gpui_ui::{
    mobile_tab_bar, AccentColor, AppearanceSettings, BackgroundTone, Density, NavItem, NotifyPrefs,
    Palette, SoundscapePrefs, ThemeMode,
};

mod audio;
mod identity;
mod notify;

const HEALTH: &str = "/api/sjel-status/health";
/// How often a waiting join asks whether the owner decided. A request expires after ten minutes
/// (`capabilities/devices/README.md`), so the wait ends there at the latest.
const JOIN_POLL: Duration = Duration::from_secs(2);
/// The size a new terminal starts at, before the grid measures itself and resizes the pty.
const TERMINAL_COLS: usize = 48;
const TERMINAL_ROWS: usize = 28;
#[cfg(target_os = "ios")]
const LOCAL_NETWORK_DENIED: &str =
    "Sjel may not use the local network. Allow it in Settings, then reopen the app.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The landing page: today's entries off the copy, the tasks and projects off the replica.
    Today,
    /// Every record the phone holds, reached from a project on Today rather than from the bar.
    Projects,
    /// The day's entries, read from the Mac's read-only copy of its data.
    Calendar,
    /// The contacts and the places they have been, read from the same copy.
    People,
    /// RoomPlan scans saved on this phone and reviewed through the paired interior capability.
    Interior,
    /// One field for whatever is in hand, kept on the phone first and queued for the Mac.
    Inbox,
    Travel,
    /// Pairing with a Mac: offered here, never required (research/offline-on-the-phone.md).
    Settings,
}

enum Event {
    Status(String),
    AgentSessions(Vec<AgentSession>, Option<String>),
    AgentPrompted(AgentSession),
    AgentAttached(String, Vec<String>, u64),
    AgentLine(String, AgentEvent),
    AgentAction(String),
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
}

#[derive(Clone, Copy)]
enum AgentControl {
    Approve(bool, bool),
    Stop,
}

/// A Mac that `host/Discovery.swift` found over Bonjour, from the TXT record the LAN listener
/// advertises (`capabilities/sjel-status/src/lan.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct Found {
    /// The Bonjour instance name, "Sjel on <mac>".
    pub name: String,
    /// The Mac's name without `.local`; the listener's certificate carries `<host>.local`.
    pub host: String,
    pub port: u16,
    /// SHA-256 of the listener's certificate, 64 lowercase hex characters.
    pub fp: String,
}

#[derive(Deserialize)]
struct RoomplanUploadAck {
    draft_id: String,
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_mark_roomplan_uploaded(text: *const std::ffi::c_char);
    fn sjel_ios_preview_roomplan(text: *const std::ffi::c_char);
    fn sjel_ios_cache_preview_roomplan(
        draft_id: *const std::ffi::c_char,
        draft_json: *const std::ffi::c_char,
        bytes: *const u8,
        length: usize,
    );
}

impl Found {
    /// `<mac name> <port> <fingerprint>`, the text `device::parse_join_target` reads.
    fn join_target(&self) -> String {
        format!("{} {} {}", self.host, self.port, self.fp)
    }

    fn address(&self) -> String {
        format!("{}.local:{}", self.host, self.port)
    }
}

/// What one browse pass reported: the nodes, and whether iOS refused local-network access.
#[cfg(target_os = "ios")]
#[derive(Deserialize)]
struct Browse {
    nodes: Vec<Found>,
    denied: bool,
}

/// The view's event sender, so the C entry point below can reach the view it feeds.
static EVENTS: OnceLock<Mutex<Option<UnboundedSender<Event>>>> = OnceLock::new();

// Opens the camera (`host/PairScanner.swift`). The screen is Swift's because a camera preview is an
// `AVCaptureVideoPreviewLayer`, and GPUI draws its own pixels; the answer comes back through
// `sjel_ios_scanned_pairing`.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbol is unsafe; the host owns its definition.
unsafe extern "C" {
    fn sjel_ios_present_pair_scanner();
    fn sjel_ios_present_roomplan();
    fn sjel_ios_import_legacy_roomplan();
    fn sjel_ios_load_roomplan_status();
    fn sjel_ios_load_roomplan_upload();
    fn sjel_ios_load_roomplan_archive();
    // Face ID before a terminal on the Mac opens; the answer is `sjel_ios_unlocked`.
    fn sjel_ios_unlock();
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn unlock() {
    // SAFETY: no arguments; the host answers on the main queue through `sjel_ios_unlocked`.
    unsafe { sjel_ios_unlock() };
}

#[cfg(not(target_os = "ios"))]
fn unlock() {}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn preview_roomplan(draft_id: &str) {
    if let Ok(text) = std::ffi::CString::new(draft_id) {
        // SAFETY: the Swift host copies this validated archive ID during the call.
        unsafe { sjel_ios_preview_roomplan(text.as_ptr()) };
    }
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn cache_preview_roomplan(draft_id: &str, draft_json: &str, bytes: Vec<u8>) {
    let (Ok(draft_id), Ok(draft_json)) = (
        std::ffi::CString::new(draft_id),
        std::ffi::CString::new(draft_json),
    ) else {
        return;
    };
    // SAFETY: Swift copies both strings and the byte slice before returning from this callback.
    unsafe {
        sjel_ios_cache_preview_roomplan(
            draft_id.as_ptr(),
            draft_json.as_ptr(),
            bytes.as_ptr(),
            bytes.len(),
        );
    }
}

/// Asks the host to open the scanner. Nothing happens on a build without a camera screen, which
/// is every non-iOS target of this crate.
fn present_pair_scanner() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call (`host/PairScanner.swift`).
    unsafe {
        sjel_ios_present_pair_scanner();
    }
}

fn present_roomplan() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host function takes no arguments and owns the presented capture controller.
    unsafe {
        sjel_ios_present_roomplan();
    }
}

#[cfg(target_os = "ios")]
fn import_legacy_roomplan() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host presents its local Files picker and retains the import coordinator.
    unsafe {
        sjel_ios_import_legacy_roomplan();
    }
}

fn load_roomplan_status() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its own app-support files and returns through the registered callback.
    unsafe {
        sjel_ios_load_roomplan_status();
    }
}

fn load_roomplan_upload() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its own app-support files and returns the upload body through FFI.
    unsafe {
        sjel_ios_load_roomplan_upload();
    }
}

fn load_roomplan_archive() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its app-support archive and returns JSON through the registered callback.
    unsafe {
        sjel_ios_load_roomplan_archive();
    }
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn mark_roomplan_uploaded(draft_id: &str) {
    if let Ok(text) = std::ffi::CString::new(draft_id) {
        // SAFETY: the host callback copies the NUL-terminated ID during this call.
        unsafe { sjel_ios_mark_roomplan_uploaded(text.as_ptr()) };
    }
}

#[cfg(not(target_os = "ios"))]
fn mark_roomplan_uploaded(_draft_id: &str) {}

/// Sends one event to the live view, if there is one. Called from the main queue.
fn announce(event: Event) {
    let sender = EVENTS.get().and_then(|slot| slot.lock().ok()?.clone());
    if let Some(sender) = sender {
        let _ = sender.unbounded_send(event);
    }
}

pub struct Capabilities {
    status: SharedString,
    /// What the last sync did, kept apart from `status` so the pairing line stays on screen.
    synced: Option<SharedString>,
    rows: Vec<(String, bool)>,
    /// Macs on this network that answered the last browse.
    found: Vec<Found>,
    /// True from the tap until the owner decides, so the button cannot be pressed twice.
    joining: bool,
    active_tab: Tab,
    travel: gpui::Entity<sjel_gpui_ui::travel::Travel>,
    /// The phone's own copy of the Mac's records and the queue beside it, or `None` until it
    /// opens. `Board` is the view model: it answers the rows to draw without this view filtering
    /// them, and it keeps the `TursoStore` the sync needs.
    board: Option<Board>,
    /// The Mac's Projects page over the replica (`sjel_gpui_ui::projects`), once the store opens.
    projects: Option<gpui::Entity<Projects>>,
    /// The landing page: the day the copy answered and the tasks the replica holds
    /// (`sjel_gpui_ui::today`). It reads both, so it exists before either has answered.
    today: gpui::Entity<Today>,
    /// The Calendar page over the Mac's read-only copy (`sjel_gpui_ui::calendar`).
    calendar: gpui::Entity<Calendar>,
    /// The capture field over the phone's own replica (`sjel_gpui_ui::inbox`).
    inbox: gpui::Entity<Inbox>,
    /// The soundscape drawer over the platform-free engine (`sjel_gpui_ui::soundscape`). The device
    /// is the host's (`host/Audio.swift`), started the first time the edge is opened on this seat.
    soundscape: gpui::Entity<Soundscape>,
    /// The right edge: one panel with seats (`sjel_gpui_ui::sidebar`), which owns whether it is
    /// showing and which seat it is on. The soundscape is one seat; the phone's own activity is the
    /// other, and the agent's step stream lands there when a client has one to stream.
    sidebar: Sidebar,
    agent_sessions: Vec<AgentSession>,
    agent_rows: Vec<String>,
    agent_selected: Option<String>,
    agent_last_seq: u64,
    agent_sessions_loading: bool,
    agent_error: Option<String>,
    agent_pending: Option<AgentEvent>,
    agent_input: Option<(gpui::Entity<InputState>, gpui::Subscription)>,
    agent_busy: bool,
    agent_text_streaming: bool,
    /// The terminal seat, kept while the app stays in the foreground: closing the edge leaves the
    /// stream attached. Going to the background drops it, so a reattach asks for Face ID.
    terminal: Option<(gpui::Entity<TerminalView>, gpui::Subscription)>,
    /// Face ID confirmed this foreground session (`docs/plans/agent.md` §6, Security).
    terminal_unlocked: bool,
    /// Why no terminal is showing, in the words the empty seat carries.
    terminal_note: Option<SharedString>,
    /// Shared Interior scan history and review, with local captures joined to Mac revisions.
    interior: gpui::Entity<RoomPlanHistory>,
    /// The People and Places registers over the same copy (`sjel_gpui_ui::people`). Read-only:
    /// the registry's writes are the operator's, so this tab draws and never offers a control that
    /// would have to be refused.
    people: gpui::Entity<People>,
    /// The copy last opened, kept so a refresh that fails cannot blank a page that is readable.
    copy: Option<Arc<Mirror>>,
    /// True while a copy is being fetched, so one tap cannot start a second fetch.
    copying: bool,
    /// Why the replica did not open, shown on the Projects page in place of a silent blank.
    store_error: Option<SharedString>,
    /// The paired Mac, once this launch has reached it. `None` means every sync rests: a phone
    /// that has not reached the Mac keeps its queue.
    client: Option<Arc<DeviceClient>>,
    /// True while a sync is in flight, so one tap cannot start a second.
    syncing: bool,
    /// True once this launch asked the host for the audio device, so it is asked once.
    audio_started: bool,
    /// What this phone remembers about its lock screen: the switch and the lead time
    /// (`ui/src/theme.rs`). What is *scheduled* is never read from here.
    notify_prefs: NotifyPrefs,
    /// The system's own answer: whether reminders may be shown, how many it holds, which is next.
    notify_state: notify::State,
    /// The last rebuild's list, for the "next" line and for the cap's own admission.
    notify_plan: reminders::Plan,
    /// Last native capture result; the draft and USDZ remain in app storage.
    roomplan_status: SharedString,
    /// This device's own store as the reminder rebuild reads it, kept from the row that also handed
    /// it to the calendar page.
    repo: Option<Repo>,
    appearance: AppearanceSettings,
    events: UnboundedSender<Event>,
}

impl Capabilities {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (events, mut received) = unbounded();
        // The C entry point reads this, so the browse can start before the view exists.
        let _ = EVENTS.set(Mutex::new(Some(events.clone())));
        load_roomplan_status();
        load_roomplan_archive();
        // The notification centre may already hold a tap that launched this launch. It is released
        // now that there is a channel to receive it, and the state comes back as an event.
        notify::ready();
        // `SJEL_NOTIFY=1` turns reminders on for a run that asks for them, the way `SJEL_SOUNDSCAPE`
        // opens the drawer: a card nobody switched on still has to be photographable.
        let notify_prefs = NotifyPrefs::load();
        if notify::launch_enabled() && !notify_prefs.enabled {
            notify::authorize();
        }
        let notify_prefs = NotifyPrefs {
            enabled: notify_prefs.enabled || notify::launch_enabled(),
            ..notify_prefs
        };
        let pair = std::env::var("SJEL_PAIR").ok();
        let join = std::env::var("SJEL_JOIN").ok();
        let failed = events.clone();
        let work = shell::run(async move {
            if let Err(error) = load(pair, join, &failed).await {
                let _ = failed.unbounded_send(Event::Status(error));
            }
        });

        // The phone's own replica, opened in the app container and read once. A phone keeps its
        // records whether or not the Mac answers; `Board` owns that copy and the queue beside it.
        // The phone's data is the real record (research/offline-on-the-phone.md, 2026-10-22), so
        // a store that cannot open is said on screen, and there is no in-memory stand-in that
        // would lose every note at the next launch.
        let store_events = events.clone();
        shell::spawn(async move {
            let opened = match Board::default_path() {
                Some(path) => Board::open(path).await.map_err(|e| e.to_string()),
                None => Err("this platform names no place for it".to_owned()),
            };
            let _ = store_events.unbounded_send(match opened {
                Ok(board) => Event::Board(board),
                Err(error) => Event::StoreFailed(format!(
                    "Notes and tasks cannot be saved on this phone: {error}"
                )),
            });
        });

        // The Calendar page, and the copy it reads. A copy already on disk is opened before the
        // Mac answers, so the calendar holds the last sync's data on a launch that never reaches
        // the Mac (`core/src/mirror.rs`).
        let calendar = cx.new(|_| Calendar::at(launch_day(), Density::Mobile));
        cx.subscribe(&calendar, |view, _, event, cx| match event {
            // Newer data: a sync when this phone has a store to sync, the Mac's copy otherwise.
            CalendarEvent::Refresh => {
                if view.board.is_some() {
                    view.maybe_sync(cx);
                } else {
                    view.refresh_copy(cx);
                }
            }
            // An entry landed in the replica: the queue moved, so the Mac can have it now.
            CalendarEvent::Wrote => view.maybe_sync(cx),
            CalendarEvent::Editing(open) => keyboard(*open),
        })
        .detach();
        // People and Places read the same copy, so one fetch feeds both pages. Refresh asks for a
        // newer copy: there is nothing to sync, because nothing here writes.
        let people = cx.new(|_| People::new(Density::Mobile));
        let interior = cx.new(|cx| {
            #[allow(unused_mut)]
            let mut history = RoomPlanHistory::new(None, cx);
            #[cfg(target_os = "ios")]
            history.set_preview_handler(Some(preview_roomplan));
            #[cfg(target_os = "ios")]
            history.set_import_handler(Some(import_legacy_roomplan));
            #[cfg(target_os = "ios")]
            history.set_cache_preview_handler(Some(cache_preview_roomplan));
            history
        });
        // The landing page reads the copy for the day and the replica for the tasks, so it is built
        // before either answers and says what it is missing until one does. A launch with
        // `SJEL_DAY` freezes the day, the way the calendar's does: a screenshot cannot be allowed to
        // disagree with itself a minute later.
        let today = cx.new(|_| match frozen_day() {
            Some(day) => Today::at(day, Density::Mobile),
            None => Today::new(Density::Mobile),
        });
        cx.subscribe(&today, |view, _, event, cx| match event {
            // A day or a project is a place in another tab: the page names it and the app goes
            // there, so a thumb never has to work out which tab holds what it just tapped.
            TodayEvent::OpenDay(day) => {
                view.active_tab = Tab::Calendar;
                view.calendar
                    .update(cx, |calendar, cx| calendar.open_day(*day, cx));
            }
            TodayEvent::OpenProject(name) => {
                view.active_tab = Tab::Projects;
                if let Some(projects) = &view.projects {
                    let name = name.clone();
                    projects.update(cx, |projects, cx| projects.open_project(name, cx));
                }
            }
            TodayEvent::Settings => {
                view.active_tab = Tab::Settings;
                notify::report();
            }
            // The edge slides in over the page rather than becoming a tab: the bar holds five and is
            // full, and the plan's own shape for it is a panel from the right edge (`ui/src/sidebar.rs`).
            TodayEvent::Soundscape => view.open_edge(Seat::Soundscape, cx),
            TodayEvent::Activity => view.open_edge(Seat::Activity, cx),
            TodayEvent::Refresh => {
                if view.board.is_some() {
                    view.maybe_sync(cx);
                } else {
                    view.refresh_copy(cx);
                }
            }
            TodayEvent::Wrote => view.maybe_sync(cx),
        })
        .detach();
        // The register and the card a screenshot needs, for the same reason `SJEL_DAY` exists: a
        // page a hand did not navigate to still has to be photographable (`launch_register`,
        // `launch_person`).
        people.update(cx, |people, cx| {
            people.set_register(launch_register(), cx);
            if let Some(name) = launch_person() {
                people.open_named(&name, cx);
            }
        });
        cx.subscribe(&people, |view, _, event, cx| match event {
            PeopleEvent::Refresh => view.refresh_copy(cx),
            PeopleEvent::Editing(open) => keyboard(*open),
        })
        .detach();
        // The capture field writes the phone's own replica, so it needs nothing from the Mac: it is
        // built empty and filled when the store opens.
        let inbox = cx.new(|_| Inbox::new(Density::Mobile));
        cx.subscribe(&inbox, |view, _, event, cx| match event {
            // A capture landed in the queue: the Mac can have it now if this launch reached it.
            InboxEvent::Wrote => view.maybe_sync(cx),
            InboxEvent::Editing(open) => keyboard(*open),
        })
        .detach();
        let copy_events = events.clone();
        shell::spawn(async move {
            let _ = copy_events.unbounded_send(Event::Copy(open_copy()));
        });
        // The drawer holds the conductor and the report from the first frame, so it can show a preset
        // and a volume before any sound exists. Its settings are the ones the last launch left; the
        // edge's own open state and seat are the sidebar's.
        let (control, visual) = audio::parts();
        let soundscape =
            cx.new(|_| Soundscape::new(control, visual, SoundscapePrefs::load(), Density::Mobile));
        let mut sidebar = Sidebar::load();
        // `SJEL_SIDEBAR=soundscape|activity` opens the edge at launch, the way `SJEL_TAB` opens a tab:
        // a screenshot of a panel no hand opened still has to be possible. `SJEL_SOUNDSCAPE=1` still
        // opens it on the sound seat, because that is what it meant before the edge had seats.
        if launch_edge_off() {
            sidebar.close();
        } else if let Some(seat) = launch_edge() {
            sidebar.open_on(seat);
        }
        // Left open last time: the edge comes back on its seat, and the device is taken when that
        // seat is the soundscape, because that is what remembering an open edge means. It starts
        // paused: `playing` is not remembered.
        let mut audio_started = false;
        if sidebar.is_open() && sidebar.seat() == Seat::Soundscape {
            audio_started = true;
            match audio::start() {
                Ok(device) => {
                    soundscape.update(cx, |drawer, cx| drawer.set_device(Some(device), None, cx))
                }
                Err(why) => {
                    soundscape.update(cx, |drawer, cx| drawer.set_device(None, Some(why), cx))
                }
            }
        }

        cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                let alive = this.update(cx, |view, cx| {
                    match event {
                        Event::Status(text) => view.status = text.into(),
                        Event::AgentSessions(sessions, error) => {
                            view.agent_sessions = sessions;
                            view.agent_error = error;
                            view.agent_sessions_loading = false;
                        }
                        Event::AgentPrompted(session) => {
                            view.agent_busy = false;
                            view.agent_sessions.retain(|item| item.id != session.id);
                            view.agent_sessions.insert(0, session.clone());
                            view.attach_agent_session(session, false);
                        }
                        Event::AgentAttached(id, rows, cursor) => {
                            if view.agent_selected.as_deref() == Some(id.as_str()) {
                                view.agent_rows = rows;
                                view.agent_last_seq = cursor;
                                view.agent_error = None;
                                view.agent_pending = None;
                                view.agent_text_streaming = false;
                            }
                        }
                        Event::AgentLine(session_id, agent_event) => {
                            if view.agent_selected.as_deref() != Some(session_id.as_str()) {
                                return;
                            }
                            if agent_event.kind == "truncated" {
                                view.agent_last_seq = agent_event.first_seq.saturating_sub(1);
                                if let Some(session) = view
                                    .agent_sessions
                                    .iter()
                                    .find(|session| session.id == session_id)
                                    .cloned()
                                {
                                    view.attach_agent_session(session, true);
                                }
                                return;
                            }
                            let terminal = agent_event.terminal();
                            view.agent_last_seq = view.agent_last_seq.max(agent_event.seq);
                            match agent_event.kind.as_str() {
                                "needs_approval" => view.agent_pending = Some(agent_event.clone()),
                                "approved" | "denied" | "done" | "stopped" | "error" => {
                                    view.agent_pending = None;
                                }
                                _ => {}
                            }
                            match agent_event.kind.as_str() {
                                "reasoning" => view.agent_text_streaming = false,
                                "text" => {
                                    if view.agent_text_streaming {
                                        if let Some(answer) = view.agent_rows.last_mut() {
                                            answer.push_str(&agent_event.content);
                                        }
                                    } else {
                                        view.agent_rows.push(agent_event.content.clone());
                                    }
                                    view.agent_text_streaming = true;
                                }
                                _ => {
                                    view.agent_text_streaming = false;
                                    view.agent_rows.push(format_agent_event(&agent_event));
                                }
                            }
                            if terminal {
                                view.refresh_agent_sessions();
                            }
                        }
                        Event::AgentAction(message) => {
                            view.agent_busy = false;
                            if message.starts_with("Agent ") || message.starts_with("Stream ") {
                                view.agent_error = Some(message);
                            } else {
                                view.agent_rows.push(message);
                            }
                        }
                        Event::StoreFailed(text) => view.store_error = Some(text.into()),
                        Event::Copy(outcome) => {
                            view.copying = false;
                            match outcome {
                                Ok(copy) => {
                                    view.copy = Some(copy.clone());
                                    view.calendar.update(cx, |calendar, cx| {
                                        calendar.set_copy(Some(copy.clone()), None, cx)
                                    });
                                    view.people.update(cx, |people, cx| {
                                        people.set_copy(Some(copy.clone()), None, cx)
                                    });
                                    view.today.update(cx, |today, cx| {
                                        today.set_copy(Some(copy), None, cx)
                                    });
                                }
                                // A refresh that fails leaves the copy already on screen alone:
                                // it is still true, only older, and the page says how old.
                                Err(error) => {
                                    if view.copy.is_none() {
                                        view.calendar.update(cx, |calendar, cx| {
                                            calendar.set_copy(None, Some(error.clone()), cx)
                                        });
                                        view.people.update(cx, |people, cx| {
                                            people.set_copy(None, Some(error.clone()), cx)
                                        });
                                        view.today.update(cx, |today, cx| {
                                            today.set_copy(None, Some(error), cx)
                                        });
                                    } else {
                                        let note = format!("Could not fetch a newer copy: {error}");
                                        view.calendar.update(cx, |calendar, cx| {
                                            calendar.set_note(Some(note.clone()), cx)
                                        });
                                        view.people.update(cx, |people, cx| {
                                            people.set_note(Some(note.clone()), cx)
                                        });
                                        view.today
                                            .update(cx, |today, cx| today.set_note(Some(note), cx));
                                    }
                                }
                            }
                        }
                        Event::Board(board) => {
                            if view.projects.is_none() {
                                if let Some(store) = board.store().cloned() {
                                    let projects = view.open_projects(store.clone(), cx);
                                    view.projects = Some(projects);
                                    // The Calendar tab writes to the same store, and reads it as
                                    // soon as its change feed has been read once — until then the
                                    // Mac's copy keeps drawing the day (`ui/src/calendar.rs`).
                                    view.inbox.update(cx, |inbox, cx| {
                                        inbox.set_store(Some(store.clone()), cx)
                                    });
                                    let repo = view.repo_for(store);
                                    view.calendar.update(cx, |calendar, cx| {
                                        calendar.set_store(Some(repo.clone()), cx)
                                    });
                                    // The reminder rebuild reads the same store: an entry written on
                                    // the phone is a reminder on the same phone, without the Mac.
                                    view.repo = Some(repo);
                                }
                            }
                            // The landing page draws the replica's tasks and projects from these
                            // rows: `Board` already read them, and a second read here could only
                            // disagree with the queue count beside them.
                            let records = board.records().to_vec();
                            let blocked = board.blocked().to_vec();
                            if let Some(projects) = &view.projects {
                                projects
                                    .update(cx, |projects, cx| projects.set_blocked(blocked, cx));
                            }
                            view.inbox.update(cx, |inbox, cx| inbox.load(cx));
                            view.today.update(cx, |today, cx| {
                                today.set_records(records, cx);
                                today.set_travel(board.pending(), view.client.is_some(), cx);
                            });
                            view.board = Some(board);
                            // The replica moved: the lock screen follows it, whether or not the Mac
                            // answered (`rebuild_reminders`).
                            view.rebuild_reminders(cx);
                            // A local write landed. If this launch has reached the Mac, it goes
                            // there now instead of waiting for the next launch.
                            view.maybe_sync(cx);
                        }
                        Event::Found(nodes) => {
                            view.found = nodes;
                            // A previous answer is not an error; keep it on screen.
                            if view.rows.is_empty() && !view.joining {
                                view.status = if view.found.is_empty() {
                                    "Looking for your Mac on this network…".into()
                                } else {
                                    "Found your Mac. Tap Join to ask for access.".into()
                                };
                            }
                        }
                        Event::Join(found) => {
                            view.joining = true;
                            view.status = format!(
                                "Asking \u{201c}{}\u{201d} to allow this device…",
                                found.name
                            )
                            .into();
                            let events = view.events.clone();
                            shell::spawn(async move {
                                let outcome = match node_file() {
                                    Ok(path) => {
                                        match join_and_wait(&found.join_target(), &events).await {
                                            Ok(node) => connect(&path, node, &events).await,
                                            Err(error) => Err(error),
                                        }
                                    }
                                    Err(error) => Err(error),
                                };
                                if let Err(error) = outcome {
                                    let _ = events.unbounded_send(Event::Status(error));
                                }
                            });
                        }
                        Event::Scanned(code) => {
                            // A scanned code makes the same claim the `SJEL_PAIR` launch path makes;
                            // it arrives after launch, so the whole claim runs here instead.
                            view.joining = true;
                            view.status = "Reading the code from your Mac…".into();
                            let events = view.events.clone();
                            shell::spawn(async move {
                                let outcome = match node_file() {
                                    Ok(path) => match claim_scanned(&code).await {
                                        Ok(node) => connect(&path, node, &events).await,
                                        Err(error) => Err(error),
                                    },
                                    Err(error) => Err(error),
                                };
                                if let Err(error) = outcome {
                                    let _ = events.unbounded_send(Event::Status(error));
                                }
                            });
                        }
                        Event::ScanFailed(text) => {
                            view.status = text.into();
                            view.joining = false;
                        }
                        Event::RoomplanResult(text) => {
                            view.roomplan_status = text.into();
                            if view.client.is_some() {
                                view.interior
                                    .update(cx, |history, cx| history.refresh(cx));
                            }
                        }
                        Event::RoomplanArchive(records) => view
                            .interior
                            .update(cx, |history, cx| history.set_local_archive(records, cx)),
                        Event::RoomplanLive(snapshot) => view
                            .interior
                            .update(cx, |history, cx| history.set_live_snapshot(snapshot, cx)),
                        Event::RoomplanUpload(body) => {
                            if let Some(client) = view.client.clone() {
                                view.roomplan_status = "Sending the room scan to your Mac…".into();
                                let events = view.events.clone();
                                shell::spawn(async move {
                                    let body = body.into_bytes();
                                    let expected_id = serde_json::from_slice::<serde_json::Value>(&body)
                                        .ok()
                                        .and_then(|value| value.get("draft")?.get("draft_id")?.as_str().map(str::to_owned));
                                    let status = match client
                                        .post::<RoomplanUploadAck>(
                                            "/interior/api/roomplan/captures",
                                            &body,
                                        )
                                        .await
                                    {
                                        Ok(ack) if expected_id.as_deref() == Some(&ack.draft_id) => {
                                            mark_roomplan_uploaded(&ack.draft_id);
                                            format!("Sent to Mac · {}", ack.draft_id)
                                        }
                                        Ok(_) => "Mac returned a mismatched scan receipt; scan remains queued.".to_owned(),
                                        Err(error) => format!("Could not send room scan: {error}"),
                                    };
                                    let _ = events.unbounded_send(Event::RoomplanResult(status));
                                });
                            } else {
                                view.roomplan_status =
                                    "Pair with your Mac before sending a scan.".into();
                            }
                        }
                        Event::Ready(node, health) => {
                            view.status =
                                format!("Paired as {} with {}", node.device_id, node.base).into();
                            view.rows = health
                                .capabilities
                                .iter()
                                .map(|(name, state)| (name.clone(), state.up))
                                .collect();
                            view.found.clear();
                            view.joining = false;
                            match DeviceClient::new(node, Arc::new(identity::sign)) {
                                Ok(client) => {
                                    let client = Arc::new(client);
                                    view.interior.update(cx, |history, cx| {
                                        history.set_source(
                                            Some(RoomPlanSource::Device(client.clone())),
                                            cx,
                                        )
                                    });
                                    view.travel.update(cx, |travel, _| {
                                        travel.set_source(sjel_gpui_core::travel::Source::Device(
                                            client.clone(),
                                        ))
                                    });
                                    view.client = Some(client);
                                }
                                Err(error) => {
                                    view.synced = Some(format!("Cannot sync: {error}").into())
                                }
                            }
                            if view.sidebar.is_open() && view.sidebar.seat() == Seat::Agent {
                                view.agent_sessions_loading = true;
                                view.refresh_agent_sessions();
                            }
                            view.maybe_sync(cx);
                            if view.client.is_some() {
                                load_roomplan_upload();
                            }
                            view.refresh_copy(cx);
                            let paired = view.client.is_some();
                            let pending = view.board.as_ref().map_or(0, Board::pending);
                            view.today
                                .update(cx, |today, cx| today.set_travel(pending, paired, cx));
                        }
                        Event::Wrote => {
                            if let Some(board) = view.board.clone() {
                                let events = view.events.clone();
                                shell::spawn(async move {
                                    let mut board = board;
                                    let _ = board.reload().await;
                                    let _ = events.unbounded_send(Event::Board(board));
                                });
                            }
                        }
                        Event::Synced(outcome, board) => {
                            view.syncing = false;
                            view.board = Some(board);
                            // The pull may have changed any record the page shows.
                            if let Some(projects) = &view.projects {
                                projects.update(cx, |projects, cx| projects.reload(cx));
                            }
                            // A pull lands entries too, and the ids a create answered with.
                            view.calendar.update(cx, |calendar, cx| calendar.load(cx));
                            // A sync is what moves a capture from "waiting" to "on the Mac".
                            view.inbox.update(cx, |inbox, cx| inbox.load(cx));
                            let (pending, paired) = (
                                view.board.as_ref().map_or(0, Board::pending),
                                view.client.is_some(),
                            );
                            let records = view
                                .board
                                .as_ref()
                                .map(|board| board.records().to_vec())
                                .unwrap_or_default();
                            // A sync is also how a refusal arrives, or stops being one.
                            let blocked = view
                                .board
                                .as_ref()
                                .map(|board| board.blocked().to_vec())
                                .unwrap_or_default();
                            if let Some(projects) = &view.projects {
                                projects
                                    .update(cx, |projects, cx| projects.set_blocked(blocked, cx));
                            }
                            view.inbox.update(cx, |inbox, cx| inbox.load(cx));
                            view.today.update(cx, |today, cx| {
                                today.set_records(records, cx);
                                today.set_travel(pending, paired, cx);
                            });
                            view.synced = Some(
                                match outcome {
                                    Ok(report) => report.summary(),
                                    Err(error) => format!("Nothing moved: {error}"),
                                }
                                .into(),
                            );
                            // A pull lands the ids a local entry was given, and the tasks the Mac
                            // ticked off: both change what earns a reminder.
                            view.rebuild_reminders(cx);
                        }
                        Event::Reminded(plan) => {
                            view.notify_plan = plan;
                        }
                        Event::FareAlertsSeen(ids) => {
                            view.notify_prefs.mark_fare_alerts_seen(&ids);
                            let _ = view.notify_prefs.save();
                        }
                        Event::NotifyState(state) => {
                            let newly_allowed = view.notify_state.authorization
                                != notify::Authorization::Allowed
                                && state.authorization == notify::Authorization::Allowed;
                            view.notify_state = state;
                            if newly_allowed {
                                view.notify_prefs.fare_alerts_last_check_ms = 0;
                                let _ = view.notify_prefs.save();
                                view.rebuild_reminders(cx);
                            }
                        }
                        Event::NotifyOpen(link) => view.open_link(&link, cx),
                        Event::Background => {
                            view.terminal_unlocked = false;
                            view.terminal = None;
                        }
                        Event::Unlocked(ok) => {
                            view.terminal_unlocked = ok;
                            if !ok {
                                view.terminal_note = Some(
                                    "Face ID did not confirm. Close and reopen the edge to try again."
                                        .into(),
                                );
                            } else if view.sidebar.is_open()
                                && view.sidebar.seat() == Seat::Terminal
                            {
                                view.open_terminal(cx);
                            }
                        }
                        Event::Foreground => {
                            load_roomplan_archive();
                            view.rebuild_reminders(cx);
                            if view.client.is_some() {
                                load_roomplan_upload();
                            }
                            // The count is the system's, so it is asked for rather than assumed:
                            // a reminder that fired while the phone was away is gone from it.
                            notify::report();
                        }
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            // The channel closed without a result: the work stopped early, for example on a panic.
            let _ = this.update(cx, |view, cx| {
                if view.rows.is_empty() && view.status.starts_with("Connecting") {
                    view.status = "The connection stopped inside the app. See the app log.".into();
                    cx.notify();
                }
            });
        })
        .detach();
        // `shell::run` already spawned the work on the HTTP runtime; the events carry its result.
        drop(work);
        Self {
            status: "Connecting to the Mac…".into(),
            agent_sessions: Vec::new(),
            agent_rows: Vec::new(),
            agent_selected: None,
            agent_last_seq: 0,
            agent_sessions_loading: false,
            agent_error: None,
            agent_pending: None,
            agent_input: None,
            agent_busy: false,
            agent_text_streaming: false,
            terminal: None,
            terminal_unlocked: false,
            terminal_note: None,
            synced: None,
            rows: Vec::new(),
            found: Vec::new(),
            joining: false,
            active_tab: launch_tab(),
            travel: cx.new(|_| sjel_gpui_ui::travel::Travel::new(None, Density::Mobile)),
            projects: None,
            calendar,
            people,
            interior,
            inbox,
            soundscape,
            sidebar,
            today,
            copy: None,
            copying: false,
            store_error: None,
            board: None,
            client: None,
            syncing: false,
            audio_started,
            notify_prefs,
            notify_state: notify::State::default(),
            notify_plan: reminders::Plan::default(),
            roomplan_status: "Room scans stay on this iPhone for now.".into(),
            repo: None,
            appearance: AppearanceSettings::load(),
            events,
        }
    }

    /// Replays the queue at the Mac and takes back what it changed, when both halves are here.
    ///
    /// Called when the pairing is live and when a local write lands. A phone that has not reached
    /// the Mac this launch never starts one: its queue is the only copy of those edits.
    fn maybe_sync(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let (Some(client), Some(board)) = (self.client.clone(), self.board.clone()) else {
            return;
        };
        self.syncing = true;
        self.synced = Some("Syncing with the Mac…".into());
        let events = self.events.clone();
        shell::spawn(async move {
            let Some(store) = board.store().cloned() else {
                return;
            };
            let outcome = sync::run(&store, &client).await;
            let mut board = board;
            let _ = board.reload().await;
            let _ = events.unbounded_send(Event::Synced(outcome, board));
        });
        cx.notify();
    }

    fn agent_new_session(&mut self, cx: &mut Context<Self>) {
        self.agent_selected = None;
        self.agent_rows.clear();
        self.agent_last_seq = 0;
        self.agent_pending = None;
        self.agent_error = None;
        self.agent_text_streaming = false;
        cx.notify();
    }

    /// Opens the right edge on `seat` (`ui/src/sidebar.rs`).
    ///
    /// The audio device is taken here and not at launch: claiming the audio route is a visible act,
    /// and belongs to opening the panel that plays. Switching seats keeps the sound running — the
    /// edge is beside the page, not away from it — and only closing it stops the sound.
    fn open_edge(&mut self, seat: Seat, cx: &mut Context<Self>) {
        self.sidebar.open_on(seat);
        if seat == Seat::Soundscape && !self.audio_started {
            self.audio_started = true;
            match audio::start() {
                Ok(device) => self
                    .soundscape
                    .update(cx, |drawer, cx| drawer.set_device(Some(device), None, cx)),
                Err(why) => self
                    .soundscape
                    .update(cx, |drawer, cx| drawer.set_device(None, Some(why), cx)),
            }
        }
        if seat == Seat::Activity {
            // The activity seat's numbers are the system's and the last sync's, so they are asked for
            // again as the seat comes into view rather than drawn from an hour ago.
            notify::report();
        }
        if seat == Seat::Agent {
            self.agent_sessions_loading = true;
            self.refresh_agent_sessions();
        }
        if seat == Seat::Terminal {
            if self.terminal_unlocked {
                self.open_terminal(cx);
            } else if self.client.is_some() {
                self.terminal_note = Some("Confirm with Face ID to open the terminal.".into());
                unlock();
            }
        }
        cx.notify();
    }

    /// The newest live terminal on the Mac, or a new login shell, in the edge.
    fn open_terminal(&mut self, cx: &mut Context<Self>) {
        if self
            .terminal
            .as_ref()
            .is_some_and(|(view, _)| view.read(cx).peer().alive)
        {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.terminal_note = Some("Opening a terminal on the Mac…".into());
        let host = Host::Device(client);
        let opener = host.clone();
        let palette = self.palette();
        let work = shell::run(async move {
            opener
                .attach_or_open(TERMINAL_COLS as u16, TERMINAL_ROWS as u16)
                .await
        });
        cx.spawn(async move |weak, cx| {
            let opened = work
                .await
                .unwrap_or_else(|| Err("The Mac did not answer.".into()));
            let _ = weak.update(cx, |view, cx| {
                match opened {
                    Ok(peer) => {
                        let terminal = cx.new(|cx| {
                            TerminalView::new(
                                peer,
                                host,
                                0,
                                TERMINAL_COLS,
                                TERMINAL_ROWS,
                                palette,
                                cx,
                            )
                            .touch()
                        });
                        terminal.update(cx, |terminal, cx| terminal.attach(cx));
                        let subscription = cx.subscribe(&terminal, |_, _, event, _| match event {
                            TerminalEvent::Editing(open) => keyboard(*open),
                        });
                        view.terminal = Some((terminal, subscription));
                        view.terminal_note = None;
                    }
                    Err(why) => {
                        view.terminal_note =
                            Some(format!("The terminal did not open: {why}").into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn palette(&self) -> Palette {
        let is_dark = self.appearance.theme_mode.is_dark(false);
        let base = if is_dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        self.appearance.apply_to_palette(base, is_dark)
    }

    fn refresh_agent_sessions(&self) {
        let (Some(client), events) = (self.client.clone(), self.events.clone()) else {
            return;
        };
        shell::spawn(async move {
            #[derive(Deserialize)]
            struct Listed {
                #[serde(default)]
                sessions: Vec<AgentSession>,
            }
            let result = client.get::<Listed>("/agent/api/agent/sessions").await;
            let event = match result {
                Ok(listed) => Event::AgentSessions(listed.sessions, None),
                Err(error) => Event::AgentSessions(Vec::new(), Some(error)),
            };
            let _ = events.unbounded_send(event);
        });
    }

    fn agent_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<InputState> {
        if let Some((input, _)) = &self.agent_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask the agent to work…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let prompt = input.read(cx).value().trim().to_owned();
                if prompt.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.agent_prompt(prompt, cx);
            }
        });
        self.agent_input = Some((input.clone(), subscription));
        input
    }

    fn agent_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.agent_busy {
            return;
        }
        let Some(client) = self.client.clone() else {
            self.agent_error =
                Some("Pair this phone with the Mac before starting an agent run.".into());
            cx.notify();
            return;
        };
        let selected = self.agent_selected.clone();
        let events = self.events.clone();
        self.agent_busy = true;
        self.agent_error = None;
        shell::spawn(async move {
            let result: Result<Option<AgentSession>, String> = async {
                let (id, active) = match selected {
                    Some(id) => {
                        let session = client
                            .get::<AgentSession>(&format!("/agent/api/agent/sessions/{id}"))
                            .await?;
                        (id, session.active)
                    }
                    None => {
                        let session = client
                            .post::<AgentSession>("/agent/api/agent/sessions", b"{}")
                            .await?;
                        (session.id, false)
                    }
                };
                let (path, body) = if active {
                    (
                        format!("/agent/api/agent/sessions/{id}/steer"),
                        serde_json::json!({ "message": prompt }),
                    )
                } else {
                    (
                        format!("/agent/api/agent/sessions/{id}/prompt"),
                        serde_json::json!({ "prompt": prompt }),
                    )
                };
                let _: serde_json::Value =
                    client.post(&path, &body.to_string().into_bytes()).await?;
                if active {
                    Ok(None)
                } else {
                    let session = client
                        .get::<AgentSession>(&format!("/agent/api/agent/sessions/{id}"))
                        .await?;
                    Ok(Some(session))
                }
            }
            .await;
            match result {
                Ok(Some(session)) => {
                    let _ = events.unbounded_send(Event::AgentPrompted(session));
                }
                Ok(None) => {
                    let _ = events.unbounded_send(Event::AgentAction(
                        "Message queued for the next model step.".into(),
                    ));
                }
                Err(error) => {
                    let _ = events.unbounded_send(Event::AgentAction(format!(
                        "Agent request failed: {error}"
                    )));
                }
            }
        });
        cx.notify();
    }

    fn agent_control(&mut self, action: AgentControl, cx: &mut Context<Self>) {
        let (Some(client), Some(id)) = (self.client.clone(), self.agent_selected.clone()) else {
            return;
        };
        let pending = self.agent_pending.clone();
        if matches!(action, AgentControl::Approve(_, _)) && pending.is_none() {
            return;
        }
        if self.agent_busy {
            return;
        }
        self.agent_busy = true;
        let events = self.events.clone();
        shell::spawn(async move {
            let result = match action {
                AgentControl::Approve(allow, for_session) => {
                    let approval = pending.map(|event| event.approval).unwrap_or_default();
                    let scope = if for_session { "session" } else { "once" };
                    let body = serde_json::json!({ "approval_id": approval, "allow": allow, "scope": scope });
                    client
                        .post::<serde_json::Value>(
                            &format!("/agent/api/agent/sessions/{id}/approve"),
                            &body.to_string().into_bytes(),
                        )
                        .await
                        .map(|_| {
                            if allow {
                                "Approval sent."
                            } else {
                                "Denial sent."
                            }
                        })
                }
                AgentControl::Stop => client
                    .post::<serde_json::Value>(
                        &format!("/agent/api/agent/sessions/{id}/stop"),
                        b"{}",
                    )
                    .await
                    .map(|_| "Stop requested."),
            };
            let message = result
                .map(str::to_owned)
                .unwrap_or_else(|error| format!("Agent action failed: {error}"));
            let _ = events.unbounded_send(Event::AgentAction(message));
        });
        cx.notify();
    }

    fn attach_agent_session(&mut self, session: AgentSession, reload: bool) {
        let Some(client) = self.client.clone() else {
            self.agent_error = Some("Pair with the Mac to attach to its agent runs.".into());
            return;
        };
        let events = self.events.clone();
        let from = if self.agent_selected.as_deref() == Some(&session.id) {
            self.agent_last_seq
        } else {
            0
        };
        let already_attached = self.agent_selected.as_deref() == Some(&session.id);
        if !already_attached {
            self.agent_selected = Some(session.id.clone());
            self.agent_rows.clear();
            self.agent_last_seq = 0;
            self.agent_pending = None;
        }
        shell::spawn(async move {
            #[derive(Deserialize)]
            struct Stored {
                #[serde(default)]
                messages: Vec<AgentTurn>,
            }
            let path = format!("/agent/api/agent/sessions/{}/transcript", session.id);
            let rows = if already_attached && !reload {
                None
            } else {
                match client.get::<Stored>(&path).await {
                    Ok(stored) => {
                        let mut turns = stored.messages;
                        if session.active
                            && turns
                                .last()
                                .is_some_and(|turn| turn.role.as_str() == "user")
                        {
                            turns.pop();
                        }
                        Some(
                            turns
                                .into_iter()
                                .filter_map(|turn| {
                                    turn.content.map(|text| format!("{}: {text}", turn.role))
                                })
                                .collect(),
                        )
                    }
                    Err(error) => {
                        let _ =
                            events.unbounded_send(Event::AgentSessions(vec![session], Some(error)));
                        return;
                    }
                }
            };
            if let Some(rows) = rows {
                let _ = events.unbounded_send(Event::AgentAttached(session.id.clone(), rows, from));
            }
            if !session.active {
                return;
            }
            let mut cursor = from;
            for attempt in 0..5 {
                let path = format!(
                    "/agent/api/agent/sessions/{}/stream?from={cursor}",
                    session.id
                );
                let mut response = match client.streaming_get(&path).await {
                    Ok(response) => response,
                    Err(error) if attempt == 4 => {
                        let _ = events.unbounded_send(Event::AgentAction(format!(
                            "Stream reconnect failed: {error}. Tap the session to retry."
                        )));
                        return;
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };
                let mut buffer = Vec::new();
                let mut ended = false;
                while !ended {
                    let chunk = match response.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) | Err(_) => break,
                    };
                    buffer.extend_from_slice(&chunk);
                    while let Some(end) = buffer.windows(2).position(|window| window == b"\n\n") {
                        let frame: Vec<u8> = buffer.drain(..end + 2).collect();
                        let frame_text = String::from_utf8_lossy(&frame);
                        let frame_kind = frame_text
                            .lines()
                            .find_map(|line| line.strip_prefix("event: "))
                            .unwrap_or_default();
                        let data = frame_text
                            .lines()
                            .filter_map(|line| line.strip_prefix("data: "))
                            .map(str::trim)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let Ok(mut event) = serde_json::from_str::<AgentEvent>(&data) else {
                            continue;
                        };
                        if frame_kind == "truncated" {
                            event.kind = "truncated".into();
                        }
                        if event.seq > cursor || event.kind == "truncated" {
                            cursor = cursor.max(event.seq);
                            let _ = events.unbounded_send(Event::AgentLine(
                                session.id.clone(),
                                event.clone(),
                            ));
                        }
                        if event.terminal() || event.kind == "truncated" {
                            ended = true;
                            break;
                        }
                    }
                }
                if ended {
                    return;
                }
                if attempt < 4 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
            let _ = events.unbounded_send(Event::AgentAction(
                "Stream disconnected after five attempts. Tap the session to retry.".into(),
            ));
        });
    }

    /// Puts the edge away, and stops the sound with it: a panel that is not on screen is not a thing
    /// that should still be playing in a pocket.
    fn close_edge(&mut self, cx: &mut Context<Self>) {
        if self.sidebar.seat() == Seat::Terminal {
            keyboard(false);
        }
        self.sidebar.close();
        self.soundscape.update(cx, |drawer, cx| drawer.pause(cx));
        cx.notify();
    }

    /// The activity seat: what this phone is doing, in its own words.
    ///
    fn activity_panel(&self, palette: Palette) -> AnyElement {
        let pending = self.board.as_ref().map_or(0, Board::pending);
        let paired = self.client.is_some();
        let with_mac: Vec<String> = vec![
            self.status.to_string(),
            match (&self.synced, paired) {
                (Some(synced), _) => synced.to_string(),
                (None, true) => "Paired. Nothing has moved this launch.".to_owned(),
                (None, false) => "Not paired: this phone keeps its own records.".to_owned(),
            },
            if pending > 0 {
                format!("{pending} queued for the Mac")
            } else {
                "Nothing queued".to_owned()
            },
        ];
        // One coherent answer, not two lines that contradict each other: with the switch off, the
        // permission's state is the Settings card's business, not a "Allowed" above "Reminders are
        // off".
        let mut reminders: Vec<String> = Vec::new();
        match notify::disabled_reason() {
            Some(reason) => reminders.push(reason),
            None if !self.notify_prefs.enabled => reminders.push("Reminders are off".to_owned()),
            None => {
                reminders.push(self.notify_state.authorization.phrase().to_owned());
                for reminder in self.notify_plan.reminders.iter().take(3) {
                    reminders.push(reminders::when(
                        reminder.at,
                        &reminder.title,
                        chrono::Local::now(),
                    ));
                }
                if reminders.len() == 1 {
                    reminders.push("Nothing scheduled".to_owned());
                }
            }
        }
        let section = |title: &'static str, lines: Vec<String>| {
            let mut column = div().flex().flex_col().gap_1().child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(title),
            );
            for line in lines {
                column = column.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(SharedString::from(line)),
                );
            }
            column
        };
        div()
            .id("activity")
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .pt(px(12.))
            .overflow_y_scroll()
            .child(section("WITH THE MAC", with_mac))
            .child(section("REMINDERS", reminders))
            .into_any_element()
    }

    /// The paired phone can start, follow, and approve a run held by the Mac.
    fn agent_panel(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let paired = self.client.is_some();
        let loading = self.agent_sessions_loading;
        let sessions = self.agent_sessions.clone();
        let rows = self.agent_rows.clone();
        let selected = self.agent_selected.clone();
        let error = self.agent_error.clone();
        let pending = self.agent_pending.clone();
        let busy = self.agent_busy;
        let selected_active = selected
            .as_ref()
            .and_then(|id| self.agent_sessions.iter().find(|session| &session.id == id))
            .is_some_and(|session| session.active);
        let input = self.agent_input(window, cx);
        div()
            .id("agent-activity")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .when(paired, |panel| {
                panel.child(
                    div()
                        .id("phone-agent-refresh")
                        .self_flex_end()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .text_color(rgb(palette.muted))
                        .child("Refresh sessions")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.agent_sessions_loading = true;
                                view.refresh_agent_sessions();
                                cx.notify();
                            }),
                        ),
                )
            })
            .when(paired && loading, |panel| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Loading agent sessions…"),
                )
            })
            .when(
                paired && !loading && sessions.is_empty() && error.is_none(),
                |panel| {
                    panel.child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("No agent sessions yet."),
                    )
                },
            )
            .when(!paired, |panel| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Pair this phone with the Mac to watch its agent runs."),
                )
            })
            .children(sessions.into_iter().map(|session| {
                let id = session.id.clone();
                let active = selected.as_deref() == Some(id.as_str());
                let label = session.title.clone().unwrap_or_else(|| session.root_name());
                div()
                    .id(SharedString::from(format!("phone-agent-session-{id}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(7.))
                    .when(active, |row| row.bg(rgb(palette.raised)))
                    .text_color(rgb(palette.text))
                    .child(div().flex_1().truncate().child(label))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if session.active {
                                palette.green
                            } else {
                                palette.muted
                            }))
                            .child(if session.active { "live" } else { "saved" }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, _cx| {
                            view.attach_agent_session(session.clone(), false)
                        }),
                    )
            }))
            .when(paired, |panel| {
                panel
                    .child(div().child(Input::new(&input)))
                    .child(
                        div()
                            .id("phone-agent-new")
                            .px_3()
                            .py_2()
                            .rounded(px(7.))
                            .text_color(rgb(palette.text))
                            .child("New session")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.agent_new_session(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("phone-agent-send")
                            .px_3()
                            .py_2()
                            .rounded(px(7.))
                            .bg(rgb(palette.raised))
                            .text_color(rgb(palette.text))
                            .child(if busy {
                                "Working…"
                            } else if selected_active {
                                "Send guidance"
                            } else {
                                "Start run"
                            })
                            .when(!busy, |button| {
                                button.on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|view, _, window, cx| {
                                        let prompt = view
                                            .agent_input
                                            .as_ref()
                                            .map(|(input, _)| {
                                                input.read(cx).value().trim().to_owned()
                                            })
                                            .unwrap_or_default();
                                        if prompt.is_empty() {
                                            return;
                                        }
                                        if let Some((input, _)) = &view.agent_input {
                                            input.update(cx, |input, cx| {
                                                input.set_value("", window, cx)
                                            });
                                        }
                                        view.agent_prompt(prompt, cx);
                                    }),
                                )
                            }),
                    )
            })
            .when_some(pending, |panel, pending| {
                let preview = if pending.preview.is_empty() {
                    pending.name.clone()
                } else {
                    pending.preview.clone()
                };
                let diff = pending.diff.clone();
                panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded(px(8.))
                        .bg(rgb(palette.raised))
                        .p_3()
                        .text_color(rgb(palette.text))
                        .child(format!("Approval · {}", pending.name))
                        .child(preview)
                        .when_some(diff, |card, diff| card.child(div().text_xs().child(diff)))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(agent_action_button(
                                    "Allow once",
                                    "phone-agent-allow-once",
                                    palette,
                                    cx,
                                    |view, cx| {
                                        view.agent_control(AgentControl::Approve(true, false), cx)
                                    },
                                ))
                                .child(agent_action_button(
                                    "Allow session",
                                    "phone-agent-allow-session",
                                    palette,
                                    cx,
                                    |view, cx| {
                                        view.agent_control(AgentControl::Approve(true, true), cx)
                                    },
                                ))
                                .child(agent_action_button(
                                    "Deny",
                                    "phone-agent-deny",
                                    palette,
                                    cx,
                                    |view, cx| {
                                        view.agent_control(AgentControl::Approve(false, false), cx)
                                    },
                                )),
                        ),
                )
            })
            .when(selected_active, |panel| {
                panel.when(!busy, |panel| {
                    panel.child(agent_action_button(
                        "Stop run",
                        "phone-agent-stop",
                        palette,
                        cx,
                        |view, cx| view.agent_control(AgentControl::Stop, cx),
                    ))
                })
            })
            .when_some(error, |panel, error| {
                panel.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(
                        rows.into_iter()
                            .map(|row| div().text_sm().text_color(rgb(palette.text)).child(row)),
                    ),
            )
            .into_any_element()
    }

    /// Rebuilds the lock screen from what this phone holds: the calendar's next weeks and the open
    /// tasks, through the same store their pages read.
    ///
    /// Called when the replica opens, after a write, after a pull and when the app comes back into
    /// view. A phone with reminders off takes every request away here rather than leaving stale ones
    /// to fire at a time nobody asked about.
    fn rebuild_reminders(&mut self, cx: &mut Context<Self>) {
        if notify::disabled_reason().is_some() {
            // This build may not touch the notification centre at all, so the card says why and
            // nothing is scheduled. The preference is left as the person set it: the switch is
            // about their intent, not this launch's build.
            self.notify_plan = reminders::Plan::default();
            self.notify_state = notify::State::default();
            notify::clear();
            cx.notify();
            return;
        }
        let reminders_enabled = self.notify_prefs.enabled;
        if !reminders_enabled {
            notify::clear();
            self.notify_plan = reminders::Plan::default();
            cx.notify();
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let lead = self.notify_prefs.lead();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let check_fare_alerts = self.notify_prefs.begin_fare_alert_check(now_ms);
        if check_fare_alerts {
            let _ = self.notify_prefs.save();
        }
        let seen: std::collections::HashSet<String> = self
            .notify_prefs
            .seen_fare_alert_ids
            .iter()
            .cloned()
            .collect();
        let authorization = self.notify_state.authorization;
        let source = self
            .client
            .clone()
            .map(sjel_gpui_core::travel::Source::Device);
        let events = self.events.clone();
        shell::spawn(async move {
            let today = chrono::Local::now().date_naive();
            let from = format!("{today}T00:00:00");
            let to = format!("{}T00:00:00", today + chrono::Duration::days(31));
            let entries = if reminders_enabled {
                repo.fetch_calendar(&from, &to).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let tasks = if reminders_enabled {
                repo.fetch_tasks(Some("open")).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let plan = reminders::plan(&entries, &tasks, lead, chrono::Local::now());
            if reminders_enabled {
                notify::apply(&plan);
            }

            if check_fare_alerts {
                let today = today.format("%Y-%m-%d").to_string();
                let saved = sjel_gpui_core::travel::saved_plans().await.ok().flatten();
                let plans = if let Some(source) = source.as_ref() {
                    match source.plans().await {
                        Ok(plans) => {
                            let _ = sjel_gpui_core::travel::save_plans(&plans).await;
                            plans
                        }
                        Err(_) => saved.map(|snapshot| snapshot.plans).unwrap_or_default(),
                    }
                } else {
                    saved.map(|snapshot| snapshot.plans).unwrap_or_default()
                };
                let mut details = Vec::new();
                for plan in plans
                    .iter()
                    .filter(|plan| sjel_gpui_core::travel::upcoming(plan, &today))
                {
                    let fresh = if let Some(source) = source.as_ref() {
                        source.details(&plan.id).await.ok()
                    } else {
                        None
                    };
                    if let Some(fresh) = fresh {
                        let _ = sjel_gpui_core::travel::save_details(fresh.clone()).await;
                        details.push(fresh);
                    } else if let Ok(Some(saved)) =
                        sjel_gpui_core::travel::saved_details(&plan.id).await
                    {
                        details.push(saved.details);
                    }
                }
                let alerts = sjel_gpui_core::travel::unseen_fare_lows(&details, &seen, &today);
                if !alerts.is_empty() {
                    match authorization {
                        notify::Authorization::Allowed => {
                            notify::deliver_fare_alerts(&alerts);
                            let ids = alerts.into_iter().map(|alert| alert.id).collect();
                            let _ = events.unbounded_send(Event::FareAlertsSeen(ids));
                        }
                        notify::Authorization::NotAsked | notify::Authorization::Denied => {}
                    }
                }
            }
            let _ = events.unbounded_send(Event::Reminded(plan));
        });
        cx.notify();
    }

    /// The switch on the Settings page. On: the permission is asked for (once in the app's life) and
    /// the requests are laid down — iOS accepts them before they may be shown, so a person who grants
    /// the prompt a moment later still gets the reminders already scheduled. Off: every request goes.
    fn toggle_reminders(&mut self, cx: &mut Context<Self>) {
        self.notify_prefs.enabled = !self.notify_prefs.enabled;
        let _ = self.notify_prefs.save();
        if self.notify_prefs.enabled {
            notify::authorize();
            self.rebuild_reminders(cx);
        } else {
            notify::clear();
            self.notify_plan = reminders::Plan::default();
        }
        cx.notify();
    }

    /// Where a tapped reminder goes: the day it announced, on the Calendar tab.
    fn open_link(&mut self, link: &str, cx: &mut Context<Self>) {
        let Some(day) = reminders::link_day(link) else {
            return;
        };
        self.active_tab = Tab::Calendar;
        self.calendar
            .update(cx, |calendar, cx| calendar.open_day(day, cx));
        cx.notify();
    }

    /// Fetches the Mac's newest copy for the Calendar page.
    ///
    /// Called when the pairing goes live and when the person taps Refresh. A launch that never
    /// reaches the Mac keeps the copy it already has: [`open_copy`] reads that one at startup,
    /// and a failed fetch reports itself beside rows that are still readable.
    fn refresh_copy(&mut self, cx: &mut Context<Self>) {
        if self.copying {
            return;
        }
        let Some(client) = self.client.clone() else {
            // A refresh needs the Mac. Saying so is better than a button that does nothing.
            self.calendar.update(cx, |calendar, cx| {
                calendar.set_note(
                    Some("Pair with your Mac in Settings to fetch a copy.".to_owned()),
                    cx,
                )
            });
            return;
        };
        self.copying = true;
        let events = self.events.clone();
        shell::spawn(async move {
            let _ = events.unbounded_send(Event::Copy(fetch_copy(client).await));
        });
        cx.notify();
    }

    /// The Projects page over the replica. Every write it makes comes back as [`Event::Wrote`].
    fn open_projects(
        &mut self,
        store: Arc<sjel_gpui_core::store::TursoStore>,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<Projects> {
        let repo = self.repo_for(store);
        let projects = cx.new(|_| Projects::new(repo).with_density(Density::Mobile));
        cx.subscribe(&projects, |view, _, event, cx| match event {
            ProjectsEvent::Editing(open) => keyboard(*open),
            // The band's "Try again": the same sync any local write starts, because the writes it
            // sends are the ones already queued. Nothing is re-typed to make a refusal go away. A
            // phone with no Mac says why instead of appearing to press a dead button.
            ProjectsEvent::Retry => {
                if view.client.is_some() {
                    view.maybe_sync(cx);
                } else {
                    view.synced = Some(
                        "Not connected: pair with your Mac in Settings, then try again.".into(),
                    );
                    cx.notify();
                }
            }
        })
        .detach();
        projects.update(cx, |projects, cx| projects.load_projects(cx));
        projects
    }

    /// The phone's `Repo` over one store: every write it takes comes back as [`Event::Wrote`], so
    /// the queue count moves and a sync may start.
    fn repo_for(&self, store: Arc<sjel_gpui_core::store::TursoStore>) -> Repo {
        let events = self.events.clone();
        Repo::replica(store, move || {
            let _ = events.unbounded_send(Event::Wrote);
        })
    }
}

/// Shows or hides the software keyboard as a note's editor opens or closes.
fn keyboard(open: bool) {
    #[cfg(target_os = "ios")]
    if open {
        gpui_mobile::show_keyboard();
    } else {
        gpui_mobile::hide_keyboard();
    }
    #[cfg(not(target_os = "ios"))]
    let _ = open;
}

/// The software keyboard's height, so the open note's caret stays above it.
fn keyboard_inset() -> f32 {
    #[cfg(target_os = "ios")]
    return gpui_mobile::keyboard_height();
    #[cfg(not(target_os = "ios"))]
    0.0
}

/// The saved node, or a new one, then the node's health as this device.
async fn load(
    pair: Option<String>,
    join: Option<String>,
    events: &UnboundedSender<Event>,
) -> Result<(), String> {
    let path = node_file()?;
    let node = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "The saved pairing is damaged.")?,
        Err(_) => {
            let node = match (pair, join) {
                (Some(pair), _) => {
                    let qr = device::parse_pairing_qr(&pair)?;
                    device::claim(&qr, label(), &identity::public_key()?).await?
                }
                (None, Some(join)) => join_and_wait(&join, events).await?,
                (None, None) => return Err("Not paired with a Mac yet.".into()),
            };
            node
        }
    };
    connect(&path, node, events).await
}

/// Saves the node, then reads the Mac's health as this device. The save comes first so a
/// connection that fails afterwards does not lose a pairing the owner already allowed.
async fn connect(path: &Path, node: Node, events: &UnboundedSender<Event>) -> Result<(), String> {
    save(path, &node)?;
    let sign: Signer = Arc::new(identity::sign);
    let client = DeviceClient::new(node, sign)?;
    let health = client.get::<NodeHealth>(HEALTH).await?;
    let _ = events.unbounded_send(Event::Ready(client.node().clone(), health));
    Ok(())
}

/// Asks to join, shows the codes to compare, and waits until the owner decides on the Mac.
async fn join_and_wait(target: &str, events: &UnboundedSender<Event>) -> Result<Node, String> {
    let (base, pin) = device::parse_join_target(target)?;
    let request = device::request_join(&base, &pin, label(), &identity::public_key()?).await?;
    let _ = events.unbounded_send(Event::Status(format!(
        "Allow \"{}\" in Devices on the Mac if it shows {} · {}",
        label(),
        device::comparison_code(&pin),
        request.key_code
    )));
    loop {
        tokio::time::sleep(JOIN_POLL).await;
        let state = device::join_state(&base, &pin, &request.id).await?;
        match (state.status.as_str(), state.device) {
            ("pending", _) => continue,
            ("approved", Some(device)) => {
                return Ok(Node {
                    base,
                    pin,
                    device_id: device.id,
                })
            }
            ("denied", _) => return Err("The Mac denied this device.".into()),
            ("expired", _) => return Err("The request expired. Ask again.".into()),
            (other, _) => return Err(format!("The Mac reported the request as {other}.")),
        }
    }
}

/// Claims a code the camera read: the Mac's address, its certificate pin and a one-time code in
/// one string (`device::PairingQr`). The pin is what makes a scan the trusted channel: the claim
/// goes to the host the code named, over a connection pinned to the certificate it named, so a
/// code read off the wrong screen cannot reach a different machine.
async fn claim_scanned(code: &str) -> Result<Node, String> {
    let qr = device::parse_pairing_qr(code)?;
    device::claim(&qr, label(), &identity::public_key()?).await
}

fn save(path: &std::path::Path, node: &Node) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Save the pairing: {e}"))?;
    }
    let json = serde_json::to_vec(node).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("Save the pairing: {e}"))
}

/// The app container's Application Support, where the pairing (not the key) is kept.
fn node_file() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("No app container (HOME is not set).")?;
    Ok(PathBuf::from(home).join("Library/Application Support/sjel-node.json"))
}

/// Where the Mac's read-only copy lives on this phone: beside the pairing, in the container.
fn copy_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("No app container (HOME is not set).")?;
    Ok(PathBuf::from(home).join("Library/Application Support/sjel-mirror"))
}

/// The copy this phone already holds, read without the Mac. The Calendar page opens on this so a
/// launch on a train shows the last sync's day instead of an empty page (`core/src/mirror.rs`).
fn open_copy() -> Result<Arc<Mirror>, String> {
    let dir = copy_dir()?;
    Mirror::open(&dir).map(Arc::new)
}

/// Asks the Mac to build a fresh copy, then fetches whatever it now has.
async fn fetch_copy(client: Arc<DeviceClient>) -> Result<Arc<Mirror>, String> {
    let dir = copy_dir()?;
    // The rebuild is a request, not a demand: the route sits on the Mac's operator gate, and a
    // device that is refused it still gets the copy the Mac already holds (whose age the page
    // states).
    let _ = mirror::rebuild(&*client).await;
    mirror::refresh(&*client, &dir).await?;
    Mirror::open(&dir).map(Arc::new)
}

/// `SJEL_REGISTER=places` opens the Places side of the People tab, and `SJEL_PERSON=Ada` opens the
/// card of the first person whose name contains that text. Both exist so a screenshot can reach a
/// page a hand did not navigate to, like `SJEL_DAY` and `SJEL_TAB`; a phone never sets either.
fn launch_register() -> sjel_gpui_ui::people::Register {
    match std::env::var("SJEL_REGISTER").unwrap_or_default().as_str() {
        "places" => sjel_gpui_ui::people::Register::Places,
        _ => sjel_gpui_ui::people::Register::People,
    }
}

fn launch_person() -> Option<String> {
    std::env::var("SJEL_PERSON")
        .ok()
        .filter(|name| !name.is_empty())
}

fn label() -> &'static str {
    if cfg!(target_abi = "sim") {
        "GPUI iOS Simulator"
    } else {
        "GPUI iPhone"
    }
}

/// `SJEL_DAY=2026-10-21` opens the calendar on that day, for a screenshot a hand did not navigate
/// to. A date it cannot read is today.
fn launch_day() -> chrono::NaiveDate {
    frozen_day().unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// `SJEL_TAB=calendar` opens that tab at launch. Today is the default, because it is the page the
/// app opens on.
///
/// A screenshot of a page that is not the first one needs no hand on the screen for this: the
/// touch checks (`host/UITests`) and `ios/run-sim` use it, and a phone never sets it.
fn launch_tab() -> Tab {
    match std::env::var("SJEL_TAB").unwrap_or_default().as_str() {
        "projects" => Tab::Projects,
        "calendar" => Tab::Calendar,
        "people" => Tab::People,
        "interior" => Tab::Interior,
        "inbox" => Tab::Inbox,
        "travel" => Tab::Travel,
        "settings" => Tab::Settings,
        _ => Tab::Today,
    }
}

/// `SJEL_SIDEBAR=soundscape|activity` opens the right edge at launch on that seat. A phone never sets
/// it; `ios/run-sim` does, because a panel no hand opened still has to be photographable.
///
/// `SJEL_SOUNDSCAPE` keeps its old meaning — `1` opens the edge on the sound seat, `0` closes it —
/// so a script written before the edge had seats still does what it says.
fn launch_edge() -> Option<Seat> {
    if let Ok(seat) = std::env::var("SJEL_SIDEBAR") {
        if seat == "0" || seat.is_empty() {
            return None;
        }
        if let Some(seat) = Seat::from_key(&seat) {
            return Some(seat);
        }
    }
    match std::env::var("SJEL_SOUNDSCAPE").ok().as_deref() {
        Some("0") | Some("") => None,
        Some(_) => Some(Seat::Soundscape),
        None => None,
    }
}

/// `SJEL_SIDEBAR=off` (or `SJEL_SOUNDSCAPE=0`) closes the edge for this launch. A run that takes a
/// picture of a page needs the page, not whatever was left open on it last time.
fn launch_edge_off() -> bool {
    let flag = std::env::var("SJEL_SIDEBAR").unwrap_or_default();
    let sound = std::env::var("SJEL_SOUNDSCAPE").unwrap_or_default();
    matches!(flag.as_str(), "off" | "0") || matches!(sound.as_str(), "off" | "0")
}

fn format_agent_event(event: &AgentEvent) -> String {
    match event.kind.as_str() {
        "turn" => format!("you: {}", event.content),
        "text" => event.content.clone(),
        "tool_call" => format!("{} · {}", event.name, event.arguments),
        "tool_result" => format!("{} · {}", event.name, event.content),
        "needs_approval" => format!("Waiting for approval · {} · {}", event.name, event.preview),
        "approved" => format!("Allowed · {}", event.name),
        "denied" => format!("Denied · {}", event.name),
        "error" => format!("Run error · {}", event.message),
        "done" => "Run finished".to_owned(),
        "stopped" => "Run stopped".to_owned(),
        other => other.to_owned(),
    }
}

fn agent_action_button(
    label: &'static str,
    id: &'static str,
    palette: Palette,
    cx: &mut Context<Capabilities>,
    action: impl Fn(&mut Capabilities, &mut Context<Capabilities>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded(px(7.))
        .bg(rgb(palette.surface))
        .text_color(rgb(palette.text))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, _, _, cx| action(view, cx)),
        )
}

/// The day a launch flag fixed, for a page that must not move under a screenshot. A phone never
/// sets `SJEL_DAY`, so the live landing page reads the clock on every rebuild.
fn frozen_day() -> Option<chrono::NaiveDate> {
    std::env::var("SJEL_DAY")
        .ok()
        .and_then(|day| sjel_gpui_core::calendar::parse_day(&day))
}

impl Capabilities {
    /// Starts offline capture and sends the saved contract to the paired Mac on request.
    fn roomplan_card(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .rounded(px(12.))
            .bg(rgb(0xffffff))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Room scan"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(self.roomplan_status.clone()),
            )
            .child(
                div()
                    .id("roomplan-start")
                    .self_start()
                    .px_3()
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(rgb(palette.accent))
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(0xffffff))
                    .child("Capture a room")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, _| present_roomplan()),
                    ),
            )
            .when(self.client.is_some(), |card| {
                card.child(
                    div()
                        .id("roomplan-send")
                        .self_start()
                        .px_3()
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(rgb(0xe5e7eb))
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(palette.text))
                        .child("Sync pending scans to Mac")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, _| load_roomplan_upload()),
                        ),
                )
            })
    }

    fn reminders_card(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.notify_prefs.enabled;
        let lead = self.notify_prefs.lead();
        let denied = self.notify_state.authorization == notify::Authorization::Denied;
        let sentence: SharedString = match notify::disabled_reason() {
            Some(reason) => reason.into(),
            None if !on => "Off · nothing is announced".into(),
            None => match self.notify_state.next {
                Some((at, ref title)) => format!(
                    "{} · {}",
                    self.notify_state.authorization.phrase(),
                    reminders::when(at, title, chrono::Local::now())
                )
                .into(),
                None if self.notify_state.scheduled > 0 => format!(
                    "{} · {} held",
                    self.notify_state.authorization.phrase(),
                    self.notify_state.scheduled
                )
                .into(),
                None => format!(
                    "{} · nothing scheduled",
                    self.notify_state.authorization.phrase()
                )
                .into(),
            },
        };
        let dropped = self.notify_plan.dropped;
        div()
            .rounded(px(12.))
            .bg(rgb(0xffffff))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Local alerts"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(sentence),
                            ),
                    )
                    .child(
                        div()
                            .id("reminders-toggle")
                            .flex_none()
                            .w(px(48.))
                            .h(px(28.))
                            .p(px(3.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .when(on, |el| el.justify_end().bg(rgb(palette.accent)))
                            .when(!on, |el| el.bg(rgb(0xe5e7eb)))
                            .child(
                                div()
                                    .size(px(22.))
                                    .rounded_full()
                                    .bg(rgb(0xffffff))
                                    .shadow_sm(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.toggle_reminders(cx)),
                            ),
                    ),
            )
            .child(
                div().text_xs().text_color(rgb(palette.muted)).child(
                    "Calendar and task reminders, plus new low-fare notes in upcoming trips.",
                ),
            )
            // A refusal is not this app's to undo: iOS allows the prompt once, so the way back is
            // the system's own page for this app.
            .when(denied, |card| {
                card.child(
                    div()
                        .id("reminders-open-settings")
                        .self_end()
                        .px_3()
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(rgb(0xe5e7eb))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(0x1c1c1e))
                        .child("Open iOS Settings")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, _| notify::open_settings()),
                        ),
                )
            })
            .when(on, |card| {
                card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("HOW EARLY"),
                        )
                        .child({
                            let mut row = div().flex().items_center().gap_1();
                            for option in reminders::Lead::ALL {
                                let is_active = lead == option;
                                row = row.child(
                                    div()
                                        .flex_1()
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .when(is_active, |el| {
                                            el.bg(rgb(palette.accent))
                                                .text_color(rgb(0xffffff))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                        })
                                        .when(!is_active, |el| {
                                            el.bg(rgb(0xf3f5f9)).text_color(rgb(palette.muted))
                                        })
                                        .child(option.label())
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |view, _, _, cx| {
                                                view.notify_prefs.set_lead(option);
                                                let _ = view.notify_prefs.save();
                                                view.rebuild_reminders(cx);
                                            }),
                                        ),
                                );
                            }
                            row
                        })
                        // The cap is admitted rather than hidden: a shortened list that says nothing
                        // reads as though it were the whole of it.
                        .when(dropped > 0, |column| {
                            column.child(div().text_xs().text_color(rgb(0x8e8e93)).child(format!(
                                "{} further date{} past the {} iOS holds",
                                dropped,
                                if dropped == 1 { "" } else { "s" },
                                reminders::MAX_PENDING
                            )))
                        }),
                )
            })
    }
}

impl Render for Capabilities {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dark = self.appearance.theme_mode.is_dark(false);
        let base_palette = if is_dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let palette = self.appearance.apply_to_palette(base_palette, is_dark);
        let page_bg = palette.window_tint >> 8;
        let is_mac = self.active_tab == Tab::Settings;
        let is_today = self.active_tab == Tab::Today;
        let is_inbox = self.active_tab == Tab::Inbox;
        let is_travel = self.active_tab == Tab::Travel;
        let is_calendar = self.active_tab == Tab::Calendar;
        let is_people = self.active_tab == Tab::People;
        let is_interior = self.active_tab == Tab::Interior;
        self.travel
            .update(cx, |travel, cx| travel.set_palette(palette, cx));
        self.calendar
            .update(cx, |calendar, cx| calendar.set_palette(palette, cx));
        self.people
            .update(cx, |people, cx| people.set_palette(palette, cx));
        self.interior
            .update(cx, |history, cx| history.set_palette(palette, cx));
        self.today
            .update(cx, |today, cx| today.set_palette(palette, cx));
        self.inbox
            .update(cx, |inbox, cx| inbox.set_palette(palette, cx));
        self.soundscape
            .update(cx, |drawer, cx| drawer.set_palette(palette, cx));
        if let Some(projects) = &self.projects {
            projects.update(cx, |projects, cx| projects.set_palette(palette, cx));
        }
        div()
            .id("capabilities")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(page_bg))
            .text_color(rgb(palette.text))
            // Scrollable content area
            .child(
                div()
                    .id("ios-scroll-content")
                    .when(is_travel, |page| {
                        page.track_scroll(&self.travel.read(cx).scroll_handle())
                    })
                    .flex_1()
                    .min_h_0()
                    // Projects scrolls inside itself, so the page holds still around it.
                    .when(
                        is_mac
                            || is_today
                            || is_inbox
                            || is_travel
                            || is_calendar
                            || is_people
                            || is_interior,
                        |page| page.overflow_y_scroll(),
                    )
                    // gpui-mobile raises no keyboard on focus. Only text fields take focus here,
                    // so after a tap the keyboard follows whether one holds it.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|_, _, window, cx| keyboard(window.focused(cx).is_some())),
                    )
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when(is_mac, |page| {
                        // Settings is reached from the landing page's gear, so the bar holds no
                        // item that lights up here: the way back is this row.
                        page.child(
                            div()
                                .id("settings-back")
                                .flex()
                                .items_center()
                                .min_h(px(44.))
                                .text_sm()
                                .text_color(rgb(palette.accent))
                                .child("← Today")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|view, _, _, cx| {
                                        view.active_tab = Tab::Today;
                                        cx.notify();
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x646b77))
                                .child(self.status.clone()),
                        )
                        .when_some(self.synced.clone(), |page, synced| {
                            page.child(div().text_xs().text_color(rgb(0x8e8e93)).child(synced))
                        })
                        // The camera is the trusted channel: one scan finds the Mac, pins its
                        // certificate and claims a code, so nothing is compared by eye. The
                        // Join row below stays for a phone whose camera is unavailable.
                        .when(self.rows.is_empty(), |sub| {
                            sub.child(
                                div()
                                    .rounded(px(12.))
                                    .bg(rgb(0xffffff))
                                    .px_4()
                                    .py(px(12.))
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            // Without this the column takes its text's own width,
                                            // the sentence runs off the card, and the pill beside
                                            // it is pushed off screen entirely.
                                            .min_w_0()
                                            .gap_1()
                                            .child("Scan your Mac’s code")
                                            .child(
                                                div().text_sm().text_color(rgb(0x646b77)).child(
                                                    "Setup on the Mac shows a square code. \
                                                         Point this phone at it.",
                                                ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .px_4()
                                            .py(px(8.))
                                            .rounded(px(8.))
                                            .text_color(rgb(0xffffff))
                                            .bg(if self.joining {
                                                rgb(0x9aa0a6)
                                            } else {
                                                rgb(0x0a84ff)
                                            })
                                            .child(if self.joining { "Working…" } else { "Scan" })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| {
                                                    if view.joining {
                                                        return;
                                                    }
                                                    view.status =
                                                        "Point the camera at the code on your Mac."
                                                            .into();
                                                    present_pair_scanner();
                                                    cx.notify();
                                                }),
                                            ),
                                    ),
                            )
                        })
                        .when(!self.found.is_empty() && self.rows.is_empty(), |sub| {
                            sub.child(
                                div()
                                    .rounded(px(12.))
                                    .bg(rgb(0xffffff))
                                    .flex()
                                    .flex_col()
                                    .children(self.found.iter().map(|found| {
                                        let tap = found.clone();
                                        div()
                                            .px_4()
                                            .py(px(12.))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(found.name.clone())
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(0x646b77))
                                                            .child(found.address()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_4()
                                                    .py(px(8.))
                                                    .rounded(px(8.))
                                                    .text_color(rgb(0xffffff))
                                                    .bg(if self.joining {
                                                        rgb(0x9aa0a6)
                                                    } else {
                                                        rgb(0x0a84ff)
                                                    })
                                                    .child(if self.joining {
                                                        "Asking…"
                                                    } else {
                                                        "Join"
                                                    })
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |view, _, _, cx| {
                                                            if view.joining {
                                                                return;
                                                            }
                                                            let _ = view.events.unbounded_send(
                                                                Event::Join(tap.clone()),
                                                            );
                                                            cx.notify();
                                                        }),
                                                    ),
                                            )
                                    })),
                            )
                        })
                        .when(!self.rows.is_empty(), |sub| {
                            sub.child(
                                div()
                                    .rounded(px(12.))
                                    .bg(rgb(0xffffff))
                                    .flex()
                                    .flex_col()
                                    .children(self.rows.iter().map(|(name, up)| {
                                        div()
                                            .px_4()
                                            .py(px(10.))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div().size(px(8.)).rounded_full().bg(rgb(if *up {
                                                    0x248a3d
                                                } else {
                                                    0x8e8e93
                                                })),
                                            )
                                            .child(name.clone())
                                    })),
                            )
                        })
                        // The reminder card comes before the appearance one: it asks for a
                        // decision, where the rest of this page records a taste.
                        .child(self.roomplan_card(palette, cx))
                        .child(self.reminders_card(palette, cx))
                        .child(
                            div()
                                .rounded(px(12.))
                                .bg(rgb(0xffffff))
                                .p_4()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child("Appearance & Theme"),
                                )
                                // Theme Mode
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child("THEME MODE"),
                                        )
                                        .child({
                                            let mut row = div().flex().items_center().gap_1();
                                            for mode in ThemeMode::ALL {
                                                let is_active = self.appearance.theme_mode == mode;
                                                row = row.child(
                                                    div()
                                                        .flex_1()
                                                        .py(px(6.))
                                                        .rounded(px(6.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .text_xs()
                                                        .when(is_active, |el| {
                                                            el.bg(rgb(palette.accent))
                                                                .text_color(rgb(0xffffff))
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                )
                                                        })
                                                        .when(!is_active, |el| {
                                                            el.bg(rgb(0xf3f5f9))
                                                                .text_color(rgb(palette.muted))
                                                        })
                                                        .child(mode.label())
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.appearance.theme_mode = mode;
                                                                let _ = view.appearance.save();
                                                                cx.notify();
                                                            }),
                                                        ),
                                                );
                                            }
                                            row
                                        }),
                                )
                                // Accent Color
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child("ACCENT COLOR"),
                                        )
                                        .child({
                                            let mut row = div()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .gap_2();
                                            for accent in AccentColor::ALL {
                                                let is_active = self.appearance.accent == accent;
                                                let color = accent.swatch_rgb();
                                                row = row.child(
                                                    div()
                                                        .size(px(26.))
                                                        .rounded_full()
                                                        .bg(rgb(color))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .when(is_active, |el| {
                                                            el.border_2()
                                                                .border_color(rgb(palette.text))
                                                        })
                                                        .child(
                                                            div()
                                                                .size(px(6.))
                                                                .rounded_full()
                                                                .when(is_active, |dot| {
                                                                    dot.bg(rgb(0xffffff))
                                                                }),
                                                        )
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.appearance.accent = accent;
                                                                let _ = view.appearance.save();
                                                                cx.notify();
                                                            }),
                                                        ),
                                                );
                                            }
                                            row
                                        }),
                                )
                                // Background Tone
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child("BACKGROUND TONE"),
                                        )
                                        .child({
                                            let mut row = div().flex().items_center().gap_1();
                                            for tone in BackgroundTone::ALL {
                                                let is_active = self.appearance.tone == tone;
                                                let preview_rgb =
                                                    tone.tint_rgb(true).unwrap_or(0x10131a);
                                                row = row.child(
                                                    div()
                                                        .flex_1()
                                                        .py(px(6.))
                                                        .rounded(px(6.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .gap_1()
                                                        .text_xs()
                                                        .when(is_active, |el| {
                                                            el.bg(rgb(palette.accent))
                                                                .text_color(rgb(0xffffff))
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                )
                                                        })
                                                        .when(!is_active, |el| {
                                                            el.bg(rgb(0xf3f5f9))
                                                                .text_color(rgb(palette.muted))
                                                        })
                                                        .child(
                                                            div()
                                                                .size(px(6.))
                                                                .rounded_full()
                                                                .bg(rgb(preview_rgb)),
                                                        )
                                                        .child(tone.label())
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.appearance.tone = tone;
                                                                let _ = view.appearance.save();
                                                                cx.notify();
                                                            }),
                                                        ),
                                                );
                                            }
                                            row
                                        }),
                                )
                                // Density
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child("TOUCH DENSITY"),
                                        )
                                        .child({
                                            let mut row = div().flex().items_center().gap_1();
                                            for den in [Density::Mobile, Density::Comfortable] {
                                                let is_active = self.appearance.density == den;
                                                let label = if den == Density::Mobile {
                                                    "Touch 44pt"
                                                } else {
                                                    "Compact 36pt"
                                                };
                                                row = row.child(
                                                    div()
                                                        .flex_1()
                                                        .py(px(6.))
                                                        .rounded(px(6.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .text_xs()
                                                        .when(is_active, |el| {
                                                            el.bg(rgb(palette.accent))
                                                                .text_color(rgb(0xffffff))
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                )
                                                        })
                                                        .when(!is_active, |el| {
                                                            el.bg(rgb(0xf3f5f9))
                                                                .text_color(rgb(palette.muted))
                                                        })
                                                        .child(label)
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.appearance.density = den;
                                                                let _ = view.appearance.save();
                                                                cx.notify();
                                                            }),
                                                        ),
                                                );
                                            }
                                            row
                                        }),
                                ),
                        )
                    })
                    .when(is_today, |page| page.child(self.today.clone()))
                    .when(is_inbox, |page| page.child(self.inbox.clone()))
                    .when(is_travel, |page| page.child(self.travel.clone()))
                    .when(is_calendar, |page| page.child(self.calendar.clone()))
                    .when(is_people, |page| page.child(self.people.clone()))
                    .when(is_interior, |page| {
                        page.child(
                            div()
                                .rounded(px(12.))
                                .bg(rgb(palette.surface))
                                .p_3()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .child(self.roomplan_status.clone()),
                                )
                                .child(
                                    div()
                                        .id("interior-capture")
                                        .flex_none()
                                        .px_3()
                                        .py(px(8.))
                                        .rounded(px(8.))
                                        .bg(rgb(palette.accent))
                                        .text_xs()
                                        .text_color(rgb(0xffffff))
                                        .child("Scan room")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|_, _, _, _| present_roomplan()),
                                        ),
                                ),
                        )
                        .child(self.interior.clone())
                    })
                    .when(
                        !is_mac
                            && !is_today
                            && !is_inbox
                            && !is_travel
                            && !is_calendar
                            && !is_people
                            && !is_interior,
                        |page| {
                            let pending = self.board.as_ref().map_or(0, Board::pending);
                            page.when_some(self.store_error.clone(), |page, error| {
                                page.child(div().text_sm().text_color(rgb(0xd70015)).child(error))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(2.))
                                            .rounded_full()
                                            .bg(if pending > 0 {
                                                rgb(0xff9500)
                                            } else {
                                                rgb(0x248a3d)
                                            })
                                            .text_color(rgb(0xffffff))
                                            .text_xs()
                                            .child(if pending > 0 {
                                                format!("{pending} queued for the Mac")
                                            } else {
                                                "Saved on this phone".into()
                                            }),
                                    )
                                    .when(self.client.is_some(), |row| {
                                        row.child(
                                            div()
                                                .px_3()
                                                .py(px(6.))
                                                .rounded(px(8.))
                                                .bg(rgb(0xe5e7eb))
                                                .text_color(if self.syncing {
                                                    rgb(0x8e8e93)
                                                } else {
                                                    rgb(0x1c1c1e)
                                                })
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .child(if self.syncing {
                                                    "Syncing…"
                                                } else {
                                                    "Sync"
                                                })
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|view, _, _, cx| {
                                                        view.maybe_sync(cx)
                                                    }),
                                                ),
                                        )
                                    }),
                            )
                            .when_some(self.synced.clone(), |page, synced| {
                                page.child(div().text_xs().text_color(rgb(0x8e8e93)).child(synced))
                            })
                            .when_some(
                                self.projects.clone(),
                                |page, projects| {
                                    page.child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .mb(px(keyboard_inset()))
                                            .child(projects),
                                    )
                                },
                            )
                        },
                    ),
            )
            // Bottom Tab Bar
            .child({
                let pending = self.board.as_ref().map_or(0, Board::pending);
                // Settings and Projects remain reachable from Today; the queue badge stays there.
                let items = [
                    NavItem {
                        id: "today",
                        label: "Today",
                        icon: "☀",
                        badge: if pending > 0 { Some(pending) } else { None },
                    },
                    NavItem {
                        id: "inbox",
                        label: "Inbox",
                        icon: "✎",
                        badge: None,
                    },
                    NavItem {
                        id: "calendar",
                        label: "Calendar",
                        icon: "📅",
                        badge: None,
                    },
                    NavItem {
                        id: "people",
                        label: "People",
                        icon: "👤",
                        badge: None,
                    },
                    NavItem {
                        id: "travel",
                        label: "Travel",
                        icon: "◎",
                        badge: None,
                    },
                    NavItem {
                        id: "interior",
                        label: "Interior",
                        icon: "⌂",
                        badge: None,
                    },
                ];
                let active_id = match self.active_tab {
                    // Settings is behind Today's gear and Projects behind a project row, so
                    // neither lights a tab of its own.
                    Tab::Settings | Tab::Today => "today",
                    Tab::Projects => "today",
                    Tab::Inbox => "inbox",
                    Tab::Travel => "travel",
                    Tab::Calendar => "calendar",
                    Tab::People => "people",
                    Tab::Interior => "interior",
                };
                let entity = cx.entity().clone();
                mobile_tab_bar(
                    &items,
                    active_id,
                    palette,
                    move |id: &'static str, _: &mut gpui::Window, app: &mut gpui::App| {
                        entity.update(app, |view, cx| {
                            if view.active_tab == Tab::Travel {
                                view.travel.update(cx, |travel, _| travel.deactivate());
                            }
                            view.active_tab = match id {
                                "inbox" => Tab::Inbox,
                                "travel" => Tab::Travel,
                                "calendar" => Tab::Calendar,
                                "people" => Tab::People,
                                "interior" => Tab::Interior,
                                _ => Tab::Today,
                            };
                            if view.active_tab == Tab::Travel {
                                view.travel.update(cx, |travel, cx| travel.activate(cx));
                            }
                            if view.active_tab == Tab::Interior {
                                view.interior.update(cx, |history, cx| history.refresh(cx));
                                load_roomplan_archive();
                            }
                            // Opening the calendar or the registers with no copy asks for one, so a
                            // phone that was paired here but never fetched does not have to know to
                            // tap Refresh.
                            if view.active_tab == Tab::Inbox {
                                view.inbox.update(cx, |inbox, cx| inbox.load(cx));
                            }
                            // Settings is where the reminder card lives, so the system's own count
                            // is asked for again as it comes into view rather than drawn from a
                            // number that was true an hour ago.
                            if view.active_tab == Tab::Settings {
                                notify::report();
                            }
                            if (view.active_tab == Tab::Today
                                || view.active_tab == Tab::Calendar
                                || view.active_tab == Tab::People)
                                && view.copy.is_none()
                                && !view.copying
                            {
                                view.refresh_copy(cx);
                            }
                            cx.notify();
                        });
                    },
                )
            })
            // The right edge sits over the page and above the bar, the way the plan puts it: from the
            // edge, not as a sixth tab. One frame with seats (`ui/src/sidebar.rs`), because two panels
            // cannot own one edge: the soundscape is one seat and the phone's activity is the other.
            .when(self.sidebar.is_open(), |page| {
                let entity = cx.entity().clone();
                let seat = self.sidebar.seat();
                let body = match seat {
                    Seat::Soundscape => self.soundscape.clone().into_any_element(),
                    Seat::Activity => self.activity_panel(palette),
                    Seat::Agent => self.agent_panel(palette, window, cx),
                    Seat::Terminal => match (&self.terminal, self.client.is_some()) {
                        (Some((terminal, _)), _) if self.terminal_unlocked => {
                            terminal.update(cx, |view, _| view.set_palette(palette));
                            terminal.clone().into_any_element()
                        }
                        (_, paired) => div()
                            .p_4()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(match (&self.terminal_note, paired) {
                                (_, false) => SharedString::from(
                                    "Pair with the Mac in Settings to open a terminal on it.",
                                ),
                                (Some(note), true) => note.clone(),
                                (None, true) => "Opening a terminal on the Mac…".into(),
                            })
                            .into_any_element(),
                    },
                };
                let on_seat = {
                    let entity = entity.clone();
                    move |seat: Seat, _: &mut Window, app: &mut gpui::App| {
                        entity.update(app, |view, cx| view.open_edge(seat, cx));
                    }
                };
                let on_close = move |_: &mut Window, app: &mut gpui::App| {
                    entity.update(app, |view, cx| view.close_edge(cx));
                };
                page.child(sidebar_scrim(
                    {
                        let entity = cx.entity().clone();
                        move |_: &mut Window, app: &mut gpui::App| {
                            entity.update(app, |view, cx| view.close_edge(cx));
                        }
                    },
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(sidebar_frame(palette, seat, body, on_seat, on_close))
                        .into_any_element(),
                ))
            })
    }
}

/// Called by `host/Discovery.swift` with one browse answer, `{"nodes":[…],"denied":false}`.
///
/// The Swift side browses `_sjel._tcp` and passes every TXT record it can use; this entry point
/// only carries them to the view, because a browse answer arrives on the main queue and the
/// view lives on GPUI's foreground executor.
///
/// # Safety
///
/// `json` must be a NUL-terminated UTF-8 string that stays valid for the call, which is what
/// `withCString` gives the Swift caller.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_nodes(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: the header in `host/Embedding.h` is the caller's contract: a NUL-terminated string
    // that outlives the call (`withCString`), read here and not kept.
    let Ok(text) = unsafe { std::ffi::CStr::from_ptr(json) }.to_str() else {
        return;
    };
    let Ok(browse) = serde_json::from_str::<Browse>(text) else {
        return;
    };
    if browse.denied {
        announce(Event::Status(LOCAL_NETWORK_DENIED.into()));
        return;
    }
    announce(Event::Found(browse.nodes));
}

/// Called by `host/PairScanner.swift` with the text of a pairing code it read.
///
/// The scanner is offered from this app's own Settings tab, so the view already exists by the
/// time a code can arrive: `EVENTS` is set in `Capabilities::new` before any button is drawn.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string that stays valid for the call, which is what
/// `withCString` gives the Swift caller.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_scanned_pairing(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: the header in `host/Embedding.h` is the caller's contract: a NUL-terminated string
    // that outlives the call (`withCString`), read here and not kept.
    let Ok(code) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::ScanFailed("That code could not be read.".into()));
        return;
    };
    announce(Event::Scanned(code.trim().to_string()));
}

/// Called by `host/PairScanner.swift` when the camera will not open: a simulator has none, and a
/// person who refused it once is told where to change that instead of seeing a black rectangle.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string that stays valid for the call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_scan_failed(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: as above: a NUL-terminated string owned by the caller for the call's duration.
    let Ok(reason) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    announce(Event::ScanFailed(reason.to_string()));
}

/// Called by `host/RoomCapture.swift` with a saved capture summary or a user-facing failure.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_result(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(result) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::RoomplanResult(
            "Room capture returned unreadable text.".into(),
        ));
        return;
    };
    announce(Event::RoomplanResult(result.to_owned()));
}

/// Called by `host/RoomCapture.swift` with a local, live semantic snapshot and coverage counters.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 JSON string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_live(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the JSON alive for this call.
    let Ok(json) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    let Ok(snapshot) = serde_json::from_str::<RoomPlanLiveUpdate>(json) else {
        return;
    };
    announce(Event::RoomplanLive(snapshot));
}

/// Called by `host/RoomCapture.swift` with the locally stored draft and USDZ upload envelope.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_upload(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(body) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::RoomplanResult(
            "Room capture could not be read for upload.".into(),
        ));
        return;
    };
    announce(Event::RoomplanUpload(body.to_owned()));
}

/// Called by `host/RoomCapture.swift` with the phone's locally archived scan summaries.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_archive(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(json) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    let Ok(records) = serde_json::from_str::<Vec<RoomPlanLocalRecord>>(json) else {
        return;
    };
    announce(Event::RoomplanArchive(records));
}

/// Called by `host/App.swift` when the scene becomes active: the phone may have been in a pocket
/// through several days, so the reminders are rebuilt from the replica the moment it is looked at.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_became_active() {
    announce(Event::Foreground);
}

/// Called by `host/App.swift` when the scene enters the background. A Face ID sheet only resigns
/// active, so this is the edge that ends a foreground session.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_entered_background() {
    announce(Event::Background);
}

/// Called by `host/App.swift` with the answer to `sjel_ios_unlock`, on the main queue.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_unlocked(ok: bool) {
    announce(Event::Unlocked(ok));
}

/// Called by `host/App.swift` before `gpui_ios_run_demo`: registers what the GPUI window shows.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_register_app() {
    use gpui::{App, WindowOptions};
    use gpui_kit::component::Root;

    gpui_mobile::ios::ffi::set_app_callback(Box::new(|cx: &mut App| {
        gpui_kit::init(cx);
        let _ = cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(Capabilities::new);
            cx.new(|cx| Root::new(view, window, cx))
        });
    }));
}
