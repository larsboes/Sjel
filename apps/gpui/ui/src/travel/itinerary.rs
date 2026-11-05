//! Detailed multi-day itinerary view with transit connections and scheduled timeline cards.
use super::forms::{FormKind, Remove};
use super::overview::{calendar_agenda, calendar_event_when};
use super::Travel;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::Disableable;
use sjel_gpui_core::{shell::CalendarEntry, travel::Item};

impl Travel {
    pub(super) fn itinerary(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };

        page = page
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(self.form_button("add-stage", "Add stage", FormKind::Stage(None), cx))
                    .children(self.form_button(
                        "add-booking",
                        "Add booking",
                        FormKind::Booking(None),
                        cx,
                    )),
            )
            .children(self.form_card(&FormKind::Stage(None), cx))
            .children(self.form_card(&FormKind::Booking(None), cx));

        let mut stages: Vec<_> = plan.stages.iter().collect();
        stages.sort_by_key(|stage| stage.sequence);

        let slot = self.selected.as_ref().and_then(|id| self.details.get(id));

        let items: Vec<Item> = slot
            .and_then(|s| s.value.as_ref())
            .map(|s| s.details.items.clone())
            .unwrap_or_default();

        let mut days = std::collections::BTreeSet::new();
        for stage in &stages {
            if let Some(date) = &stage.date {
                days.insert(date.clone());
            }
        }
        for item in &items {
            if let Some(day) = &item.day {
                days.insert(day.clone());
            }
        }
        if let Some(slot) = slot {
            for link in &slot.calendar_links {
                if let Some(at) = link.at.as_deref().and_then(|at| at.get(..10)) {
                    if chrono::NaiveDate::parse_from_str(at, "%Y-%m-%d").is_ok() {
                        days.insert(at.to_string());
                    }
                }
            }
            for event in &slot.calendar_events {
                if let Some(day) = event.starts_at.get(..10) {
                    if chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok() {
                        days.insert(day.to_string());
                    }
                }
            }
        }
        if days.is_empty() && !plan.date_start.is_empty() {
            days.insert(plan.date_start.clone());
            if !plan.date_end.is_empty() && plan.date_end != plan.date_start {
                days.insert(plan.date_end.clone());
            }
        }

        if stages.is_empty() && items.is_empty() && days.is_empty() {
            return page
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No travel stages or items saved for this trip."),
                )
                .into_any_element();
        }

        let day_filters = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("filter-day-all")
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .when(self.selected_day_filter.is_none(), |el| {
                        el.bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(self.selected_day_filter.is_some(), |el| {
                        el.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.selected_day_filter = None;
                        cx.notify();
                    }))
                    .child("All days"),
            )
            .children(days.iter().enumerate().map(|(idx, day_str)| {
                let this_day = day_str.clone();
                let active = self.selected_day_filter.as_deref() == Some(day_str.as_str());
                let short_date = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d")
                    .ok()
                    .map(|d| d.format("%b %-d").to_string())
                    .unwrap_or_else(|| day_str.clone());
                div()
                    .id(gpui::SharedString::from(format!("filter-day-{idx}")))
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .when(active, |el| {
                        el.bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(!active, |el| {
                        el.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected_day_filter = Some(this_day.clone());
                        cx.notify();
                    }))
                    .child(format!("Day {} · {}", idx + 1, short_date))
            }));

        page = page.child(day_filters);

        let filtered_days: Vec<&String> = if let Some(target) = self.selected_day_filter.as_ref() {
            days.iter().filter(|d| *d == target).collect()
        } else {
            days.iter().collect()
        };

        for day_str in filtered_days {
            let day_stages: Vec<(usize, &sjel_gpui_core::travel::Stage)> = stages
                .iter()
                .enumerate()
                .filter(|(_, s)| s.date.as_deref() == Some(day_str.as_str()))
                .map(|(i, s)| (i, *s))
                .collect();
            let day_items: Vec<&Item> = items
                .iter()
                .filter(|item| item.day.as_deref() == Some(day_str.as_str()))
                .collect();

            let (day_linked_events, day_nearby_events) = if let Some(slot) = slot {
                let linked: Vec<_> = slot
                    .calendar_links
                    .iter()
                    .filter(|l| l.via == "trip materialization")
                    .collect();
                let linked_ids: std::collections::HashSet<_> =
                    linked.iter().map(|l| l.id.as_str()).collect();

                let linked_events: Vec<(&sjel_gpui_core::record::Link, Option<&CalendarEntry>)> =
                    linked
                        .into_iter()
                        .filter_map(|link| {
                            let entry = slot.calendar_events.iter().find(|e| e.id == link.id);
                            let matches = entry
                                .map(|e| e.starts_at.starts_with(day_str.as_str()))
                                .or_else(|| {
                                    link.at
                                        .as_deref()
                                        .map(|at| at.starts_with(day_str.as_str()))
                                })
                                .unwrap_or(false);
                            if matches {
                                Some((link, entry))
                            } else {
                                None
                            }
                        })
                        .collect();

                let nearby_events: Vec<&CalendarEntry> = slot
                    .calendar_events
                    .iter()
                    .filter(|e| {
                        e.starts_at.starts_with(day_str.as_str())
                            && !linked_ids.contains(e.id.as_str())
                    })
                    .collect();

                (linked_events, nearby_events)
            } else {
                (Vec::new(), Vec::new())
            };

            let mut counts = Vec::new();
            if !day_stages.is_empty() {
                counts.push(format!("{} stage(s)", day_stages.len()));
            }
            if !day_items.is_empty() {
                counts.push(format!("{} booking(s)", day_items.len()));
            }
            let total_cal = day_linked_events.len() + day_nearby_events.len();
            if total_cal > 0 {
                counts.push(format!("{} event(s)", total_cal));
            }
            let counts_summary = if counts.is_empty() {
                "Schedule open".to_string()
            } else {
                counts.join(" · ")
            };

            let formatted_day = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d")
                .ok()
                .map(|d| d.format("%A, %B %-d, %Y").to_string())
                .unwrap_or_else(|| day_str.clone());

            let mut day_card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border));

            day_card = day_card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .pb_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(formatted_day),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(counts_summary),
                    ),
            );

            for (stage_idx, (orig_i, stage)) in day_stages.iter().enumerate() {
                let editing = FormKind::Stage(Some(stage.id.clone()));
                if let Some(card) = self.form_card(&editing, cx) {
                    day_card = day_card.child(card);
                    continue;
                }
                let mode = stage
                    .transport_modes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "transit".into());

                let mode_icon = match mode.to_lowercase().as_str() {
                    "train" | "rail" => "⇄",
                    "flight" | "fly" | "plane" | "air" => "✈",
                    "bus" => "→",
                    "car" | "drive" => "→",
                    "ferry" | "boat" => "≈",
                    _ => "◎",
                };

                let destination = stage.destination.clone();
                let mut stage_row = div().flex().flex_col().gap_1().py_2().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                                .border_1()
                                .border_color(rgba(crate::theme::tint(palette.accent, 0x35)))
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .child(format!("{mode_icon} {}", mode.to_uppercase())),
                        )
                        .child(div().text_sm().font_weight(gpui::FontWeight::MEDIUM).child(
                            format!("{} → {}", stage.origin.name, stage.destination.name),
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(stage.status.replace('_', " ")),
                        ),
                );

                let mut actions = div().flex().flex_wrap().gap_2();
                actions = actions.child(
                    self.button(gpui::SharedString::from(format!("stage-map-{orig_i}")))
                        .ghost()
                        .label("Map")
                        .disabled(destination.coordinate().is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.show_stop(destination.clone(), cx)
                        })),
                );
                if !stage.id.is_empty() {
                    actions = actions
                        .children(self.form_button(
                            gpui::SharedString::from(format!("stage-edit-{orig_i}")),
                            "Edit",
                            editing,
                            cx,
                        ))
                        .children(self.remove_button(Remove::Stage(stage.id.clone()), cx));
                }
                stage_row = stage_row.child(actions);
                day_card = day_card.child(stage_row);

                if stage_idx + 1 < day_stages.len() {
                    let (_, next_stage) = day_stages[stage_idx + 1];
                    let same_stop = stage.destination.name == next_stage.origin.name;
                    day_card = day_card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(rgba(if same_stop {
                                crate::theme::tint(palette.green, 0x16)
                            } else {
                                crate::theme::tint(palette.amber, 0x16)
                            }))
                            .border_1()
                            .border_color(rgba(if same_stop {
                                crate::theme::tint(palette.green, 0x35)
                            } else {
                                crate::theme::tint(palette.amber, 0x35)
                            }))
                            .text_xs()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(if same_stop {
                                        palette.green
                                    } else {
                                        palette.amber
                                    }))
                                    .child(if same_stop {
                                        "✓ Transfer:"
                                    } else {
                                        "⚠️ Transfer buffer required:"
                                    }),
                            )
                            .child(div().text_color(rgb(palette.text)).child(if same_stop {
                                format!("At {} (seamless station transfer)", stage.destination.name)
                            } else {
                                format!(
                                    "{} → {} (station transfer required)",
                                    stage.destination.name, next_stage.origin.name
                                )
                            })),
                    );
                } else if *orig_i + 1 < stages.len() {
                    let next_stage = stages[*orig_i + 1];
                    if next_stage.date.as_deref() != Some(day_str.as_str()) {
                        let next_date_str = next_stage.date.as_deref().unwrap_or("subsequent date");
                        let same_hub = stage.destination.name == next_stage.origin.name;
                        day_card = day_card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py(px(4.))
                                .rounded(px(6.))
                                .bg(rgba(palette.raised))
                                .border_1()
                                .border_color(rgba(palette.border))
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child("☾ Overnight / Layover:"),
                                )
                                .child(div().text_color(rgb(palette.text)).child(if same_hub {
                                    format!(
                                        "Stay at {} until departure on {}",
                                        stage.destination.name, next_date_str
                                    )
                                } else {
                                    format!(
                                        "At {} · transfer to {} before departure on {}",
                                        stage.destination.name,
                                        next_stage.origin.name,
                                        next_date_str
                                    )
                                })),
                        );
                    }
                }
            }

            if !day_items.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Bookings & Accommodations"),
                );
                for item in day_items {
                    day_card = day_card.child(self.item_row(item, cx));
                }
            }

            if !day_linked_events.is_empty() || !day_nearby_events.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child("Calendar & Programme"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} event(s)", total_cal)),
                        ),
                );

                for (link, entry) in day_linked_events {
                    let when = entry
                        .map(calendar_event_when)
                        .or_else(|| {
                            link.at.as_deref().map(|at| {
                                at.split_once('T')
                                    .map(|(d, time)| {
                                        format!("{d} · {}", time.get(..5).unwrap_or(time))
                                    })
                                    .unwrap_or_else(|| at.to_owned())
                            })
                        })
                        .unwrap_or_else(|| "Trip event".into());
                    day_card = day_card.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_2()
                            .border_b_1()
                            .border_color(rgba(crate::theme::tint(palette.border, 0x66)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(1.5))
                                            .rounded(px(4.))
                                            .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                                            .border_1()
                                            .border_color(rgba(crate::theme::tint(
                                                palette.accent,
                                                0x35,
                                            )))
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(palette.accent))
                                            .child("TRIP EVENT"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child(when),
                                    ),
                            )
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
                                        .child(format!("◎ {location}")),
                                )
                            })
                            .when_some(entry.and_then(|e| e.notes.clone()), |row, notes| {
                                row.child(calendar_agenda(&notes, palette))
                            }),
                    );
                }

                for event in day_nearby_events {
                    let when = calendar_event_when(event);
                    day_card = day_card.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_2()
                            .border_b_1()
                            .border_color(rgba(crate::theme::tint(palette.border, 0x66)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(1.5))
                                            .rounded(px(4.))
                                            .bg(rgba(palette.raised))
                                            .border_1()
                                            .border_color(rgba(palette.border))
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child("NEARBY CALENDAR"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child(when),
                                    ),
                            )
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
                                        .child(format!("◎ {location}")),
                                )
                            })
                            .when_some(event.notes.clone(), |row, notes| {
                                row.child(calendar_agenda(&notes, palette))
                            }),
                    );
                }
            }

            page = page.child(day_card);
        }

        let undated_stages: Vec<(usize, &sjel_gpui_core::travel::Stage)> = stages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.date.is_none())
            .map(|(i, s)| (i, *s))
            .collect();
        let undated_items: Vec<&Item> = items.iter().filter(|i| i.day.is_none()).collect();

        if !undated_stages.is_empty() || !undated_items.is_empty() {
            let mut undated_card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Undated Stages & Items"),
                );
            for (orig_i, stage) in undated_stages {
                let editing = FormKind::Stage(Some(stage.id.clone()));
                if let Some(card) = self.form_card(&editing, cx) {
                    undated_card = undated_card.child(card);
                    continue;
                }
                undated_card = undated_card.child(
                    div()
                        .py_1()
                        .child(format!(
                            "{} → {}",
                            stage.origin.name, stage.destination.name
                        ))
                        .children(self.form_button(
                            gpui::SharedString::from(format!("stage-edit-{orig_i}")),
                            "Edit",
                            editing,
                            cx,
                        )),
                );
            }
            for item in undated_items {
                undated_card = undated_card.child(self.item_row(item, cx));
            }
            page = page.child(undated_card);
        }

        page.into_any_element()
    }
}
