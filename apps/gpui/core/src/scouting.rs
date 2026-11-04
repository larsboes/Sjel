//! Shared contracts and rules for the Scouting capability's Discover workflow.

use chrono::{Duration, NaiveDate, NaiveTime};
pub use scouting_api::*;

/// Whether a stored event has enough routing and date evidence to offer calendar promotion.
pub fn can_promote_to_calendar(opportunity: &ScoutingOpportunity) -> bool {
    opportunity.opportunity_type == "event"
        && opportunity
            .event_route
            .as_ref()
            .is_some_and(|route| route.route.can_add_to_calendar())
        && parse_instant(&opportunity.starts_at).is_some()
}

/// The default event body used by the review form before the operator confirms it.
///
/// The `(source, external_id)` pair makes repeated promotion an update in Calendar, while the
/// payload keeps the scouting result's provenance available to the Calendar detail view.
pub fn calendar_entry_body(
    opportunity: &ScoutingOpportunity,
    title: &str,
    starts_at: &str,
    ends_at: &str,
    location: &str,
) -> Result<serde_json::Value, &'static str> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Give the calendar entry a title.");
    }
    let start = parse_instant(starts_at).ok_or("Use a date or local date and time.")?;
    let end = parse_instant(ends_at).ok_or("Use a date or local date and time for the end.")?;
    let (start_value, end_value, all_day) = match (start.time, end.time) {
        (None, None) => {
            if end.date <= start.date {
                return Err("The end date must be after the start date.");
            }
            (
                start.date.format("%Y-%m-%d").to_string(),
                end.date.format("%Y-%m-%d").to_string(),
                true,
            )
        }
        (Some(start_time), Some(end_time)) => {
            let start = start.date.and_time(start_time);
            let end = end.date.and_time(end_time);
            if end <= start {
                return Err("The end time must be after the start time.");
            }
            (
                start.format("%Y-%m-%dT%H:%M:%S").to_string(),
                end.format("%Y-%m-%dT%H:%M:%S").to_string(),
                false,
            )
        }
        _ => return Err("Use either dates or date and time for both fields."),
    };
    let location = meaningful_text(location).or_else(|| meaningful_text(&opportunity.city));
    Ok(serde_json::json!({
        "kind": "event",
        "commitment": "planned",
        "title": title,
        "starts_at": start_value,
        "ends_at": end_value,
        "all_day": all_day,
        "location": location,
        "notes": null,
        "source": "scouting",
        "external_id": opportunity.id,
        "payload": {
            "producer": "scouting",
            "opportunity_id": opportunity.id,
            "opportunity_type": opportunity.opportunity_type,
            "source": opportunity.source,
            "url": opportunity.url,
            "event_route": opportunity.event_route,
        }
    }))
}

#[derive(Clone, Copy)]
struct Instant {
    date: NaiveDate,
    time: Option<NaiveTime>,
}

fn parse_instant(value: &str) -> Option<Instant> {
    let value = value.trim();
    let date = NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d").ok()?;
    let suffix = value.get(10..)?;
    if suffix.is_empty() {
        return Some(Instant { date, time: None });
    }
    let time = suffix
        .strip_prefix('T')
        .or_else(|| suffix.strip_prefix(' '))?;
    let time = NaiveTime::parse_from_str(time.get(..5)?, "%H:%M").ok()?;
    Some(Instant {
        date,
        time: Some(time),
    })
}

fn meaningful_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    trimmed
        .chars()
        .any(char::is_alphanumeric)
        .then_some(trimmed)
}

pub fn calendar_start_input(starts_at: &str) -> Option<String> {
    let start = parse_instant(starts_at)?;
    match start.time {
        None => Some(start.date.format("%Y-%m-%d").to_string()),
        Some(time) => Some(
            start
                .date
                .and_time(time)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
        ),
    }
}

/// The dashboard treats a source's date-only end as inclusive, while Calendar stores an
/// exclusive end. Timed events keep a valid source end, otherwise they default to one hour.
pub fn default_calendar_end(starts_at: &str, source_ends_at: &str) -> Option<String> {
    let start = parse_instant(starts_at)?;
    let source_end = parse_instant(source_ends_at);
    match start.time {
        None => {
            let inclusive_end = source_end
                .filter(|end| end.time.is_none() && end.date >= start.date)
                .map_or(start.date, |end| end.date);
            let exclusive_end = if inclusive_end > start.date {
                inclusive_end + Duration::days(1)
            } else {
                start.date + Duration::days(1)
            };
            Some(exclusive_end.format("%Y-%m-%d").to_string())
        }
        Some(start_time) => {
            let start_datetime = start.date.and_time(start_time);
            let end = source_end
                .and_then(|end| end.time.map(|time| end.date.and_time(time)))
                .filter(|end| *end > start_datetime)
                .unwrap_or(start_datetime + Duration::hours(1));
            Some(end.format("%Y-%m-%dT%H:%M:%S").to_string())
        }
    }
}
