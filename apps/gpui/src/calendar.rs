//! The calendar entries still ahead today and tomorrow. Same rules as `Today` in
//! `apps/mac/Sources/SjelRelay/Today.swift`, so both native surfaces show the same strip.

use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::shell::CalendarEntry;

const DAY: &str = "%Y-%m-%d";
const TIME: &str = "%Y-%m-%dT%H:%M:%S";

/// The query range for today and tomorrow. `to` is exclusive
/// (`capabilities/calendar/src/store.rs`, `list_entries`), so it is the day after tomorrow.
pub fn range(now: NaiveDateTime) -> (String, String) {
    let today = now.date();
    let after = today + Duration::days(2);
    (today.format(DAY).to_string(), after.format(DAY).to_string())
}

fn start(entry: &CalendarEntry) -> Option<NaiveDateTime> {
    parse(&entry.starts_at, entry.all_day)
}

fn end(entry: &CalendarEntry) -> Option<NaiveDateTime> {
    parse(&entry.ends_at, entry.all_day)
}

fn parse(value: &str, all_day: bool) -> Option<NaiveDateTime> {
    if all_day {
        NaiveDate::parse_from_str(value, DAY)
            .ok()?
            .and_hms_opt(0, 0, 0)
    } else {
        NaiveDateTime::parse_from_str(value, TIME).ok()
    }
}

/// Entries not yet over: all-day first, then by start.
pub fn ahead(mut entries: Vec<CalendarEntry>, now: NaiveDateTime) -> Vec<CalendarEntry> {
    entries.retain(|entry| end(entry).is_none_or(|end| end > now));
    entries.sort_by_key(|entry| (!entry.all_day, start(entry)));
    entries
}

/// The label right of the row: `14:30`, `tomorrow 09:00`, `all day`, `until Oct 14`.
pub fn when(entry: &CalendarEntry, now: NaiveDateTime) -> String {
    let Some(start) = start(entry) else {
        return String::new();
    };
    let tomorrow = start.date() > now.date();
    if entry.all_day {
        if let Some(last) = end(entry).map(|end| end.date() - Duration::days(1)) {
            if last > start.date() && start <= now {
                return format!("until {}", last.format("%b %-d"));
            }
        }
        return if tomorrow { "tomorrow" } else { "all day" }.to_owned();
    }
    let clock = start.format("%H:%M");
    if tomorrow {
        format!("tomorrow {clock}")
    } else {
        clock.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(starts_at: &str, ends_at: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            title: "t".into(),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day,
            commitment: "planned".into(),
            ..CalendarEntry::default()
        }
    }

    fn at(value: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(value, TIME).unwrap()
    }

    #[test]
    fn range_covers_today_and_tomorrow_with_an_exclusive_end() {
        assert_eq!(
            range(at("2026-12-31T23:00:00")),
            ("2026-12-31".into(), "2027-01-02".into())
        );
    }

    #[test]
    fn finished_entries_drop_and_all_day_leads() {
        let now = at("2026-10-15T12:00:00");
        let entries = vec![
            entry("2026-10-15T09:00:00", "2026-10-15T10:00:00", false),
            entry("2026-10-15T15:00:00", "2026-10-15T16:00:00", false),
            entry("2026-10-15", "2026-10-16", true),
        ];
        let ahead = ahead(entries, now);
        assert_eq!(ahead.len(), 2);
        assert!(ahead[0].all_day);
        assert_eq!(when(&ahead[1], now), "15:00");
    }

    #[test]
    fn labels_tomorrow_and_multi_day() {
        let now = at("2026-10-15T12:00:00");
        assert_eq!(
            when(
                &entry("2026-10-16T09:00:00", "2026-10-16T10:00:00", false),
                now
            ),
            "tomorrow 09:00"
        );
        assert_eq!(
            when(&entry("2026-10-16", "2026-10-17", true), now),
            "tomorrow"
        );
        assert_eq!(
            when(&entry("2026-10-15", "2026-10-16", true), now),
            "all day"
        );
        assert_eq!(
            when(&entry("2026-10-13", "2026-10-18", true), now),
            "until Oct 17"
        );
    }
}
