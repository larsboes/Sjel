//! Index metrics, volume integrity summary, and mounted volumes table.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use sjel_gpui_core::media::{format_bytes, total_volume_bytes, MediaStatus, MediaVolume};

use super::MediaPage;
use crate::primitives::{card, dot, mono_num, stat_card, stat_strip};
use crate::theme::Palette;

pub fn render_index_summary(
    status: &MediaStatus,
    palette: Palette,
    cx: &mut Context<MediaPage>,
) -> AnyElement {
    let total_bytes = total_volume_bytes(status);
    let volume_count = status.volumes.len();

    let header = div()
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
                        .items_center()
                        .gap(px(8.0))
                        .child(dot(palette.accent))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child("Media Ledger & Ingest Gate"),
                        ),
                )
                .child(
                    Button::new("media-refresh")
                        .small()
                        .ghost()
                        .label("Refresh")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh(cx);
                        })),
                ),
        )
        .child(stat_strip(vec![
            stat_card(
                "UNIQUE DIGESTS",
                status.files.to_string(),
                None,
                palette.text,
                palette,
            ),
            stat_card(
                "INDEXED PATHS",
                status.locations.to_string(),
                None,
                palette.text,
                palette,
            ),
            stat_card(
                "TOTAL VOLUME",
                format_bytes(total_bytes),
                Some(format!("across {} volume(s)", volume_count).into()),
                palette.text,
                palette,
            ),
        ]));

    card(header.into_any_element(), palette)
}

pub fn render_volumes_table(volumes: &[MediaVolume], palette: Palette) -> AnyElement {
    let volume_rows = if volumes.is_empty() {
        div()
            .p(px(12.0))
            .text_size(px(12.0))
            .text_color(rgb(palette.muted))
            .child("No volumes indexed yet. Run `media index` against a mounted library.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .children(volumes.iter().enumerate().map(|(idx, vol)| {
                div()
                    .id(SharedString::from(format!("vol-row-{}", idx)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(px(8.0))
                    .rounded(px(6.0))
                    .bg(if idx % 2 == 0 {
                        rgba(palette.surface)
                    } else {
                        rgba(palette.window_tint)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(240.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(vol.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .font_family(".AppleSystemUIFontMonospaced")
                                    .child(vol.uuid.clone()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(100.0))
                            .text_size(px(12.0))
                            .child(mono_num(format!("{} paths", vol.locations), palette.text)),
                    )
                    .child(
                        div()
                            .w(px(100.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(mono_num(format_bytes(vol.bytes), palette.text)),
                    )
                    .child(
                        div()
                            .w(px(160.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("Last seen: {}", vol.last_seen)),
                    )
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
                .child(format!("Mounted Volumes ({})", volumes.len())),
        )
        .child(volume_rows);

    card(content.into_any_element(), palette)
}
