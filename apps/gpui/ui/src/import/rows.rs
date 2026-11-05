use gpui_kit::{div, prelude::*, px, rgb, AnyElement, IntoElement};
use sjel_gpui_core::shell::{ImportPlan, ImportStatus};

use crate::backup::size;
use crate::primitives::{dot, row};
use crate::theme::Palette;

pub fn plan_rows(plan: &ImportPlan, palette: Palette) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = plan
        .routes
        .iter()
        .map(|route| {
            let tone = if route.route == "skip" {
                palette.muted
            } else {
                palette.accent
            };
            line(
                dot(tone),
                &route.route,
                route.files,
                Some(size(route.bytes)),
                palette,
            )
        })
        .collect();
    rows.extend(plan.skipped.iter().map(|skip| {
        line(
            div(),
            &format!("skip: {}", skip.reason),
            skip.files,
            None,
            palette,
        )
    }));
    rows
}

pub fn status_rows(status: &ImportStatus, palette: Palette) -> Vec<AnyElement> {
    status
        .rows
        .iter()
        .map(|row| {
            let tone = match row.outcome.as_str() {
                "imported" | "present" => palette.green,
                "pending" => palette.amber,
                "failed" => palette.red,
                _ => palette.muted,
            };
            let label = match &row.detail {
                Some(detail) => format!("{} {} · {detail}", row.route, row.outcome),
                None => format!("{} {}", row.route, row.outcome),
            };
            line(dot(tone), &label, row.files, None, palette)
        })
        .collect()
}

pub fn line(
    lead: impl IntoElement,
    label: &str,
    files: usize,
    bytes: Option<String>,
    palette: Palette,
) -> AnyElement {
    row()
        .child(div().w(px(7.)).child(lead))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(label.to_owned()),
        )
        .child(
            div()
                .w(px(80.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{files} files")),
        )
        .child(
            div()
                .w(px(72.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(bytes.unwrap_or_default()),
        )
        .into_any_element()
}
