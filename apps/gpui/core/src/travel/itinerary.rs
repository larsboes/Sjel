//! Shared itinerary title, timing and place interpretation.
use super::{Place, Plan};

pub fn parse_transit_title(title: &str) -> Option<(String, String)> {
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

pub fn extract_time_str(s: &str) -> Option<String> {
    let time_part = s.split_once('T').map(|(_, t)| t).unwrap_or(s);
    let trimmed = time_part.trim();
    if trimmed.len() >= 5 && trimmed.chars().nth(2) == Some(':') {
        Some(trimmed[..5].to_string())
    } else {
        None
    }
}

pub fn compute_duration(start: &str, end: &str) -> Option<String> {
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

pub fn is_office_day_event(title: &str, notes: Option<&str>) -> bool {
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

pub fn find_place_for_name(name: &str, plan: &Plan) -> Option<Place> {
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
