//! Metrics summary and architectural unit cards.

use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::self_model::{unit_degree, unit_liveness, SelfModelResponse, SelfUnit};

use super::SelfPage;
use crate::primitives::{dot, stat_card, stat_strip};
use crate::theme::Palette;

pub fn stats_summary(res: &SelfModelResponse, palette: Palette) -> AnyElement {
    let model = &res.model;
    let units_count = model.units.len();
    let coupling_count = model.coupling.len();
    let upstreams_count = model.upstreams.len();

    stat_strip(vec![
        stat_card(
            "UNITS",
            units_count.to_string(),
            None,
            palette.text,
            palette,
        ),
        stat_card(
            "COUPLINGS",
            coupling_count.to_string(),
            None,
            palette.text,
            palette,
        ),
        stat_card(
            "UPSTREAMS",
            upstreams_count.to_string(),
            None,
            palette.text,
            palette,
        ),
    ])
}

pub fn unit_card(
    page: &SelfPage,
    u: &SelfUnit,
    res: &SelfModelResponse,
    palette: Palette,
    cx: &mut Context<SelfPage>,
) -> AnyElement {
    let is_selected = page.selected.as_deref() == Some(&u.name);
    let liveness = unit_liveness(res, &u.name);
    let degree = unit_degree(&res.model, &u.name);
    let files = u.code.as_ref().map(|c| c.files).unwrap_or(0);
    let unit_name = u.name.clone();

    let dot_color = match liveness {
        Some(true) => palette.green,
        Some(false) => palette.amber,
        None => palette.border,
    };

    let card_border = if is_selected {
        palette.accent
    } else {
        palette.border
    };

    div()
        .id(SharedString::from(format!("unit-card-{}", u.name)))
        .flex()
        .flex_col()
        .justify_between()
        .p_3()
        .min_w(px(220.))
        .min_h(px(90.))
        .rounded(px(8.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(card_border))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(palette.raised)))
        .on_click(cx.listener(move |page, _, _, cx| {
            if page.selected.as_deref() == Some(&unit_name) {
                page.selected = None;
            } else {
                page.selected = Some(unit_name.clone());
            }
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(dot(dot_color))
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_sm()
                                .text_color(rgb(palette.text))
                                .child(u.name.clone()),
                        ),
                )
                .child(
                    div()
                        .px_1()
                        .py(px(1.))
                        .rounded(px(3.))
                        .bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .text_xs()
                        .child(u.kind.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pt_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{files} files"))
                .child(if degree > 0 {
                    format!("{degree} couplings")
                } else {
                    "isolated".to_string()
                }),
        )
        .into_any_element()
}
