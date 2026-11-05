//! Table of recent ingest runs and status badges.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString,
};
use sjel_gpui_core::media::{run_label, MediaIngestRun};

use super::MediaPage;
use crate::primitives::card;
use crate::theme::Palette;

pub fn render_ingest_runs_table(
    runs: &[MediaIngestRun],
    selected_id: Option<i64>,
    palette: Palette,
    cx: &mut Context<MediaPage>,
) -> AnyElement {
    let run_rows = if runs.is_empty() {
        div()
            .p(px(12.0))
            .text_size(px(12.0))
            .text_color(rgb(palette.muted))
            .child("No ingest runs recorded yet.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .children(runs.iter().enumerate().map(|(idx, run)| {
                let run_id = run.id;
                let is_selected = selected_id == Some(run_id);
                let label = run_label(run);
                let finished_str = run.finished.as_deref().unwrap_or("in progress…");

                let inspect_btn =
                    Button::new(SharedString::from(format!("run-inspect-{}", run_id)))
                        .small()
                        .when(!is_selected, |b| b.ghost())
                        .label(if is_selected { "Close" } else { "Inspect" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_run(run_id, cx);
                        }));

                div()
                    .id(SharedString::from(format!("run-row-{}", idx)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(px(8.0))
                    .rounded(px(6.0))
                    .bg(if is_selected {
                        rgba(palette.raised)
                    } else if idx % 2 == 0 {
                        rgba(palette.surface)
                    } else {
                        rgba(palette.window_tint)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(220.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .child(run.staging.clone()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.text))
                            .child(format!("{} seen", run.considered)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.green))
                            .child(format!("{} imported", run.imported)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("{} dup", run.duplicates)),
                    )
                    .child(div().w(px(80.0)).child(if run.refused > 0 {
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(rgb(palette.amber))
                            .text_size(px(10.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xffffff))
                            .child(format!("{} refused", run.refused))
                    } else {
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child("0 refused")
                    }))
                    .child(div().w(px(70.0)).child(if run.failed > 0 {
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(rgb(palette.red))
                            .text_size(px(10.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xffffff))
                            .child(format!("{} failed", run.failed))
                    } else {
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child("0 failed")
                    }))
                    .child(
                        div()
                            .w(px(130.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(finished_str.to_string()),
                    )
                    .child(inspect_btn)
            }))
            .into_any_element()
    };

    let content = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child(format!("Recent Ingest Runs ({})", runs.len())),
        )
        .child(run_rows);

    card(content.into_any_element(), palette)
}
