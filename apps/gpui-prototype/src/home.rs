//! Home answers "what needs me now": waiting agent writes and broken capabilities first, then
//! what is still ahead today and tomorrow, then what the agents changed.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, SharedString};

use crate::calendar;
use crate::selection::Item;
use crate::shell::{Approval, ShellError};
use crate::theme::Palette;
use crate::ui::{dot, list, note, placeholder, row, section};
use crate::workspace::Workspace;

const RECENT_WRITES: usize = 6;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_8()
        .child(needs_you(view, palette, cx))
        .child(ahead(view, palette, cx))
        .child(agent_writes(view, palette, cx))
}

fn needs_you(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    // Row indexes follow `Workspace::items`: approvals, then down capabilities, then entries.
    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(agent) = &view.agent.value {
        for approval in &agent.pending {
            rows.push(approval_row(view, rows.len(), approval, palette, cx));
        }
    }
    if let Some(health) = &view.health.value {
        for name in health.down() {
            rows.push(down_row(view, rows.len(), name, palette, cx));
        }
    }

    let body = match (&view.health.error, rows.is_empty()) {
        (
            Some(
                error @ (ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback),
            ),
            _,
        ) => note(error.to_string(), palette.red),
        (_, false) => list(rows, palette),
        (_, true) if view.health.first_load() => placeholder(56.),
        (_, true) => note("Nothing needs you.", palette.muted),
    };
    section("Needs you", body, palette)
}

fn approval_row(
    view: &Workspace,
    index: usize,
    approval: &Approval,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&approval.id);
    let mono = cx.theme().mono_font_family.clone();
    let (allow_id, deny_id) = (approval.id.clone(), approval.id.clone());
    view.selection
        .mark(row(), index, palette)
        .child(Tag::warning().small().child("agent write"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().font_family(mono).text_sm().child(format!(
                    "{} {} · {}",
                    approval.method, approval.path, approval.capability
                )))
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(approval.preview.clone()),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("deny-{}", approval.id)))
                .ghost()
                .small()
                .label("Deny")
                .tooltip("Deny (⌘⌫)")
                .disabled(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.decide(deny_id.clone(), false, window, cx)
                })),
        )
        .child(
            Button::new(SharedString::from(format!("allow-{}", approval.id)))
                .primary()
                .small()
                .label("Allow")
                .tooltip("Allow (↩)")
                .loading(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.decide(allow_id.clone(), true, window, cx)
                })),
        )
        .into_any_element()
}

fn down_row(
    view: &Workspace,
    index: usize,
    name: String,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&name);
    let start_name = name.clone();
    view.selection
        .mark(row(), index, palette)
        .child(dot(palette.red))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_sm().child(name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Kept running, but not answering"),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("start-{name}")))
                .small()
                .label("Start")
                .tooltip("Start (↩)")
                .loading(busy)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.set_running(start_name.clone(), true, window, cx)
                })),
        )
        .into_any_element()
}

fn ahead(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let now = chrono::Local::now().naive_local();
    let mono = cx.theme().mono_font_family.clone();
    let first = view
        .items()
        .iter()
        .take_while(|item| **item != Item::Entry)
        .count();
    let body = match (&view.calendar.value, &view.calendar.error) {
        (_, Some(error)) => note(
            format!("The calendar did not answer. {error}"),
            palette.muted,
        ),
        (None, None) => placeholder(96.),
        (Some(_), None) => {
            let entries = view.ahead();
            if entries.is_empty() {
                note("Nothing else today or tomorrow.", palette.muted)
            } else {
                list(
                    entries
                        .iter()
                        .enumerate()
                        .map(|(offset, entry)| {
                            view.selection
                                .mark(row(), first + offset, palette)
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_none()
                                        .font_family(mono.clone())
                                        .text_sm()
                                        .text_color(rgb(palette.muted))
                                        .child(calendar::when(entry, now)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(entry.title.clone()),
                                )
                                .when(entry.commitment == "possible", |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child("possible"),
                                    )
                                })
                                .into_any_element()
                        })
                        .collect(),
                    palette,
                )
            }
        }
    };
    section("Ahead", body, palette)
}

fn agent_writes(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
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
    section("Agent writes", body, palette)
}
