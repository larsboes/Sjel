//! Trip overview, readiness check, who-is-around contacts, and timeline.
use super::forms::Remove;
use super::Travel;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context};
use gpui_kit::component::Disableable;
use sjel_gpui_core::{
    shell::CalendarEntry,
    travel::{Item, Place, Plan, Retrospective, TripSection},
};

impl Travel {
    pub(super) fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };
        let items: Vec<Item> = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
            .map(|s| s.details.items.clone())
            .unwrap_or_default();

        page = page.child(self.readiness_strip(plan, &items));
        page = page.child(self.cost_strip(plan));
        if let Some(retro) = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
            .and_then(|s| s.details.retrospective.as_ref())
        {
            page = page.child(self.retrospective_card(retro));
        }
        let has_unlocated = plan.places().any(|p| p.coordinate().is_none());
        page = page
            .child(div().text_sm().child(format!("From {}", plan.origin.name)))
            .child(div().text_sm().child(format!(
                "{} stages · {} locations",
                plan.stages.len(),
                plan.places().count()
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("open-itinerary")
                            .label("Open itinerary")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open_section(TripSection::Itinerary, cx)
                            })),
                    )
                    .child(self.button("open-map").label("View map").on_click(
                        cx.listener(|view, _, _, cx| view.open_section(TripSection::Map, cx)),
                    ))
                    .when(has_unlocated, |bar| {
                        bar.child(
                            self.button("resolve-plan-places")
                                .label(if self.resolving {
                                    "Locating stops…"
                                } else {
                                    "Locate stops"
                                })
                                .disabled(self.resolving)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.resolve_current_plan(false, cx)
                                })),
                        )
                    })
                    .when(!has_unlocated, |bar| {
                        bar.child(
                            self.button("refresh-plan-places")
                                .label(if self.resolving {
                                    "Locating stops…"
                                } else {
                                    "Refresh coordinates"
                                })
                                .disabled(self.resolving)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.resolve_current_plan(true, cx)
                                })),
                        )
                    }),
            );
        page = page.child(self.trip_calendar_timeline());
        page = page.child(self.who_is_around(plan, &items));
        for (i, place) in plan.places().enumerate() {
            page = page.child(self.place_row(i, place, cx));
        }
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(rgba(self.palette.border))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .child("Delete this trip and its saved bookings."),
                )
                .children(self.remove_button(Remove::Plan(plan.id.clone()), cx)),
        );
        page.into_any_element()
    }

    /// People within reach of each dated leg, the hosts among them, plan companions, and the meetups.
    pub(super) fn who_is_around(&self, plan: &Plan, items: &[Item]) -> AnyElement {
        use sjel_gpui_core::travel::around;
        let palette = self.palette;
        let mut any = false;

        let mut content = div().flex().flex_col().gap_3();

        // 1. Companions on the trip
        if !plan.travelers.is_empty() {
            any = true;
            let mut travelers_row = div().flex().flex_wrap().items_center().gap_2();
            travelers_row = travelers_row.child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("Trip Companions:"),
            );
            for traveler in &plan.travelers {
                travelers_row = travelers_row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(6.))
                        .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                        .border_1()
                        .border_color(rgba(crate::theme::tint(palette.accent, 0x35)))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(palette.accent))
                        .child(format!("👤 {traveler}")),
                );
            }
            content = content.child(travelers_row);
        }

        // 2. People near each stage stop
        let mut stages: Vec<_> = plan.stages.iter().collect();
        stages.sort_by_key(|stage| stage.sequence);

        let mut nearby_stops = div().flex().flex_col().gap_2();
        let mut has_nearby = false;

        for stage in stages {
            let Some(Some(located)) = stage.date.as_ref().and_then(|day| self.located.get(day))
            else {
                continue;
            };
            let near = around::around(stage, located);
            if near.is_empty() {
                continue;
            }
            any = true;
            has_nearby = true;

            let stop_header = format!(
                "{} · {}",
                stage.date.as_deref().unwrap_or("Undated"),
                stage.destination.name
            );

            let mut people_grid = div().flex().flex_wrap().gap_2();
            for p in &near {
                let mut card = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.surface))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .min_w(px(140.));

                let header_line = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(p.person.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{:.0} km", p.distance_km)),
                    );
                card = card.child(header_line);

                if p.host {
                    let mut host_badge = "🛏 Can host".to_string();
                    if let Some(note) = p.note.as_deref().filter(|n| !n.is_empty()) {
                        host_badge = format!("🛏 {note}");
                    }
                    card = card.child(
                        div()
                            .px(px(4.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.green))
                            .child(host_badge),
                    );
                } else if !p.place.is_empty() && p.place != stage.destination.name {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("in {}", p.place)),
                    );
                }
                people_grid = people_grid.child(card);
            }

            nearby_stops = nearby_stops.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.muted))
                            .child(stop_header),
                    )
                    .child(people_grid),
            );
        }

        if has_nearby {
            content = content.child(nearby_stops);
        }

        // 3. Meetups on the plan
        let meetups = around::meetups(items);
        if !meetups.is_empty() {
            any = true;
            let mut meetups_col = div().flex().flex_col().gap_1();
            meetups_col = meetups_col.child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("Trip Meetups:"),
            );

            let mut meetups_row = div().flex().flex_wrap().gap_2();
            for meetup in &meetups {
                let status_color = match meetup.status.to_lowercase().as_str() {
                    "confirmed" => palette.green,
                    "planned" => palette.accent,
                    _ => palette.muted,
                };

                let mut details_str = format!("with {}", meetup.people.join(", "));
                if let Some(day) = &meetup.day {
                    details_str.push_str(&format!(" · {day}"));
                }
                if let Some(place) = &meetup.place {
                    details_str.push_str(&format!(" · {place}"));
                }

                meetups_row = meetups_row.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded(px(6.))
                        .bg(rgba(palette.surface))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(format!("☕ {}", meetup.title)),
                                )
                                .child(
                                    div()
                                        .px(px(4.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(rgba(crate::theme::tint(status_color, 0x22)))
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(rgb(status_color))
                                        .child(meetup.status.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(details_str),
                        ),
                );
            }
            meetups_col = meetups_col.child(meetups_row);
            content = content.child(meetups_col);
        }

        if !any {
            return div().into_any_element();
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("COMPANIONS & WHO IS AROUND"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Radius: {:.0} km", around::AROUND_RADIUS_KM)),
                    ),
            )
            .child(content)
            .into_any_element()
    }

    pub(super) fn readiness_strip(&self, plan: &Plan, items: &[Item]) -> AnyElement {
        let palette = self.palette;
        let start_date = chrono::NaiveDate::parse_from_str(&plan.date_start, "%Y-%m-%d").ok();
        let end_date = chrono::NaiveDate::parse_from_str(&plan.date_end, "%Y-%m-%d").ok();
        let total_nights = match (start_date, end_date) {
            (Some(s), Some(e)) => (e - s).num_days().max(0) as usize,
            _ => 0,
        };
        let lodging_count = items
            .iter()
            .filter(|it| {
                let ty = it.item_type.to_lowercase();
                ty.contains("lodging") || ty.contains("stay") || ty.contains("hotel")
            })
            .count();
        let unbooked_stages = plan
            .stages
            .iter()
            .filter(|s| s.status.as_str() != "booked")
            .count();
        let missing_codes = items
            .iter()
            .filter(|it| it.booking_code().is_none())
            .count();

        let accom_pill = if total_nights == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Day trip")
        } else if lodging_count >= total_nights {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ {lodging_count}/{total_nights} nights covered"))
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.amber, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(format!("⚠ {lodging_count}/{total_nights} nights covered"))
        };

        let stages_pill = if plan.stages.is_empty() {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("No stages")
        } else if unbooked_stages == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ All {} stages booked", plan.stages.len()))
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.amber, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(format!("⚠ {unbooked_stages} unbooked stage(s)"))
        };

        let codes_pill = if items.is_empty() {
            div().into_any_element()
        } else if missing_codes == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ All {} booking codes", items.len()))
                .into_any_element()
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{}/{} booking codes",
                    items.len() - missing_codes,
                    items.len()
                ))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("TRIP READINESS & GAPS"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{} – {}", plan.date_start, plan.date_end)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(accom_pill)
                    .child(stages_pill)
                    .child(codes_pill),
            )
            .into_any_element()
    }

    fn cost_strip(&self, plan: &Plan) -> AnyElement {
        let palette = self.palette;
        let slot = self.details.get(&plan.id);
        let mut card = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(crate::theme::tint(palette.surface, 0x55)))
            .border_1()
            .border_color(rgba(crate::theme::tint(palette.border, 0x44)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Trip cost & spend"),
                    )
                    .when_some(
                        slot.and_then(|s| s.cost.as_ref())
                            .and_then(|c| c.actuals.posting_count)
                            .filter(|&c| c > 0),
                        |header, count| {
                            header.child(
                                div()
                                    .px_2()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                                    .text_xs()
                                    .text_color(rgb(palette.accent))
                                    .child(format!("{count} ledger postings")),
                            )
                        },
                    ),
            );

        if let Some(cost) = slot.and_then(|slot| slot.cost.as_ref()) {
            let mut values = div().flex().flex_wrap().gap_4();
            for (label, amount) in cost_rows(cost) {
                values = values.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
                        .child(div().text_sm().child(amount)),
                );
            }
            card = card.child(values);

            if cost.by_currency.len() > 1 {
                let mut curr_row = div().flex().flex_wrap().items_center().gap_2();
                curr_row = curr_row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("By currency:"),
                );
                for c in &cost.by_currency {
                    curr_row = curr_row.child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgba(crate::theme::tint(palette.surface, 0x77)))
                            .text_xs()
                            .child(format!(
                                "{}: {} ({} {})",
                                c.currency,
                                cost_amount(Some(c.booked_cents), Some(&c.currency)),
                                c.item_count,
                                if c.item_count == 1 {
                                    "booking"
                                } else {
                                    "bookings"
                                }
                            )),
                    );
                }
                card = card.child(curr_row);
            }

            if !cost.by_stage.is_empty() {
                let mut stages_container = div()
                    .flex()
                    .flex_col()
                    .gap_1_5()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgba(crate::theme::tint(palette.border, 0x33)))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.muted))
                            .child("Spend by stage"),
                    );

                let mut stages_list = div().flex().flex_col().gap_1();
                for stage in &cost.by_stage {
                    let amount_str = cost_amount(stage.booked_cents, stage.currency.as_deref());
                    stages_list = stages_list.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .py(px(2.))
                            .px_2()
                            .rounded(px(4.))
                            .bg(rgba(crate::theme::tint(palette.surface, 0x44)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(format!(
                                                "{} → {}",
                                                stage.origin, stage.destination
                                            )),
                                    )
                                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                        format!(
                                            "({} {})",
                                            stage.item_count,
                                            if stage.item_count == 1 {
                                                "item"
                                            } else {
                                                "items"
                                            }
                                        ),
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(amount_str),
                            ),
                    );
                }
                stages_container = stages_container.child(stages_list);
                card = card.child(stages_container);
            }

            for reason in [cost.booked_reason.as_ref(), cost.actuals.reason.as_ref()]
                .into_iter()
                .flatten()
            {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(reason.clone()),
                );
            }
        } else {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Cost figures are unavailable until the Mac responds."),
            );
        }
        if let Some(error) = slot.and_then(|slot| slot.cost_error.as_ref()) {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        card.into_any_element()
    }

    fn retrospective_card(&self, retro: &Retrospective) -> AnyElement {
        let palette = self.palette;
        let again_pill = match retro.again.to_lowercase().as_str() {
            "yes" | "definitely" => div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child("Repeat: Yes"),
            "no" | "never" => div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.red, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.red))
                .child("Repeat: No"),
            _ => div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("Repeat: {}", retro.again)),
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("TRIP RETROSPECTIVE & CLOSE-OUT"),
                    )
                    .child(
                        div().text_xs().text_color(rgb(palette.muted)).child(
                            retro
                                .filled_at
                                .get(..10)
                                .unwrap_or(&retro.filled_at)
                                .to_string(),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(again_pill)
                    .when_some(retro.cost_cents, |row, cost| {
                        row.child(
                            div()
                                .px_2()
                                .py(px(2.))
                                .rounded(px(6.))
                                .bg(rgba(palette.raised))
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(format!(
                                    "Final cost: {}",
                                    cost_amount(Some(cost), retro.currency.as_deref())
                                )),
                        )
                    }),
            )
            .when(!retro.change_note.trim().is_empty(), |card| {
                card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("Notes: {}", retro.change_note)),
                )
            })
            .into_any_element()
    }

    pub(super) fn trip_calendar_timeline(&self) -> AnyElement {
        let palette = self.palette;
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Trip timeline"),
            );
        let Some(id) = self.selected.as_ref() else {
            return section.into_any_element();
        };
        let Some(slot) = self.details.get(id) else {
            return section
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Loading trip events…"),
                )
                .into_any_element();
        };
        if !slot.calendar_links_loaded
            || slot.calendar_links_loading
            || !slot.calendar_events_loaded
            || slot.calendar_events_loading
        {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Loading trip events…"),
            );
        } else if let Some(error) = &slot.calendar_links_error {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.amber))
                    .child(format!("Trip events could not load: {error}")),
            );
        } else if let Some(error) = &slot.calendar_events_error {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.amber))
                    .child(format!("Nearby Calendar events could not load: {error}")),
            );
        } else {
            let linked: Vec<_> = slot
                .calendar_links
                .iter()
                .filter(|link| link.via == "trip materialization")
                .collect();
            let linked_ids: std::collections::HashSet<_> =
                linked.iter().map(|link| link.id.as_str()).collect();
            let nearby: Vec<_> = slot
                .calendar_events
                .iter()
                .filter(|event| !linked_ids.contains(event.id.as_str()))
                .collect();
            if linked.is_empty() && nearby.is_empty() {
                section = section.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No mail or Calendar events fall in this trip window."),
                );
            } else {
                for event in linked {
                    let entry = slot
                        .calendar_events
                        .iter()
                        .find(|entry| entry.id == event.id);
                    section = section.child(calendar_link_row(event, entry, palette));
                }
                if !nearby.is_empty() {
                    section = section
                        .child(
                            div()
                                .pt_2()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Nearby Calendar events · not linked to this trip"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Calendar grouped these by destination and travel window. Add one to the itinerary to link it."),
                        );
                    for event in nearby {
                        section = section.child(calendar_event_row(event, palette));
                    }
                }
            }
        }
        section.into_any_element()
    }

    pub(super) fn place_row(
        &self,
        index: usize,
        place: &Place,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let place = place.clone();
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(rgba(self.palette.border))
            .child(div().text_sm().child(place.name.clone()))
            .when_some(place.address.clone(), |row, address| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .child(address),
                )
            });
        if place.coordinate().is_some() {
            row = row.child(
                self.button(gpui::SharedString::from(format!("place-{index}")))
                    .label("Show on map")
                    .on_click(cx.listener(move |view, _, _, cx| view.show_stop(place.clone(), cx))),
            );
        } else {
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child("Coordinates not available"),
                    )
                    .child(
                        self.button(gpui::SharedString::from(format!("place-locate-{index}")))
                            .label(if self.resolving {
                                "Locating…"
                            } else {
                                "Locate"
                            })
                            .disabled(self.resolving)
                            .on_click(
                                cx.listener(|view, _, _, cx| view.resolve_current_plan(false, cx)),
                            ),
                    ),
            );
        }
        row.into_any_element()
    }
}

pub(super) fn calendar_event_row(event: &CalendarEntry, palette: crate::Palette) -> AnyElement {
    let when = calendar_event_when(event);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .border_t_1()
        .border_color(rgba(palette.border))
        .child(div().text_xs().text_color(rgb(palette.muted)).child(when))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(event.title.clone()),
        )
        .when_some(event.location.clone(), |row, location| {
            row.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(location),
            )
        })
        .when_some(event.notes.clone(), |row, notes| {
            row.child(calendar_agenda(&notes, palette))
        })
        .into_any_element()
}

pub(super) fn calendar_agenda(notes: &str, palette: crate::Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(crate::doc_view::render_markdown(
            notes,
            palette,
            crate::Density::Desktop,
        ))
        .into_any_element()
}

pub(super) fn calendar_event_when(event: &CalendarEntry) -> String {
    if event.all_day {
        format!(
            "{} · all day",
            event.starts_at.get(..10).unwrap_or(&event.starts_at)
        )
    } else if let Some((day, time)) = event.starts_at.split_once('T') {
        let start = time.get(..5).unwrap_or(time);
        let end = event
            .ends_at
            .split_once('T')
            .map(|(_, time)| time.get(..5).unwrap_or(time))
            .unwrap_or("");
        if end.is_empty() {
            format!("{day} · {start}")
        } else {
            let end_day = event.ends_at.get(..10).unwrap_or(day);
            let end = if end_day == day {
                end.to_owned()
            } else {
                format!("{end_day} {end}")
            };
            format!("{day} · {start}–{end}")
        }
    } else {
        event.starts_at.clone()
    }
}

pub(super) fn calendar_link_row(
    link: &sjel_gpui_core::record::Link,
    entry: Option<&CalendarEntry>,
    palette: crate::Palette,
) -> AnyElement {
    let when = entry
        .map(calendar_event_when)
        .or_else(|| {
            link.at.as_deref().map(|at| {
                at.split_once('T')
                    .map(|(day, time)| format!("{day} · {}", time.get(..5).unwrap_or(time)))
                    .unwrap_or_else(|| at.to_owned())
            })
        })
        .unwrap_or_else(|| "Date unavailable".into());
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .border_t_1()
        .border_color(rgba(palette.border))
        .child(div().text_xs().text_color(rgb(palette.muted)).child(when))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(link.title.clone()),
        )
        .when_some(link.meta.clone(), |row, location| {
            row.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(location),
            )
        })
        .when_some(entry.and_then(|entry| entry.notes.clone()), |row, notes| {
            row.child(calendar_agenda(&notes, palette))
        })
        .into_any_element()
}

fn cost_amount(cents: Option<i64>, currency: Option<&str>) -> String {
    cents
        .zip(currency)
        .and_then(|(cents, currency)| {
            sjel_gpui_core::finance::format_exact_money(&cents.to_string(), 2, currency)
        })
        .unwrap_or_else(|| "—".into())
}

fn cost_rows(cost: &sjel_gpui_core::travel::CostRollup) -> [(&'static str, String); 6] {
    let unit = cost.currency.as_deref();
    let actual_unit = cost.actuals.currency.as_deref();
    [
        ("Planned", cost_amount(cost.planned_cents, unit)),
        ("Committed", cost_amount(cost.booked_cents, unit)),
        (
            "Personal",
            cost_amount(cost.actuals.personal_cents, actual_unit),
        ),
        (
            "Cash outflow",
            cost_amount(cost.actuals.gross_cash_outflow_cents, actual_unit),
        ),
        (
            "Reimbursed",
            cost_amount(cost.actuals.reimbursed_cents, actual_unit),
        ),
        (
            "Outstanding",
            cost_amount(cost.actuals.outstanding_cents, actual_unit),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_and_negative_money_are_not_relabelled_or_zeroed() {
        assert_eq!(cost_amount(None, Some("EUR")), "—");
        assert_eq!(cost_amount(Some(100), None), "—");
        assert!(cost_amount(Some(-50), Some("USD")).contains("-0,50"));
        assert!(cost_amount(Some(i64::MIN), Some("EUR")).starts_with('-'));
    }

    #[test]
    fn cost_rows_keep_actuals_in_the_finance_currency() {
        let cost = serde_json::from_value(serde_json::json!({
            "plan_id": "trip", "currency": "EUR", "planned_cents": 10000,
            "booked_cents": null, "booked_reason": "Mixed currencies",
            "actuals": {"ok": true, "reason": null, "currency": "USD",
                "personal_cents": 5000, "gross_cash_outflow_cents": 7000,
                "reimbursed_cents": 1000, "outstanding_cents": 1000, "posting_count": 2},
            "selected_options": {"total": 42.0, "currency": null, "priced_items": 1, "note": "offered, not paid"},
            "by_currency": [], "by_stage": [],
            "unattributed": {"booked_cents": null, "currency": null, "reason": null, "item_count": 0},
            "sources": []
        })).unwrap();
        let rows = cost_rows(&cost);
        assert_eq!(rows[1], ("Committed", "—".into()));
        assert!(rows[0].1.contains("EUR"));
        for row in &rows[2..] {
            assert!(row.1.contains("USD"));
            assert!(!row.1.contains("EUR"));
        }
    }

    #[test]
    fn who_is_around_renders_companions_and_meetups() {
        let plan = Plan {
            id: "trip-1".into(),
            title: "Berlin Tour".into(),
            origin: Place {
                name: "Bonn".into(),
                ..Default::default()
            },
            destinations: vec![Place {
                name: "Berlin".into(),
                ..Default::default()
            }],
            travelers: vec!["Ada".into(), "Linus".into()],
            ..Default::default()
        };
        let items = vec![Item {
            id: "act-1".into(),
            item_type: "activity".into(),
            title: "Coffee Chat".into(),
            day: Some("2026-11-10".into()),
            payload: serde_json::json!({
                "with": [{"person": "Ron"}],
                "status": "confirmed",
                "place": "Café Einstein"
            }),
            ..Default::default()
        }];
        let meetups = sjel_gpui_core::travel::around::meetups(&items);
        assert_eq!(meetups.len(), 1);
        assert_eq!(meetups[0].title, "Coffee Chat");
        assert_eq!(meetups[0].people, vec!["Ron"]);
        assert_eq!(meetups[0].status, "confirmed");
        assert_eq!(plan.travelers, vec!["Ada", "Linus"]);
    }

    #[test]
    fn cost_rollup_supports_by_stage_and_by_currency() {
        let cost: sjel_gpui_core::travel::CostRollup = serde_json::from_value(serde_json::json!({
            "plan_id": "trip-2", "currency": null, "planned_cents": 25000,
            "booked_cents": null, "booked_reason": "Multiple currencies",
            "actuals": {"ok": true, "reason": null, "currency": "EUR",
                "personal_cents": 12000, "gross_cash_outflow_cents": 15000,
                "reimbursed_cents": 3000, "outstanding_cents": 0, "posting_count": 4},
            "selected_options": {"total": 50.0, "currency": null, "priced_items": 1, "note": "options"},
            "by_currency": [
                {"currency": "EUR", "booked_cents": 12000, "item_count": 2},
                {"currency": "GBP", "booked_cents": 8500, "item_count": 1}
            ],
            "by_stage": [
                {"stage_id": "s1", "sequence": 1, "origin": "Bonn", "destination": "Berlin", "booked_cents": 12000, "currency": "EUR", "reason": null, "item_count": 2},
                {"stage_id": "s2", "sequence": 2, "origin": "Berlin", "destination": "London", "booked_cents": 8500, "currency": "GBP", "reason": null, "item_count": 1}
            ],
            "unattributed": {"booked_cents": null, "currency": null, "reason": null, "item_count": 0},
            "sources": []
        })).unwrap();

        assert_eq!(cost.by_currency.len(), 2);
        assert_eq!(cost.by_stage.len(), 2);
        assert_eq!(cost.actuals.posting_count, Some(4));
        assert_eq!(cost.by_stage[0].origin, "Bonn");
        assert_eq!(cost.by_stage[1].destination, "London");
    }
}
