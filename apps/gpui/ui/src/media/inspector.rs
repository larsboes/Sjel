//! Ingest run detail inspector and disposition filters.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::media::{format_bytes, run_label, sort_run_items, MediaIngestItem};

use super::MediaPage;
use crate::data::Remote;
use crate::primitives::{card, placeholder};
use crate::theme::Palette;

pub fn render_run_detail_card(
    page: &MediaPage,
    palette: Palette,
    cx: &mut Context<MediaPage>,
) -> Option<AnyElement> {
    let run_id = page.selected_run_id?;

    let body: AnyElement = if let Some(detail) = page.selected_run_detail.value.as_ref() {
        let run = &detail.run;
        let sorted = sort_run_items(&detail.items);

        let filter = page.filter_disposition.as_str();
        let filtered_items: Vec<&MediaIngestItem> = match filter {
            "attention" => sorted
                .iter()
                .filter(|i| matches!(i.disposition.as_str(), "failed" | "refused"))
                .collect(),
            "imported" => sorted
                .iter()
                .filter(|i| i.disposition == "imported")
                .collect(),
            "duplicate" => sorted
                .iter()
                .filter(|i| i.disposition == "duplicate")
                .collect(),
            _ => sorted.iter().collect(),
        };

        let close_btn = Button::new("close-run-detail")
            .small()
            .ghost()
            .label("Close Inspector")
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected_run_id = None;
                this.selected_run_detail = Remote::default();
                cx.notify();
            }));

        let filter_tabs = div()
            .flex()
            .gap(px(6.0))
            .child(
                Button::new("disp-all")
                    .small()
                    .when(filter != "all", |b| b.ghost())
                    .label(format!("All ({})", sorted.len()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_disposition = "all".to_string();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("disp-attention")
                    .small()
                    .when(filter != "attention", |b| b.ghost())
                    .label(format!("Attention ({})", run.refused + run.failed))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_disposition = "attention".to_string();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("disp-imported")
                    .small()
                    .when(filter != "imported", |b| b.ghost())
                    .label(format!("Imported ({})", run.imported))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_disposition = "imported".to_string();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("disp-duplicate")
                    .small()
                    .when(filter != "duplicate", |b| b.ghost())
                    .label(format!("Duplicate ({})", run.duplicates))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_disposition = "duplicate".to_string();
                        cx.notify();
                    })),
            );

        let item_list = if filtered_items.is_empty() {
            div()
                .p(px(8.0))
                .text_size(px(11.0))
                .text_color(rgb(palette.muted))
                .child("No paths matching this filter.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .children(filtered_items.into_iter().enumerate().map(|(idx, item)| {
                    let badge_bg = match item.disposition.as_str() {
                        "failed" => rgb(palette.red),
                        "refused" => rgb(palette.amber),
                        "imported" => rgb(palette.green),
                        "resumed" => rgb(palette.accent),
                        _ => rgb(palette.muted),
                    };

                    let imported_text = item
                        .imported_relpath
                        .as_deref()
                        .map(|p| format!(" → {}", p))
                        .unwrap_or_default();

                    div()
                        .id(SharedString::from(format!("item-{}", idx)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(6.0))
                        .rounded(px(4.0))
                        .bg(rgba(palette.surface))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .px(px(6.0))
                                                .py(px(1.0))
                                                .rounded(px(3.0))
                                                .bg(badge_bg)
                                                .text_size(px(10.0))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(0xffffff))
                                                .child(item.disposition.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(rgb(palette.text))
                                                .child(item.relpath.clone()),
                                        )
                                        .children(item.verified.then(|| {
                                            div()
                                                .px(px(4.0))
                                                .py(px(1.0))
                                                .rounded(px(3.0))
                                                .bg(rgb(palette.green))
                                                .text_size(px(9.0))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(0xffffff))
                                                .child("verified")
                                        })),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child(format!("{}{}", item.reason, imported_text)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(item.size.map(format_bytes).unwrap_or_default()),
                        )
                }))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(format!("Ingest Run Inspection: {}", run_label(run))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "Source: {} | Staging: {}",
                                        run.source, run.staging
                                    )),
                            ),
                    )
                    .child(close_btn),
            )
            .child(filter_tabs)
            .child(item_list)
            .into_any_element()
    } else if let Some(err) = &page.selected_run_detail.error {
        div()
            .p(px(8.0))
            .text_size(px(12.0))
            .text_color(rgb(palette.red))
            .child(format!("Could not load run #{}: {}", run_id, err))
            .into_any_element()
    } else {
        placeholder(120.0).into_any_element()
    };

    Some(card(body, palette))
}
