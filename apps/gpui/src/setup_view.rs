//! Node Setup & Device Pairing: Probed machine requirements, Turso replica setup, and device pairing.
//!
//! Replaces the dashboard `/setup` route. Backed by `capabilities/sjel-status` and `capabilities/devices`.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, Entity, IntoElement,
    Subscription, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::theme::Palette;
use crate::ui::{card, dot, note, placeholder, section};
use chrono::Local;
use sjel_gpui_core::setup::{
    has_apply_input, merge_verification_probes, minutes_left, pairing_payload, partition_devices,
    qr_matrix, short_pin, stamp, waiting_joins, LanInfo, PairedDevice, PairingChallenge, ProbeMark,
    QrMatrix, SetupApplyRequest, SetupJoinRequest, SetupPlan, SetupView,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupTab {
    Machine,
    Devices,
}

pub struct SetupPage {
    repo: Repo,
    setup: Remote<SetupView>,
    busy: Busy,
    notice: Option<String>,
    write_error: Option<String>,

    active_tab: SetupTab,
    verifying: bool,
    confirming_component: Option<String>,
    /// The component whose apply is in flight, so the answer's sentence can name it: a `then`
    /// callback gets no captured state, only the page.
    applying_component: Option<String>,
    applied_log: Vec<String>,

    /// The one-time code this Mac is showing, and the picture of it. Both are transient: a code is
    /// single-use and lives ten minutes, so nothing here survives a reload.
    challenge: Option<PairingChallenge>,
    challenge_payload: Option<String>,
    challenge_matrix: Option<QrMatrix>,
    challenge_error: Option<String>,
    /// The device whose revoke is waiting for a second press.
    confirming_revoke: Option<String>,

    url_input: Option<(Entity<InputState>, Subscription)>,
    credential_input: Option<(Entity<InputState>, Subscription)>,
    interval_input: Option<(Entity<InputState>, Subscription)>,
}

impl SetupPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            setup: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,

            active_tab: SetupTab::Machine,
            verifying: false,
            confirming_component: None,
            applying_component: None,
            applied_log: Vec::new(),

            challenge: None,
            challenge_payload: None,
            challenge_matrix: None,
            challenge_error: None,
            confirming_revoke: None,

            url_input: None,
            credential_input: None,
            interval_input: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.setup,
            &repo,
            |shell| async move { shell.setup_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn switch_tab(&mut self, tab: SetupTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    fn ensure_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.url_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("libsql://your-database.turso.io"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.url_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_credential_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.credential_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Primary auth token (piped to stdin, value hidden)")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.credential_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_interval_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.interval_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("60"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.interval_input = Some((input.clone(), subscription));
        input
    }

    fn verify_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self.verifying = true;
        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(
                |s| &mut s.busy,
                "setup:verify",
                "Primary connection verified.",
            ),
            |shell| async move { shell.setup_verify().await },
            |this, result, cx| {
                this.verifying = false;
                match result {
                    Some(verification) => {
                        if let Some(view) = &mut this.setup.value {
                            view.plans = merge_verification_probes(&view.plans, &verification);
                        }
                        this.notice = Some("Primary connection verified.".to_string());
                    }
                    None => {
                        this.write_error =
                            Some("Connection check failed or could not reach primary.".to_string());
                    }
                }
                cx.notify();
            },
        );
    }

    fn apply_setup(&mut self, component: String, window: &mut Window, cx: &mut Context<Self>) {
        let url_field = self.ensure_url_input(window, cx);
        let cred_field = self.ensure_credential_input(window, cx);
        let int_field = self.ensure_interval_input(window, cx);

        let url = url_field.read(cx).value().trim().to_string();
        let token = cred_field.read(cx).value().trim().to_string();
        let interval_str = int_field.read(cx).value().trim().to_string();

        if !has_apply_input(&url, &token, &interval_str) {
            self.write_error = Some("Supply a URL, credential, or interval to apply.".to_string());
            cx.notify();
            return;
        }

        let interval_secs = interval_str.parse::<u64>().ok();

        let req = SetupApplyRequest {
            component: Some(component.clone()),
            url: if url.is_empty() { None } else { Some(url) },
            interval_secs,
            token: if token.is_empty() { None } else { Some(token) },
        };

        let repo = self.repo.clone();
        let key = format!("setup:apply:{}", component);

        self.notice = None;
        self.write_error = None;
        self.applying_component = Some(component);

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Setup steps applied."),
            move |shell| async move { shell.setup_apply(&req).await },
            |this, result, cx| {
                let component = this.applying_component.take();
                match result {
                    Some(applied) => {
                        let lines: Vec<String> = applied
                            .components
                            .into_iter()
                            .flat_map(|c| c.applied)
                            .collect();
                        this.applied_log = lines;
                        this.notice =
                            component.map(|name| format!("Applied setup settings for {name}."));
                        this.confirming_component = None;
                        // The credential is write-only: dropping the field is the only way this
                        // page can forget a value that must never be shown again.
                        this.credential_input = None;
                    }
                    None => {
                        this.write_error = Some("Failed to apply setup settings.".to_string());
                    }
                }
                this.load(cx);
            },
        );
    }

    fn approve_join(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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

    fn deny_join(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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

    // ── Components ────────────────────────────────────────────────────────────

    /// Mints one single-use code, then draws it. The picture is built here rather than per frame:
    /// a code never changes while it is shown, and the matrix is thousands of modules.
    fn create_challenge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                let lan = this.setup.value.as_ref().and_then(|view| view.lan.clone());
                this.challenge = Some(challenge.clone());
                this.challenge_payload = None;
                this.challenge_matrix = None;
                match lan {
                    Some(lan) => {
                        let payload = pairing_payload(&lan, &challenge);
                        match qr_matrix(&payload) {
                            Ok(matrix) => {
                                this.challenge_matrix = Some(matrix);
                                this.challenge_payload = Some(payload);
                            }
                            Err(error) => this.challenge_error = Some(error),
                        }
                    }
                    None => {
                        this.challenge_error = Some(
                            "This node has no Same Wi-Fi listener, so a phone cannot scan a code \
                             for it. The one-time code below can still be typed by hand."
                                .to_string(),
                        );
                    }
                }
                cx.notify();
            },
        );
    }

    /// Revokes one device's key at the node. The record stays: what was trusted, and until when.
    fn revoke_device(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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
    fn copy(&mut self, text: String, what: &'static str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notice = Some(format!("{what} copied."));
        cx.notify();
    }

    fn render_tabs(&self, _palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let tab_btn = |tab: SetupTab, label: &'static str, cx: &mut Context<Self>| {
            let active = self.active_tab == tab;
            Button::new(label)
                .small()
                .when(!active, |b| b.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_tab(tab, cx);
                }))
        };

        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(tab_btn(SetupTab::Machine, "Machine Setup", cx))
            .child(tab_btn(SetupTab::Devices, "Paired Devices", cx))
            .into_any_element()
    }

    // ── Tab 1: Machine Setup ──────────────────────────────────────────────────

    fn render_machine_tab(
        &mut self,
        plans: &[SetupPlan],
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if plans.is_empty() {
            return note(
                "Nothing to set up: no component has an active plan on this build.",
                palette.muted,
            );
        }

        let mut panels = Vec::new();

        // Action bar
        let action_bar = div()
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
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("What this machine still needs"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new("setup-re-read")
                            .small()
                            .ghost()
                            .label("Re-read")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load(cx);
                            })),
                    )
                    .child(
                        Button::new("setup-verify")
                            .small()
                            .ghost()
                            .label(if self.verifying {
                                "Checking…"
                            } else {
                                "Check the connection"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.verify_connection(window, cx);
                            })),
                    ),
            );

        panels.push(card(action_bar.into_any_element(), palette));

        // Applied log
        if !self.applied_log.is_empty() {
            let log_lines = self.applied_log.iter().map(|line| {
                div()
                    .text_size(px(11.0))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .text_color(rgb(palette.text))
                    .child(line.clone())
                    .into_any_element()
            });

            let log_card = div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(8.0))
                .bg(rgb(palette.surface))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(palette.green))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.green))
                        .child("Recently Applied Settings:"),
                )
                .children(log_lines);

            panels.push(log_card.into_any_element());
        }

        // Each plan card
        for plan in plans {
            let comp = plan.component.clone();
            let is_confirming = self.confirming_component.as_deref() == Some(&comp);

            // Probes list
            let probe_rows = plan.probes.iter().map(|probe| {
                let parsed = probe.parsed_mark();
                let mark_color = match parsed {
                    ProbeMark::Ok => palette.green,
                    ProbeMark::Missing => palette.red,
                    ProbeMark::Warn => palette.amber,
                    ProbeMark::Blocked => palette.muted,
                };

                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .p(px(8.0))
                    .bg(rgb(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .w(px(20.0))
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(mark_color))
                            .child(parsed.symbol()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(probe.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .bg(rgb(palette.raised))
                                            .rounded(px(3.0))
                                            .text_size(px(9.0))
                                            .text_color(rgb(mark_color))
                                            .child(parsed.human_label()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .line_height(px(16.0))
                                    .child(probe.detail.clone()),
                            ),
                    )
                    .into_any_element()
            });

            // Steps list
            let step_rows = plan.steps.iter().map(|step| {
                let (sym, col) = if step.done {
                    ("✓", palette.green)
                } else {
                    ("·", palette.muted)
                };

                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .p(px(6.0))
                    .child(
                        div()
                            .w(px(16.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(col))
                            .child(sym),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if step.done {
                                        rgb(palette.muted)
                                    } else {
                                        rgb(palette.text)
                                    })
                                    .child(step.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child(step.detail.clone()),
                            ),
                    )
                    .into_any_element()
            });

            // Form or trigger
            let form_or_button = if is_confirming {
                let url_field = self.ensure_url_input(window, cx);
                let cred_field = self.ensure_credential_input(window, cx);
                let int_field = self.ensure_interval_input(window, cx);
                let comp_for_apply = comp.clone();

                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .bg(rgb(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.muted))
                            .line_height(px(17.0))
                            .child(
                                "Sjel writes the settings to the private overlay and restarts what follows the primary. \
                                Piped to tool stdin, never logged, never shown again.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Primary URL"),
                            )
                            .child(Input::new(&url_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Credential / Auth Token"),
                            )
                            .child(Input::new(&cred_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Pull Interval, seconds (default 60)"),
                            )
                            .child(Input::new(&int_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .pt(px(4.0))
                            .child(
                                Button::new("cancel-apply")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirming_component = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("submit-apply")
                                    .small()
                                    .label("Apply Settings")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.apply_setup(comp_for_apply.clone(), window, cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            } else {
                let comp_to_open = comp.clone();
                div()
                    .flex()
                    .justify_start()
                    .pt(px(4.0))
                    .child(
                        Button::new(format!("btn-open-setup-{}", comp))
                            .small()
                            .label(format!("Configure {}", comp))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirming_component = Some(comp_to_open.clone());
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            };

            let plan_card = div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(rgb(palette.border))
                        .pb(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(15.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(plan.component.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(palette.muted))
                                        .child(plan.purpose.clone()),
                                ),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(rgb(palette.surface))
                                .rounded(px(4.0))
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.accent))
                                .child(format!("{} open steps", plan.applyable_steps())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child("Machine Probes"),
                        )
                        .children(probe_rows),
                )
                .when(!plan.steps.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Plan Steps"),
                            )
                            .children(step_rows),
                    )
                })
                .child(form_or_button);

            panels.push(card(plan_card.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }

    // ── Tab 2: Devices & Pairing ──────────────────────────────────────────────

    fn render_devices_tab(
        &mut self,
        view: &SetupView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Local::now().timestamp();
        let mut panels: Vec<AnyElement> = Vec::new();

        // ── The listener a phone can reach ───────────────────────────────────
        let (reachable, listener_body) = match (&view.lan, &view.lan_error) {
            (Some(lan), _) => (true, listener_line(lan, palette)),
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

    /// One waiting join: the code both screens show, and the two answers an owner gives.
    fn join_row(
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
            .bg(rgb(palette.surface))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(palette.border))
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
    fn device_row(
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
            .border_color(rgb(palette.border))
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
}

/// The listener, in the one line a person needs to recognise this Mac.
fn listener_line(lan: &LanInfo, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(12.0))
                .font_family(".AppleSystemUIFontMonospaced")
                .text_color(rgb(palette.text))
                .child(format!("{}.local:{}", lan.host, lan.port)),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(palette.muted))
                .child(format!("certificate pin {}", short_pin(&lan.fingerprint))),
        )
        .into_any_element()
}

/// The column names of the trusted-device table, so the values below them are never unlabelled.
fn device_header(palette: Palette) -> AnyElement {
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
        .border_color(rgb(palette.border))
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
fn qr_image(matrix: &QrMatrix) -> AnyElement {
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

impl Render for SetupPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(view) = self.setup.value.clone() else {
            let body = match &self.setup.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Setup", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }
        if let Some(error) = &self.write_error {
            panels.push(note(error.as_str(), palette.red));
        }

        panels.push(self.render_tabs(palette, cx));

        let active_content = match self.active_tab {
            SetupTab::Machine => self.render_machine_tab(&view.plans, palette, window, cx),
            SetupTab::Devices => self.render_devices_tab(&view, palette, cx),
        };
        panels.push(active_content);

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = SetupPage::new(repo);
        assert_eq!(page.active_tab, SetupTab::Machine);
        assert!(page.confirming_component.is_none());
        assert!(!page.verifying);
    }
}
