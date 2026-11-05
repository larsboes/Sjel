//! Unit inspector panel, runtime dependency list, coupling details, and service lifecycle controls.

use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::self_model::{
    incoming_couplings, outgoing_couplings, unit_liveness, SelfModelResponse,
};

use super::SelfPage;
use crate::primitives::card;
use crate::theme::Palette;

pub fn unit_inspector(
    page: &SelfPage,
    res: &SelfModelResponse,
    palette: Palette,
    cx: &mut Context<SelfPage>,
) -> Option<AnyElement> {
    let name = page.selected.as_ref()?;
    let unit = res.model.units.iter().find(|u| &u.name == name)?;
    let is_busy = !page.busy.is_empty();

    let liveness = unit_liveness(res, &unit.name);
    let outgoing = outgoing_couplings(&res.model, &unit.name);
    let incoming = incoming_couplings(&res.model, &unit.name);

    let mut header = div().flex().items_center().justify_between().child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(rgb(palette.text))
                    .child(unit.name.clone()),
            )
            .child(
                div()
                    .px_2()
                    .py(px(1.5))
                    .rounded(px(4.))
                    .bg(rgba(palette.raised))
                    .text_color(rgb(palette.muted))
                    .text_xs()
                    .child(unit.kind.clone()),
            ),
    );

    if let Some(service) = &unit.service {
        let is_up = liveness == Some(true);
        let u_name_toggle = unit.name.clone();
        let action_toggle = if is_up { "stop" } else { "start" };
        let toggle_label = if is_up { "Stop" } else { "Start" };

        let u_name_restart = unit.name.clone();
        let toggle_button = Button::new("toggle-unit-service")
            .small()
            .disabled(is_busy)
            .label(toggle_label)
            .on_click(cx.listener(move |page, _, window, cx| {
                page.run_lifecycle(u_name_toggle.clone(), action_toggle, window, cx);
            }));

        let restart_button = Button::new("restart-unit-service")
            .small()
            .disabled(is_busy)
            .label("↻ Restart")
            .on_click(cx.listener(move |page, _, window, cx| {
                page.run_lifecycle(u_name_restart.clone(), "restart", window, cx);
            }));

        header = header.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(service.port.as_ref().map(|p| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("port :{p}"))
                }))
                .child(toggle_button)
                .child(restart_button),
        );
    }

    let mut details = Vec::new();

    // Dependencies (requires)
    if let Some(service) = &unit.service {
        if !service.requires.is_empty() {
            details.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("Runtime Dependencies"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(service.requires.join(", ")),
                    ),
            );
        }
    }

    // Outgoing couplings
    if !outgoing.is_empty() {
        let out_labels: Vec<String> = outgoing
            .iter()
            .map(|c| format!("{} ({})", c.to, c.kinds.join(", ")))
            .collect();
        details.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Compiles In (Outgoing)"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .child(out_labels.join(" · ")),
                ),
        );
    }

    // Incoming couplings
    if !incoming.is_empty() {
        let in_labels: Vec<String> = incoming
            .iter()
            .map(|c| format!("{} ({})", c.from, c.kinds.join(", ")))
            .collect();
        details.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Compiled Into (Incoming)"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .child(in_labels.join(" · ")),
                ),
        );
    }

    Some(card(
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .children(details)
            .into_any_element(),
        palette,
    ))
}
