//! Conversion between capability-local calendar values and Ely's zoned events.
use crate::ely::calendar::{
    jiff::{
        civil::{Date, DateTime},
        tz::TimeZone,
        Timestamp,
    },
    Event, EventDraft, When,
};
use sjel_gpui_core::{
    calendar::{self, EntryEdit},
    shell::CalendarEntry,
};

pub(super) fn date(day: chrono::NaiveDate) -> Date {
    day.format("%Y-%m-%d")
        .to_string()
        .parse()
        .expect("a supported calendar date")
}

pub(super) fn day(day: Date) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(&day.to_string(), "%Y-%m-%d")
        .expect("a supported calendar date")
}

pub(super) fn timestamp(value: &str, zone: &TimeZone) -> Result<Timestamp, String> {
    let civil = value
        .parse::<DateTime>()
        .map_err(|error| format!("Invalid calendar time: {error}"))?;
    zone.to_ambiguous_timestamp(civil)
        .unambiguous()
        .map_err(|error| format!("Choose an unambiguous local time: {error}"))
}

pub(super) fn event(
    entry: &CalendarEntry,
    zone: &TimeZone,
    selected: Option<&str>,
) -> Result<Event, String> {
    let hue = calendar::KINDS
        .iter()
        .position(|(kind, _, _)| *kind == entry.kind)
        .unwrap_or(0)
        % 8;
    let mut event = if entry.all_day {
        let first = calendar::parse_day(&entry.starts_at).ok_or("Invalid start date")?;
        let last = calendar::parse_day(&entry.ends_at)
            .and_then(|end| end.pred_opt())
            .ok_or("Invalid end date")?;
        if last < first {
            return Err("The end must follow the start".into());
        }
        Event::all_day(
            entry.id.clone(),
            entry.title.clone(),
            date(first),
            date(last),
            hue,
        )
    } else {
        let start = timestamp(&entry.starts_at, zone)?;
        let end = timestamp(&entry.ends_at, zone)?;
        if end <= start {
            return Err("The end must follow the start".into());
        }
        Event::timed(entry.id.clone(), entry.title.clone(), start, end, hue)
    };
    event.place = entry.location.clone().map(Into::into);
    event.color = Some(gpui::rgb(calendar::kind_color(&entry.kind)).into());
    event.tentative = matches!(entry.commitment.as_str(), "possible" | "tentative");
    event.selected = selected == Some(entry.id.as_str());
    Ok(event)
}

pub(super) fn local(at: Timestamp, zone: &TimeZone) -> String {
    at.to_zoned(zone.clone())
        .strftime("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

pub(super) fn draft(entry: &CalendarEntry, zone: &TimeZone) -> Result<EventDraft, String> {
    let mut draft = EventDraft::of(&event(entry, zone, None)?);
    draft.notes = entry.notes.clone().unwrap_or_default().into();
    Ok(draft)
}

pub(super) fn patch(
    draft: &EventDraft,
    kind: &str,
    commitment: &str,
    zone: &TimeZone,
) -> Result<serde_json::Value, String> {
    let (start_date, end_date, start_time, end_time, all_day) = match draft.when {
        When::AllDay { first, last } => (
            first.to_string(),
            last.to_string(),
            "09:00".into(),
            "10:00".into(),
            true,
        ),
        When::Timed { start, end } => {
            if end <= start {
                return Err("The end must follow the start".into());
            }
            let (start, end) = (start.to_zoned(zone.clone()), end.to_zoned(zone.clone()));
            // Recheck local values before serializing into the capability's naive contract.
            timestamp(&start.strftime("%Y-%m-%dT%H:%M:%S").to_string(), zone)?;
            timestamp(&end.strftime("%Y-%m-%dT%H:%M:%S").to_string(), zone)?;
            (
                start.date().to_string(),
                end.date().to_string(),
                start.strftime("%H:%M").to_string(),
                end.strftime("%H:%M").to_string(),
                false,
            )
        }
    };
    calendar::entry_edit_patch(&EntryEdit {
        title: &draft.title,
        kind,
        commitment,
        all_day,
        start_date: &start_date,
        end_date: &end_date,
        start_time: &start_time,
        end_time: &end_time,
        location: &draft.place,
        notes: &draft.notes,
    })
    .map_err(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(start: &str, end: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            id: "calendar:fixture".into(),
            title: "Review".into(),
            starts_at: start.into(),
            ends_at: end.into(),
            all_day,
            kind: "event".into(),
            commitment: "possible".into(),
            location: Some("Room".into()),
            notes: Some("Keep this".into()),
            ..Default::default()
        }
    }
    #[test]
    fn exclusive_all_day_end_and_metadata_round_trip() {
        let entry = entry("2026-11-05", "2026-11-08", true);
        let draft = draft(&entry, &TimeZone::UTC).unwrap();
        assert!(
            matches!(draft.when, When::AllDay { last, .. } if last.to_string() == "2026-11-07")
        );
        let body = patch(&draft, &entry.kind, &entry.commitment, &TimeZone::UTC).unwrap();
        assert_eq!(body["ends_at"], entry.ends_at);
        assert_eq!(body["notes"], "Keep this");
        assert_eq!(body["location"], "Room");
        assert_eq!(body["commitment"], "possible");
    }
    #[test]
    fn invalid_entries_and_dst_ambiguity_do_not_reach_asserting_constructors() {
        let zone = TimeZone::get("Europe/Berlin").unwrap();
        for (start, end) in [
            ("broken", "2026-11-05T10:00:00"),
            ("2026-11-05T10:00:00", "2026-11-05T09:00:00"),
            ("2026-03-29T02:30:00", "2026-03-29T04:00:00"),
            ("2026-10-25T02:30:00", "2026-10-25T04:00:00"),
        ] {
            assert!(event(&entry(start, end, false), &zone, None).is_err());
        }
    }
    #[test]
    fn timed_midnight_end_and_commitment_survive_conversion() {
        let entry = entry("2026-03-28T23:30:00", "2026-03-29T03:30:00", false);
        let zone = TimeZone::get("Europe/Berlin").unwrap();
        let converted = event(&entry, &zone, Some(&entry.id)).unwrap();
        assert!(converted.tentative && converted.selected);
        let body = patch(&draft(&entry, &zone).unwrap(), "event", "possible", &zone).unwrap();
        assert_eq!(body["starts_at"], entry.starts_at);
        assert_eq!(body["ends_at"], entry.ends_at);
    }
}
