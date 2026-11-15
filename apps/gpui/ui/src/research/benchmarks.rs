//! Benchmark suites and model evaluation metrics tab.
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row};
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
            let run_rows = suite
                .runs
                .iter()
                .map(|run| {
                    let result = &run.result;
                    let tone = if result.accuracy >= 0.90 {
                        Tone::Success
                    } else if result.accuracy >= 0.75 {
                        Tone::Warning
                    } else {
                        Tone::Danger
                    };
                    Row::new(
                        run.file.clone(),
                        [
                            Cell::Text(result.model.clone().into()),
                            Cell::Text(result.backend.clone().into()),
                            Cell::Figure(
                                format!("{}/{}", result.correct, result.n).into(),
                                result.correct as f64,
                                Tone::Neutral,
                            ),
                            Cell::Figure(
                                format!("{:.0}%", result.accuracy * 100.0).into(),
                                result.accuracy,
                                tone,
                            ),
                            Cell::Figure(
                                format!("{:.0} ms", result.latency_ms.p50).into(),
                                result.latency_ms.p50,
                                Tone::Neutral,
                            ),
                            Cell::Figure(
                                format!("{:.0} ms", result.latency_ms.p95).into(),
                                result.latency_ms.p95,
                                Tone::Neutral,
                            ),
                            Cell::Text(result.host.clone().into()),
                            Cell::Text(result.date.clone().into()),
                        ],
                    )
                })
                .collect::<Vec<_>>();
            let runs_table = DataTable::new(
                format!("benchmark-runs-{}", suite.name),
                [
                    Column::new("model", "MODEL"),
                    Column::new("backend", "BACKEND"),
                    Column::new("correct", "CORRECT").end(),
                    Column::new("accuracy", "ACCURACY").end(),
                    Column::new("p50", "P50").end(),
                    Column::new("p95", "P95").end(),
                    Column::new("host", "HOST"),
                    Column::new("date", "DATE"),
                ],
            )
            .rows(run_rows);

            // Per-case matrix table
            let case_rows = suite.case_ids.iter().map(|case_id| {
                let verdicts = suite.runs.iter().map(|run| {
                    let passed = run
                        .result
                        .cases
                        .iter()
                        .find(|c| c.id == *case_id)
                        .is_some_and(|c| c.expected == c.got);

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
                .child(runs_table)
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
