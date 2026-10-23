//! Every capability the shell knows, by name, with start and stop, and what the agent has written
//! to them lately. A start or stop changes a row's dot and button, never its place.
//!
//! Both halves live under Sjel: runtime capability and agent inspection is what Sjel is, not what
//! today is.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};

use crate::selection::Item;
use crate::shell::{CapabilityHealth, Liveness};
use crate::theme::Palette;
use crate::ui::{dot, list, note, placeholder, row, section};
use crate::workspace::Workspace;

/// How many of the most recent agent calls the audit log names.
const RECENT_WRITES: usize = 6;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_8()
        .child(capabilities(view, palette, cx))
        .child(agent_writes(view, palette, cx))
}

fn capabilities(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(health) = &view.health.value else {
        let body = match &view.health.error {
            Some(error) => note(error.to_string(), palette.red),
            None => placeholder(320.),
        };
        return section("Capabilities", body, palette).into_any_element();
    };

    let count = |state| {
        health
            .capabilities
            .values()
            .filter(|h| h.liveness() == state)
            .count()
    };
    let summary = format!(
        "{} running · {} idle · {} down",
        count(Liveness::Up),
        count(Liveness::Idle),
        count(Liveness::Down)
    );
    let selected = view.selected();
    let rows: Vec<AnyElement> = health
        .capabilities
        .iter()
        .map(|(name, state)| {
            let marked = selected
                == Some(Item::Capability {
                    name: name.clone(),
                    up: state.up,
                });
            capability_row(view, marked, name, *state, palette, cx)
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(summary),
        )
        .child(list(rows, palette))
        .into_any_element()
}

fn capability_row(
    view: &Workspace,
    selected: bool,
    name: &str,
    state: CapabilityHealth,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(name);
    // Running is the normal state: the dot says it, and only the exceptions get words.
    let (tone, label) = match state.liveness() {
        Liveness::Up => (palette.green, ""),
        Liveness::Idle => (palette.muted, "idle"),
        Liveness::Down => (palette.red, "down"),
    };
    let start = !state.up;
    let target = name.to_owned();
    let button = Button::new(SharedString::from(format!("toggle-{name}")))
        .small()
        .w(px(64.))
        .when(!start, |button| button.ghost())
        .label(if start { "Start" } else { "Stop" })
        .tooltip(if start {
            "Start (↩)"
        } else {
            "Stop (⌘⌫)"
        })
        .loading(busy)
        .on_click(cx.listener(move |view, _, window, cx| {
            view.set_running(target.clone(), start, window, cx)
        }));
    view.selection
        .mark(row(), selected, palette)
        .group("capability")
        .hover(move |style| style.bg(rgba(palette.raised)))
        .child(dot(tone))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(name.to_owned()),
        )
        .child(
            div()
                .w(px(96.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(if state.autostart {
                    "kept running"
                } else {
                    "on demand"
                }),
        )
        .child(
            div()
                .w(px(40.))
                .text_xs()
                .text_color(rgb(tone))
                .child(label),
        )
        // Stop is rare and costly, so it waits for hover; Start is the useful action.
        .child(
            div()
                .when(!start && !busy && !selected, |slot| {
                    slot.opacity(0.)
                        .group_hover("capability", |style| style.opacity(1.))
                })
                .child(button),
        )
        .into_any_element()
}

/// What the agent changed, newest first: the same rows Today used to carry, under the area that
/// owns the runtime they changed.
fn agent_writes(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    let body = match (&view.agent.value, &view.agent.error) {
        (_, Some(error)) => note(error.to_string(), palette.muted),
        (None, None) => placeholder(56.),
        (Some(agent), None) => {
            let writes: Vec<_> = agent
                .calls
                .iter()
                .filter(|call| call.decision != "read")
                .take(RECENT_WRITES)
                .collect();
            if writes.is_empty() {
                note(
                    format!("Only reads in the last {} agent calls.", agent.calls.len()),
                    palette.muted,
                )
            } else {
                list(
                    writes
                        .into_iter()
                        .map(|call| {
                            let at = chrono::DateTime::from_timestamp(call.at, 0)
                                .map(|t| {
                                    t.with_timezone(&chrono::Local).format("%H:%M").to_string()
                                })
                                .unwrap_or_default();
                            let tone = if call.status >= 400 {
                                palette.red
                            } else {
                                palette.muted
                            };
                            row()
                                .font_family(mono.clone())
                                .text_sm()
                                .child(
                                    div()
                                        .w(px(56.))
                                        .flex_none()
                                        .text_color(rgb(palette.muted))
                                        .child(at),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(format!(
                                    "{} {} · {}",
                                    call.method, call.path, call.capability
                                )))
                                .child(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child(call.decision.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(36.))
                                        .flex_none()
                                        .text_right()
                                        .text_color(rgb(tone))
                                        .child(call.status.to_string()),
                                )
                                .into_any_element()
                        })
                        .collect(),
                    palette,
                )
            }
        }
    };
    section("Agent writes", body, palette).into_any_element()
}
