//! Rhythm → concrete entry materialization. Rhythms are scoped rules
//! ("Tue–Thu on-site, September through October"), never open-ended
//! recurrence: `valid_from`/`valid_until` are required. Materialization is
//! forward-only — instances in the past stay as historical record even when
//! the rhythm changes — and idempotent via the store's
//! `(rhythm_id, starts_at)` unique slot.

use crate::date;
use crate::model::{Commitment, NewEntry, Rhythm};

/// Dates a rhythm produces inside [valid_from, valid_until], on or after
/// `not_before` (inclusive; callers pass `date::today_days()`).
pub fn materialize_dates(rhythm: &Rhythm, not_before: i64) -> Result<Vec<i64>, String> {
    let from = date::parse_date(&rhythm.valid_from).ok_or("rhythm has invalid valid_from")?;
    let until = date::parse_date(&rhythm.valid_until).ok_or("rhythm has invalid valid_until")?;
    let weekdays: Vec<u32> = rhythm
        .byweekday
        .iter()
        .map(|token| date::parse_weekday(token).ok_or_else(|| format!("bad weekday: {token}")))
        .collect::<Result<_, _>>()?;
    let start = from.max(not_before);
    if start > until {
        return Ok(vec![]);
    }
    Ok((start..=until)
        .filter(|day| weekdays.contains(&date::weekday(*day)))
        .collect())
}

/// The concrete instances for a rhythm. All-day rhythms produce
/// date-only entries with an exclusive next-day end; timed rhythms produce
/// naive local wall times (see README's time-model block).
pub fn instance_entries(rhythm: &Rhythm, not_before: i64) -> Result<Vec<NewEntry>, String> {
    materialize_dates(rhythm, not_before)?
        .into_iter()
        .map(|day| {
            let date = date::format_date(day);
            let (starts_at, ends_at, all_day) = match (&rhythm.start_time, &rhythm.end_time) {
                (Some(start), Some(end)) => (
                    format!("{date}T{start}:00"),
                    format!("{date}T{end}:00"),
                    false,
                ),
                _ => (date.clone(), date::format_date(day + 1), true),
            };
            Ok(NewEntry {
                kind: rhythm.kind.clone(),
                // A rhythm is a standing fact about the week — "Tuesdays I am
                // in the office" — not a maybe. The operator created the rule,
                // so its instances are as real as anything in the calendar.
                commitment: Commitment::Committed,
                title: rhythm.title.clone(),
                starts_at,
                ends_at,
                all_day,
                location: rhythm.location.clone(),
                notes: None,
                attendees: Vec::new(),
                source: "rhythm".into(),
                external_id: None,
                rhythm_id: Some(rhythm.id.clone()),
                payload: serde_json::Value::Null,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn office_rhythm() -> Rhythm {
        Rhythm {
            id: "cal:rhythm:test".into(),
            kind: "work_onsite".into(),
            title: "Office days".into(),
            location: Some("Office".into()),
            byweekday: vec!["tu".into(), "th".into()],
            start_time: None,
            end_time: None,
            valid_from: "2026-08-03".into(),  // a Monday
            valid_until: "2026-08-16".into(), // a Sunday
            active: true,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    #[test]
    fn materializes_only_matching_weekdays_in_window() {
        let days = materialize_dates(&office_rhythm(), 0).unwrap();
        let dates: Vec<String> = days.iter().map(|d| date::format_date(*d)).collect();
        assert_eq!(
            dates,
            vec!["2026-08-04", "2026-08-06", "2026-08-11", "2026-08-13"]
        );
    }

    #[test]
    fn horizon_is_forward_only_and_inclusive() {
        let rhythm = office_rhythm();
        let horizon = date::parse_date("2026-08-06").unwrap();
        let days = materialize_dates(&rhythm, horizon).unwrap();
        assert_eq!(
            date::format_date(days[0]),
            "2026-08-06",
            "horizon day itself is kept"
        );
        assert_eq!(days.len(), 3);
        let past_end = date::parse_date("2026-09-01").unwrap();
        assert!(materialize_dates(&rhythm, past_end).unwrap().is_empty());
    }

    #[test]
    fn all_day_instances_get_exclusive_next_day_end() {
        let entries = instance_entries(&office_rhythm(), 0).unwrap();
        let first = &entries[0];
        assert_eq!(first.starts_at, "2026-08-04");
        assert_eq!(first.ends_at, "2026-08-05");
        assert!(first.all_day);
        assert_eq!(first.source, "rhythm");
        assert_eq!(first.rhythm_id.as_deref(), Some("cal:rhythm:test"));
        for entry in &entries {
            entry.validate().unwrap();
        }
    }

    #[test]
    fn timed_instances_carry_wall_times() {
        let mut rhythm = office_rhythm();
        rhythm.start_time = Some("09:00".into());
        rhythm.end_time = Some("17:30".into());
        let entries = instance_entries(&rhythm, 0).unwrap();
        assert_eq!(entries[0].starts_at, "2026-08-04T09:00:00");
        assert_eq!(entries[0].ends_at, "2026-08-04T17:30:00");
        assert!(!entries[0].all_day);
        for entry in &entries {
            entry.validate().unwrap();
        }
    }
}
