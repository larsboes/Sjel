use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, SharedString};

use crate::route::Page;
use crate::selection::Item;
use crate::shell::{Approval, ShellError};
use crate::workspace::Workspace;
use sjel_gpui_ui::primitives::{dot, list, note, placeholder, row, section};
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::theme::Palette;

/// Nothing that needs you draws nothing: a line saying so pushes the day down for no reason.
/// The first answer still gets its height, so the page does not jump when it arrives.
pub fn needs_you(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    let selected = view.selected();
    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(agent) = &view.agent.value {
        for approval in &agent.pending {
            let marked = selected == Some(Item::Approval(approval.id.clone()));
            rows.push(approval_row(view, marked, approval, palette, cx));
        }
    }
    if let Some(health) = &view.health.value {
        for name in health.down() {
            let marked = selected == Some(Item::Down(name.clone()));
            rows.push(down_row(view, marked, name, palette, cx));
        }
    }

    let error = view.health.error.as_ref().filter(|error| {
        matches!(
            error,
            ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback
        )
    });
    let body = match error {
        Some(error) => note(error.to_string(), palette.red),
        // Nothing waiting and no shell to ask: the row would be the truth, not the answer.
        None if rows.is_empty() => {
            if !view.health.first_load() {
                return None;
            }
            placeholder(56.)
        }
        None => list(rows, palette),
    };
    Some(section("Needs you", body, palette).into_any_element())
}

pub fn approval_row(
    view: &Workspace,
    selected: bool,
    approval: &Approval,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&approval.id);
    let mono = cx.theme().mono_font_family.clone();
    let (allow_id, deny_id) = (approval.id.clone(), approval.id.clone());
    view.selection
        .mark(row(), selected, palette)
        .child(Tag::warning().small().child("agent write"))
        .child(
            div()
                .id(SharedString::from(format!("approval-open-{}", approval.id)))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .cursor_pointer()
                .hover(move |s| s.opacity(0.85))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.right_panel.open_on(Seat::Agent);
                    cx.notify();
                }))
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

pub fn down_row(
    view: &Workspace,
    selected: bool,
    name: String,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(&name);
    let start_name = name.clone();
    view.selection
        .mark(row(), selected, palette)
        .child(dot(palette.red))
        .child(
            div()
                .id(SharedString::from(format!("down-open-{name}")))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .cursor_pointer()
                .hover(move |s| s.opacity(0.85))
                .on_click(cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx)))
                .child(div().text_sm().child(name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Kept running, but not answering · Click to inspect"),
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
