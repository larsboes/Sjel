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
use sjel_gpui_core::blocks::Doc;
use sjel_gpui_core::data::Board;
use sjel_gpui_core::device::{self, DeviceClient, Node, Signer};
use sjel_gpui_core::shell::{self, NodeHealth, ProjectRecord};
use sjel_gpui_core::sync::{self, Report};
use sjel_gpui_ui::{
    mobile_tab_bar, render_doc, AccentColor, AppearanceSettings, BackgroundTone, Density, NavItem,
    Palette, ThemeMode,
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
    TasksNotes,
    /// Pairing with a Mac: offered here, never required (research/offline-on-the-phone.md).
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    All,
    Tasks,
    Notes,
}

enum Event {
    Status(String),
    /// What the last Bonjour browse found. Empty is a normal answer, not an error. Only an iOS
    /// host browses (`sjel_ios_nodes`), so the macOS build of this crate never constructs it.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Found(Vec<Found>),
    /// The person tapped Join on one of them.
    Join(Found),
    Ready(Node, NodeHealth),
    /// The phone's replica after it was opened, read or written: its rows and queued count.
    Board(Board),
    /// The replica did not open, and why. Nothing typed can be kept until it does.
    StoreFailed(String),
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

/// Sends one event to the live view, if there is one. Called from the main queue.
#[cfg(target_os = "ios")]
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
    filter: FilterKind,
    /// The phone's own copy of the Mac's records and the queue beside it, or `None` until it
    /// opens. `Board` is the view model: it answers the rows to draw without this view filtering
    /// them, and it keeps the `TursoStore` the sync needs.
    board: Option<Board>,
    selected_note: Option<ProjectRecord>,
    /// Why the replica did not open, shown on the Tasks & Notes page in place of a silent blank.
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

        cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                let alive = this.update(cx, |view, cx| {
                    match event {
                        Event::Status(text) => view.status = text.into(),
                        Event::StoreFailed(text) => view.store_error = Some(text.into()),
                        Event::Board(board) => {
                            // The open note follows the replica, so a body this app just saved is
                            // the one still on screen.
                            if let Some(ref current) = view.selected_note {
                                if let Some(updated) =
                                    board.records().iter().find(|r| r.path == current.path)
                                {
                                    view.selected_note = Some(updated.clone());
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
                                Ok(client) => view.client = Some(Arc::new(client)),
                                Err(error) => {
                                    view.synced = Some(format!("Cannot sync: {error}").into())
                                }
                            }
                            view.maybe_sync(cx);
                        }
                        Event::Synced(outcome, board) => {
                            view.syncing = false;
                            view.board = Some(board);
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
            active_tab: Tab::TasksNotes,
            filter: FilterKind::All,
            selected_note: None,
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

    fn toggle_task(&mut self, path: String, done: bool, cx: &mut Context<Self>) {
        let Some(mut board) = self.board.clone() else {
            return;
        };
        // The tick lands in the frame it was made in; the write below settles it.
        board.mark_done(&path, done);
        self.board = Some(board.clone());
        cx.notify();

        let events = self.events.clone();
        shell::spawn(async move {
            let mut board = board;
            let _ = board.toggle_task(&path, done).await;
            let _ = events.unbounded_send(Event::Board(board));
        });
    }

    fn add_quick_task(&mut self, cx: &mut Context<Self>) {
        let Some(board) = self.board.clone() else {
            return;
        };
        let events = self.events.clone();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let title = format!("Mobile Task #{timestamp}");

        shell::spawn(async move {
            let mut board = board;
            // The path is the replica's to name, so two captures a second apart cannot collide.
            let _ = board.add_task("Inbox", &title).await;
            let _ = events.unbounded_send(Event::Board(board));
        });
        cx.notify();
    }

    fn open_note(&mut self, note: ProjectRecord, cx: &mut Context<Self>) {
        self.selected_note = Some(note);
        cx.notify();
    }

    fn close_note(&mut self, cx: &mut Context<Self>) {
        self.selected_note = None;
        cx.notify();
    }

    fn add_quick_note(&mut self, cx: &mut Context<Self>) {
        let Some(board) = self.board.clone() else {
            return;
        };
        let events = self.events.clone();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = format!("Inbox/Note-{timestamp}.md");
        let title = format!("Mobile Note #{timestamp}");
        let content = format!("# {title}\n\n- [ ] First thought\n\nCaptured offline on iOS.");

        shell::spawn(async move {
            let mut board = board;
            let _ = board.save_note("Inbox", &path, &title, &content).await;
            let _ = events.unbounded_send(Event::Board(board));
        });
        cx.notify();
    }

    fn cycle_task_priority(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(mut board) = self.board.clone() else {
            return;
        };
        let next_p = board.cycle_priority(&path);
        self.board = Some(board.clone());
        cx.notify();

        let events = self.events.clone();
        shell::spawn(async move {
            let mut board = board;
            let _ = board.set_priority(&path, next_p).await;
            let _ = events.unbounded_send(Event::Board(board));
        });
    }

    fn delete_note(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(board) = self.board.clone() else {
            return;
        };
        self.selected_note = None;
        cx.notify();

        let events = self.events.clone();
        shell::spawn(async move {
            let mut board = board;
            let _ = board.delete_record(&path).await;
            let _ = events.unbounded_send(Event::Board(board));
        });
    }

    fn toggle_note_block_todo(&mut self, block_idx: usize, cx: &mut Context<Self>) {
        let Some(ref mut note) = self.selected_note else {
            return;
        };
        let mut doc = Doc::parse(&note.content);
        if let Some(block) = doc.blocks.get_mut(block_idx) {
            block.toggle_done();
            let new_content = doc.markdown();
            note.content = new_content.clone();
            let path = note.path.clone();
            let title = note.title.clone();
            let project = path.split('/').next().unwrap_or("Inbox").to_string();

            // The new body shows in the list at once; `save_note` below writes it and queues the
            // write for the Mac.
            if let Some(board) = self.board.as_mut() {
                board.set_content(&path, &new_content);
            }
            cx.notify();

            if let Some(board) = self.board.clone() {
                let events = self.events.clone();
                shell::spawn(async move {
                    let mut board = board;
                    let _ = board.save_note(&project, &path, &title, &new_content).await;
                    let _ = events.unbounded_send(Event::Board(board));
                });
            }
        }
    }

    fn append_to_selected_note(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(ref mut note) = self.selected_note else {
            return;
        };
        let mut new_content = note.content.clone();
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(text);
        new_content.push('\n');
        note.content = new_content.clone();
        let path = note.path.clone();
        let title = note.title.clone();
        let project = path.split('/').next().unwrap_or("Inbox").to_string();

        if let Some(board) = self.board.as_mut() {
            board.set_content(&path, &new_content);
        }
        cx.notify();

        if let Some(board) = self.board.clone() {
            let events = self.events.clone();
            shell::spawn(async move {
                let mut board = board;
                let _ = board.save_note(&project, &path, &title, &new_content).await;
                let _ = events.unbounded_send(Event::Board(board));
            });
        }
    }
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

fn label() -> &'static str {
    if cfg!(target_abi = "sim") {
        "GPUI iOS Simulator"
    } else {
        "GPUI iPhone"
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
                    .flex_1()
                    .overflow_y_scroll()
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
                    .when(!is_mac, |page| {
                        if let Some(note) = self.selected_note.clone() {
                            let doc = Doc::parse(&note.content);
                            let title = note.title.clone();
                            let path = note.path.clone();

                            page.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .pb_2()
                                    .border_b_1()
                                    .border_color(rgb(0xe5e7eb))
                                    .child(
                                        div()
                                            .px_3()
                                            .py(px(6.))
                                            .rounded(px(8.))
                                            .bg(rgb(0xe5e7eb))
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(0x1c1c1e))
                                            .child("← Back")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| {
                                                    view.close_note(cx);
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(title),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(0x8e8e93))
                                                    .child(path.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(rgb(0x0a84ff))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(rgb(0xffffff))
                                                    .child("+ Todo")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|view, _, _, cx| {
                                                            view.append_to_selected_note(
                                                                "- [ ] New thought",
                                                                cx,
                                                            );
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(rgb(0xe5e7eb))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(rgb(0x1c1c1e))
                                                    .child("+ Section")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|view, _, _, cx| {
                                                            view.append_to_selected_note(
                                                                "## New Section",
                                                                cx,
                                                            );
                                                        }),
                                                    ),
                                            )
                                            .child({
                                                let del_path = path.clone();
                                                div()
                                                    .px_2()
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(rgb(0xffebee))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(rgb(0xd32f2f))
                                                    .child("Delete")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |view, _, _, cx| {
                                                            view.delete_note(del_path.clone(), cx);
                                                        }),
                                                    )
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .rounded(px(12.))
                                    .bg(rgb(0xffffff))
                                    .p_4()
                                    .child({
                                        let entity = cx.entity().clone();
                                        render_doc(
                                            &doc,
                                            palette,
                                            Density::Mobile,
                                            Some(move |idx: usize, _: &gpui_kit::ClickEvent, _: &mut gpui::Window, app: &mut gpui::App| {
                                                entity.update(app, |view, cx| {
                                                    view.toggle_note_block_todo(idx, cx);
                                                });
                                            }),
                                        )
                                    }),
                            )
                        } else {
                            // The replica answers the rows, so this view filters nothing itself.
                            let (open_tasks, notes, pending) = match self.board.as_ref() {
                                Some(board) => (
                                    board.tasks().cloned().collect::<Vec<_>>(),
                                    board.notes().cloned().collect::<Vec<_>>(),
                                    board.pending(),
                                ),
                                None => (Vec::new(), Vec::new(), 0),
                            };

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
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child("On-Device Store (Turso)"),
                                            )
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
                                                        format!("{pending} queued")
                                                    } else {
                                                        "Local ✓".into()
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
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
                                            })
                                            .child(
                                                div()
                                                    .px_3()
                                                    .py(px(6.))
                                                    .rounded(px(8.))
                                                    .bg(rgb(0x0a84ff))
                                                    .text_color(rgb(0xffffff))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .child("+ Task")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|view, _, _, cx| {
                                                            view.add_quick_task(cx)
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_3()
                                                    .py(px(6.))
                                                    .rounded(px(8.))
                                                    .bg(rgb(0x34c759))
                                                    .text_color(rgb(0xffffff))
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .child("+ Note")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|view, _, _, cx| {
                                                            view.add_quick_note(cx)
                                                        }),
                                                    ),
                                            ),
                                    ),
                            )
                            .when_some(self.synced.clone(), |page, synced| {
                                page.child(div().text_xs().text_color(rgb(0x8e8e93)).child(synced))
                            })
                            // Filter Pills
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .px_3()
                                            .py(px(4.))
                                            .rounded_full()
                                            .bg(if self.filter == FilterKind::All { rgb(0x0a84ff) } else { rgb(0xffffff) })
                                            .text_color(if self.filter == FilterKind::All { rgb(0xffffff) } else { rgb(0x646b77) })
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(format!("All ({})", open_tasks.len() + notes.len()))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| {
                                                    view.filter = FilterKind::All;
                                                    cx.notify();
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .px_3()
                                            .py(px(4.))
                                            .rounded_full()
                                            .bg(if self.filter == FilterKind::Tasks { rgb(0x0a84ff) } else { rgb(0xffffff) })
                                            .text_color(if self.filter == FilterKind::Tasks { rgb(0xffffff) } else { rgb(0x646b77) })
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(format!("Tasks ({})", open_tasks.len()))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| {
                                                    view.filter = FilterKind::Tasks;
                                                    cx.notify();
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .px_3()
                                            .py(px(4.))
                                            .rounded_full()
                                            .bg(if self.filter == FilterKind::Notes { rgb(0x0a84ff) } else { rgb(0xffffff) })
                                            .text_color(if self.filter == FilterKind::Notes { rgb(0xffffff) } else { rgb(0x646b77) })
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(format!("Notes ({})", notes.len()))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|view, _, _, cx| {
                                                    view.filter = FilterKind::Notes;
                                                    cx.notify();
                                                }),
                                            ),
                                    ),
                            )
                            .when(self.filter != FilterKind::Notes, |page| {
                                page.child(
                                    div()
                                        .rounded(px(12.))
                                        .bg(rgb(0xffffff))
                                        .p_3()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0x646b77))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(format!("Tasks ({})", open_tasks.len())),
                                        )
                                        .children(open_tasks.into_iter().map(|task| {
                                            let path = task.path.clone();
                                            let done = task.done;
                                            let p_path = task.path.clone();
                                            let del_path = task.path.clone();
                                            let (bg_color, text_color) = match task.priority {
                                                1 => (rgb(0xffe5e5), rgb(0xd9383a)),
                                                2 => (rgb(0xfff3d6), rgb(0xb36b00)),
                                                _ => (rgb(0xe5e7eb), rgb(0x646b77)),
                                            };
                                            div()
                                                .px_2()
                                                .py(px(6.))
                                                .flex()
                                                .items_center()
                                                .gap_3()
                                                .rounded(px(6.))
                                                .bg(rgb(0xf9fafb))
                                                .child(
                                                    div()
                                                        .size(px(20.))
                                                        .rounded(px(4.))
                                                        .border_1()
                                                        .border_color(if done {
                                                            rgb(0x248a3d)
                                                        } else {
                                                            rgb(0x9aa0a6)
                                                        })
                                                        .bg(if done {
                                                            rgb(0x248a3d)
                                                        } else {
                                                            rgb(0xffffff)
                                                        })
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .text_color(rgb(0xffffff))
                                                        .text_xs()
                                                        .child(if done { "✓" } else { "" })
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.toggle_task(
                                                                    path.clone(),
                                                                    !done,
                                                                    cx,
                                                                );
                                                            }),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .text_sm()
                                                        .text_color(if done {
                                                            rgb(0x8e8e93)
                                                        } else {
                                                            rgb(0x1c1c1e)
                                                        })
                                                        .child(task.title.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .px_2()
                                                        .py(px(2.))
                                                        .rounded(px(4.))
                                                        .bg(bg_color)
                                                        .text_color(text_color)
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .child(format!("P{}", task.priority))
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.cycle_task_priority(p_path.clone(), cx);
                                                            }),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .px_1()
                                                        .text_xs()
                                                        .text_color(rgb(0xb0b5bd))
                                                        .child("×")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(move |view, _, _, cx| {
                                                                view.delete_note(del_path.clone(), cx);
                                                            }),
                                                        ),
                                                )
                                        }))
                                )
                            })
                            .when(self.filter != FilterKind::Tasks, |page| {
                                page.child(
                                    div()
                                        .rounded(px(12.))
                                        .bg(rgb(0xffffff))
                                        .p_3()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0x646b77))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(format!("Offline Notes ({})", notes.len())),
                                        )
                                        .children(notes.into_iter().map(|note| {
                                            let note_for_tap = note.clone();
                                            div()
                                                .px_2()
                                                .py(px(8.))
                                                .flex()
                                                .items_center()
                                                .gap_3()
                                                .rounded(px(6.))
                                                .bg(rgb(0xf9fafb))
                                                .child(div().text_sm().child("📄"))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                                .child(note.title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(rgb(0x8e8e93))
                                                                .child(note.path.clone()),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0x8e8e93))
                                                        .child("→"),
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |view, _, _, cx| {
                                                        view.open_note(note_for_tap.clone(), cx);
                                                    }),
                                                )
                                        }))
                                )
                            })
                        }
                    }),
            )
            // Bottom Tab Bar
            .child({
                let pending = self.board.as_ref().map_or(0, Board::pending);
                let items = [
                    NavItem {
                        id: "tasks_notes",
                        label: "Tasks & Notes",
                        icon: "📝",
                        badge: if pending > 0 { Some(pending) } else { None },
                    },
                    NavItem {
                        id: "settings",
                        label: "Settings",
                        icon: "⚙️",
                        badge: None,
                    },
                ];
                let active_id = if is_mac { "settings" } else { "tasks_notes" };
                let entity = cx.entity().clone();
                mobile_tab_bar(
                    &items,
                    active_id,
                    palette,
                    move |id: &'static str, _: &mut gpui::Window, app: &mut gpui::App| {
                        entity.update(app, |view, cx| {
                            view.active_tab = match id {
                                "settings" => Tab::Settings,
                                _ => Tab::TasksNotes,
                            };
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
