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
use serde::Deserialize;

use gpui::{div, prelude::*, px, rgb, Context, MouseButton, SharedString, Window};
use sjel_gpui_core::data::Board;
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::device::{self, DeviceClient, Node, Signer};
use sjel_gpui_core::mirror::{self, Mirror};
use sjel_gpui_core::shell::{self, NodeHealth};
use sjel_gpui_core::sync::{self, Report};
use sjel_gpui_ui::calendar::{Calendar, CalendarEvent};
use sjel_gpui_ui::projects::{Projects, ProjectsEvent};
use sjel_gpui_ui::{
    mobile_tab_bar, AccentColor, AppearanceSettings, BackgroundTone, Density, NavItem, Palette,
    ThemeMode,
};

mod identity;

const HEALTH: &str = "/api/sjel-status/health";
/// How often a waiting join asks whether the owner decided. A request expires after ten minutes
/// (`capabilities/devices/README.md`), so the wait ends there at the latest.
const JOIN_POLL: Duration = Duration::from_secs(2);
#[cfg(target_os = "ios")]
const LOCAL_NETWORK_DENIED: &str =
    "Sjel may not use the local network. Allow it in Settings, then reopen the app.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Projects,
    /// The day's entries, read from the Mac's read-only copy of its data.
    Calendar,
    Travel,
    /// Pairing with a Mac: offered here, never required (research/offline-on-the-phone.md).
    Settings,
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
    /// The camera could not be opened, and why, in the words the person needs.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    ScanFailed(String),
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
    /// The Calendar page over the Mac's read-only copy (`sjel_gpui_ui::calendar`).
    calendar: gpui::Entity<Calendar>,
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
    appearance: AppearanceSettings,
    events: UnboundedSender<Event>,
}

impl Capabilities {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (events, mut received) = unbounded();
        // The C entry point reads this, so the browse can start before the view exists.
        let _ = EVENTS.set(Mutex::new(Some(events.clone())));
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
            CalendarEvent::Refresh => view.refresh_copy(cx),
        })
        .detach();
        let copy_events = events.clone();
        shell::spawn(async move {
            let _ = copy_events.unbounded_send(Event::Copy(open_copy()));
        });

        cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                let alive = this.update(cx, |view, cx| {
                    match event {
                        Event::Status(text) => view.status = text.into(),
                        Event::StoreFailed(text) => view.store_error = Some(text.into()),
                        Event::Copy(outcome) => {
                            view.copying = false;
                            match outcome {
                                Ok(copy) => {
                                    view.copy = Some(copy.clone());
                                    view.calendar.update(cx, |calendar, cx| {
                                        calendar.set_mirror(Some(copy), None, cx)
                                    });
                                }
                                // A refresh that fails leaves the copy already on screen alone:
                                // it is still true, only older, and the page says how old.
                                Err(error) => {
                                    if view.copy.is_none() {
                                        view.calendar.update(cx, |calendar, cx| {
                                            calendar.set_mirror(None, Some(error), cx)
                                        });
                                    } else {
                                        view.calendar.update(cx, |calendar, cx| {
                                            calendar.set_note(
                                                Some(format!(
                                                    "Could not fetch a newer copy: {error}"
                                                )),
                                                cx,
                                            )
                                        });
                                    }
                                }
                            }
                        }
                        Event::Board(board) => {
                            if view.projects.is_none() {
                                if let Some(store) = board.store().cloned() {
                                    let projects = view.open_projects(store, cx);
                                    view.projects = Some(projects);
                                }
                            }
                            view.board = Some(board);
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
                            view.maybe_sync(cx);
                            view.refresh_copy(cx);
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
                            view.synced = Some(
                                match outcome {
                                    Ok(report) => report.summary(),
                                    Err(error) => format!("Nothing moved: {error}"),
                                }
                                .into(),
                            );
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
            synced: None,
            rows: Vec::new(),
            found: Vec::new(),
            joining: false,
            active_tab: launch_tab(),
            travel: cx.new(|_| sjel_gpui_ui::travel::Travel::new(None, Density::Mobile)),
            projects: None,
            calendar,
            copy: None,
            copying: false,
            store_error: None,
            board: None,
            client: None,
            syncing: false,
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
        let events = self.events.clone();
        let repo = Repo::replica(store, move || {
            let _ = events.unbounded_send(Event::Wrote);
        });
        let projects = cx.new(|_| Projects::new(repo).with_density(Density::Mobile));
        cx.subscribe(&projects, |_, _, event, _| match event {
            ProjectsEvent::Editing(open) => keyboard(*open),
        })
        .detach();
        projects.update(cx, |projects, cx| projects.load_projects(cx));
        projects
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
    std::env::var("SJEL_DAY")
        .ok()
        .and_then(|day| sjel_gpui_core::calendar::parse_day(&day))
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// `SJEL_TAB=calendar` opens that tab at launch.
///
/// A screenshot of a page that is not the first one needs no hand on the screen for this: the
/// touch checks (`host/UITests`) and `ios/run-sim` use it, and a phone never sets it.
fn launch_tab() -> Tab {
    match std::env::var("SJEL_TAB").unwrap_or_default().as_str() {
        "calendar" => Tab::Calendar,
        "travel" => Tab::Travel,
        "settings" => Tab::Settings,
        _ => Tab::Projects,
    }
}

impl Render for Capabilities {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dark = self.appearance.theme_mode.is_dark(false);
        let base_palette = if is_dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let palette = self.appearance.apply_to_palette(base_palette, is_dark);
        let page_bg = palette.window_tint >> 8;
        let is_mac = self.active_tab == Tab::Settings;
        let is_travel = self.active_tab == Tab::Travel;
        let is_calendar = self.active_tab == Tab::Calendar;
        self.travel
            .update(cx, |travel, cx| travel.set_palette(palette, cx));
        self.calendar
            .update(cx, |calendar, cx| calendar.set_palette(palette, cx));
        if let Some(projects) = &self.projects {
            projects.update(cx, |projects, cx| projects.set_palette(palette, cx));
        }
        div()
            .id("capabilities")
            .size_full()
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
                    .when(is_mac || is_travel || is_calendar, |page| {
                        page.overflow_y_scroll()
                    })
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
                        page.child(
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
                    .when(is_travel, |page| page.child(self.travel.clone()))
                    .when(is_calendar, |page| page.child(self.calendar.clone()))
                    .when(!is_mac && !is_travel && !is_calendar, |page| {
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
                                            .child(if self.syncing { "Syncing…" } else { "Sync" })
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| view.maybe_sync(cx)),
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
                    }),
            )
            // Bottom Tab Bar
            .child({
                let pending = self.board.as_ref().map_or(0, Board::pending);
                let items = [
                    NavItem {
                        id: "projects",
                        label: "Projects",
                        icon: "📝",
                        badge: if pending > 0 { Some(pending) } else { None },
                    },
                    NavItem {
                        id: "calendar",
                        label: "Calendar",
                        icon: "📅",
                        badge: None,
                    },
                    NavItem {
                        id: "travel",
                        label: "Travel",
                        icon: "◎",
                        badge: None,
                    },
                    NavItem {
                        id: "settings",
                        label: "Settings",
                        icon: "⚙️",
                        badge: None,
                    },
                ];
                let active_id = match self.active_tab {
                    Tab::Settings => "settings",
                    Tab::Travel => "travel",
                    Tab::Calendar => "calendar",
                    Tab::Projects => "projects",
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
                                "settings" => Tab::Settings,
                                "travel" => Tab::Travel,
                                "calendar" => Tab::Calendar,
                                _ => Tab::Projects,
                            };
                            if view.active_tab == Tab::Travel {
                                view.travel.update(cx, |travel, cx| travel.activate(cx));
                            }
                            // Opening the calendar with no copy asks for one, so a phone that was
                            // paired here but never fetched does not have to know to tap Refresh.
                            if view.active_tab == Tab::Calendar
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
