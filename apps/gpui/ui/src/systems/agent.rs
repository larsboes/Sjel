//! Agent permission gates, live approvals, and capability call history.
use chrono::{Local, TimeZone};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement, SharedString, Window};

use crate::data::{self, Busy};
use crate::primitives::{dot, note, placeholder, row, section};
use crate::theme::Palette;
use sjel_gpui_core::shell::{AgentGate, AgentView, Approval};

use super::{heading, SystemsPage};

impl SystemsPage {
    pub(super) fn read_agent(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.agent,
            &repo,
            |shell| async move { shell.agent().await },
        );
    }

    pub(super) fn set_agent_mode(
        &mut self,
        gate: AgentGate,
        mode: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("agent:{}", gate.capability);
        let capability = gate.capability.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Mode saved."),
            move |shell| async move { shell.set_agent_mode(&capability, mode).await },
            |this, saved, cx| {
                if saved.is_some() {
                    this.read_agent(cx);
                }
            },
        );
    }

    pub(super) fn decide_agent(
        &mut self,
        id: String,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("approval:{id}");
        let done = if allow { "Allowed." } else { "Denied." };
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, done),
            move |shell| async move { shell.decide(&id, allow).await },
            |this, _, cx| this.read_agent(cx),
        );
    }

    pub(super) fn agent_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let view = self.agent.value.clone();
        let body = match &view {
            None => match &self.agent.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(160.),
            },
            Some(agent) => {
                let mut rows: Vec<AnyElement> = Vec::new();

                if !agent.pending.is_empty() {
                    rows.push(heading("Waiting for you", palette));
                    for approval in &agent.pending {
                        rows.push(approval_row(approval, palette, &self.busy, cx));
                    }
                }

                rows.push(heading(
                    if agent.enrolled {
                        "What an agent may do"
                    } else {
                        "What an agent may do — no agent is enrolled"
                    },
                    palette,
                ));
                if agent.capabilities.is_empty() {
                    rows.push(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("No capability has started an agent gate on this machine yet.")
                            .into_any_element(),
                    );
                } else {
                    for gate in &agent.capabilities {
                        rows.push(gate_row(gate, agent, palette, &self.busy, cx));
                    }
                }

                if !agent.calls.is_empty() {
                    rows.push(heading("Latest calls", palette));
                    for call in agent.calls.iter().take(8) {
                        let (tone, decision) = match call.decision.as_str() {
                            "read" => (palette.muted, "read"),
                            "allowed" | "auto" => (palette.green, "allowed"),
                            "denied" => (palette.red, "denied"),
                            // A decision this build does not know prints as the tool wrote it,
                            // so a fifth one reaches the reader instead of being swallowed.
                            other => (palette.amber, other),
                        };
                        rows.push(
                            row()
                                .text_xs()
                                .child(dot(tone))
                                .child(
                                    div()
                                        .w(px(120.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(call.capability.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(format!("{} {}", call.method, call.path)),
                                )
                                .child(
                                    div()
                                        .w(px(56.))
                                        .text_color(rgb(palette.muted))
                                        .child(call.status.to_string()),
                                )
                                .child(
                                    div()
                                        .w(px(64.))
                                        .text_color(rgb(tone))
                                        .child(decision.to_owned()),
                                )
                                .into_any_element(),
                        );
                    }
                }

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        section("Agent", body, palette).into_any_element()
    }
}

fn gate_row(
    gate: &AgentGate,
    agent: &AgentView,
    palette: Palette,
    busy: &Busy,
    cx: &mut Context<SystemsPage>,
) -> AnyElement {
    let modes: Vec<Button> = agent
        .modes
        .iter()
        .map(|mode| {
            let chosen = gate.mode == *mode;
            let picked = mode.clone();
            let in_flight = busy.contains(&format!("agent:{}", gate.capability));
            let gate_clone = gate.clone();
            Button::new(SharedString::from(format!(
                "agent-{}-{mode}",
                gate.capability
            )))
            .small()
            .when(!chosen, |button| button.ghost())
            .label(mode.clone())
            .loading(in_flight && chosen)
            .on_click(cx.listener(move |page, _, window, cx| {
                if !chosen {
                    let mode: &'static str = match picked.as_str() {
                        "off" => "off",
                        "read-only" => "read-only",
                        "ask" => "ask",
                        _ => "auto",
                    };
                    page.set_agent_mode(gate_clone.clone(), mode, window, cx);
                }
            }))
        })
        .collect();
    let asks = if gate.confirm.is_empty() {
        None
    } else {
        Some(format!(
            "{} action(s) always ask: {}",
            gate.confirm.len(),
            gate.confirm.join(", ")
        ))
    };
    row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_sm().child(gate.capability.clone()))
                .children(asks.map(|asks| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(asks)
                })),
        )
        .child(div().flex().flex_none().gap_1().children(modes))
        .into_any_element()
}

fn approval_row(
    approval: &Approval,
    palette: Palette,
    busy: &Busy,
    cx: &mut Context<SystemsPage>,
) -> AnyElement {
    let in_flight = busy.contains(&format!("approval:{}", approval.id));
    let allow_id = approval.id.clone();
    let deny_id = approval.id.clone();
    let when = Local
        .timestamp_opt(approval.created_at, 0)
        .single()
        .map(|at| at.format("%H:%M").to_string())
        .unwrap_or_default();
    row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .child(format!("{} {}", approval.method, approval.path)),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} · {when}", approval.capability)),
                )
                .children((!approval.preview.is_empty()).then(|| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(approval.preview.clone())
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("deny-{}", approval.id)))
                        .small()
                        .ghost()
                        .label("Deny")
                        .loading(in_flight)
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.decide_agent(deny_id.clone(), false, window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("allow-{}", approval.id)))
                        .small()
                        .label("Allow")
                        .loading(in_flight)
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.decide_agent(allow_id.clone(), true, window, cx);
                        })),
                ),
        )
        .into_any_element()
}
