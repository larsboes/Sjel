//! Metrics summary and architectural unit cards.

use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::self_model::{unit_degree, unit_liveness, SelfModelResponse, SelfUnit};

use super::SelfPage;
use crate::ely::data_display::{KpiCard, Statistic};
use crate::primitives::{chart_card, dot, stat_strip};
use crate::theme::{Density, Palette};

pub fn stats_summary(res: &SelfModelResponse, _palette: Palette) -> AnyElement {
    let model = &res.model;
    let units_count = model.units.len() as f64;
    let coupling_count = model.coupling.len() as f64;
    let upstreams_count = model.upstreams.len() as f64;

    stat_strip(vec![
        KpiCard::new(Statistic::new("self-units", "UNITS", units_count))
            .flex_1()
            .min_w(px(120.))
            .into_any_element(),
        KpiCard::new(Statistic::new(
            "self-couplings",
            "COUPLINGS",
            coupling_count,
        ))
        .flex_1()
        .min_w(px(120.))
        .into_any_element(),
        KpiCard::new(Statistic::new(
            "self-upstreams",
            "UPSTREAMS",
            upstreams_count,
        ))
        .flex_1()
        .min_w(px(120.))
        .into_any_element(),
    ])
}

/// Live Mermaid dependency diagram of the top unit couplings in the self-model.
pub fn architecture_diagram(res: &SelfModelResponse, palette: Palette) -> Option<AnyElement> {
    if res.model.coupling.is_empty() {
        return None;
    }
    let mut lines = vec!["```mermaid".to_string(), "flowchart LR".to_string()];
    for edge in res.model.coupling.iter().take(8) {
        let from_id: String = edge
            .from
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let to_id: String = edge
            .to
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        lines.push(format!(
            "  {from_id}[\"{}\"] --> {to_id}[\"{}\"]",
            edge.from, edge.to
        ));
    }
    lines.push("```".to_string());
    let md = lines.join("\n");
    Some(chart_card(
        "DEPENDENCY TOPOLOGY",
        Some(format!("Showing up to 8 of {} couplings", res.model.coupling.len()).into()),
        crate::doc_view::render_markdown(&md, palette, Density::Desktop),
        palette,
    ))
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
