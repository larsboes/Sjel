//! The calendar's grids and entry rules, for any client that draws the calendar capability.
//!
//! The capability stores an all-day entry as `YYYY-MM-DD` with an exclusive end, and a timed
//! one as local `YYYY-MM-DDTHH:MM:SS`; a day window is `[from, to)`
//! (`capabilities/calendar/src/store.rs`, `list_entries`). Kinds and commitments were the
//! dashboard's (`dashboard/src/lib/calendar/types.ts`); this module owns them now.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::shell::CalendarEntry;

pub const DAY: &str = "%Y-%m-%d";
const TIME: &str = "%Y-%m-%dT%H:%M:%S";

/// Known kinds with a label and a colour (`0xRRGGBB`). Kind is open text; an unknown one is
/// drawn with [`FALLBACK_COLOR`] and its key as the label.
pub const KINDS: [(&str, &str, u32); 8] = [
    ("busy", "Busy", 0xe11d48),
    ("work_onsite", "Work (on site)", 0x2563eb),
    ("work_remote", "Work (remote)", 0x0891b2),
    ("away", "Away", 0xd97706),
    ("event", "Event", 0x7c3aed),
    ("nightlife", "Nightlife / Open Air", 0xdb2777),
    ("deadline", "Deadline", 0xb45309),
    ("travel_ok", "Travel possible", 0x16a34a),
];
pub const FALLBACK_COLOR: u32 = 0x64748b;

/// How binding an entry is: `possible` blocks nothing, `planned` soft-blocks, `committed`
/// blocks the day.
pub const COMMITMENTS: [(&str, &str); 3] = [
    ("possible", "Possible"),
    ("planned", "Planned"),
    ("committed", "Committed"),
];

pub fn kind_label(kind: &str) -> &str {
    KINDS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or(kind, |(_, label, _)| label)
}

pub fn kind_color(kind: &str) -> u32 {
    KINDS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or(FALLBACK_COLOR, |(_, _, color)| *color)
}

pub fn parse_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..10)?, DAY).ok()
}

fn monday_of(day: NaiveDate) -> NaiveDate {
    day - Duration::days(day.weekday().num_days_from_monday().into())
}

/// The seven days of `day`'s week, Monday first.
pub fn week_dates(day: NaiveDate) -> Vec<NaiveDate> {
    let monday = monday_of(day);
    (0..7).map(|i| monday + Duration::days(i)).collect()
}

/// The whole weeks a month is drawn on, Monday first, padded with the neighbouring months.
pub fn month_dates(day: NaiveDate) -> Vec<NaiveDate> {
    let first = day.with_day(1).unwrap_or(day);
    let next = first
        .checked_add_months(chrono::Months::new(1))
        .unwrap_or(first);
    let last = next - Duration::days(1);
    let start = monday_of(first);
    let end = monday_of(last) + Duration::days(6);
    (0..=(end - start).num_days())
        .map(|i| start + Duration::days(i))
        .collect()
}

/// The `[from, to)` query window for the days on screen.
pub fn window(dates: &[NaiveDate]) -> Option<(String, String)> {
    let (first, last) = (dates.first()?, dates.last()?);
    Some((
        first.format(DAY).to_string(),
        (*last + Duration::days(1)).format(DAY).to_string(),
    ))
}

fn timed(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, TIME).ok()
}

/// Whether `entry` touches `day`. A timed entry ending at midnight does not touch the next day.
pub fn covers_day(entry: &CalendarEntry, day: NaiveDate) -> bool {
    if entry.all_day {
        let (Some(start), Some(end)) = (parse_day(&entry.starts_at), parse_day(&entry.ends_at))
        else {
            return false;
        };
        return start <= day && day < end;
    }
    let (Some(start), Some(end)) = (timed(&entry.starts_at), timed(&entry.ends_at)) else {
        return false;
    };
    let day_start = day.and_time(NaiveTime::MIN);
    start < day_start + Duration::days(1) && end > day_start
}

/// The entries on `day`, all-day first, then by start.
pub fn entries_on(entries: &[CalendarEntry], day: NaiveDate) -> Vec<&CalendarEntry> {
    let mut on: Vec<&CalendarEntry> = entries.iter().filter(|e| covers_day(e, day)).collect();
    on.sort_by(|a, b| (!a.all_day, &a.starts_at).cmp(&(!b.all_day, &b.starts_at)));
    on
}

/// `09:00–10:30`, `all day`, or `from 22:00` on a day an overnight entry starts.
pub fn time_label(entry: &CalendarEntry, day: NaiveDate) -> String {
    if entry.all_day {
        return "all day".into();
    }
    let (Some(start), Some(end)) = (timed(&entry.starts_at), timed(&entry.ends_at)) else {
        return String::new();
    };
    match (
        start.date() == day,
        end.date() == day || end == (day + Duration::days(1)).and_time(NaiveTime::MIN),
    ) {
        (true, true) => format!("{}–{}", start.format("%H:%M"), end.format("%H:%M")),
        (true, false) => format!("from {}", start.format("%H:%M")),
        (false, true) => format!("until {}", end.format("%H:%M")),
        (false, false) => "all day".into(),
    }
}

/// What a new-entry form produces, checked before it reaches the capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub title: String,
    pub day: NaiveDate,
    /// `HH:MM`; ignored when `all_day`.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub kind: String,
    pub commitment: String,
}

impl Draft {
    /// The entry this draft describes, with `id`: empty for one this device is about to name
    /// ([`crate::store::LOCAL_ENTRY_ID_PREFIX`]), or the id the Mac assigned. `Err` is why the
    /// draft cannot be saved.
    ///
    /// One place decides what a form's fields mean, because two callers ask: the Mac posts
    /// [`Self::body`] to the capability, and the phone writes the entry straight into its replica
    /// and lets `crate::sync` replay it.
    pub fn entry(&self, id: String) -> Result<CalendarEntry, &'static str> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Give the entry a title.");
        }
        let (starts_at, ends_at) = if self.all_day {
            (
                self.day.format(DAY).to_string(),
                (self.day + Duration::days(1)).format(DAY).to_string(),
            )
        } else {
            let clock = |t: &str| NaiveTime::parse_from_str(t.trim(), "%H:%M").ok();
            let (Some(start), Some(end)) = (clock(&self.start), clock(&self.end)) else {
                return Err("Times are HH:MM, for example 09:30.");
            };
            let start = self.day.and_time(start);
            // An end at or before the start runs past midnight into the next day.
            let mut end = self.day.and_time(end);
            if end <= start {
                end += Duration::days(1);
            }
            (start.format(TIME).to_string(), end.format(TIME).to_string())
        };
        Ok(CalendarEntry {
            id,
            kind: self.kind.clone(),
            commitment: self.commitment.clone(),
            title: title.to_owned(),
            starts_at,
            ends_at,
            all_day: self.all_day,
            location: None,
            notes: None,
            source: "manual".to_owned(),
        })
    }

    /// The `POST /calendar/api/entries` body, or why the draft cannot be saved.
    pub fn body(&self) -> Result<serde_json::Value, &'static str> {
        let entry = self.entry(String::new())?;
        Ok(serde_json::json!({
            "kind": entry.kind,
            "commitment": entry.commitment,
            "title": entry.title,
            "starts_at": entry.starts_at,
            "ends_at": entry.ends_at,
            "all_day": entry.all_day,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        parse_day(s).unwrap()
    }

    fn entry(starts_at: &str, ends_at: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            title: "x".into(),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            all_day,
            commitment: "planned".into(),
            ..CalendarEntry::default()
        }
    }

    #[test]
    fn grids_start_on_monday_and_cover_whole_weeks() {
        let month = month_dates(day("2026-10-22"));
        assert_eq!(month.first(), Some(&day("2026-09-28")));
        assert_eq!(month.last(), Some(&day("2026-11-01")));
        assert_eq!(month.len() % 7, 0);
        let week = week_dates(day("2026-10-25"));
        assert_eq!(week[0], day("2026-10-19"));
        assert_eq!(week[6], day("2026-10-25"));
        assert_eq!(
            window(&week),
            Some(("2026-10-19".into(), "2026-10-26".into()))
        );
    }

    #[test]
    fn coverage_respects_exclusive_ends() {
        let all_day = entry("2026-10-08", "2026-10-10", true);
        assert!(covers_day(&all_day, day("2026-10-09")));
        assert!(!covers_day(&all_day, day("2026-10-10")));
        let to_midnight = entry("2026-10-08T22:00:00", "2026-10-09T00:00:00", false);
        assert!(covers_day(&to_midnight, day("2026-10-08")));
        assert!(!covers_day(&to_midnight, day("2026-10-09")));
        assert_eq!(time_label(&to_midnight, day("2026-10-08")), "22:00–00:00");
        let overnight = entry("2026-10-08T22:00:00", "2026-10-09T02:00:00", false);
        assert_eq!(time_label(&overnight, day("2026-10-08")), "from 22:00");
        assert_eq!(time_label(&overnight, day("2026-10-09")), "until 02:00");
    }

    #[test]
    fn a_draft_becomes_the_capabilitys_shape() {
        let mut draft = Draft {
            title: " Dentist ".into(),
            day: day("2026-10-23"),
            start: "09:30".into(),
            end: "10:15".into(),
            all_day: false,
            kind: "busy".into(),
            commitment: "committed".into(),
        };
        let body = draft.body().unwrap();
        assert_eq!(body["title"], "Dentist");
        assert_eq!(body["starts_at"], "2026-10-23T09:30:00");
        assert_eq!(body["ends_at"], "2026-10-23T10:15:00");
        draft.end = "01:00".into();
        assert_eq!(draft.body().unwrap()["ends_at"], "2026-10-24T01:00:00");
        draft.all_day = true;
        let body = draft.body().unwrap();
        assert_eq!(body["starts_at"], "2026-10-23");
        assert_eq!(body["ends_at"], "2026-10-24");
        draft.title = "  ".into();
        assert!(draft.body().is_err());
        draft.title = "x".into();
        draft.all_day = false;
        draft.start = "9.30".into();
        assert!(draft.body().is_err());
    }

    /// Two callers ask what a form means: the Mac posts [`Draft::body`] and the phone writes
    /// [`Draft::entry`] into its replica. They must not disagree about the same fields.
    #[test]
    fn the_entry_a_draft_names_is_the_one_its_body_posts() {
        let draft = Draft {
            title: "  Dentist ".into(),
            day: day("2026-10-24"),
            start: "09:00".into(),
            end: "10:00".into(),
            all_day: false,
            kind: "event".into(),
            commitment: "planned".into(),
        };
        let entry = draft.entry("calendar:local:1".into()).expect("entry");
        assert_eq!(entry.id, "calendar:local:1", "the id is the caller's");
        assert_eq!(entry.title, "Dentist", "the title is trimmed");
        assert_eq!(
            entry.source, "manual",
            "a phone-made entry is the operator's"
        );

        let body = draft.body().expect("body");
        assert_eq!(body["title"], entry.title);
        assert_eq!(body["starts_at"], entry.starts_at);
        assert_eq!(body["ends_at"], entry.ends_at);
        assert_eq!(body["all_day"], false);
        assert_eq!(body["kind"], entry.kind);
        assert_eq!(body["commitment"], entry.commitment);

        // An all-day entry runs to the next midnight, wherever it is written.
        let all_day = Draft {
            all_day: true,
            ..draft
        };
        let entry = all_day.entry(String::new()).expect("entry");
        assert_eq!(entry.starts_at, "2026-10-24");
        assert_eq!(entry.ends_at, "2026-10-25");
    }
}
