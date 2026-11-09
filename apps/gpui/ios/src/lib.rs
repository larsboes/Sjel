//! The iOS app's GPUI content. UIKit owns the window, the navigation bar and the safe areas
//! (`host/App.swift`); GPUI draws inside them (`../README.md`, "GPUI for every client").
//!
//! The app is a paired device (`crates/devices/README.md`): it claims the one-time code
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

// GPUI macros expand to `gpui::` paths. The Kit re-exports the GPUI it is built on.
extern crate gpui_kit as gpui;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde::Deserialize;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, MouseButton, SharedString, Window,
};
use sjel_gpui_core::data::Board;
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::device::{self, DeviceClient, Node, Signer};
use sjel_gpui_core::interior::{RoomPlanLiveUpdate, RoomPlanLocalRecord, RoomPlanSource};
use sjel_gpui_core::mirror::{self, Mirror};
use sjel_gpui_core::notify as reminders;
use sjel_gpui_core::route::{self, FeedTab, Page, Target};
use sjel_gpui_core::shell::{self, NodeHealth};
use sjel_gpui_core::sync::{self, Report};
use sjel_gpui_core::terminal::Host;
use sjel_gpui_ui::agent_seat::AgentSeat;
use sjel_gpui_ui::calendar::{Calendar, CalendarEvent};
use sjel_gpui_ui::inbox::{Inbox, InboxEvent};
use sjel_gpui_ui::interior::RoomPlanHistory;
use sjel_gpui_ui::people::{People, PeopleEvent};
use sjel_gpui_ui::projects::{Projects, ProjectsEvent};
use sjel_gpui_ui::sidebar::{sidebar_frame, sidebar_scrim, Seat, Sidebar};
use sjel_gpui_ui::soundscape::Soundscape;
use sjel_gpui_ui::terminal::{TerminalDeck, TerminalEvent};
use sjel_gpui_ui::nsh_today::{self, TodayEvent};
use sjel_gpui_ui::{
    mobile_tab_bar, step_terminal_font_size, AccentColor, AppearanceSettings, BackgroundTone,
    Density, NavItem, NotifyPrefs, Palette, SoundscapePrefs, TerminalFont, ThemeContext, ThemeMode,
    WindowOpacity, TERMINAL_FONT_SIZES,
};

mod audio;
mod edge;
mod events;
mod host;
mod identity;
mod launch;
mod nav;
mod notify;
mod nsh;
mod pairing;

use host::*;
use launch::*;
use pairing::*;

const HEALTH: &str = "/api/sjel-status/health";
/// How often a waiting join asks whether the owner decided. A request expires after ten minutes
/// (`crates/devices/README.md`), so the wait ends there at the latest.
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
    /// Triage and mail proposals synced from the Mac.
    Mail,
    /// Document library and Core Graphics PDF reader, reached from Today.
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
    /// The Calendar page over the Mac's read-only copy (`sjel_gpui_ui::calendar`).
    calendar: gpui::Entity<Calendar>,
    /// The capture field over the phone's own replica (`sjel_gpui_ui::inbox`).
    inbox: gpui::Entity<Inbox>,
    /// The Mail triage proposals synced from the Mac (`sjel_gpui_ui::mail`).
    mail: gpui::Entity<sjel_gpui_ui::mail::Mail>,
    /// The Document library and PDF reader (`sjel_gpui_ui::documents`).
    documents: gpui::Entity<sjel_gpui_ui::documents::Documents>,
    /// The soundscape drawer over the platform-free engine (`sjel_gpui_ui::soundscape`). The device
    /// is the host's (`host/Audio.swift`), started the first time the edge is opened on this seat.
    soundscape: gpui::Entity<Soundscape>,
    /// The right edge: one panel with seats (`sjel_gpui_ui::sidebar`), which owns whether it is
    /// showing and which seat it is on. The soundscape is one seat; the phone's own activity is the
    /// other, and the agent's step stream lands there when a client has one to stream.
    sidebar: Sidebar,
    /// The agent seat over the paired Mac (`sjel_gpui_ui::agent_seat`); the text field is the
    /// phone's own.
    agent: gpui::Entity<AgentSeat>,
    agent_input: Option<(gpui::Entity<InputState>, gpui::Subscription)>,
    /// The terminal seat, kept while the app stays in the foreground: closing the edge leaves the
    /// stream attached. Going to the background drops it, so a reattach asks for Face ID.
    terminal: Option<(gpui::Entity<TerminalDeck>, gpui::Subscription)>,
    /// The pinch scale the font size last stepped at; a new pinch starts from 1.
    pinch_base: f64,
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
        let mail = cx.new(|cx| {
            sjel_gpui_ui::mail::Mail::new(
                sjel_gpui_core::data::Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp)),
                cx,
            )
        });
        mail.update(cx, |mail, cx| mail.set_density(Density::Mobile, cx));
        let documents = cx.new(|_| {
            sjel_gpui_ui::documents::Documents::new(sjel_gpui_core::data::Repo::new(Err(
                sjel_gpui_core::shell::ShellError::NotSetUp,
            )))
        });
        documents.update(cx, |docs, cx| docs.set_density(Density::Mobile, cx));
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
        // The landing page is the NSH Today screen (`sjel_gpui_ui::nsh_today`). It reads the copy
        // for the day and the replica for the tasks; what it cannot do, it hands back here. A launch
        // with `SJEL_DAY` freezes the day, so a screenshot cannot disagree with itself.
        if let Some(day) = frozen_day() {
            sjel_gpui_ui::nsh_today::set_day(day);
        }
        sjel_gpui_ui::nsh_today::set_host(|event| crate::host::announce(Event::Today(event)));
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
        let soundscape = cx.new(|_| Soundscape::new(control, visual, SoundscapePrefs::load()));
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
                    view.handle(event, cx);
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
            agent: cx.new(|_| {
                AgentSeat::new(
                    None,
                    true,
                    "Pair this phone with the Mac to watch its agent runs.",
                )
            }),
            agent_input: None,
            terminal: None,
            terminal_unlocked: false,
            pinch_base: 1.0,
            terminal_note: None,
            synced: None,
            rows: Vec::new(),
            found: Vec::new(),
            joining: false,
            active_tab: launch_tab(),
            travel: cx.new(|cx| sjel_gpui_ui::travel::Travel::new(None, Density::Mobile, cx)),
            projects: None,
            calendar,
            people,
            interior,
            inbox,
            mail,
            documents,
            soundscape,
            sidebar,
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

    /// The Terminal rows of Settings, the same four the Mac's Appearance page has.
    fn terminal_settings(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.appearance;
        let fonts = TerminalFont::available(cx);
        let font_options = fonts
            .iter()
            .map(|font| (font.label().into(), settings.terminal_font == *font))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(choice_row(
                "TERMINAL FONT",
                font_options,
                palette,
                cx,
                move |appearance, ix| appearance.terminal_font = fonts[ix],
            ))
            .child(choice_row(
                "TERMINAL FONT SIZE",
                TERMINAL_FONT_SIZES
                    .iter()
                    .map(|size| {
                        (
                            size.to_string().into(),
                            settings.terminal_font_size == *size,
                        )
                    })
                    .collect(),
                palette,
                cx,
                |appearance, ix| appearance.terminal_font_size = TERMINAL_FONT_SIZES[ix],
            ))
            .child(choice_row(
                "TERMINAL BACKGROUND",
                BackgroundTone::ALL
                    .iter()
                    .map(|tone| (tone.label().into(), settings.terminal_tone == *tone))
                    .collect(),
                palette,
                cx,
                |appearance, ix| appearance.terminal_tone = BackgroundTone::ALL[ix],
            ))
            .child(choice_row(
                "TERMINAL OPACITY",
                WindowOpacity::ALL
                    .iter()
                    .map(|o| (o.short_label().into(), settings.terminal_opacity == *o))
                    .collect(),
                palette,
                cx,
                |appearance, ix| appearance.terminal_opacity = WindowOpacity::ALL[ix],
            ))
            .into_any_element()
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
        let projects = cx.new(|cx| Projects::new(repo, cx).with_density(Density::Mobile));
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
/// How far a pinch travels, as a scale ratio, before it steps the font size once.
const PINCH_RATIO: f64 = 1.2;

/// The font-size steps a pinch at `scale` makes since `base`, moving `base` to where it stepped.
fn pinch_steps(base: &mut f64, scale: f64) -> i32 {
    let ratio = scale / *base;
    let steps = if ratio >= PINCH_RATIO {
        1
    } else if ratio <= 1.0 / PINCH_RATIO {
        -1
    } else {
        0
    };
    if steps != 0 {
        *base = scale;
    }
    steps
}

/// One labelled row of choices on the phone's Settings page. A tap writes `appearance.json`, the
/// way the theme and tone rows above it do.
fn choice_row(
    title: &'static str,
    options: Vec<(SharedString, bool)>,
    palette: Palette,
    cx: &mut Context<Capabilities>,
    pick: impl Fn(&mut AppearanceSettings, usize) + 'static,
) -> AnyElement {
    let pick = std::rc::Rc::new(pick);
    let mut row = div().flex().items_center().gap_1();
    for (ix, (label, active)) in options.into_iter().enumerate() {
        let pick = pick.clone();
        row = row.child(
            div()
                .flex_1()
                .py(px(6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .when(active, |el| {
                    el.bg(rgb(palette.accent))
                        .text_color(rgb(0xffffff))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when(!active, |el| {
                    el.bg(rgb(0xf3f5f9)).text_color(rgb(palette.muted))
                })
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        pick(&mut view.appearance, ix);
                        let _ = view.appearance.save();
                        cx.notify();
                    }),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(rgb(palette.muted)).child(title))
        .child(row)
        .into_any_element()
}

/// The card an unpaired phone shows on a page whose data lives only on the Mac.
fn needs_mac(
    id: &'static str,
    page: &'static str,
    palette: Palette,
    cx: &mut Context<Capabilities>,
) -> AnyElement {
    div()
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_base()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(format!("{page} needs your Mac")),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(format!(
                "{page} is read from the Mac this phone is paired with. This phone is not paired."
            )),
        )
        .child(
            div()
                .id(id)
                .min_h(px(44.))
                .flex()
                .items_center()
                .text_sm()
                .text_color(rgb(palette.accent))
                .child("Pair in Settings →")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| {
                        view.active_tab = Tab::Settings;
                        cx.notify();
                    }),
                ),
        )
        .into_any_element()
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
        let palette = ThemeContext::sync(self.appearance, window, cx);
        let page_bg = palette.window_tint >> 8;
        let is_mac = self.active_tab == Tab::Settings;
        let is_today = self.active_tab == Tab::Today;
        let is_inbox = self.active_tab == Tab::Inbox;
        let is_travel = self.active_tab == Tab::Travel;
        let is_calendar = self.active_tab == Tab::Calendar;
        let is_people = self.active_tab == Tab::People;
        let is_interior = self.active_tab == Tab::Interior;
        let is_mail = self.active_tab == Tab::Mail;
        let is_documents = self.active_tab == Tab::Documents;
        // Mail and Documents are read live from the Mac (docs/plans/gpui.md, "What may live on the
        // phone"), so an unpaired phone says so instead of showing an empty list.
        let paired = self.client.is_some();
        self.mail
            .update(cx, |mail, cx| mail.set_palette(palette, cx));
        self.documents
            .update(cx, |docs, cx| docs.set_palette(palette, cx));
        self.travel
            .update(cx, |travel, cx| travel.set_palette(palette, cx));
        self.calendar
            .update(cx, |calendar, cx| calendar.set_palette(palette, cx));
        self.people
            .update(cx, |people, cx| people.set_palette(palette, cx));
        self.interior
            .update(cx, |history, cx| history.set_palette(palette, cx));
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
                            || is_interior
                            || is_mail,
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
                        // Join row below stays for a phone whose camera is unavailable. A
                        // connected phone keeps the row to pair again: a new code can carry
                        // another address, such as the Mac's tailnet one.
                        .child({
                            let paired = !self.rows.is_empty();
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
                                        .child(if paired {
                                            "Pair again"
                                        } else {
                                            "Scan your Mac’s code"
                                        })
                                        .child(div().text_sm().text_color(rgb(0x646b77)).child(
                                            if paired {
                                                "Scan a new code from Setup on the Mac, \
                                                         for example to reach it over the \
                                                         tailnet. This phone keeps its \
                                                         pairing; only the address changes."
                                            } else {
                                                "Setup on the Mac shows a square code. \
                                                         Point this phone at it."
                                            },
                                        )),
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
                                )
                                .child(self.terminal_settings(palette, cx)),
                        )
                    })
                    .when(is_mail && paired, |page| page.child(self.mail.clone()))
                    .when(is_mail && !paired, |page| {
                        page.child(needs_mac("needs-mac-mail", "Mail", palette, cx))
                    })
                    .when(is_inbox, |page| page.child(self.inbox.clone()))
                    .when(is_travel, |page| page.child(self.travel.clone()))
                    .when(is_calendar, |page| page.child(self.calendar.clone()))
                    .when(is_people, |page| page.child(self.people.clone()))
                    .when(is_documents, |page| {
                        page.child(
                            div()
                                .id("documents-back")
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
                        .when(paired, |page| {
                            page.child(div().flex_1().min_h_0().child(self.documents.clone()))
                        })
                        .when(!paired, |page| {
                            page.child(needs_mac("needs-mac-documents", "Documents", palette, cx))
                        })
                    })
                    .when(is_interior, |page| {
                        page.child(
                            div()
                                .rounded(px(12.))
                                .bg(rgba(palette.surface))
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
                            && !is_mail
                            && !is_documents
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
                        id: "mail",
                        label: "Mail",
                        icon: "✉",
                        badge: None,
                    },
                    NavItem {
                        id: "calendar",
                        label: "Calendar",
                        icon: "▦",
                        badge: None,
                    },
                    NavItem {
                        id: "travel",
                        label: "Travel",
                        icon: "◎",
                        badge: None,
                    },
                    NavItem {
                        id: "agent",
                        label: "Agent",
                        icon: "⚡",
                        badge: None,
                    },
                ];
                let active_id = match self.active_tab {
                    // Settings is behind Today's gear and Projects behind a project row, so
                    // neither lights a tab of its own.
                    Tab::Settings | Tab::Today => "today",
                    Tab::Projects => "today",
                    Tab::Mail => "mail",
                    Tab::Travel => "travel",
                    Tab::Calendar => "calendar",
                    Tab::Inbox => "today",
                    Tab::People => "today",
                    Tab::Interior => "today",
                    Tab::Documents => "today",
                };
                let entity = cx.entity().clone();
                mobile_tab_bar(
                    &items,
                    active_id,
                    palette,
                    move |id: &'static str, _: &mut gpui::Window, app: &mut gpui::App| {
                        entity.update(app, |view, cx| match id {
                            "agent" => view.open_edge(Seat::Agent, cx),
                            "mail" => view.select_tab(Tab::Mail, cx),
                            "travel" => view.select_tab(Tab::Travel, cx),
                            "calendar" => view.select_tab(Tab::Calendar, cx),
                            _ => view.select_tab(Tab::Today, cx),
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
                        (Some((deck, _)), _) if self.terminal_unlocked => {
                            let settings = self.appearance;
                            deck.update(cx, |deck, _| {
                                deck.set_style(
                                    palette,
                                    settings.terminal_font,
                                    settings.terminal_font_size,
                                )
                            });
                            deck.clone().into_any_element()
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
                    // The edge lies over the page, so it is always filled; the frame leaves the
                    // terminal seat clear for the Mac, where it docks beside the window instead.
                    div()
                        .flex_1()
                        .min_h_0()
                        .bg(gpui::rgba(palette.panel()))
                        .child(sidebar_frame(palette, seat, body, on_seat, on_close))
                        .into_any_element(),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinch_steps_once_per_ratio_and_restarts_from_where_it_stepped() {
        let mut base = 1.0;
        assert_eq!(pinch_steps(&mut base, 1.1), 0);
        assert_eq!(pinch_steps(&mut base, 1.25), 1);
        // The next step is measured from 1.25, not from where the pinch began.
        assert_eq!(pinch_steps(&mut base, 1.4), 0);
        assert_eq!(pinch_steps(&mut base, 1.5), 1);
        assert_eq!(pinch_steps(&mut base, 1.2), -1);
    }
}
