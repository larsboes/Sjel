//! The iOS app's GPUI content. UIKit owns the window, the navigation bar and the safe areas
//! (`host/App.swift`); GPUI draws inside them (`../README.md`, "GPUI for every client").
//!
//! The app is a paired device (`capabilities/devices/README.md`): it claims the one-time code
//! from the Mac's pairing QR code, then signs every request with its Keychain key
//! (`host/DeviceIdentity.swift`) over TLS pinned to the Mac's LAN listener.
//!
//! Until Bonjour discovery and a camera scanner exist, the Mac's address arrives in the launch
//! environment: `SJEL_JOIN=<mac name> <port> <fingerprint>` asks to join, and the owner allows it
//! in the dashboard's Devices panel (`ios/run-sim --join`). `SJEL_PAIR` takes the QR code's text
//! instead and claims its one-time code.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;

use gpui::{div, prelude::*, px, rgb, Context, SharedString, Window};
use sjel_gpui_core::device::{self, DeviceClient, Node, Signer};
use sjel_gpui_core::shell::{self, NodeHealth};

mod identity;

const HEALTH: &str = "/api/sjel-status/health";
/// How often a waiting join asks whether the owner decided. A request expires after ten minutes
/// (`capabilities/devices/README.md`), so the wait ends there at the latest.
const JOIN_POLL: Duration = Duration::from_secs(2);

enum Event {
    Status(String),
    Ready(Node, NodeHealth),
}

pub struct Capabilities {
    status: SharedString,
    rows: Vec<(String, bool)>,
}

impl Capabilities {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (events, mut received) = unbounded();
        let pair = std::env::var("SJEL_PAIR").ok();
        let join = std::env::var("SJEL_JOIN").ok();
        let failed = events.clone();
        let work = shell::run(async move {
            if let Err(error) = load(pair, join, &events).await {
                let _ = failed.unbounded_send(Event::Status(error));
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                let alive = this.update(cx, |view, cx| {
                    match event {
                        Event::Status(text) => view.status = text.into(),
                        Event::Ready(node, health) => {
                            view.status =
                                format!("Paired as {} with {}", node.device_id, node.base).into();
                            view.rows = health
                                .capabilities
                                .iter()
                                .map(|(name, state)| (name.clone(), state.up))
                                .collect();
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
            rows: Vec::new(),
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
            save(&path, &node)?;
            node
        }
    };
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
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("capabilities")
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(0xf3f5f9))
            .text_color(rgb(0x1c1c1e))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x646b77))
                    .child(self.status.clone()),
            )
            .when(!self.rows.is_empty(), |page| {
                page.child(
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
                                .child(div().size(px(8.)).rounded_full().bg(rgb(if *up {
                                    0x248a3d
                                } else {
                                    0x8e8e93
                                })))
                                .child(name.clone())
                        })),
                )
            })
    }
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
