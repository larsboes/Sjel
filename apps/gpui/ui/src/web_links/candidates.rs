//! Browser history candidate review rows.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, AnyElement, Context, IntoElement, SharedString};
use sjel_gpui_core::web_links::WebLinkCandidate;

use super::LinksPage;
use crate::theme::Palette;

pub(crate) fn render_candidate_row(
    candidate: &WebLinkCandidate,
    palette: Palette,
    cx: &mut Context<LinksPage>,
) -> AnyElement {
    let keep = candidate.clone();
    let dismiss = candidate.clone();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
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
                        .child(if candidate.title.is_empty() {
                            candidate.host.clone()
                        } else {
                            candidate.title.clone()
                        }),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} · {} visits · {}",
                            candidate.browser, candidate.visits, candidate.url
                        )),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("keep-{}", candidate.id)))
                .small()
                .label("Keep")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.history_decision(keep.clone(), "keep", window, cx)
                })),
        )
        .child(
            Button::new(SharedString::from(format!("dismiss-{}", candidate.id)))
                .small()
                .ghost()
                .label("Dismiss")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.history_decision(dismiss.clone(), "dismiss", window, cx)
                })),
        )
        .into_any_element()
}
