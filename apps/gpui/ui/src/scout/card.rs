//! Opportunity cards, calendar status badges, and action rows.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::scouting::{can_promote_to_calendar, OpportunityStatus, ScoutingOpportunity};

use crate::theme::Palette;

use super::types::{calendar_verdict_text, muted, rationale_text, type_label};
use super::ScoutingPage;

impl ScoutingPage {
    pub(super) fn render_opportunity_card(
        &self,
        opportunity: &ScoutingOpportunity,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = opportunity.id.clone();
        let status = opportunity.status;
        let busy = self.busy.as_deref() == Some(id.as_str());
        let route = opportunity.event_route.as_ref();
        let calendar_verdict = self.calendar_verdicts.get(&id);
        let already_in_calendar =
            calendar_verdict.is_some_and(|verdict| verdict.already_in_calendar);
        let summary = rationale_text(&opportunity.rationale);
        let can_calendar = can_promote_to_calendar(opportunity);
        let original_url = opportunity.url.clone();
        let promote = opportunity.clone();
        let save_id = id.clone();
        let dismiss_id = id.clone();

        div()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(if status == OpportunityStatus::Dismissed {
                rgba(palette.surface)
            } else {
                rgba(palette.raised)
            })
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "{} · {}{}",
                                        type_label(&opportunity.opportunity_type),
                                        opportunity.source,
                                        route.map_or(String::new(), |route| {
                                            format!(" · {}", route.route.label())
                                        })
                                    )),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(opportunity.title.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if opportunity.score >= 0.35 {
                                palette.green
                            } else {
                                palette.muted
                            }))
                            .child(format!("Match {:.0}", opportunity.score * 100.)),
                    ),
            )
            .when(!summary.is_empty(), |card| {
                card.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(summary),
                )
            })
            .when_some(
                route.filter(|route| !route.reason.trim().is_empty()),
                |card, route| card.child(muted(route.reason.clone(), palette)),
            )
            .child(muted(
                [
                    (!opportunity.starts_at.is_empty())
                        .then_some(opportunity.starts_at.get(..10).unwrap_or_default()),
                    (!opportunity.city.is_empty()).then_some(opportunity.city.as_str()),
                    (!opportunity.matched_focus.is_empty())
                        .then_some(opportunity.matched_focus.as_str()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                palette,
            ))
            .when_some(calendar_verdict, |card, verdict| {
                card.child(muted(calendar_verdict_text(verdict), palette))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("scout-original-{id}")))
                            .small()
                            .ghost()
                            .label("Open original ↗")
                            .on_click(move |_, _, cx| cx.open_url(&original_url)),
                    )
                    .when(can_calendar, |row| {
                        row.child(
                            Button::new(SharedString::from(format!("scout-calendar-{id}")))
                                .small()
                                .ghost()
                                .label(if already_in_calendar {
                                    "Already in Calendar"
                                } else {
                                    "Calendar…"
                                })
                                .disabled(busy || already_in_calendar)
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.open_promotion(promote.clone(), window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("scout-save-{id}")))
                            .small()
                            .when(status == OpportunityStatus::Saved, |button| {
                                button.primary()
                            })
                            .when(status != OpportunityStatus::Saved, |button| button.ghost())
                            .label(if busy {
                                "Saving…"
                            } else if status == OpportunityStatus::Saved {
                                "Saved · Restore"
                            } else {
                                "Save"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                let next = if status == OpportunityStatus::Saved {
                                    OpportunityStatus::New
                                } else {
                                    OpportunityStatus::Saved
                                };
                                page.set_status(save_id.clone(), next, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("scout-dismiss-{id}")))
                            .small()
                            .ghost()
                            .label(if status == OpportunityStatus::Dismissed {
                                "Restore"
                            } else {
                                "Dismiss"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                let next = if status == OpportunityStatus::Dismissed {
                                    OpportunityStatus::New
                                } else {
                                    OpportunityStatus::Dismissed
                                };
                                page.set_status(dismiss_id.clone(), next, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
