//! Device pairing, QR challenge minting, join authorization and device management.
use chrono::Local;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::{div, point, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, Window};
use sjel_gpui_core::setup::{
    minutes_left, pairing_payload, partition_devices, qr_matrix, short_pin, stamp, waiting_joins,
    LanInfo, PairedDevice, QrMatrix, SetupJoinRequest, SetupView,
};

use crate::data;
use crate::primitives::{card, dot, note};
use crate::theme::Palette;

use super::SetupPage;

impl SetupPage {
    pub(super) fn approve_join(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let key = format!("setup:approve:{}", id);
        self.notice = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Device allowed."),
            move |shell| async move { shell.setup_approve_join(&id).await },
            |this, _, cx| this.load(cx),
        );
    }

    pub(super) fn deny_join(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let key = format!("setup:deny:{}", id);
        self.notice = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Device denied."),
            move |shell| async move { shell.setup_deny_join(&id).await },
            |this, _, cx| this.load(cx),
        );
    }

    /// Mints one single-use code, then draws it. The picture is built here rather than per frame:
    /// a code never changes while it is shown, and the matrix is thousands of modules.
    pub(super) fn create_challenge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self.challenge_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, "setup:challenge", "Pairing code ready."),
            |shell| async move { shell.setup_create_challenge().await },
            |this, created, cx| {
                let Some(challenge) = created else {
                    this.challenge_error =
                        Some("This node did not mint a code. Try again.".to_string());
                    cx.notify();
                    return;
                };
                this.challenge = Some(challenge);
                this.draw_code();
                cx.notify();
            },
        );
    }

    /// The host the next code carries: the one picked on this page while the listener still
    /// offers it, else the listener's first.
    pub(super) fn pairing_host(&self, lan: &LanInfo) -> Option<String> {
        let hosts = lan.pairing_hosts();
        self.pairing_host
            .clone()
            .filter(|host| hosts.contains(host))
            .or_else(|| hosts.into_iter().next())
    }

    /// Draws the code being shown for the picked host. A new host redraws the same one-time code.
    pub(super) fn draw_code(&mut self) {
        self.challenge_payload = None;
        self.challenge_matrix = None;
        self.challenge_error = None;
        let Some(challenge) = self.challenge.as_ref() else {
            return;
        };
        let lan = self.setup.value.as_ref().and_then(|view| view.lan.as_ref());
        let Some((lan, host)) = lan.and_then(|lan| Some((lan, self.pairing_host(lan)?))) else {
            self.challenge_error = Some(
                "This node has no Same Wi-Fi listener, so a phone cannot scan a code \
                 for it. The one-time code below can still be typed by hand."
                    .to_string(),
            );
            return;
        };
        let payload = pairing_payload(lan, &host, challenge);
        match qr_matrix(&payload) {
            Ok(matrix) => {
                self.challenge_matrix = Some(matrix);
                self.challenge_payload = Some(payload);
            }
            Err(error) => self.challenge_error = Some(error),
        }
    }

    /// Revokes one device's key at the node. The record stays: what was trusted, and until when.
    pub(super) fn revoke_device(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("setup:revoke:{}", id);

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Device revoked."),
            move |shell| async move { shell.setup_revoke_device(&id).await },
            |this, _, cx| {
                this.confirming_revoke = None;
                this.load(cx);
            },
        );
    }

    /// Puts text on the clipboard, which is the only way a pairing payload leaves this page.
    pub(super) fn copy(&mut self, text: String, what: &'static str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notice = Some(format!("{what} copied."));
        cx.notify();
    }

    /// The listener's addresses, one per button, and the certificate pin. The pressed address is
    /// the one the code carries; a single address shows as a plain line.
    pub(super) fn listener_picker(
        &self,
        lan: &LanInfo,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hosts = lan.pairing_hosts();
        let picked = self.pairing_host(lan);
        let address = |host: &str| {
            let base = sjel_gpui_core::device::node_base(host, lan.port);
            base.trim_start_matches("https://").to_string()
        };
        let mut row = div().flex().items_center().gap(px(8.0));
        if hosts.len() == 1 {
            row = row.child(
                div()
                    .text_size(px(12.0))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .text_color(rgb(palette.text))
                    .child(address(&hosts[0])),
            );
        } else {
            for (index, host) in hosts.into_iter().enumerate() {
                let label = if lan.tailnet.as_deref() == Some(host.as_str()) {
                    format!("Tailnet {}", address(&host))
                } else {
                    format!("Same Wi-Fi {}", address(&host))
                };
                let active = picked.as_deref() == Some(host.as_str());
                row = row.child(
                    Button::new(("pairing-host", index))
                        .small()
                        .when(!active, |b| b.ghost())
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pairing_host = Some(host.clone());
                            this.draw_code();
                            cx.notify();
                        })),
                );
            }
        }
        row.child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(palette.muted))
                .child(format!("certificate pin {}", short_pin(&lan.fingerprint))),
        )
        .into_any_element()
    }

    /// One waiting join: the code both screens show, and the two answers an owner gives.
    pub(super) fn join_row(
        &mut self,
        join: &SetupJoinRequest,
        now: i64,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_allow = join.id.clone();
        let id_deny = join.id.clone();
        let code = if join.key_code.is_empty() {
            join.id.clone()
        } else {
            join.key_code.clone()
        };
        let button_key = join.id.clone();

        div()
            .flex()
            .justify_between()
            .items_center()
            .p(px(10.0))
            .bg(rgba(palette.surface))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .font_family(".AppleSystemUIFontMonospaced")
                            .text_color(rgb(palette.text))
                            .child(code),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(match (join.label.is_empty(), join.platform.is_empty()) {
                                (false, false) => format!("{} · {}", join.label, join.platform),
                                (false, true) => join.label.clone(),
                                (true, false) => join.platform.clone(),
                                (true, true) => join.id.clone(),
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(minutes_left(join.expires_at, now)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new(format!("allow-{button_key}"))
                            .small()
                            .label("Allow")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.approve_join(id_allow.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new(format!("deny-{button_key}"))
                            .small()
                            .ghost()
                            .label("Deny")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.deny_join(id_deny.clone(), window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// One trusted device. Revoking is two presses, and the second one says what it does.
    pub(super) fn device_row(
        &mut self,
        device: &PairedDevice,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = device.id.clone();
        let armed = self.confirming_revoke.as_deref() == Some(id.as_str());

        let action: AnyElement = if armed {
            let target = id.clone();
            let keep = id.clone();
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    Button::new(format!("keep-{keep}"))
                        .small()
                        .ghost()
                        .label("Keep")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirming_revoke = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("revoke-now-{target}"))
                        .small()
                        .label("Revoke now")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.revoke_device(target.clone(), window, cx);
                        })),
                )
                .into_any_element()
        } else {
            let arm = id.clone();
            Button::new(format!("revoke-{arm}"))
                .small()
                .ghost()
                .label("Revoke")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.confirming_revoke = Some(arm.clone());
                    cx.notify();
                }))
                .into_any_element()
        };

        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .w(px(170.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(device.name().to_string()),
                    )
                    .when(!device.platform.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(palette.muted))
                                .child(device.platform.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(150.0))
                    .text_size(px(11.0))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .text_color(rgb(palette.muted))
                    .child(short_pin(&device.fingerprint)),
            )
            .child(
                div()
                    .w(px(150.0))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.muted))
                    .child(stamp(Some(device.created_at))),
            )
            .child(
                div()
                    .w(px(150.0))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.muted))
                    .child(format!("seen {}", stamp(device.last_seen_at))),
            )
            .child(div().flex_1().child(action))
            .into_any_element()
    }

    pub(super) fn render_devices_tab(
        &mut self,
        view: &SetupView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Local::now().timestamp();
        let mut panels: Vec<AnyElement> = Vec::new();

        // ── The listener a phone can reach ───────────────────────────────────
        let (reachable, listener_body) = match (&view.lan, &view.lan_error) {
            (Some(lan), _) => (true, self.listener_picker(lan, palette, cx)),
            (None, Some(error)) => (
                false,
                note(
                    format!("Could not read this node's listener: {error}"),
                    palette.red,
                ),
            ),
            (None, None) => (
                false,
                note(
                    "Same Wi-Fi is off on this node, so an iPhone can neither find this Mac nor \
                     scan a code for it. SJEL_LAN_PORT in the overlay's deployment record turns it on.",
                    palette.muted,
                ),
            ),
        };

        let listener_card = div().flex().flex_col().gap(px(10.0)).child(
            div()
                .flex()
                .justify_between()
                .items_start()
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .child(dot(if reachable {
                            palette.accent
                        } else {
                            palette.muted
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child("Add an iPhone"),
                                )
                                .child(listener_body),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            Button::new("setup-refresh-devices")
                                .small()
                                .ghost()
                                .label("Refresh")
                                .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                        )
                        .child(
                            Button::new("add-iphone")
                                .small()
                                .label(if self.challenge.is_some() {
                                    "New code"
                                } else {
                                    "Add iPhone"
                                })
                                // Nothing to scan without a listener: a code no phone can
                                // reach is a code that fails after the work of typing it.
                                .disabled(!reachable)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_challenge(window, cx);
                                })),
                        ),
                ),
        );

        panels.push(card(listener_card.into_any_element(), palette));

        // ── The code, once this Mac has minted one ───────────────────────────
        if let Some(challenge) = self.challenge.clone() {
            let code = challenge.code.clone();
            let expires_at = challenge.expires_at;
            let matrix = self.challenge_matrix.clone();
            let payload = self.challenge_payload.clone();
            let error = self.challenge_error.clone();

            let mut body = div().flex().flex_col().gap(px(12.0)).child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(dot(palette.accent))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Scan this on the phone"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "Works once, until {} ({}).",
                                sjel_gpui_core::setup::clock(expires_at),
                                minutes_left(expires_at, now)
                            )),
                    ),
            );

            let picture: AnyElement = match &matrix {
                Some(matrix) => qr_image(matrix),
                None => note(
                    error
                        .clone()
                        .unwrap_or_else(|| "No code to draw.".to_string()),
                    palette.amber,
                ),
            };

            let hand_off = div()
                .flex()
                .flex_1()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("ONE-TIME CODE"),
                )
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_family(".AppleSystemUIFontMonospaced")
                        .text_color(rgb(palette.text))
                        .child(code.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .text_color(rgb(palette.muted))
                        .child(
                            "The phone claims this code with its own key, which never leaves the \
                             phone. Typed by hand when the camera cannot be used.",
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .pt(px(2.0))
                        .child(
                            Button::new("copy-code")
                                .small()
                                .ghost()
                                .label("Copy code")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy(code.clone(), "Code", cx);
                                })),
                        )
                        .child(
                            Button::new("copy-payload")
                                .small()
                                .ghost()
                                .label("Copy payload")
                                .disabled(payload.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(payload) = payload.clone() {
                                        this.copy(payload, "Pairing payload", cx);
                                    }
                                })),
                        ),
                );

            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(16.0))
                    .child(picture)
                    .child(hand_off),
            );

            body = body.child(
                Button::new("challenge-dismiss")
                    .small()
                    .ghost()
                    .label("Done")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.challenge = None;
                        this.challenge_payload = None;
                        this.challenge_matrix = None;
                        this.challenge_error = None;
                        cx.notify();
                    })),
            );

            panels.push(card(body.into_any_element(), palette));
        }

        // ── Joins waiting for a decision ─────────────────────────────────────
        let waiting = waiting_joins(&view.pending_joins);
        if !waiting.is_empty() {
            let mut rows = div().flex().flex_col().gap(px(6.0));
            for join in &waiting {
                rows = rows.child(self.join_row(join, now, palette, cx));
            }

            let pending_card = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(dot(palette.amber))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(palette.text))
                                .child(format!("Asking to join ({})", waiting.len())),
                        ),
                )
                .child(rows);

            panels.push(card(pending_card.into_any_element(), palette));
        }

        // ── The keys this node trusts ────────────────────────────────────────
        let (active, revoked) = partition_devices(&view.devices);

        let mut trusted = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(device_header(palette));

        if active.is_empty() {
            trusted = trusted.child(note(
                "No companion device is paired to this Mac yet.",
                palette.muted,
            ));
        } else {
            for device in &active {
                trusted = trusted.child(self.device_row(device, palette, cx));
            }
        }

        let trusted_card = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(dot(palette.accent))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format!("Trusted devices ({})", active.len())),
                    ),
            )
            .child(trusted);

        panels.push(card(trusted_card.into_any_element(), palette));

        // ── What a revoke left behind ────────────────────────────────────────
        if !revoked.is_empty() {
            let mut rows = div().flex().flex_col().gap(px(4.0));
            for device in &revoked {
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .py(px(3.0))
                        .child(
                            div()
                                .w(px(160.0))
                                .text_size(px(12.0))
                                .text_color(rgb(palette.muted))
                                .child(device.name().to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("revoked {}", stamp(device.revoked_at))),
                        )
                        .into_any_element(),
                );
            }

            let revoked_card = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(dot(palette.muted))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child(format!("Revoked ({})", revoked.len())),
                        ),
                )
                .child(rows);

            panels.push(card(revoked_card.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }
}

/// The column names of the trusted-device table, so the values below them are never unlabelled.
pub(super) fn device_header(palette: Palette) -> AnyElement {
    let column = move |label: &'static str| {
        div()
            .text_size(px(10.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(palette.muted))
            .child(label)
    };

    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .pb(px(3.0))
        .border_b_1()
        .border_color(rgba(palette.border))
        .child(div().w(px(170.0)).child(column("DEVICE")))
        .child(div().w(px(150.0)).child(column("CERTIFICATE")))
        .child(div().w(px(150.0)).child(column("TRUSTED")))
        .child(div().w(px(150.0)).child(column("LAST SEEN")))
        .child(div().flex_1().child(column("")))
        .into_any_element()
}

/// The code itself: black on white whatever the theme, because a camera reads contrast. A themed
/// code in dark mode is a rectangle of nothing to a scanner.
///
/// The quiet zone is part of the picture, not padding: without it a camera cannot find the code's
/// edges, so it is drawn rather than left to the layout.
pub(super) fn qr_image(matrix: &QrMatrix) -> AnyElement {
    /// One module in pixels: large enough to scan from a phone held out at arm's length.
    const MODULE: f32 = 5.0;
    /// The border the standard asks for, in modules.
    const QUIET: usize = 2;

    let width = matrix.width;
    let modules = matrix.dark.clone();
    let side = (width + QUIET * 2) as f32 * MODULE;

    gpui_kit::canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            window.paint_quad(gpui_kit::PaintQuad {
                bounds,
                corner_radii: gpui_kit::Corners::all(px(4.0)),
                background: rgb(0xffffff).into(),
                border_widths: gpui_kit::Edges::default(),
                border_color: rgba(0x00000000).into(),
                border_style: gpui_kit::BorderStyle::default(),
            });

            for y in 0..width {
                for x in 0..width {
                    if !modules[y * width + x] {
                        continue;
                    }
                    let origin = point(
                        bounds.origin.x + px((x + QUIET) as f32 * MODULE),
                        bounds.origin.y + px((y + QUIET) as f32 * MODULE),
                    );
                    window.paint_quad(gpui_kit::PaintQuad {
                        bounds: gpui_kit::Bounds {
                            origin,
                            size: gpui_kit::size(px(MODULE), px(MODULE)),
                        },
                        corner_radii: gpui_kit::Corners::default(),
                        background: rgb(0x000000).into(),
                        border_widths: gpui_kit::Edges::default(),
                        border_color: rgba(0x00000000).into(),
                        border_style: gpui_kit::BorderStyle::default(),
                    });
                }
            }
        },
    )
    .flex_none()
    .w(px(side))
    .h(px(side))
    .into_any_element()
}
