//! The timestamp arithmetic `tools/updates` and `tools/harnesses` need, written once.
//!
//! Three things the ported TypeScript got from `Date`: the current instant, an ISO-8601 string
//! to write into a receipt, and the epoch-milliseconds of a string already in one. `civil-date`
//! owns the calendar half; this file owns the clock and the wire format around it.
//!
//! `now_iso` moved here from `harnesses/mod.rs` when `updates/` needed the same string: two
//! copies of `new Date().toISOString()` is the drift this crate exists to remove.

/// `Date.now()`: Unix epoch milliseconds.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// `new Date().toISOString()`: UTC, millisecond precision, `Z`.
pub fn now_iso() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, day) = civil_date::unix_day_to_ymd(days);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// `Date.parse`: the epoch milliseconds of an ISO-8601 string, or `None` when it is not one.
///
/// The forms this deployment actually writes or reads: the `Z` string [`now_iso`] produces, a
/// bare `YYYY-MM-DD` (what the agent-integration markers hold), and an explicit `±HH:MM` offset
/// for a receipt written elsewhere. A date-only string is UTC midnight, as `Date.parse` reads it.
pub fn parse_iso_ms(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, time) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], Some(s[i + 1..].trim())),
        None => (s, None),
    };

    let mut d = date.split('-');
    let (y, m, day) = match (d.next(), d.next(), d.next(), d.next()) {
        (Some(y), Some(m), Some(day), None) => (
            y.parse::<i64>().ok()?,
            m.parse::<u32>().ok()?,
            day.parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    let mut secs = civil_date::ymd_to_unix_day(y, m, day) * 86_400;
    let mut millis: i64 = 0;

    if let Some(time) = time {
        // A trailing `Z` or an explicit offset is subtracted so the result is UTC.
        let (clock, offset) = split_offset(time)?;
        let mut t = clock.split(':');
        let (h, mi, sec) = match (t.next(), t.next(), t.next(), t.next()) {
            (Some(h), Some(mi), Some(sec), None) => (h, mi, sec),
            _ => return None,
        };
        let h: i64 = h.parse().ok()?;
        let mi: i64 = mi.parse().ok()?;
        let (sec, frac) = match sec.split_once('.') {
            Some((s, f)) => (s, f),
            None => (sec, ""),
        };
        let sec: i64 = sec.parse().ok()?;
        if !frac.is_empty() {
            if frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut ms: i64 = frac.parse().ok()?;
            for _ in frac.len()..3 {
                ms *= 10;
            }
            millis = ms;
        }
        secs += h * 3600 + mi * 60 + sec;
        secs -= offset;
    }

    Some((secs * 1000 + millis) as f64)
}

/// Split `HH:MM:SS(.mmm)Z` or `...±HH:MM` into the clock and the offset in seconds east of UTC.
fn split_offset(time: &str) -> Option<(&str, i64)> {
    if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        return Some((clock, 0));
    }
    // The sign that starts the offset, not one inside the clock: search past the clock's colons.
    if let Some(i) = time.rfind(['+', '-']) {
        if i > 0 {
            let (clock, off) = time.split_at(i);
            let sign = if off.starts_with('-') { -1 } else { 1 };
            let mut p = off[1..].split(':');
            let (h, m) = match (p.next(), p.next(), p.next()) {
                (Some(h), Some(m), None) => (h, m),
                (Some(h), None, None) => (h, "0"),
                _ => return None,
            };
            let h: i64 = h.parse().ok()?;
            let m: i64 = m.parse().ok()?;
            return Some((clock, sign * (h * 3600 + m * 60)));
        }
    }
    Some((time, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timestamp_is_iso_with_milliseconds() {
        let now = now_iso();
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z'), "{now}");
        assert_eq!(&now[10..11], "T");
    }

    #[test]
    fn a_z_string_round_trips_through_the_epoch() {
        // 2026-10-01T00:00:00.000Z
        assert_eq!(
            parse_iso_ms("2026-10-01T00:00:00.000Z"),
            Some(1_790_812_800_000.0)
        );
        assert_eq!(
            parse_iso_ms("2026-10-01T00:00:00Z"),
            Some(1_790_812_800_000.0)
        );
        assert_eq!(
            parse_iso_ms("2026-10-01T00:00:01.250Z"),
            Some(1_790_812_801_250.0)
        );
    }

    #[test]
    fn a_date_only_string_is_utc_midnight_as_date_parse_reads_it() {
        assert_eq!(parse_iso_ms("2026-10-01"), Some(1_790_812_800_000.0));
        assert_eq!(
            parse_iso_ms("2026-10-01 06:00:00"),
            Some(1_790_834_400_000.0)
        );
    }

    #[test]
    fn an_explicit_offset_is_subtracted() {
        assert_eq!(
            parse_iso_ms("2026-10-01T02:00:00+02:00"),
            Some(1_790_812_800_000.0)
        );
        assert_eq!(
            parse_iso_ms("2026-09-30T22:00:00-02:00"),
            Some(1_790_812_800_000.0)
        );
    }

    #[test]
    fn anything_else_is_not_a_date() {
        assert_eq!(parse_iso_ms(""), None);
        assert_eq!(parse_iso_ms("not a date"), None);
        assert_eq!(parse_iso_ms("2026-10-01T00:00"), None);
        assert_eq!(parse_iso_ms("2026-10-01T00:00:00.1234Z"), None);
    }
}
