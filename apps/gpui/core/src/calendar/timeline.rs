//! Day clipping, overlap columns and available intervals shared by native calendars.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::shell::CalendarEntry;

/// A timed entry clipped to a day and assigned a column within its overlap group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub entry_index: usize,
    pub start: u16,
    pub end: u16,
    pub column: usize,
    pub columns: usize,
}

/// A half-open interval of local minutes within one day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub start: u16,
    pub end: u16,
}

/// An all-day entry spanning visible day columns, with an exclusive end column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub entry_index: usize,
    pub start: usize,
    pub end: usize,
    pub lane: usize,
}

/// Pack all-day entries into lanes while clipping them to the visible dates.
pub fn spans(entries: &[CalendarEntry], dates: &[NaiveDate]) -> Vec<Span> {
    let mut spans: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(entry_index, entry)| {
            if !entry.all_day {
                return None;
            }
            let start = dates
                .iter()
                .position(|&day| super::covers_day(entry, day))?;
            let end = dates
                .iter()
                .rposition(|&day| super::covers_day(entry, day))?
                + 1;
            Some(Span {
                entry_index,
                start,
                end,
                lane: 0,
            })
        })
        .collect();
    spans.sort_by_key(|span| (span.start, std::cmp::Reverse(span.end), span.entry_index));
    let mut ends: Vec<usize> = Vec::new();
    for span in &mut spans {
        span.lane = ends
            .iter()
            .position(|&end| end <= span.start)
            .unwrap_or(ends.len());
        if span.lane == ends.len() {
            ends.push(span.end);
        } else {
            ends[span.lane] = span.end;
        }
    }
    spans
}

fn timestamp(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").ok()
}

/// Move a timed entry, preserving duration, or change its end at a quarter-hour boundary.
///
/// # Errors
/// Rejects all-day entries, invalid timestamps, and an end at or before the start.
pub fn reschedule(
    entry: &CalendarEntry,
    day: NaiveDate,
    minute: u16,
    resize: bool,
) -> Result<CalendarEntry, &'static str> {
    if entry.all_day || minute >= 1440 {
        return Err("Choose a time on a timed entry.");
    }
    let start = timestamp(&entry.starts_at).ok_or("The entry has an invalid start time.")?;
    let end = timestamp(&entry.ends_at).ok_or("The entry has an invalid end time.")?;
    if end <= start {
        return Err("The end must be after the start.");
    }
    let target = day.and_time(NaiveTime::MIN) + Duration::minutes(i64::from(minute / 15 * 15));
    let (start, end) = if resize {
        (start, target)
    } else {
        (
            target,
            target
                .checked_add_signed(end - start)
                .ok_or("The end date is out of range.")?,
        )
    };
    if end <= start {
        return Err("The end must be after the start.");
    }
    let mut changed = entry.clone();
    changed.starts_at = start.format("%Y-%m-%dT%H:%M:%S").to_string();
    changed.ends_at = end.format("%Y-%m-%dT%H:%M:%S").to_string();
    Ok(changed)
}

/// Shift an entry by a relative number of days (e.g. +1 for tomorrow, +7 for next week).
///
/// Preserves start time and duration for timed entries; shifts start and end date for all-day entries.
pub fn shift_days(entry: &CalendarEntry, days: i64) -> Result<CalendarEntry, &'static str> {
    if days == 0 {
        return Ok(entry.clone());
    }
    let duration = Duration::days(days);
    let mut changed = entry.clone();
    if entry.all_day {
        let start_str = entry.starts_at.get(..10).unwrap_or(&entry.starts_at);
        let end_str = entry.ends_at.get(..10).unwrap_or(&entry.ends_at);
        let start = NaiveDate::parse_from_str(start_str, "%Y-%m-%d")
            .map_err(|_| "Invalid start date on all-day entry.")?;
        let end = NaiveDate::parse_from_str(end_str, "%Y-%m-%d")
            .map_err(|_| "Invalid end date on all-day entry.")?;
        let new_start = start
            .checked_add_signed(duration)
            .ok_or("Date out of range.")?;
        let new_end = end
            .checked_add_signed(duration)
            .ok_or("Date out of range.")?;
        changed.starts_at = new_start.format("%Y-%m-%d").to_string();
        changed.ends_at = new_end.format("%Y-%m-%d").to_string();
        Ok(changed)
    } else {
        let start = timestamp(&entry.starts_at).ok_or("The entry has an invalid start time.")?;
        let end = timestamp(&entry.ends_at).ok_or("The entry has an invalid end time.")?;
        let new_start = start
            .checked_add_signed(duration)
            .ok_or("Date out of range.")?;
        let new_end = end
            .checked_add_signed(duration)
            .ok_or("Date out of range.")?;
        changed.starts_at = new_start.format("%Y-%m-%dT%H:%M:%S").to_string();
        changed.ends_at = new_end.format("%Y-%m-%dT%H:%M:%S").to_string();
        Ok(changed)
    }
}

/// Shift a timed entry by minutes (+15, -15, +60, -60) or resize its duration.
pub fn shift_minutes(
    entry: &CalendarEntry,
    minutes: i64,
    resize: bool,
) -> Result<CalendarEntry, &'static str> {
    if entry.all_day {
        return Err("Cannot adjust minutes on an all-day entry.");
    }
    let start = timestamp(&entry.starts_at).ok_or("The entry has an invalid start time.")?;
    let end = timestamp(&entry.ends_at).ok_or("The entry has an invalid end time.")?;
    let duration = Duration::minutes(minutes);
    let (new_start, new_end) = if resize {
        let new_end = end
            .checked_add_signed(duration)
            .ok_or("Time out of range.")?;
        if new_end <= start {
            return Err("The end must be after the start.");
        }
        (start, new_end)
    } else {
        let new_start = start
            .checked_add_signed(duration)
            .ok_or("Time out of range.")?;
        let new_end = end
            .checked_add_signed(duration)
            .ok_or("Time out of range.")?;
        (new_start, new_end)
    };
    let mut changed = entry.clone();
    changed.starts_at = new_start.format("%Y-%m-%dT%H:%M:%S").to_string();
    changed.ends_at = new_end.format("%Y-%m-%dT%H:%M:%S").to_string();
    Ok(changed)
}

/// Place timed entries by duration, with connected overlaps sharing a column count.
pub fn blocks(entries: &[CalendarEntry], day: NaiveDate) -> Vec<Block> {
    let midnight = day.and_time(NaiveTime::MIN);
    let next = midnight + Duration::days(1);
    let mut blocks: Vec<Block> = entries
        .iter()
        .enumerate()
        .filter_map(|(entry_index, entry)| {
            if entry.all_day {
                return None;
            }
            let start = timestamp(&entry.starts_at)?;
            let end = timestamp(&entry.ends_at)?;
            if start >= next || end <= midnight || end <= start {
                return None;
            }
            let start = (start.max(midnight) - midnight).num_minutes() as u16;
            let end = ((end.min(next) - midnight).num_seconds() + 59) / 60;
            Some(Block {
                entry_index,
                start,
                end: end as u16,
                column: 0,
                columns: 1,
            })
        })
        .collect();
    blocks.sort_by_key(|block| (block.start, std::cmp::Reverse(block.end), block.entry_index));
    let mut first = 0;
    while first < blocks.len() {
        let mut last = first + 1;
        let mut end = blocks[first].end;
        while last < blocks.len() && blocks[last].start < end {
            end = end.max(blocks[last].end);
            last += 1;
        }
        let mut column_ends: Vec<u16> = Vec::new();
        for block in &mut blocks[first..last] {
            let column = column_ends
                .iter()
                .position(|&end| end <= block.start)
                .unwrap_or(column_ends.len());
            if column == column_ends.len() {
                column_ends.push(block.end);
            } else {
                column_ends[column] = block.end;
            }
            block.column = column;
        }
        for block in &mut blocks[first..last] {
            block.columns = column_ends.len();
        }
        first = last;
    }
    blocks
}

/// Gaps in the requested hours; possible entries and travel-availability entries do not block.
pub fn gaps(entries: &[CalendarEntry], day: NaiveDate, hours: Interval) -> Vec<Interval> {
    if hours.start >= hours.end || hours.end > 1440 {
        return Vec::new();
    }
    let blocks_time =
        |entry: &CalendarEntry| entry.commitment != "possible" && entry.kind != "travel_ok";
    if entries
        .iter()
        .any(|entry| entry.all_day && blocks_time(entry) && super::covers_day(entry, day))
    {
        return Vec::new();
    }
    let mut cursor = hours.start;
    let mut gaps = Vec::new();
    for block in blocks(entries, day) {
        if !blocks_time(&entries[block.entry_index])
            || block.end <= cursor
            || block.start >= hours.end
        {
            continue;
        }
        if block.start > cursor {
            gaps.push(Interval {
                start: cursor,
                end: block.start.min(hours.end),
            });
        }
        cursor = cursor.max(block.end).min(hours.end);
    }
    if cursor < hours.end {
        gaps.push(Interval {
            start: cursor,
            end: hours.end,
        });
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }
    fn entry(start: &str, end: &str) -> CalendarEntry {
        CalendarEntry {
            starts_at: start.into(),
            ends_at: end.into(),
            commitment: "committed".into(),
            kind: "event".into(),
            ..CalendarEntry::default()
        }
    }

    #[test]
    fn move_resize_and_undo_keep_the_entry_id_and_queue_replica_writes() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let store =
                    std::sync::Arc::new(crate::store::TursoStore::in_memory().await.unwrap());
                let repo = crate::data::Repo::replica(store.clone(), || {});
                let mut original = entry("2026-11-05T09:00:00", "2026-11-05T10:00:00");
                original.id = "calendar:fixture".into();
                original.source = "manual".into();
                repo.save_calendar_entry(original.clone()).await.unwrap();
                let moved = reschedule(&original, day(), 600, false).unwrap();
                repo.save_calendar_entry(moved.clone()).await.unwrap();
                let resized = reschedule(&moved, day(), 690, true).unwrap();
                repo.save_calendar_entry(resized).await.unwrap();
                repo.save_calendar_entry(original.clone()).await.unwrap();
                assert_eq!(
                    store.calendar_entry(&original.id).await.unwrap().unwrap(),
                    original
                );
                let pending = store.pending_mutations().await.unwrap();
                assert_eq!(pending.len(), 4);
                assert!(pending
                    .iter()
                    .all(|write| write.path == original.id && write.action == "calendar_save"));
                let moved: serde_json::Value = serde_json::from_str(&pending[1].payload).unwrap();
                assert_eq!(moved["starts_at"], "2026-11-05T10:00:00");
                let resized: serde_json::Value = serde_json::from_str(&pending[2].payload).unwrap();
                assert_eq!(resized["ends_at"], "2026-11-05T11:30:00");
                let restored: serde_json::Value =
                    serde_json::from_str(&pending[3].payload).unwrap();
                assert_eq!(restored["starts_at"], original.starts_at);
            });
    }

    #[test]
    fn all_day_spans_clip_at_week_edges_and_reuse_lanes() {
        let days = super::super::week_dates(day());
        let mut trip = entry("2026-11-01", "2026-11-06");
        trip.all_day = true;
        let mut other = entry("2026-11-04", "2026-11-05");
        other.all_day = true;
        let mut later = entry("2026-11-06", "2026-11-08");
        later.all_day = true;
        let result = spans(&[trip, other, later], &days);
        assert_eq!(
            result
                .iter()
                .map(|span| (span.start, span.end, span.lane))
                .collect::<Vec<_>>(),
            [(0, 4, 0), (2, 3, 1), (4, 6, 0)]
        );
    }

    #[test]
    fn moving_preserves_overnight_duration_and_resizing_rejects_negative_duration() {
        let original = entry("2026-11-05T23:30:00", "2026-11-06T01:00:00");
        let moved = reschedule(&original, day().succ_opt().unwrap(), 8 * 60 + 22, false).unwrap();
        assert_eq!(moved.starts_at, "2026-11-06T08:15:00");
        assert_eq!(moved.ends_at, "2026-11-06T09:45:00");
        assert!(reschedule(&original, day(), 9 * 60, true).is_err());
        let resized = reschedule(&original, day().succ_opt().unwrap(), 2 * 60, true).unwrap();
        assert_eq!(resized.starts_at, original.starts_at);
        assert_eq!(resized.ends_at, "2026-11-06T02:00:00");
    }

    #[test]
    fn overlaps_share_columns_and_touching_events_reuse_them() {
        let entries = vec![
            entry("2026-11-05T09:00:00", "2026-11-05T11:00:00"),
            entry("2026-11-05T09:30:00", "2026-11-05T10:00:00"),
            entry("2026-11-05T10:00:00", "2026-11-05T12:00:00"),
            entry("2026-11-05T12:00:00", "2026-11-05T13:00:00"),
        ];
        let blocks = blocks(&entries, day());
        assert_eq!(
            blocks
                .iter()
                .map(|b| (b.column, b.columns))
                .collect::<Vec<_>>(),
            [(0, 2), (1, 2), (1, 2), (0, 1)]
        );
    }

    #[test]
    fn overnight_entries_clip_and_midnight_ends_do_not_leak() {
        let entries = vec![
            entry("2026-11-04T22:00:00", "2026-11-05T02:30:00"),
            entry("2026-11-05T23:00:00", "2026-11-06T01:00:00"),
            entry("2026-11-04T22:00:00", "2026-11-05T00:00:00"),
        ];
        let blocks = blocks(&entries, day());
        assert_eq!(
            blocks.iter().map(|b| (b.start, b.end)).collect::<Vec<_>>(),
            [(0, 150), (1380, 1440)]
        );
    }

    #[test]
    fn availability_merges_overlaps_but_keeps_possible_time_free() {
        let mut entries = vec![
            entry("2026-11-05T09:00:00", "2026-11-05T11:00:00"),
            entry("2026-11-05T10:00:00", "2026-11-05T12:00:00"),
            entry("2026-11-05T14:00:00", "2026-11-05T15:00:00"),
        ];
        entries[2].commitment = "possible".into();
        assert_eq!(
            gaps(
                &entries,
                day(),
                Interval {
                    start: 480,
                    end: 1080
                }
            ),
            [
                Interval {
                    start: 480,
                    end: 540
                },
                Interval {
                    start: 720,
                    end: 1080
                }
            ]
        );
        entries.push(CalendarEntry {
            all_day: true,
            starts_at: "2026-11-05".into(),
            ends_at: "2026-11-06".into(),
            commitment: "committed".into(),
            ..CalendarEntry::default()
        });
        assert!(gaps(
            &entries,
            day(),
            Interval {
                start: 480,
                end: 1080
            }
        )
        .is_empty());
    }

    #[test]
    fn quick_reschedule_shift_days_and_minutes() {
        let timed = entry("2026-11-05T09:00:00", "2026-11-05T10:30:00");
        let shifted = shift_days(&timed, 1).expect("shift +1 day");
        assert_eq!(shifted.starts_at, "2026-11-06T09:00:00");
        assert_eq!(shifted.ends_at, "2026-11-06T10:30:00");

        let week = shift_days(&timed, 7).expect("shift +7 days");
        assert_eq!(week.starts_at, "2026-11-12T09:00:00");
        assert_eq!(week.ends_at, "2026-11-12T10:30:00");

        let min_shift = shift_minutes(&timed, 15, false).expect("shift +15m");
        assert_eq!(min_shift.starts_at, "2026-11-05T09:15:00");
        assert_eq!(min_shift.ends_at, "2026-11-05T10:45:00");

        let extended = shift_minutes(&timed, 30, true).expect("extend +30m");
        assert_eq!(extended.starts_at, "2026-11-05T09:00:00");
        assert_eq!(extended.ends_at, "2026-11-05T11:00:00");

        let shortened = shift_minutes(&timed, -30, true).expect("shorten -30m");
        assert_eq!(shortened.starts_at, "2026-11-05T09:00:00");
        assert_eq!(shortened.ends_at, "2026-11-05T10:00:00");

        // Shortening past start must fail
        assert!(shift_minutes(&timed, -90, true).is_err());

        // All-day event shifting
        let all_day = CalendarEntry {
            all_day: true,
            starts_at: "2026-11-05".into(),
            ends_at: "2026-11-06".into(),
            ..CalendarEntry::default()
        };
        let all_day_shifted = shift_days(&all_day, 2).expect("shift all-day +2 days");
        assert_eq!(all_day_shifted.starts_at, "2026-11-07");
        assert_eq!(all_day_shifted.ends_at, "2026-11-08");
        assert!(shift_minutes(&all_day, 15, false).is_err());
    }
}
