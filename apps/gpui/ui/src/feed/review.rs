use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight};
use sjel_gpui_core::shell::FeedQualityFlag;

use crate::theme::Palette;

use super::reading::{muted, title};
use super::Feed;

impl Feed {
    /// Flags suggest a look; they never change a decision or start enrichment.
    pub(crate) fn review_list(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let items = flagged(self.review.value.iter().flatten());
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(match items.len() {
                        1 => "1 entry to inspect".to_owned(),
                        n => format!("{n} entries to inspect"),
                    }),
            )
            .child(
                Button::new("feed-review-refresh")
                    .small()
                    .label(if self.refreshing {
                        "Computing…"
                    } else {
                        "Recompute flags"
                    })
                    .disabled(self.refreshing)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh_review(cx))),
            );
        let mut list = div().flex().flex_col().gap_3();
        if let Some(r) = &self.review_refresh {
            list = list.child(muted(
                &format!(
                    "Checked {}; flagged {} entries with {} signals; provider calls {}.",
                    r.reviewed, r.flagged_items, r.flag_count, r.provider_calls
                ),
                palette,
            ));
        }
        if let Some(error) = &self.review.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        } else if self.review.value.is_none() && self.review.loading {
            list = list.child(muted("Loading stored flags…", palette));
        } else if items.is_empty() && !self.review.loading {
            list = list.child(muted(
                "No stored flags. Recompute, or the evidence is inside the thresholds.",
                palette,
            ));
        }
        for (i, flags) in items.into_iter().enumerate() {
            let first = flags[0];
            let id = first.feed_id.clone();
            let url = first.url.clone();
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(8.))
                .bg(rgba(palette.raised))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id(("feed-review-title", i))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(rgb(palette.accent)))
                                .on_click(
                                    cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)),
                                )
                                .child(title(first.title.as_deref(), &first.url)),
                        )
                        .child(
                            Button::new(("feed-review-original", i))
                                .small()
                                .ghost()
                                .label("Original ↗")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                )
                .child(muted(
                    &format!(
                        "{} · content {} · {}",
                        first.status,
                        first.content_status,
                        first
                            .derived_at
                            .get(..16)
                            .unwrap_or(&first.derived_at)
                            .replace('T', " ")
                    ),
                    palette,
                ));
            for flag in flags {
                card = card.child(
                    div()
                        .flex()
                        .gap_3()
                        .pt_2()
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .child(signal_label(&flag.signal)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_sm().child(flag.reason.clone()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(flag.evidence.clone()),
                                ),
                        ),
                );
            }
            list = list.child(card);
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_row(cx))
            .child(head)
            .child(list)
            .into_any_element()
    }
}

/// The queue's rows grouped by entry, in the order comms returned the entries.
pub(crate) fn flagged<'a>(
    flags: impl Iterator<Item = &'a FeedQualityFlag>,
) -> Vec<Vec<&'a FeedQualityFlag>> {
    let mut items: Vec<Vec<&FeedQualityFlag>> = Vec::new();
    for flag in flags {
        match items
            .iter_mut()
            .find(|item| item[0].feed_id == flag.feed_id)
        {
            Some(item) => item.push(flag),
            None => items.push(vec![flag]),
        }
    }
    items
}

/// The dashboard's names for comms' signal keys (`capabilities/comms/src/quality.rs`).
pub(crate) fn signal_label(signal: &str) -> String {
    match signal {
        "content_status" => "Content",
        "extraction_path" => "Extraction",
        "retention" => "Retention",
        "boilerplate_leakage" => "Boilerplate",
        "summary_attempts" => "Summary retries",
        "ranking_basis" => "Ranking basis",
        other => other,
    }
    .to_owned()
}
