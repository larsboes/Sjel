//! Capture tag pills, rows, delivery state indicators, and routing previews.

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, FontWeight};
use sjel_gpui_core::capture::{self, Draft};
use sjel_gpui_core::store::LocalCapture;

use crate::primitives::tone_chip;
use crate::theme::tint;
use crate::{Density, Palette};

pub fn capture_tag_pill(tag: &str, palette: Palette) -> AnyElement {
    let raw = tag.trim_start_matches('#').trim_end_matches('?');
    let tag_color = match raw {
        "todo" | "task" | "buy" | "price" => palette.accent,
        "idea" | "look" => palette.amber,
        "place" | "go" | "learn" => palette.green,
        _ => palette.muted,
    };
    let label = if tag.starts_with('#') {
        tag.to_owned()
    } else {
        format!("#{tag}")
    };
    tone_chip(label, tag_color)
}

/// The line that says where a capture is, in the Mac's own terms once it has one.
pub fn state_of(capture: &LocalCapture, palette: Palette) -> (String, u32) {
    if !capture.on_the_mac() {
        return ("Waiting for the Mac".to_owned(), palette.amber);
    }
    match capture.status.as_deref() {
        Some("done") => ("Done on the Mac".to_owned(), palette.green),
        Some("dismissed") => ("Dismissed on the Mac".to_owned(), palette.muted),
        _ => ("On the Mac".to_owned(), palette.green),
    }
}

/// What the line under the field says.
pub fn preview_label(typed: &str, tag: Option<&str>) -> String {
    match Draft::parse(typed, tag) {
        Ok(draft) => draft.preview(),
        Err(_) => capture::route_for(tag, None)
            .map(|route| route.label().to_owned())
            .unwrap_or_else(|| "Stays in the inbox".to_owned()),
    }
}

pub fn render_capture_row(
    capture: &LocalCapture,
    refusal: Option<String>,
    index: usize,
    density: Density,
    palette: Palette,
) -> AnyElement {
    let tag_pill = match capture.tag.as_deref() {
        Some(tag) => capture_tag_pill(tag, palette),
        None => div()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child("no tag")
            .into_any_element(),
    };
    let url_line = capture.url.as_deref().and_then(|url| {
        if capture.text.as_deref() != Some(url) {
            Some(
                div()
                    .text_xs()
                    .text_color(rgb(palette.accent))
                    .truncate()
                    .child(url.to_string()),
            )
        } else {
            None
        }
    });

    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(1.))
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.text))
                .truncate()
                .child(capture.label()),
        )
        .children(url_line)
        .child(div().flex().items_center().gap_2().child(tag_pill).when(
            refusal.is_none(),
            |line| {
                let (state, colour) = state_of(capture, palette);
                line.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .py(px(0.5))
                        .rounded_full()
                        .bg(rgba(tint(colour, 0x18)))
                        .child(div().size(px(5.)).rounded_full().bg(rgb(colour)))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(colour))
                                .child(state),
                        ),
                )
            },
        ));

    if let Some(reason) = refusal {
        column = column.child(
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(format!("The Mac refused it: {reason}")),
        );
    }

    div()
        .id(("inbox-capture", index))
        .flex()
        .items_center()
        .gap_3()
        .px(density.pad_x())
        .py(density.pad_y())
        .min_h(density.row_height())
        .child(column)
        .when_some(capture.tag.clone(), |row, tag| {
            row.child(
                div()
                    .flex_none()
                    .px_2()
                    .py(px(1.5))
                    .rounded(px(4.))
                    .bg(rgba(tint(palette.raised, 0x66)))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(
                        capture::route_for(Some(&tag), capture.url.as_deref())
                            .map(|route| route.label().to_owned())
                            .unwrap_or_else(|| "Inbox".to_owned()),
                    ),
            )
        })
        .into_any_element()
}
