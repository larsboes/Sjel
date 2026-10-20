//! The date, and the two conversions either side of it.
//!
//! These lived in `src/server.rs`, which is a `[[bin]]` (`Cargo.toml`'s
//! `[[bin]] name = "finance-server"`). Nothing in the library and nothing in
//! `finance-cli` could name them, so a lib module that needed today's date had
//! the choice of taking a timestamp through every signature or writing a ninth
//! copy of Hinnant's algorithm. Moving them here is subtraction: `server.rs`
//! calls the same functions it used to define.
//!
//! UTC, with no timezone database. A billing date, a price observation date and
//! a proposal date are not precise to the hour, and a tz dependency would be
//! bought for a boundary case this capability does not have.

/// Today as an ISO date, from the wall clock, with no date dependency.
pub fn today() -> String {
    civil_date::today()
}

/// Now as second-granular RFC3339.
///
/// The granularity is load-bearing rather than cosmetic: `finance_decision_events`
/// keys on `(decision_id, event, recorded_at)`, so a date-granular stamp would
/// turn a same-day corrected verdict into a constraint error.
pub fn now_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let seconds = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_from_days(secs.div_euclid(86_400)),
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// Days since the Unix epoch as an ISO date.
pub fn civil_from_days(days: i64) -> String {
    civil_date::iso_of_unix_day(days)
}

/// An ISO date as days since the Unix epoch, or `None` when the string is not a
/// **real** date. The inverse of [`civil_from_days`].
///
/// Stricter than `civil_date::unix_day_of_iso`, which checks shape rather than the
/// calendar: the gate below is what refuses 2026-02-29, and it is the reason this
/// wrapper still exists after the arithmetic moved out.
pub fn iso_day(value: &str) -> Option<i64> {
    if !valid_iso_date(value) {
        return None;
    }
    civil_date::unix_day_of_iso(value)
}

/// Whether a string is a real calendar date in `YYYY-MM-DD`. February 30th is
/// refused, not clamped.
pub fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }

    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=last_day).contains(&day)
}

/// Whole days between two ISO dates, `None` when either is not a date.
pub fn days_between(from: &str, to: &str) -> Option<i64> {
    Some(iso_day(to)? - iso_day(from)?)
}

/// The `YYYY-MM` a date falls in, or `None` when it is not a date.
pub fn month_of(date: &str) -> Option<String> {
    valid_iso_date(date).then(|| date[0..7].to_string())
}

/// One [`now_timestamp`] stamp back as epoch seconds, or `None` when the string
/// is not one.
///
/// The inverse of [`now_timestamp`] and of nothing else: exactly twenty
/// characters, `YYYY-MM-DDTHH:MM:SSZ`, UTC. Every stamp this reads was written
/// by that function, so a wider parser would only widen what can be misread.
/// `None` rather than a best effort, because the one caller is
/// `GET /__sjel/freshness`: a guessed epoch there is a claim about when data
/// last arrived, and a wrong one reads as green.
///
/// Rejects the shape AND the calendar -- `iso_day` refuses 2026-02-29 -- and the
/// clock: hour 24 and minute 60 are not times. Second 60 is refused with them,
/// because `now_timestamp` divides a Unix count and never writes a leap second.
pub fn epoch_seconds(timestamp: &str) -> Option<i64> {
    let bytes = timestamp.as_bytes();
    if bytes.len() != 20 || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if bytes[19] != b'Z' {
        return None;
    }
    let day = iso_day(&timestamp[0..10])?;
    let hour = two_digits(&timestamp[11..13], 23)?;
    let minute = two_digits(&timestamp[14..16], 59)?;
    let second = two_digits(&timestamp[17..19], 59)?;
    Some(day * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Two ASCII digits as a number, bounded. `parse` alone would accept `" 9"`,
/// `"+9"` and `"9\u{0}"`; the digit check is what makes the bound mean anything.
fn two_digits(value: &str, max: i64) -> Option<i64> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let parsed = value.parse::<i64>().ok()?;
    (parsed <= max).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(19_723), "2024-01-01");
        // 2024 was a leap year; the day after 02-28 is 02-29, not 03-01.
        assert_eq!(civil_from_days(19_782), "2024-02-29");
        assert_eq!(civil_from_days(20_673), "2026-08-08");
        assert_eq!(iso_day("1970-01-01"), Some(0));
        assert_eq!(iso_day("2024-02-29"), Some(19_782));
        assert_eq!(iso_day("2026-08-08"), Some(20_673));
    }

    #[test]
    fn today_is_an_iso_date() {
        let value = today();
        assert_eq!(value.len(), 10);
        assert!(valid_iso_date(&value));
    }

    #[test]
    fn dates_are_calendar_dates_not_only_ten_characters() {
        assert!(valid_iso_date("2024-02-29"));
        assert!(valid_iso_date("2026-08-08"));
        assert!(!valid_iso_date("2026-02-29"));
        assert!(!valid_iso_date("2026-13-01"));
        assert!(!valid_iso_date("202610-01-08"));
    }

    /// The round trip is the property worth asserting: whatever `now_timestamp`
    /// writes, `epoch_seconds` reads back, so the freshness contract reports the
    /// second the fetch row records rather than an offset from it.
    #[test]
    fn a_timestamp_round_trips_through_epoch_seconds() {
        assert_eq!(epoch_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_seconds("2026-09-07T21:40:05Z"), Some(1_788_817_205));
        // Against the reference implementation, not against a second copy of the
        // arithmetic: the day count comes from civil_date, the rest is a division.
        //
        // Read the clock on BOTH sides of `now_timestamp`, and compare only when
        // the two agree. `now_timestamp` takes its own reading, so one reading
        // taken before it is a different second whenever the boundary falls
        // between the two calls -- roughly one run in a million, and a test that
        // fails for no reason teaches its reader to re-run rather than to look.
        // Watched: with a 1.1s sleep between the two calls the original form fails
        // `left: Some(1788886710), right: Some(1788886709)`. The retry keeps the
        // exact equality rather than widening it into a range.
        let unix_seconds = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        };
        let (stamp, now) = loop {
            let before = unix_seconds();
            let stamp = now_timestamp();
            if before == unix_seconds() {
                break (stamp, before);
            }
        };
        assert_eq!(epoch_seconds(&stamp), Some(now));
    }

    #[test]
    fn a_stamp_that_is_not_a_timestamp_is_none_rather_than_a_guess() {
        // Shape.
        assert_eq!(epoch_seconds(""), None);
        assert_eq!(epoch_seconds("2026-09-07"), None);
        assert_eq!(epoch_seconds("2026-09-07T21:40:05"), None);
        assert_eq!(epoch_seconds("2026-09-07T21:40:05+02:00"), None);
        assert_eq!(epoch_seconds("2026-09-07 21:40:05Z"), None);
        assert_eq!(epoch_seconds("2026-09-07T21:40:05.0Z"), None);
        // Calendar.
        assert_eq!(epoch_seconds("2026-02-29T00:00:00Z"), None);
        // Clock.
        assert_eq!(epoch_seconds("2026-09-07T24:00:00Z"), None);
        assert_eq!(epoch_seconds("2026-09-07T21:60:00Z"), None);
        assert_eq!(epoch_seconds("2026-09-07T21:40:60Z"), None);
        // A digit check `parse` alone would let through.
        assert_eq!(epoch_seconds("2026-09-07T+1:40:05Z"), None);
    }

    #[test]
    fn a_timestamp_is_second_granular_so_a_corrected_verdict_is_a_second_row() {
        let value = now_timestamp();
        assert_eq!(value.len(), 20, "{value}");
        assert!(value.ends_with('Z'));
        assert!(valid_iso_date(&value[0..10]));
    }

    #[test]
    fn a_month_is_the_first_seven_characters_of_a_real_date() {
        assert_eq!(month_of("2026-09-05").as_deref(), Some("2026-09"));
        assert_eq!(month_of("2026-02-30"), None);
        assert_eq!(days_between("2026-09-01", "2026-09-05"), Some(4));
    }
}
