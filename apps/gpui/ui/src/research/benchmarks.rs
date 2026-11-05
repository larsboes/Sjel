//! Benchmark suites and model evaluation metrics tab.
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement};
use sjel_gpui_core::research::BenchmarkSuite;

use crate::primitives::{card, dot, note};
use crate::theme::Palette;

use super::ResearchPage;

impl ResearchPage {
    pub(super) fn render_benchmarks_tab(
        &self,
        suites: &[BenchmarkSuite],
        palette: Palette,
    ) -> AnyElement {
        if suites.is_empty() {
            return note("No benchmark suites recorded.", palette.muted);
        }

        let mut panels = Vec::new();

        for suite in suites {
            // Runs table
            let run_rows = suite.runs.iter().map(|run| {
                let acc = run.result.accuracy;
                let acc_pct = format!("{:.0}%", acc * 100.0);
                let acc_color = if acc >= 0.90 {
                    palette.green
                } else if acc >= 0.75 {
                    palette.amber
                } else {
                    palette.red
                };

                div()
                    .flex()
                    .items_center()
                    .py(px(6.0))
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .w(px(180.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(run.result.model.clone()),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.backend.clone()),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(12.0))
                            .text_color(rgb(palette.text))
                            .child(format!("{}/{}", run.result.correct, run.result.n)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(acc_color))
                            .child(acc_pct),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("{:.0} ms", run.result.latency_ms.p50)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("{:.0} ms", run.result.latency_ms.p95)),
                    )
                    .child(
                        div()
                            .w(px(110.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.host.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.date.clone()),
                    )
                    .into_any_element()
            });

            let runs_table_header = div()
                .flex()
                .items_center()
                .py(px(4.0))
                .px(px(8.0))
                .border_b_1()
                .border_color(rgba(palette.border))
                .text_size(px(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(div().w(px(180.0)).child("MODEL"))
                .child(div().w(px(90.0)).child("BACKEND"))
                .child(div().w(px(80.0)).child("CORRECT"))
                .child(div().w(px(80.0)).child("ACCURACY"))
                .child(div().w(px(80.0)).child("P50"))
                .child(div().w(px(80.0)).child("P95"))
                .child(div().w(px(110.0)).child("HOST"))
                .child(div().flex_1().child("DATE"));

            // Per-case matrix table
            let case_rows = suite.case_ids.iter().map(|case_id| {
                let verdicts = suite.runs.iter().map(|run| {
                    let passed = run
                        .result
                        .cases
                        .iter()
                        .find(|c| c.id == *case_id)
                        .map_or(false, |c| c.expected == c.got);

                    let (symbol, color) = if passed {
                        ("✓", palette.green)
                    } else {
                        ("✗", palette.red)
                    };

                    div()
                        .w(px(80.0))
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(symbol)
                        .into_any_element()
                });

                div()
                    .flex()
                    .items_center()
                    .py(px(4.0))
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .w(px(220.0))
                            .text_size(px(11.0))
                            .font_family(".AppleSystemUIFontMonospaced")
                            .text_color(rgb(palette.text))
                            .child(case_id.clone()),
                    )
                    .children(verdicts)
                    .into_any_element()
            });

            let matrix_header_runs = suite.runs.iter().map(|run| {
                div()
                    .w(px(80.0))
                    .text_size(px(10.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(run.result.model.clone())
                    .into_any_element()
            });

            let matrix_header = div()
                .flex()
                .items_center()
                .py(px(4.0))
                .px(px(8.0))
                .border_b_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .w(px(220.0))
                        .text_size(px(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("EVALUATION CASE"),
                )
                .children(matrix_header_runs);

            let suite_card = div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(dot(palette.accent))
                                .child(
                                    div()
                                        .text_size(px(15.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(format!("Suite: {}", suite.name)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("{} runs evaluated", suite.runs.len())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .bg(rgba(palette.surface))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .child(runs_table_header)
                        .children(run_rows),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(format!(
                                    "Per-Case Breakdown ({} test cases)",
                                    suite.case_ids.len()
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .bg(rgba(palette.surface))
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(rgba(palette.border))
                                .child(matrix_header)
                                .children(case_rows),
                        ),
                );

            panels.push(card(suite_card.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }
}
