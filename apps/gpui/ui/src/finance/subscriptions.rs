//! Recurring subscriptions, monthly burn analysis, and billing cycle monitors.

use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::scroll::ScrollableElement;
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{calculate_monthly_burn, SubscriptionSummary};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::charts::{BarChart, Series};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, SortKey};
use crate::ely::theme::{ControlSize, Density};
use crate::theme::Palette;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, FontWeight, SharedString,
};

use super::FinancePage;

fn sub_columns() -> [Column; 5] {
    [
        Column::new("name", "SUBSCRIPTION"),
        Column::new("category", "CATEGORY").width(rems(8.)),
        Column::new("cycle", "CYCLE").width(rems(7.)).muted(),
        Column::new("status", "STATUS").width(rems(6.)),
        Column::new("monthly", "MONTHLY").width(rems(8.)).end(),
    ]
}

fn sub_row(sub: &SubscriptionSummary) -> Row {
    let status_tone = match sub.status.as_str() {
        "active" => Tone::Success,
        "trial" => Tone::Accent,
        "paused" => Tone::Warning,
        _ => Tone::Neutral,
    };
    let monthly_text = format!(
        "{}/mo",
        format_money(-sub.monthly_equivalent_cents, &sub.currency)
    );
    let billed_text = format!("{} billed", format_money(-sub.amount_cents, &sub.currency));
    Row::new(
        sub.name.clone(),
        [
            Cell::Lines(sub.name.clone().into(), billed_text.into()),
            Cell::Tag(sub.category.clone().into(), Tone::Neutral),
            Cell::Text(sub.cycle.clone().into()),
            Cell::Tag(sub.status.to_uppercase().into(), status_tone),
            Cell::Figure(
                monthly_text.into(),
                sub.monthly_equivalent_cents as f64,
                Tone::Danger,
            ),
        ],
    )
}

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

    pub(super) fn render_subscriptions(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_subs_derived();
        let monthly_burn = self.subs_derived.monthly_burn;
        let annual_burn = monthly_burn * 12;
        let active_count = self.subs_derived.active_count;
        let summaries = &self.subs_derived.summaries;
        let mut categories = std::collections::BTreeMap::<(String, String), i64>::new();
        for sub in summaries
            .iter()
            .filter(|sub| sub.status == "active" || sub.status == "trial")
        {
            *categories
                .entry((sub.category.clone(), sub.currency.clone()))
                .or_default() += sub.monthly_equivalent_cents;
        }
        let charts = categories
            .keys()
            .map(|(_, currency)| currency.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|currency| {
                let mut entries = categories
                    .iter()
                    .filter(|((_, unit), _)| *unit == currency)
                    .map(|((cat, _), cents)| (cat.clone(), *cents))
                    .collect::<Vec<_>>();
                entries.sort_by(|a, b| b.1.cmp(&a.1));
                let unit = currency.clone();
                BarChart::new(
                    format!("subscription-categories-{currency}"),
                    entries.iter().map(|(category, _)| category.clone()),
                )
                .series(Series::new(
                    format!("Monthly recurring · {currency}"),
                    entries.iter().map(|(_, cents)| *cents as f64),
                ))
                .horizontal()
                .format(move |cents| format_money(cents.round() as i64, &unit))
                .h(px((entries.len() as f32 * 28. + 48.).clamp(120., 400.)))
                .w_full()
            });

        let status_chips = div().flex().items_center().gap_1().children(
            [
                (None, "All"),
                (Some("active"), "Active"),
                (Some("trial"), "Trial"),
                (Some("paused"), "Paused"),
                (Some("cancelled"), "Cancelled"),
            ]
            .into_iter()
            .map(|(status, label)| {
                let selected = self.sub_status_filter == status;
                Button::new(SharedString::from(format!("sub-filter-{label}")), label)
                    .size(ControlSize::Sm)
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.variant(ButtonVariant::Ghost))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.sub_status_filter = status;
                        cx.notify();
                    }))
            }),
        );

        let header_stat = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
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
                                    .child("MONTHLY RECURRING BURN"),
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
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("ANNUALIZED"),
                            )
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child(format_money(-annual_burn, "EUR")),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{active_count} active of {} tracked",
                                summaries.len()
                            )),
                    )
                    .child(status_chips),
            );

        let rows: Vec<Row> = summaries
            .iter()
            .filter(|sub| {
                self.sub_status_filter
                    .is_none_or(|status| sub.status == status)
            })
            .map(sub_row)
            .collect();
        let table = DataTable::new("finance-subscriptions-table", sub_columns())
            .label("Recurring subscriptions")
            .rows(Rc::new(rows))
            .density(Density::Comfortable)
            .sorts([SortKey {
                column: "monthly".into(),
                rising: false,
            }])
            .empty("No subscriptions match this filter.");

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
                    .flex()
                    .flex_col()
                    .gap_4()
                    .overflow_y_scroll()
                    .track_scroll(&self.subscriptions_scroll)
                    .vertical_scrollbar(&self.subscriptions_scroll)
                    .pb_6()
                    .children(charts)
                    .child(table),
            )
            .into_any_element()
    }
}
