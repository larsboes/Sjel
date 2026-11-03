//! Proleptic-Gregorian date math for rhythm materialization and window queries,
//! plus the entry-instant parsing built on it.
//!
//! The conversion pair itself is `libs/civil-date` — no chrono there either, for
//! the reason this module recorded first: the dependency set stays identical to
//! capabilities/trips (no new `upstreams.toml` verdicts), and the algorithm is a
//! few lines with round-trip tests. What stays here is what is calendar's own:
//! weekday tokens, the strict `parse_date` gate, and `parse_instant`.

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    civil_date::ymd_to_unix_day(year, month, day)
}

/// Inverse of `days_from_civil`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    civil_date::unix_day_to_ymd(z)
}

/// Weekday of a day count, 0 = Monday .. 6 = Sunday. 1970-01-01 was a Thursday.
pub fn weekday(days: i64) -> u32 {
    (((days + 3) % 7 + 7) % 7) as u32
}

pub const WEEKDAY_TOKENS: [&str; 7] = ["mo", "tu", "we", "th", "fr", "sa", "su"];

pub fn parse_weekday(token: &str) -> Option<u32> {
    WEEKDAY_TOKENS
        .iter()
        .position(|t| *t == token)
        .map(|i| i as u32)
}

/// Strictly parse "YYYY-MM-DD" into a day count; rejects impossible dates
/// (2026-02-30) by round-tripping through `civil_from_days`.
pub fn parse_date(text: &str) -> Option<i64> {
    if text.len() != 10 {
        return None;
    }
    let mut parts = text.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let days = days_from_civil(y, m, d);
    let (ry, rm, rd) = civil_from_days(days);
    if (ry, rm, rd) == (y, m, d) {
        Some(days)
    } else {
        None
    }
}

pub fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Strictly parse "HH:MM" (24h).
pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    if text.len() != 5 {
        return None;
    }
    let (h, m) = text.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h < 24 && m < 60 {
        Some((h, m))
    } else {
        None
    }
}

/// Parse an entry instant: either an all-day date ("YYYY-MM-DD") or a naive
/// local wall time ("YYYY-MM-DDTHH:MM" / "...THH:MM:SS"). Returns the day
/// count plus the wall time when timed. See README's time-model block for why
/// this is naive local, not offset-bearing.
pub fn parse_instant(text: &str) -> Option<(i64, Option<(u32, u32)>)> {
    if let Some(days) = parse_date(text) {
        return Some((days, None));
    }
    let (date_part, time_part) = text.split_once('T')?;
    let days = parse_date(date_part)?;
    let mut fields = time_part.split(':');
    let h: u32 = fields.next()?.parse().ok()?;
    let m: u32 = fields.next()?.parse().ok()?;
    let seconds_ok = match fields.next() {
        None => true,
        Some(s) => s.parse::<u32>().ok()? < 60,
    };
    if !seconds_ok || fields.next().is_some() || h >= 24 || m >= 60 {
        return None;
    }
    Some((days, Some((h, m))))
}

/// An entry instant as minutes since 1970-01-01T00:00 local; an all-day date
/// is that day's midnight. Overlap math needs this scalar rather than the
/// `(day, Option<time>)` tuple `parse_instant` returns: `None` sorts before
/// `Some`, so a timed entry ending exactly at `2026-08-14T00:00` compares as
/// *after* the all-day `2026-08-14` that starts at the same instant, and the
/// two would report an overlap that exclusive ends say cannot exist.
pub fn instant_minutes(text: &str) -> Option<i64> {
    let (days, time) = parse_instant(text)?;
    let (hours, minutes) = time.unwrap_or((0, 0));
    Some(days * 1440 + hours as i64 * 60 + minutes as i64)
}

/// Today's date (UTC) as a day count — the forward-materialization horizon
/// for rhythms. UTC rather than operator-local is fine at day granularity for
/// the single-home-zone v1; documented in the README's time-model block.
pub fn today_days() -> i64 {
    civil_date::today_unix_day()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_zero_and_a_thursday() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(0), 3);
        assert_eq!(format_date(0), "1970-01-01");
    }

    #[test]
    fn civil_round_trips_across_leap_boundaries() {
        // Steps of 97 days cover Feb-29 and century boundaries without a
        // multi-second exhaustive loop.
        let mut days = days_from_civil(1890, 1, 1);
        let end = days_from_civil(2130, 12, 31);
        while days <= end {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(
                days_from_civil(y, m, d),
                days,
                "round-trip failed for {days}"
            );
            days += 97;
        }
    }

    #[test]
    fn parse_date_rejects_impossible_dates() {
        assert!(parse_date("2026-02-29").is_none()); // 2026 not a leap year
        assert!(parse_date("2000-02-29").is_some()); // 2000 was
        assert!(parse_date("1900-02-29").is_none()); // 1900 was not
        assert!(parse_date("2026-13-01").is_none());
        assert!(parse_date("2026-00-10").is_none());
        assert!(parse_date("2026-1-5").is_none()); // strict zero-padding
        assert!(parse_date("not-a-date").is_none());
    }

    #[test]
    fn known_weekdays() {
        let claude_munich_example = parse_date("2026-08-14").unwrap();
        assert_eq!(weekday(claude_munich_example), 4); // a Friday
        assert_eq!(weekday(parse_date("2026-08-16").unwrap()), 6); // Sunday
    }

    #[test]
    fn parse_time_is_strict() {
        assert_eq!(parse_time("09:30"), Some((9, 30)));
        assert!(parse_time("24:00").is_none());
        assert!(parse_time("9:30").is_none());
        assert!(parse_time("12:60").is_none());
    }

    #[test]
    fn instant_minutes_puts_all_day_starts_at_midnight() {
        let midnight = instant_minutes("2026-08-14").unwrap();
        assert_eq!(instant_minutes("2026-08-14T00:00").unwrap(), midnight);
        assert_eq!(instant_minutes("2026-08-14T09:30").unwrap(), midnight + 570);
        assert_eq!(instant_minutes("2026-08-15").unwrap(), midnight + 1440);
        assert!(instant_minutes("2026-08-14 09:30").is_none());
    }

    #[test]
    fn parse_instant_accepts_dates_and_wall_times() {
        assert_eq!(
            parse_instant("2026-08-14"),
            Some((parse_date("2026-08-14").unwrap(), None))
        );
        let (days, time) = parse_instant("2026-08-14T18:30").unwrap();
        assert_eq!(time, Some((18, 30)));
        assert_eq!(days, parse_date("2026-08-14").unwrap());
        assert!(parse_instant("2026-08-14T18:30:45").is_some());
        assert!(parse_instant("2026-08-14T18:30:61").is_none());
        assert!(parse_instant("2026-08-14 18:30").is_none());
    }
}
