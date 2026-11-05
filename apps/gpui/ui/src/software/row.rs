//! Interactive software app row with toggle expansion and decision actions.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::shell::SoftwareApp;

use super::types::{decision_look, last_used, size, uninstall_label, DECISIONS};
use super::Software;
use crate::primitives::{dot, row};
use crate::theme::Palette;

pub fn render_app_row(
    software: &Software,
    app: &SoftwareApp,
    palette: Palette,
    cx: &mut Context<Software>,
) -> AnyElement {
    let open = software.open.as_deref() == Some(app.bundle_id.as_str());
    let (tone, label) = decision_look(app.decision.as_deref(), palette);
    let clicked = app.clone();
    let header = row()
        .id(SharedString::from(format!("app-{}", app.bundle_id)))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_click(cx.listener(move |view, _, window, cx| view.toggle(&clicked, window, cx)))
        .child(dot(tone))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_sm().child(app.name.clone()))
                .children(app.review.clone().map(|r| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .child(r)
                }))
                .children(
                    app.note
                        .clone()
                        .filter(|n| !n.is_empty() && !open)
                        .map(|n| {
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(n)
                        }),
                ),
        )
        .child(
            div()
                .w(px(72.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(size(app.app_bytes + app.data_bytes)),
        )
        .child(
            div()
                .w(px(80.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(match app.kind.as_str() {
                    "tool" => app.source.clone(),
                    "leftover" => "no app".to_owned(),
                    _ => last_used(app.last_used.as_deref()),
                }),
        )
        .child(
            div()
                .w(px(64.))
                .text_xs()
                .text_color(rgb(tone))
                .child(label),
        );
    if !open {
        return header.into_any_element();
    }
    let busy = software.busy.contains(&format!("decide:{}", app.bundle_id));
    let buttons = DECISIONS.iter().map(|(decision, title)| {
        let decision: &'static str = decision;
        Button::new(SharedString::from(format!("decide-{decision}")))
            .small()
            .when(app.decision.as_deref() != Some(decision), |b| b.ghost())
            .label(*title)
            .loading(busy)
            .on_click(cx.listener(move |view, _, window, cx| view.decide(decision, window, cx)))
    });
    let uninstalling = software
        .busy
        .contains(&format!("uninstall:{}", app.bundle_id));
    let target = app.bundle_id.clone();
    let uninstall = (app.decision.as_deref() == Some("remove")).then(|| {
        Button::new("software-uninstall")
            .small()
            .danger()
            .label(uninstall_label(app))
            .loading(uninstalling)
            .on_click(
                cx.listener(move |view, _, window, cx| view.uninstall(target.clone(), window, cx)),
            )
    });
    let input = software.note.clone();
    div()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .px_3()
                .pb_3()
                .children(input.map(|input| Input::new(&input)))
                .child(div().flex().gap_2().children(buttons).children(uninstall))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} · app {} · data {}",
                            app.bundle_id,
                            size(app.app_bytes),
                            size(app.data_bytes)
                        )),
                ),
        )
        .into_any_element()
}
