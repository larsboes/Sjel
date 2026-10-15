//! Every capability the shell knows, by name, with start and stop. A start or stop changes a
//! row's dot and button, never its place.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};

use crate::shell::{CapabilityHealth, Liveness};
use crate::theme::Palette;
use crate::ui::{dot, list, note, placeholder, row, section};
use crate::workspace::Workspace;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
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
    let rows: Vec<AnyElement> = health
        .capabilities
        .iter()
        .enumerate()
        .map(|(index, (name, state))| capability_row(view, index, name, *state, palette, cx))
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
    index: usize,
    name: &str,
    state: CapabilityHealth,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let busy = view.busy.contains(name);
    let selected = view.selection.is(index);
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
        .mark(row(), index, palette)
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
