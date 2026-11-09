//! Detailed multi-day itinerary view with transit connections and scheduled timeline cards.
use super::forms::{FormKind, Remove};
use super::overview::{calendar_agenda, calendar_event_when};
use super::Travel;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::Disableable;
use sjel_gpui_core::{
    record::Link,
    shell::CalendarEntry,
    travel::{Item, Place, Plan, Stage},
};

#[derive(Clone, Debug)]
struct TimelineTransitLeg {
    key: String,
    origin: String,
    destination: String,
    dep_time: Option<String>,
    arr_time: Option<String>,
    duration: Option<String>,
    is_return: bool,
    mode: String,
    status: Option<String>,
    stage_id: Option<String>,
    item_id: Option<String>,
    destination_place: Option<Place>,
}

#[derive(Clone, Debug)]
struct TimelineOfficeDay {
    key: String,
    title: String,
    location: Option<String>,
    when_label: String,
    notes: Option<String>,
    item_id: Option<String>,
    place: Option<Place>,
}

fn parse_transit_title(title: &str) -> Option<(String, String)> {
    if let Some((from, to)) = title.split_once(" → ") {
        Some((from.trim().to_string(), to.trim().to_string()))
    } else if let Some((from, to)) = title.split_once(" -> ") {
        Some((from.trim().to_string(), to.trim().to_string()))
    } else if let Some((from, to)) = title.split_once('→') {
        Some((from.trim().to_string(), to.trim().to_string()))
    } else {
        None
    }
}

fn extract_time_str(s: &str) -> Option<String> {
    let time_part = s.split_once('T').map(|(_, t)| t).unwrap_or(s);
    let trimmed = time_part.trim();
    if trimmed.len() >= 5 && trimmed.chars().nth(2) == Some(':') {
        Some(trimmed[..5].to_string())
    } else {
        None
    }
}

fn compute_duration(start: &str, end: &str) -> Option<String> {
    let parse_mins = |s: &str| -> Option<i64> {
        let time_part = s.split_once('T').map(|(_, t)| t).unwrap_or(s);
        let mut parts = time_part.split(':');
        let h: i64 = parts.next()?.parse().ok()?;
        let m: i64 = parts.next()?.parse().ok()?;
        Some(h * 60 + m)
    };
    let start_m = parse_mins(start)?;
    let end_m = parse_mins(end)?;
    let diff = if end_m >= start_m {
        end_m - start_m
    } else {
        end_m + 24 * 60 - start_m
    };
    if diff <= 0 {
        return None;
    }
    let hours = diff / 60;
    let mins = diff % 60;
    if hours > 0 && mins > 0 {
        Some(format!("{hours}h {mins:02}m"))
    } else if hours > 0 {
        Some(format!("{hours}h"))
    } else {
        Some(format!("{mins}m"))
    }
}

fn is_office_day_event(title: &str, notes: Option<&str>) -> bool {
    let lower_title = title.to_lowercase();
    let lower_notes = notes.unwrap_or("").to_lowercase();
    let has_office_keywords = lower_title.contains("day trip")
        || lower_title.contains("office")
        || lower_title.contains("büro")
        || lower_title.contains("onsite")
        || lower_title.contains("on-site")
        || lower_title.contains("work trip")
        || lower_title.contains("client visit")
        || lower_notes.contains("office")
        || lower_notes.contains("büro");

    parse_transit_title(title).is_none() && has_office_keywords
}

fn find_place_for_name(name: &str, plan: &Plan) -> Option<Place> {
    let lower = name.to_lowercase();
    let clean = lower
        .replace(" hbf", "")
        .replace(" bahnhof", "")
        .replace(" station", "")
        .replace(" airport", "")
        .replace(" flughafen", "")
        .trim()
        .to_string();

    for p in plan.places() {
        let plower = p.name.to_lowercase();
        if lower.contains(&plower)
            || plower.contains(&lower)
            || (!clean.is_empty() && (plower.contains(&clean) || clean.contains(&plower)))
        {
            return Some(p.clone());
        }
    }
    for s in &plan.stages {
        let dest_l = s.destination.name.to_lowercase();
        if lower.contains(&dest_l)
            || dest_l.contains(&lower)
            || (!clean.is_empty() && (dest_l.contains(&clean) || clean.contains(&dest_l)))
        {
            return Some(s.destination.clone());
        }
        let orig_l = s.origin.name.to_lowercase();
        if lower.contains(&orig_l)
            || orig_l.contains(&lower)
            || (!clean.is_empty() && (orig_l.contains(&clean) || clean.contains(&orig_l)))
        {
            return Some(s.origin.clone());
        }
    }
    None
}

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

        let plan_origin_name = plan.origin.name.trim();

        for day_str in filtered_days {
            let day_stages: Vec<(usize, &Stage)> = stages
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

                let linked_events: Vec<(&Link, Option<&CalendarEntry>)> = linked
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

            // Deduplication and extraction into cohesive timeline entries
            let mut consumed_stage_indices = std::collections::HashSet::new();
            let mut consumed_item_ids = std::collections::HashSet::new();
            let mut consumed_cal_ids = std::collections::HashSet::new();

            let mut transit_legs: Vec<TimelineTransitLeg> = Vec::new();
            let mut office_events: Vec<TimelineOfficeDay> = Vec::new();

            // 1. Process linked calendar events (most accurate departure/arrival timestamps)
            for (link, entry) in &day_linked_events {
                let title = link.title.trim();
                if let Some((from, to)) = parse_transit_title(title) {
                    consumed_cal_ids.insert(link.id.clone());

                    let matched_item = day_items.iter().find(|it| {
                        it.title == title || (it.title.contains(&from) && it.title.contains(&to))
                    });
                    if let Some(it) = matched_item {
                        consumed_item_ids.insert(it.id.clone());
                    }

                    let matched_stage = day_stages.iter().find(|(_, s)| {
                        (from.to_lowercase().contains(&s.origin.name.to_lowercase())
                            || s.origin.name.to_lowercase().contains(&from.to_lowercase()))
                            && (to
                                .to_lowercase()
                                .contains(&s.destination.name.to_lowercase())
                                || s.destination
                                    .name
                                    .to_lowercase()
                                    .contains(&to.to_lowercase()))
                    });
                    if let Some((orig_i, _)) = matched_stage {
                        consumed_stage_indices.insert(*orig_i);
                    }

                    let dep_time = entry
                        .and_then(|e| extract_time_str(&e.starts_at))
                        .or_else(|| link.at.as_deref().and_then(extract_time_str));
                    let arr_time = entry.and_then(|e| extract_time_str(&e.ends_at));

                    let duration = match (
                        entry.map(|e| e.starts_at.as_str()),
                        entry.map(|e| e.ends_at.as_str()),
                    ) {
                        (Some(s), Some(e)) => compute_duration(s, e),
                        _ => None,
                    };

                    let is_return = if !plan_origin_name.is_empty() {
                        to.to_lowercase().contains(&plan_origin_name.to_lowercase())
                    } else {
                        transit_legs.iter().any(|prev: &TimelineTransitLeg| {
                            prev.origin.to_lowercase().contains(&to.to_lowercase())
                                || to.to_lowercase().contains(&prev.origin.to_lowercase())
                        })
                    };

                    let destination_place = find_place_for_name(&to, plan);

                    transit_legs.push(TimelineTransitLeg {
                        key: format!("cal-{}", link.id),
                        origin: from,
                        destination: to,
                        dep_time,
                        arr_time,
                        duration,
                        is_return,
                        mode: matched_stage
                            .and_then(|(_, s)| {
                                s.transport_modes.first().map(|m| m.as_str().to_string())
                            })
                            .unwrap_or_else(|| "rail".to_string()),
                        status: matched_stage.map(|(_, s)| s.status.as_str().to_string()),
                        stage_id: matched_stage.map(|(_, s)| s.id.clone()),
                        item_id: matched_item.map(|it| it.id.clone()),
                        destination_place,
                    });
                } else if is_office_day_event(title, entry.and_then(|e| e.notes.as_deref())) {
                    consumed_cal_ids.insert(link.id.clone());
                    let matched_item = day_items.iter().find(|it| it.title == title);
                    if let Some(it) = matched_item {
                        consumed_item_ids.insert(it.id.clone());
                    }
                    let location = link
                        .meta
                        .clone()
                        .or_else(|| entry.and_then(|e| e.location.clone()));
                    let place = location
                        .as_deref()
                        .and_then(|loc| find_place_for_name(loc, plan));
                    let when_label = if entry.map(|e| e.all_day).unwrap_or(true) {
                        "All day".to_string()
                    } else if let Some(e) = entry {
                        calendar_event_when(e)
                    } else {
                        "All day".to_string()
                    };

                    office_events.push(TimelineOfficeDay {
                        key: format!("cal-{}", link.id),
                        title: title.to_string(),
                        location,
                        when_label,
                        notes: entry.and_then(|e| e.notes.clone()),
                        item_id: matched_item.map(|it| it.id.clone()),
                        place,
                    });
                }
            }

            // 2. Process day items not yet consumed
            for item in &day_items {
                if consumed_item_ids.contains(&item.id)
                    || item.item_type == "packing"
                    || item.item_type == "gear"
                {
                    continue;
                }
                let title = item.title.trim();
                if let Some((from, to)) = parse_transit_title(title) {
                    consumed_item_ids.insert(item.id.clone());
                    let matched_stage = day_stages.iter().find(|(_, s)| {
                        (from.to_lowercase().contains(&s.origin.name.to_lowercase())
                            || s.origin.name.to_lowercase().contains(&from.to_lowercase()))
                            && (to
                                .to_lowercase()
                                .contains(&s.destination.name.to_lowercase())
                                || s.destination
                                    .name
                                    .to_lowercase()
                                    .contains(&to.to_lowercase()))
                    });
                    if let Some((orig_i, _)) = matched_stage {
                        consumed_stage_indices.insert(*orig_i);
                    }
                    let is_return = if !plan_origin_name.is_empty() {
                        to.to_lowercase().contains(&plan_origin_name.to_lowercase())
                    } else {
                        transit_legs.iter().any(|prev: &TimelineTransitLeg| {
                            prev.origin.to_lowercase().contains(&to.to_lowercase())
                        })
                    };
                    let destination_place = find_place_for_name(&to, plan);
                    transit_legs.push(TimelineTransitLeg {
                        key: format!("item-{}", item.id),
                        origin: from,
                        destination: to,
                        dep_time: item
                            .payload
                            .get("time")
                            .and_then(serde_json::Value::as_str)
                            .and_then(extract_time_str),
                        arr_time: None,
                        duration: None,
                        is_return,
                        mode: matched_stage
                            .and_then(|(_, s)| {
                                s.transport_modes.first().map(|m| m.as_str().to_string())
                            })
                            .unwrap_or_else(|| "rail".to_string()),
                        status: matched_stage.map(|(_, s)| s.status.as_str().to_string()),
                        stage_id: matched_stage.map(|(_, s)| s.id.clone()),
                        item_id: Some(item.id.clone()),
                        destination_place,
                    });
                } else if item.item_type != "stay"
                    && is_office_day_event(
                        title,
                        item.payload
                            .get("notes")
                            .and_then(serde_json::Value::as_str),
                    )
                {
                    consumed_item_ids.insert(item.id.clone());
                    let location = item
                        .payload
                        .get("address")
                        .or_else(|| item.payload.get("location"))
                        .and_then(serde_json::Value::as_str)
                        .map(|s| s.to_string());
                    let place = location
                        .as_deref()
                        .and_then(|loc| find_place_for_name(loc, plan));
                    office_events.push(TimelineOfficeDay {
                        key: format!("item-{}", item.id),
                        title: title.to_string(),
                        location,
                        when_label: "All day".to_string(),
                        notes: item
                            .payload
                            .get("notes")
                            .and_then(serde_json::Value::as_str)
                            .map(|s| s.to_string()),
                        item_id: Some(item.id.clone()),
                        place,
                    });
                }
            }

            // 3. Process remaining unconsumed stages
            for (orig_i, stage) in &day_stages {
                if consumed_stage_indices.contains(orig_i) {
                    continue;
                }
                let is_return = if !plan_origin_name.is_empty() {
                    stage
                        .destination
                        .name
                        .to_lowercase()
                        .contains(&plan_origin_name.to_lowercase())
                } else {
                    false
                };
                transit_legs.push(TimelineTransitLeg {
                    key: format!("stage-{}", orig_i),
                    origin: stage.origin.name.clone(),
                    destination: stage.destination.name.clone(),
                    dep_time: None,
                    arr_time: None,
                    duration: None,
                    is_return,
                    mode: stage
                        .transport_modes
                        .first()
                        .map_or("rail", |m| m.as_str())
                        .to_string(),
                    status: Some(stage.status.as_str().to_string()),
                    stage_id: Some(stage.id.clone()),
                    item_id: None,
                    destination_place: Some(stage.destination.clone()),
                });
            }

            // 4. Collect remaining accommodations, agenda events, and other bookings
            let day_stays: Vec<&Item> = day_items
                .iter()
                .filter(|it| !consumed_item_ids.contains(&it.id) && it.item_type == "stay")
                .copied()
                .collect();

            let day_other_items: Vec<&Item> = day_items
                .iter()
                .filter(|it| {
                    !consumed_item_ids.contains(&it.id)
                        && it.item_type != "stay"
                        && it.item_type != "packing"
                        && it.item_type != "gear"
                })
                .copied()
                .collect();

            let day_remaining_linked: Vec<(&Link, Option<&CalendarEntry>)> = day_linked_events
                .iter()
                .filter(|(link, _)| !consumed_cal_ids.contains(&link.id))
                .copied()
                .collect();

            let day_remaining_nearby: Vec<&CalendarEntry> = day_nearby_events
                .iter()
                .filter(|e| !consumed_cal_ids.contains(&e.id))
                .copied()
                .collect();

            // Format day counts summary
            let mut summary_parts = Vec::new();
            if !transit_legs.is_empty() {
                summary_parts.push(format!("🚆 {} rail journey(s)", transit_legs.len()));
            }
            if !office_events.is_empty() {
                summary_parts.push("🏢 Office / on-site".to_string());
            }
            if !day_stays.is_empty() {
                summary_parts.push(format!("⌂ {} stay(s)", day_stays.len()));
            }
            let total_other =
                day_remaining_linked.len() + day_remaining_nearby.len() + day_other_items.len();
            if total_other > 0 {
                summary_parts.push(format!("{total_other} scheduled"));
            }
            let counts_summary = if summary_parts.is_empty() {
                "Schedule open".to_string()
            } else {
                summary_parts.join(" · ")
            };

            let formatted_day = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d")
                .ok()
                .map(|d| d.format("%A, %B %-d, %Y").to_string())
                .unwrap_or_else(|| day_str.clone());

            let mut day_card = div()
                .flex()
                .flex_col()
                .gap_3()
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

            // Separate outbound and return rail journeys for chronological day flow
            let outbound_legs: Vec<&TimelineTransitLeg> =
                transit_legs.iter().filter(|l| !l.is_return).collect();
            let return_legs: Vec<&TimelineTransitLeg> =
                transit_legs.iter().filter(|l| l.is_return).collect();

            // Render Outbound Rail Journeys
            for (idx, leg) in outbound_legs.iter().enumerate() {
                day_card = day_card.child(self.render_train_journey_card(leg, day_str, cx));
                if idx + 1 < outbound_legs.len() {
                    let next = outbound_legs[idx + 1];
                    let same_hub = leg.destination == next.origin;
                    day_card = day_card.child(self.render_station_transfer(
                        &leg.destination,
                        &next.origin,
                        same_hub,
                    ));
                } else if !office_events.is_empty() || !day_stays.is_empty() {
                    // Narrative connector between arrival and on-site programme
                    let arr = leg.arr_time.as_deref().unwrap_or("Arrival");
                    day_card = day_card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(3.))
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("↓")
                            .child(format!(
                                "{arr} at {} · On-site programme begins",
                                leg.destination
                            )),
                    );
                }
            }

            // Render Office Day & On-site Programme
            for office in &office_events {
                day_card = day_card.child(self.render_office_day_card(office, cx));
            }

            // Render Stays & Accommodations
            if !day_stays.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Accommodations & Stays"),
                );
                for stay in &day_stays {
                    day_card = day_card.child(self.item_row(stay, cx));
                }
            }

            // Render Other Scheduled Agenda / Meetings
            if !day_remaining_linked.is_empty() || !day_remaining_nearby.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Scheduled Agenda & Meetings"),
                );
                for (link, entry) in &day_remaining_linked {
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
                                            .child("MEETING"),
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
                for event in &day_remaining_nearby {
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

            // Render Return Rail Journeys
            if !return_legs.is_empty() && (!office_events.is_empty() || !outbound_legs.is_empty()) {
                let dep = return_legs
                    .first()
                    .and_then(|l| l.dep_time.as_deref())
                    .unwrap_or("Evening");
                let orig = return_legs
                    .first()
                    .map(|l| l.origin.as_str())
                    .unwrap_or("destination");
                day_card = day_card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(3.))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("↓")
                        .child(format!("{dep} Departure from {orig} · Return journey")),
                );
            }

            for (idx, leg) in return_legs.iter().enumerate() {
                day_card = day_card.child(self.render_train_journey_card(leg, day_str, cx));
                if idx + 1 < return_legs.len() {
                    let next = return_legs[idx + 1];
                    let same_hub = leg.destination == next.origin;
                    day_card = day_card.child(self.render_station_transfer(
                        &leg.destination,
                        &next.origin,
                        same_hub,
                    ));
                }
            }

            // Render Other Remaining Bookings
            if !day_other_items.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Other Bookings & Tickets"),
                );
                for item in &day_other_items {
                    day_card = day_card.child(self.item_row(item, cx));
                }
            }

            page = page.child(day_card);
        }

        if let Some(packing_card) = self.packing_checklist_card(&items, cx) {
            page = page.child(packing_card);
        }

        let undated_stages: Vec<(usize, &Stage)> = stages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.date.is_none())
            .map(|(i, s)| (i, *s))
            .collect();
        let undated_items: Vec<&Item> = items
            .iter()
            .filter(|i| i.day.is_none() && i.item_type != "packing" && i.item_type != "gear")
            .collect();

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
                        .child("Undated Stages & Bookings"),
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

    /// Renders a dedicated train journey card with station track visualization and direct live actions.
    fn render_train_journey_card(
        &self,
        leg: &TimelineTransitLeg,
        day_str: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;

        // If stage or item form is actively open, render that editor inline
        if let Some(stage_id) = &leg.stage_id {
            let editing = FormKind::Stage(Some(stage_id.clone()));
            if let Some(card) = self.form_card(&editing, cx) {
                return card;
            }
        }
        if let Some(item_id) = &leg.item_id {
            let editing = FormKind::Booking(Some(item_id.clone()));
            if let Some(card) = self.form_card(&editing, cx) {
                return card;
            }
        }

        let (mode_icon, mode_label) = match leg.mode.to_lowercase().as_str() {
            "flight" | "fly" | "plane" | "air" => ("✈", "Flight Transit"),
            "bus" => ("🚌", "Bus Transit"),
            "car" | "drive" => ("🚗", "Road Transit"),
            "ferry" | "boat" => ("⛴", "Ferry Transit"),
            _ => ("🚆", "Regional & Intercity Rail"),
        };

        let is_return = leg.is_return;
        let badge_text = if is_return {
            format!("{mode_icon} RETURN {}", leg.mode.to_uppercase())
        } else {
            format!("{mode_icon} OUTBOUND {}", leg.mode.to_uppercase())
        };
        let badge_tint = if is_return {
            palette.green
        } else {
            palette.accent
        };

        let origin_name = leg.origin.clone();
        let dest_name = leg.destination.clone();
        let stage_date = Some(day_str.to_string());

        let mut card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(crate::theme::tint(badge_tint, 0x0a)))
            .border_1()
            .border_color(rgba(crate::theme::tint(badge_tint, 0x30)));

        // Header line
        card = card.child(
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
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(crate::theme::tint(badge_tint, 0x18)))
                                .border_1()
                                .border_color(rgba(crate::theme::tint(badge_tint, 0x35)))
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(badge_tint))
                                .child(badge_text),
                        )
                        .when_some(leg.duration.clone(), |el, dur| {
                            el.child(
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
                                    .child(format!("⏱ {dur}")),
                            )
                        })
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(
                            if is_return {
                                "Return rail journey"
                            } else {
                                "Outbound rail journey"
                            },
                        )),
                )
                .when_some(leg.status.clone(), |el, status| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(status.replace('_', " ")),
                    )
                }),
        );

        // Station Track layout
        let track = div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .px_3()
            .rounded(px(6.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            // Departure station row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(55.))
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(leg.dep_time.clone().unwrap_or_else(|| "--:--".into())),
                    )
                    .child(div().text_xs().text_color(rgb(badge_tint)).child("●"))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(leg.origin.clone()),
                    ),
            )
            // Connecting track row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(55.)))
                    .child(
                        div()
                            .w(px(8.))
                            .flex()
                            .justify_center()
                            .text_color(rgb(palette.border))
                            .child("│"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(mode_icon)
                            .child(
                                leg.duration
                                    .as_ref()
                                    .map(|d| format!("{d} · "))
                                    .unwrap_or_default(),
                            )
                            .child(mode_label),
                    ),
            )
            // Arrival station row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(55.))
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(leg.arr_time.clone().unwrap_or_else(|| "--:--".into())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if is_return { badge_tint } else { palette.green }))
                            .child("●"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(leg.destination.clone()),
                    ),
            );

        card = card.child(track);

        // Action Toolbar
        let mut actions = div().flex().flex_wrap().items_center().gap_2();

        actions = actions.child(
            self.button(gpui::SharedString::from(format!("train-conn-{}", leg.key)))
                .ghost()
                .label("⇄ Check connections & delays")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.find_stage_connection(
                        origin_name.clone(),
                        dest_name.clone(),
                        stage_date.clone(),
                        window,
                        cx,
                    );
                })),
        );

        if let Some(dest_place) = leg.destination_place.clone() {
            let has_coord = dest_place.coordinate().is_some();
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("train-map-{}", leg.key)))
                    .ghost()
                    .label("📍 Station on map")
                    .disabled(!has_coord)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.show_stop(dest_place.clone(), cx);
                    })),
            );
        }

        if let Some(stage_id) = leg.stage_id.clone() {
            let editing = FormKind::Stage(Some(stage_id.clone()));
            actions = actions
                .children(self.form_button(
                    gpui::SharedString::from(format!("train-edit-stage-{}", leg.key)),
                    "Edit stage",
                    editing,
                    cx,
                ))
                .children(self.remove_button(Remove::Stage(stage_id), cx));
        } else if let Some(item_id) = leg.item_id.clone() {
            let editing = FormKind::Booking(Some(item_id.clone()));
            actions = actions
                .children(self.form_button(
                    gpui::SharedString::from(format!("train-edit-item-{}", leg.key)),
                    "Edit",
                    editing,
                    cx,
                ))
                .children(self.remove_button(Remove::Item(item_id), cx));
        }

        card = card.child(actions);
        card.into_any_element()
    }

    /// Renders a dedicated on-site programme / office day card.
    fn render_office_day_card(
        &self,
        office: &TimelineOfficeDay,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;

        if let Some(item_id) = &office.item_id {
            let editing = FormKind::Booking(Some(item_id.clone()));
            if let Some(card) = self.form_card(&editing, cx) {
                return card;
            }
        }

        let mut card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(crate::theme::tint(palette.green, 0x0a)))
            .border_1()
            .border_color(rgba(crate::theme::tint(palette.green, 0x30)));

        let location_label = office.location.as_deref().unwrap_or("On-site location");

        // Header
        card = card.child(
            div().flex().items_center().justify_between().child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py(px(1.5))
                            .rounded(px(4.))
                            .bg(rgba(crate::theme::tint(palette.green, 0x18)))
                            .border_1()
                            .border_color(rgba(crate::theme::tint(palette.green, 0x35)))
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.green))
                            .child(format!("🏢 ONSITE · {}", location_label.to_uppercase())),
                    )
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
                            .child(office.when_label.clone()),
                    ),
            ),
        );

        // Body
        card = card.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(office.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("📍")
                        .child(location_label.to_string()),
                ),
        );

        if let Some(notes) = &office.notes {
            card = card.child(calendar_agenda(notes, palette));
        }

        // Actions
        let mut actions = div().flex().flex_wrap().items_center().gap_2();

        if let Some(place) = office.place.clone() {
            let has_coord = place.coordinate().is_some();
            actions = actions.child(
                self.button(gpui::SharedString::from(format!(
                    "office-map-{}",
                    office.key
                )))
                .ghost()
                .label("📍 Show on map")
                .disabled(!has_coord)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.show_stop(place.clone(), cx);
                })),
            );
        }

        if let Some(item_id) = office.item_id.clone() {
            let editing = FormKind::Booking(Some(item_id.clone()));
            actions = actions
                .children(self.form_button(
                    gpui::SharedString::from(format!("office-edit-{}", office.key)),
                    "Edit",
                    editing,
                    cx,
                ))
                .children(self.remove_button(Remove::Item(item_id), cx));
        }

        card = card.child(actions);
        card.into_any_element()
    }

    /// Renders a station transfer chip.
    fn render_station_transfer(&self, from: &str, to: &str, same_hub: bool) -> AnyElement {
        let palette = self.palette;
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(5.))
            .rounded(px(6.))
            .bg(rgba(if same_hub {
                crate::theme::tint(palette.green, 0x16)
            } else {
                crate::theme::tint(palette.amber, 0x16)
            }))
            .border_1()
            .border_color(rgba(if same_hub {
                crate::theme::tint(palette.green, 0x35)
            } else {
                crate::theme::tint(palette.amber, 0x35)
            }))
            .text_xs()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(if same_hub {
                        palette.green
                    } else {
                        palette.amber
                    }))
                    .child(if same_hub {
                        "✓ Transfer:"
                    } else {
                        "⚠️ Transfer buffer required:"
                    }),
            )
            .child(div().text_color(rgb(palette.text)).child(if same_hub {
                format!("At {from} (seamless station transfer)")
            } else {
                format!("{from} → {to} (station transfer required)")
            }))
            .into_any_element()
    }

    fn packing_checklist_card(&self, items: &[Item], cx: &mut Context<Self>) -> Option<AnyElement> {
        let packing_items: Vec<&Item> = items
            .iter()
            .filter(|it| it.item_type == "packing" || it.item_type == "gear")
            .collect();
        if packing_items.is_empty() {
            return None;
        }

        let palette = self.palette;
        let total = packing_items.len();
        let packed = packing_items.iter().filter(|it| it.is_packed()).count();

        let progress_pill = if packed == total {
            div()
                .px_2()
                .py(px(1.5))
                .rounded(px(4.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .border_1()
                .border_color(rgba(crate::theme::tint(palette.green, 0x35)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child("✓ All packed")
        } else {
            div()
                .px_2()
                .py(px(1.5))
                .rounded(px(4.))
                .bg(rgba(crate::theme::tint(palette.amber, 0x22)))
                .border_1()
                .border_color(rgba(crate::theme::tint(palette.amber, 0x35)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(format!("{packed}/{total} packed"))
        };

        let mut card = div()
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("🎒 Packing & Gear Checklist"),
                    )
                    .child(progress_pill),
            );

        for item in packing_items {
            card = card.child(self.item_row(item, cx));
        }

        Some(card.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transit_titles_correctly() {
        assert_eq!(
            parse_transit_title("Bonn Hbf → Dortmund Hbf"),
            Some(("Bonn Hbf".to_string(), "Dortmund Hbf".to_string()))
        );
        assert_eq!(
            parse_transit_title("Dortmund Hbf -> Bonn Hbf"),
            Some(("Dortmund Hbf".to_string(), "Bonn Hbf".to_string()))
        );
        assert_eq!(parse_transit_title("Dortmund day trip"), None);
    }

    #[test]
    fn computes_durations_accurately() {
        assert_eq!(
            compute_duration("2026-11-10T06:23:00", "2026-11-10T08:30:00"),
            Some("2h 07m".to_string())
        );
        assert_eq!(
            compute_duration("06:23", "08:30"),
            Some("2h 07m".to_string())
        );
        assert_eq!(compute_duration("10:00", "11:00"), Some("1h".to_string()));
        assert_eq!(compute_duration("10:00", "10:45"), Some("45m".to_string()));
    }

    #[test]
    fn classifies_office_day_events() {
        assert!(is_office_day_event("Dortmund day trip", None));
        assert!(is_office_day_event(
            "Executive Workshop",
            Some("Office in Dortmund")
        ));
        assert!(!is_office_day_event("Bonn Hbf → Dortmund Hbf", None));
        assert!(!is_office_day_event("Museum admission", None));
        assert!(!is_office_day_event(
            "Hotel reservation",
            Some("Provider reference ABC123")
        ));
    }
}
