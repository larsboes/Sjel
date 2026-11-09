//! Location markers and categories for travel maps.
use std::collections::HashMap;

use super::{Coordinate, Place, Plan, SavedDetails};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceCategory {
    Stay,
    Station,
    Airport,
    Place,
}

impl PlaceCategory {
    pub fn from_name_and_type(name: &str, item_type: Option<&str>) -> Self {
        if item_type == Some("stay") || item_type == Some("accommodation") {
            return Self::Stay;
        }
        let lower = name.to_lowercase();
        if lower.contains("airport")
            || lower.contains("flughafen")
            || lower.contains("aéroport")
            || lower.contains("aeropuerto")
            || lower.contains("airfield")
        {
            Self::Airport
        } else if lower.contains("hbf")
            || lower.contains("bahnhof")
            || lower.contains("station")
            || lower.contains("gare ")
            || lower.ends_with("gare")
            || lower.contains("estación")
            || lower.contains("terminal")
        {
            Self::Station
        } else if lower.contains("hotel") || lower.contains("hostel") || lower.contains("airbnb") {
            Self::Stay
        } else {
            Self::Place
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Stay => "⌂",
            Self::Station => "⇄",
            Self::Airport => "✈",
            Self::Place => "◎",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Stay => "Stay",
            Self::Station => "Station",
            Self::Airport => "Airport",
            Self::Place => "Place",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocationMarker {
    pub plan_id: String,
    pub place: Place,
    pub from: String,
    pub to: String,
    pub label: String,
    pub category: PlaceCategory,
}

pub fn build_location_markers(
    plans: &[Plan],
    details: &HashMap<String, &SavedDetails>,
    selected_category_filter: Option<PlaceCategory>,
) -> Vec<LocationMarker> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let horizon = (chrono::Local::now().date_naive() + chrono::Duration::days(180))
        .format("%Y-%m-%d")
        .to_string();
    let mut markers = Vec::new();
    for plan in plans
        .iter()
        .filter(|plan| plan.date_end >= today && plan.date_start <= horizon)
        .take(8)
    {
        let from = plan.date_start.clone();
        let to = plan.date_end.clone();
        for place in plan.places() {
            markers.push(LocationMarker {
                plan_id: plan.id.clone(),
                place: place.clone(),
                from: from.clone(),
                to: to.clone(),
                label: format!("{} · {}–{}", plan.title, from, to),
                category: PlaceCategory::from_name_and_type(&place.name, None),
            });
        }
        if let Some(details) = details.get(&plan.id) {
            for item in &details.details.items {
                if item.item_type != "stay" {
                    continue;
                }
                let Some(latitude) = item
                    .payload
                    .get("latitude")
                    .and_then(serde_json::Value::as_f64)
                else {
                    continue;
                };
                let Some(longitude) = item
                    .payload
                    .get("longitude")
                    .and_then(serde_json::Value::as_f64)
                else {
                    continue;
                };
                let at = Coordinate {
                    latitude,
                    longitude,
                };
                let Some(day) = item
                    .payload
                    .get("check_in")
                    .and_then(serde_json::Value::as_str)
                    .or(item.day.as_deref())
                else {
                    continue;
                };
                let to = item
                    .payload
                    .get("check_out")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(day)
                    .to_owned();
                let place = Place {
                    name: item.title.clone(),
                    address: item
                        .payload
                        .get("address")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    latitude: Some(at.latitude),
                    longitude: Some(at.longitude),
                    ..Default::default()
                };
                markers.push(LocationMarker {
                    plan_id: plan.id.clone(),
                    place,
                    from: day.to_owned(),
                    to,
                    label: format!("{} · stay", plan.title),
                    category: PlaceCategory::Stay,
                });
            }
        }
    }
    if let Some(cat) = selected_category_filter {
        markers.retain(|m| m.category == cat);
    }
    markers
}

pub fn build_current_plan_markers(
    plan: &Plan,
    details: Option<&SavedDetails>,
    selected_category_filter: Option<PlaceCategory>,
) -> Vec<LocationMarker> {
    let mut markers = Vec::new();
    for place in plan.places() {
        markers.push(LocationMarker {
            plan_id: plan.id.clone(),
            place: place.clone(),
            from: plan.date_start.clone(),
            to: plan.date_end.clone(),
            label: plan.title.clone(),
            category: PlaceCategory::from_name_and_type(&place.name, None),
        });
    }
    if let Some(details) = details {
        for item in &details.details.items {
            if item.item_type != "stay" {
                continue;
            }
            let Some(latitude) = item
                .payload
                .get("latitude")
                .and_then(serde_json::Value::as_f64)
            else {
                continue;
            };
            let Some(longitude) = item
                .payload
                .get("longitude")
                .and_then(serde_json::Value::as_f64)
            else {
                continue;
            };
            let day = item
                .payload
                .get("check_in")
                .and_then(serde_json::Value::as_str)
                .or(item.day.as_deref())
                .unwrap_or(&plan.date_start);
            let to = item
                .payload
                .get("check_out")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(day);
            markers.push(LocationMarker {
                plan_id: plan.id.clone(),
                place: Place {
                    name: item.title.clone(),
                    address: item
                        .payload
                        .get("address")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    latitude: Some(latitude),
                    longitude: Some(longitude),
                    ..Default::default()
                },
                from: day.to_owned(),
                to: to.to_owned(),
                label: format!("{} · stay", plan.title),
                category: PlaceCategory::Stay,
            });
        }
    }
    if let Some(cat) = selected_category_filter {
        markers.retain(|m| m.category == cat);
    }
    markers
}
