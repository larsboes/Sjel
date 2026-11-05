//! Attempt history ledger and individual run display.
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::backup::{
    format_bytes, format_duration, run_outcome, BackupRunAttemptRecord,
};

use crate::theme::{tint, Palette};

use super::BackupPage;

impl BackupPage {
    pub(super) fn render_history_view(&self, palette: Palette, _cx: &mut Context<Self>) -> AnyElement {
        let Some((runs, _running)) = &self.runs.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading attempt history ledger…")
                .into_any_element();
        };

        if runs.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No recent backup runs recorded.")
                .into_any_element();
        }

        let mut rows = Vec::new();
        for r in runs {
            rows.push(self.render_run_row(r, palette));
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_run_row(&self, r: &BackupRunAttemptRecord, palette: Palette) -> AnyElement {
        let outcome = run_outcome(r.exit_code, r.finished_epoch);
        let (outcome_bg, outcome_fg, outcome_label) = match outcome {
            "ok" => (rgba(tint(palette.green, 0x20)), palette.green, "exit 0"),
            "running" => (rgba(tint(palette.accent, 0x20)), palette.accent, "running"),
            _ => (rgba(tint(palette.red, 0x20)), palette.red, "failed"),
        };

        let duration_label = format_duration(r.started_epoch, r.finished_epoch);

        div()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgba(tint(palette.border, 0x20)))
            .bg(rgba(tint(palette.raised, 0x30)))
            .flex()
            .flex_col()
            .gap_1p5()
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
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(r.capability.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("→ {}", r.target)),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(outcome_bg)
                                    .text_color(rgb(outcome_fg))
                                    .child(outcome_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("duration: {duration_label}"))
                            .child(r.started_at.clone()),
                    ),
            )
            .children(r.archive.as_ref().map(|arc| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .child(format!(
                        "Archive: {} ({})",
                        arc.name,
                        format_bytes(Some(arc.bytes as u64))
                    ))
                    .child(format!("SHA: {}", &arc.sha256[..12.min(arc.sha256.len())]))
            }))
            .children(if !r.detail.is_empty() {
                Some(
                    div()
                        .text_xs()
                        .text_color(if outcome == "failed" {
                            rgb(palette.red)
                        } else {
                            rgb(palette.muted)
                        })
                        .font_family(".AppleSystemUIFontMonospaced")
                        .child(r.detail.clone()),
                )
            } else {
                None
            })
            .into_any_element()
    }
}
