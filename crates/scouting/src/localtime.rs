//! UTC instant → naive local wall time, for the calendar promotion.
//!
//! Luma reports every event as a UTC instant (`2026-07-30T16:00:00.000Z`)
//! plus a separate IANA zone. `capabilities/calendar` stores naive local wall
//! time in the operator's one home zone and explicitly refuses to guess
//! schedule metadata (its README § Sources). Something has to bridge those,
//! and this is it.
//!
//! **Why a hand-rolled zone instead of `chrono-tz`:** the promotion needs
//! exactly one zone — the operator's — and pulling a full tz database to answer
//! one question is the machinery this repo keeps
//! declining. `capabilities/calendar/src/date.rs` already hand-rolls the same
//! civil-date arithmetic. The cost of that choice is a closed set of
//! supported zones, so an unsupported one is a hard error: the promotion
//! refuses rather than silently writing a wrong wall time, which is the same
//! no-guessing contract calendar states for dates.

/// How a supported zone's offset is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// Constant offset in minutes east of UTC.
    Fixed(i32),
    /// EU summer-time rule, standard offset in minutes. Since 1996 every EU
    /// zone switches together: forward on the last Sunday of March at 01:00
    /// UTC, back on the last Sunday of October at 01:00 UTC.
    EuSummerTime(i32),
}

/// A resolved home timezone. Construct with [`HomeTimezone::parse`].
#[derive(Debug, Clone)]
pub struct HomeTimezone {
    name: String,
    rule: Rule,
}

/// IANA zones that follow the EU rule, grouped by standard offset. Only zones
/// actually reachable from this repo's use are listed; anything else errors.
const EU_CENTRAL: &[&str] = &[
    "Europe/Amsterdam",
    "Europe/Berlin",
    "Europe/Brussels",
    "Europe/Budapest",
    "Europe/Copenhagen",
    "Europe/Madrid",
    "Europe/Oslo",
    "Europe/Paris",
    "Europe/Prague",
    "Europe/Rome",
    "Europe/Stockholm",
    "Europe/Vienna",
    "Europe/Warsaw",
    "Europe/Zurich",
];

const EU_WESTERN: &[&str] = &["Europe/Dublin", "Europe/Lisbon", "Europe/London"];

impl HomeTimezone {
    /// Accepts an IANA name from the supported set above, `UTC`, or a fixed
    /// `+HH:MM`/`-HH:MM` offset. Everything else is an error naming what is
    /// supported — a wrong wall time is worse than a refused promotion.
    pub fn parse(name: &str) -> Result<Self, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("timezone is empty".into());
        }
        let rule = if trimmed.eq_ignore_ascii_case("UTC") || trimmed == "Z" {
            Rule::Fixed(0)
        } else if EU_CENTRAL.iter().any(|z| z.eq_ignore_ascii_case(trimmed)) {
            Rule::EuSummerTime(60)
        } else if EU_WESTERN.iter().any(|z| z.eq_ignore_ascii_case(trimmed)) {
            Rule::EuSummerTime(0)
        } else if let Some(minutes) = parse_fixed_offset(trimmed) {
            Rule::Fixed(minutes)
        } else {
            return Err(format!(
                "unsupported timezone '{trimmed}'. Supported: UTC, a fixed ±HH:MM offset, or one of {}, {}",
                EU_CENTRAL.join(", "),
                EU_WESTERN.join(", ")
            ));
        };
        Ok(Self {
            name: trimmed.to_string(),
            rule,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Converts a UTC ISO-8601 instant to `YYYY-MM-DDTHH:MM:SS` wall time in
    /// this zone. Returns `Err` for anything that is not an explicit UTC
    /// instant — a bare local time carries no offset to convert from, and
    /// assuming one is exactly the guess this module exists to avoid.
    pub fn wall_time(&self, utc_instant: &str) -> Result<String, String> {
        let secs = parse_utc_instant(utc_instant)?;
        let shifted = secs + i64::from(self.offset_minutes(secs)) * 60;
        Ok(format_naive(shifted))
    }

    fn offset_minutes(&self, utc_secs: i64) -> i32 {
        match self.rule {
            Rule::Fixed(m) => m,
            Rule::EuSummerTime(standard) => {
                if in_eu_summer_time(utc_secs) {
                    standard + 60
                } else {
                    standard
                }
            }
        }
    }
}

fn parse_fixed_offset(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    let sign = match bytes.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &text[1..];
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h, m),
        None if rest.len() == 4 => (&rest[..2], &rest[2..]),
        None => (rest, "00"),
    };
    let hours: i32 = h.parse().ok()?;
    let minutes: i32 = m.parse().ok()?;
    if !(0..=14).contains(&hours) || !(0..60).contains(&minutes) {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

/// Howard Hinnant's civil-from-days pair, same algorithm
/// `capabilities/calendar/src/date.rs` uses. Duplicated rather than shared:
/// the two crates have no dependency edge, and this is 20 lines of arithmetic
/// with a fixed definition.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Day-of-week for a days-since-epoch value; 0 = Sunday.
fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// UTC seconds of the last Sunday of `month` in `year`, at `hour_utc`.
fn last_sunday_utc(year: i64, month: u32, hour_utc: i64) -> i64 {
    let last_day = match month {
        3 => 31,
        10 => 31,
        _ => unreachable!("EU transitions are March and October only"),
    };
    let days = days_from_civil(year, month, last_day);
    let sunday = days - weekday(days);
    sunday * 86_400 + hour_utc * 3_600
}

fn in_eu_summer_time(utc_secs: i64) -> bool {
    let (year, _, _) = civil_from_days(utc_secs.div_euclid(86_400));
    let start = last_sunday_utc(year, 3, 1);
    let end = last_sunday_utc(year, 10, 1);
    utc_secs >= start && utc_secs < end
}

/// Parses `YYYY-MM-DDTHH:MM[:SS][.fff]` with a `Z`/`+00:00` UTC marker.
fn parse_utc_instant(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let body = if let Some(stripped) = t.strip_suffix('Z').or_else(|| t.strip_suffix('z')) {
        stripped
    } else if let Some(stripped) = t
        .strip_suffix("+00:00")
        .or_else(|| t.strip_suffix("+0000"))
        .or_else(|| t.strip_suffix("+00"))
    {
        stripped
    } else {
        return Err(format!(
            "'{t}' is not an explicit UTC instant (needs a 'Z' or '+00:00' suffix)"
        ));
    };

    let (date, time) = body
        .split_once('T')
        .or_else(|| body.split_once(' '))
        .ok_or_else(|| format!("'{t}' has no time component"))?;

    let mut date_parts = date.split('-');
    let year: i64 = next_num(&mut date_parts, t, "year")?;
    let month: u32 = next_num(&mut date_parts, t, "month")?;
    let day: u32 = next_num(&mut date_parts, t, "day")?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("'{t}' has an out-of-range date"));
    }

    // Fractional seconds carry no information calendar stores.
    let time = time.split('.').next().unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour: i64 = next_num(&mut time_parts, t, "hour")?;
    let minute: i64 = next_num(&mut time_parts, t, "minute")?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().unwrap_or(0);
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&second) {
        return Err(format!("'{t}' has an out-of-range time"));
    }

    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn next_num<'a, T: std::str::FromStr>(
    parts: &mut impl Iterator<Item = &'a str>,
    original: &str,
    field: &str,
) -> Result<T, String> {
    parts
        .next()
        .ok_or_else(|| format!("'{original}' is missing its {field}"))?
        .parse()
        .map_err(|_| format!("'{original}' has a non-numeric {field}"))
}

/// A Unix timestamp as the UTC ISO-8601 instant the rest of this crate stores.
///
/// The inverse of what `wall_time` consumes, for an adapter whose upstream
/// sends epoch seconds rather than an ISO string. Shared rather than derived
/// again in the adapter: the civil-from-days arithmetic is already here, and
/// two copies of it is how the two come to disagree.
pub fn utc_instant_from_epoch(secs: i64) -> String {
    format!("{}Z", format_naive(secs))
}

fn format_naive(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> HomeTimezone {
        HomeTimezone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn converts_a_real_luma_instant_in_summer() {
        // The live Berlin Claude event: 16:00Z on 2026-07-30 is 18:00 CEST.
        assert_eq!(
            berlin().wall_time("2026-07-30T16:00:00.000Z").unwrap(),
            "2026-07-30T18:00:00"
        );
    }

    #[test]
    fn converts_in_winter() {
        assert_eq!(
            berlin().wall_time("2026-01-15T16:00:00.000Z").unwrap(),
            "2026-01-15T17:00:00"
        );
    }

    #[test]
    fn crosses_midnight_into_the_next_day() {
        // A US-evening event lands on the following Berlin date. Getting this
        // wrong would file the entry on the wrong calendar day.
        assert_eq!(
            berlin().wall_time("2026-07-30T23:00:00.000Z").unwrap(),
            "2026-07-31T01:00:00"
        );
    }

    #[test]
    fn honours_the_eu_dst_boundaries() {
        // 2026: forward 29 Mar, back 25 Oct — both at 01:00 UTC.
        assert_eq!(
            berlin().wall_time("2026-03-29T00:59:00Z").unwrap(),
            "2026-03-29T01:59:00"
        );
        assert_eq!(
            berlin().wall_time("2026-03-29T01:00:00Z").unwrap(),
            "2026-03-29T03:00:00"
        );
        assert_eq!(
            berlin().wall_time("2026-10-25T00:59:00Z").unwrap(),
            "2026-10-25T02:59:00"
        );
        assert_eq!(
            berlin().wall_time("2026-10-25T01:00:00Z").unwrap(),
            "2026-10-25T02:00:00"
        );
    }

    #[test]
    fn london_is_an_hour_behind_berlin() {
        let london = HomeTimezone::parse("Europe/London").unwrap();
        assert_eq!(
            london.wall_time("2026-07-30T16:00:00.000Z").unwrap(),
            "2026-07-30T17:00:00"
        );
        assert_eq!(
            london.wall_time("2026-01-15T16:00:00.000Z").unwrap(),
            "2026-01-15T16:00:00"
        );
    }

    #[test]
    fn accepts_utc_and_fixed_offsets() {
        assert_eq!(
            HomeTimezone::parse("UTC")
                .unwrap()
                .wall_time("2026-07-30T16:00:00Z")
                .unwrap(),
            "2026-07-30T16:00:00"
        );
        assert_eq!(
            HomeTimezone::parse("+05:30")
                .unwrap()
                .wall_time("2026-07-30T16:00:00Z")
                .unwrap(),
            "2026-07-30T21:30:00"
        );
        assert_eq!(
            HomeTimezone::parse("-08:00")
                .unwrap()
                .wall_time("2026-07-30T16:00:00Z")
                .unwrap(),
            "2026-07-30T08:00:00"
        );
    }

    #[test]
    fn rejects_an_unsupported_zone_instead_of_guessing() {
        let err = HomeTimezone::parse("America/Toronto").unwrap_err();
        assert!(err.contains("unsupported timezone"), "got: {err}");
    }

    #[test]
    fn rejects_an_instant_with_no_utc_marker() {
        let err = berlin().wall_time("2026-07-30T16:00:00").unwrap_err();
        assert!(err.contains("explicit UTC instant"), "got: {err}");
    }

    #[test]
    fn rejects_a_date_only_value() {
        // Calendar would need all_day semantics for this; the promotion
        // reports it as unpromotable rather than inventing a time.
        assert!(berlin().wall_time("2026-07-30").is_err());
    }
}
