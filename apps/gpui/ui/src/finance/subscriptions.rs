//! Recurring subscriptions, monthly burn analysis, and billing cycle monitors.

use std::time::Instant;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, FontWeight};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{calculate_monthly_burn, SubscriptionSummary};

use crate::primitives::tone_chip;
use crate::theme::Palette;

use super::FinancePage;

#[derive(Default)]
pub(crate) struct SubsDerived {
    pub(super) key: Option<Option<Instant>>,
    pub(super) monthly_burn: i64,
    pub(super) active_count: usize,
    pub(super) summaries: Vec<SubscriptionSummary>,
}

impl FinancePage {
    pub(super) fn sync_subs_derived(&mut self) {
        let key = self.subscriptions.loaded_at;
        if self.subs_derived.key == Some(key) {
            return;
        }
        let records = self.subscriptions.value.as_deref().unwrap_or_default();
        let monthly_burn = calculate_monthly_burn(records);
        let summaries: Vec<_> = records
            .iter()
            .map(SubscriptionSummary::from_record)
            .collect();
        let active_count = summaries
            .iter()
            .filter(|s| s.status == "active" || s.status == "trial")
            .count();
        self.subs_derived = SubsDerived {
            key: Some(key),
            monthly_burn,
            active_count,
            summaries,
        };
    }

    pub(super) fn render_subscriptions(&mut self, palette: Palette) -> AnyElement {
        self.sync_subs_derived();
        let monthly_burn = self.subs_derived.monthly_burn;
        let summaries = &self.subs_derived.summaries;

        let header_stat = div()
            .flex()
            .items_center()
            .justify_between()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("TOTAL RECURRING SUBSCRIPTIONS"),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format_money(-monthly_burn, "EUR")),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(format!("{} subscriptions tracked", summaries.len())),
            );

        let cards_grid = if summaries.is_empty() {
            div()
                .p_8()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No subscriptions found.")
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().gap_2();
            for sub in summaries {
                let status_color = match sub.status.as_str() {
                    "active" => palette.green,
                    "trial" => palette.accent,
                    "paused" => palette.amber,
                    _ => palette.muted,
                };

                let card = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(tone_chip(sub.status.to_uppercase(), status_color))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.text))
                                            .child(sub.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(format!("{} · {}", sub.category, sub.cycle)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child(format!(
                                        "{}/mo",
                                        format_money(-sub.monthly_equivalent_cents, &sub.currency)
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "{} billed",
                                        format_money(-sub.amount_cents, &sub.currency)
                                    )),
                            ),
                    );
                list = list.child(card);
            }
            list.into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .size_full()
            .child(header_stat)
            .child(
                div()
                    .id("finance-subscriptions-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.subscriptions_scroll)
                    .vertical_scrollbar(&self.subscriptions_scroll)
                    .pb_6()
                    .child(cards_grid),
            )
            .into_any_element()
    }
}
