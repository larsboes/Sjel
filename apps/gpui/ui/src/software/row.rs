//! Decision editor and uninstall action for the opened software record.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, rgb, AnyElement, Context, SharedString};
use sjel_gpui_core::shell::SoftwareApp;

use super::types::{size, uninstall_label, DECISIONS};
use super::Software;
use crate::theme::Palette;

pub(super) fn render_app_detail(
    software: &Software,
    app: &SoftwareApp,
    palette: Palette,
    cx: &mut Context<Software>,
) -> AnyElement {
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
        .child(div().text_sm().child(app.name.clone()))
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
