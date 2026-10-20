//! Offset-bearing instant ⇄ naive home wall time.
//!
//! Phase E is where offsets stop being hypothetical. Google emits real
//! RFC 3339 (`2026-08-14T18:00:00+02:00`), this capability stores naive local
//! wall time (README § Time model), and something has to be the boundary
//! between the two. This module is that boundary, in both directions:
//! `wall_time` for import, `rfc3339` for export.
//!
//! **Why a hand-rolled zone instead of `chrono-tz`.** Same call
//! `capabilities/scouting/src/localtime.rs` made and for the same reason: the
//! operator has exactly one home zone, and pulling a full tz database to
//! answer one question is machinery this repo keeps declining —
//! `src/date.rs` already hand-rolls the civil-date arithmetic. The cost is a
//! closed set of supported zones, so an unsupported one is a hard error
//! rather than a silently wrong wall time.
//!
//! **Why a copy and not a shared lib.** scouting's version converts UTC → wall
//! only; this one has to accept an arbitrary offset (Google does not normalize
//! to UTC) and run the conversion *backwards* for export, which is the half
//! that has to reason about nonexistent and ambiguous wall times at all. The
//! two crates have no dependency edge, and a `libs/sjel-time` with one real
//! consumer plus a near-duplicate still sitting in scouting would be worse
//! than an honest documented copy. Folding both into one lib is a follow-up
//! that has to touch scouting.

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

const SECS_PER_DAY: i64 = 86_400;

impl HomeTimezone {
    /// Accepts an IANA name from the supported set above, `UTC`, or a fixed
    /// `+HH:MM`/`-HH:MM` offset. Everything else is an error naming what is
    /// supported — a wrong wall time is worse than a refused import.
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
        } else if let Some(minutes) = parse_offset(trimmed) {
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

    /// Import direction: an offset-bearing RFC 3339 instant → the
    /// `YYYY-MM-DDTHH:MM:SS` naive wall time this capability stores.
    ///
    /// The offset applied is the one in effect *at that instant*, not a fixed
    /// one — that is the whole point of doing this properly in Phase E. An
    /// input without an offset is an error: a bare local time carries no zone
    /// to convert from, and assuming one is the guess this module exists to
    /// avoid.
    pub fn wall_time(&self, instant: &str) -> Result<String, String> {
        let utc = parse_rfc3339(instant)?;
        Ok(format_naive(utc + i64::from(self.offset_at_utc(utc)) * 60))
    }

    /// Export direction: a stored naive wall time → RFC 3339 with the offset
    /// that wall time actually had, which is what Google's `dateTime` wants.
    ///
    /// Errors on a wall time that does not exist (the hour skipped at
    /// spring-forward). A clock in this zone never showed it, so an entry
    /// carrying one is bad data, and picking an offset for it would push a
    /// different instant to Google than the one on screen.
    pub fn rfc3339(&self, wall: &str) -> Result<String, String> {
        let offset = self.offset_for_wall(wall)?;
        Ok(format!("{wall}{}", format_offset(offset)))
    }

    /// The offset in effect for a naive wall time, in minutes east of UTC.
    ///
    /// Resolved by consistency rather than by a transition table: for each
    /// offset the zone can have, check whether interpreting the wall time
    /// through it lands on an instant that really has that offset. Zero
    /// consistent candidates means the wall time is in the spring-forward gap;
    /// two means it is in the repeated autumn hour, and the summer offset wins
    /// (the first of the two occurrences — a convention, stated here because
    /// there is no fact of the matter).
    pub fn offset_for_wall(&self, wall: &str) -> Result<i32, String> {
        let wall_secs = parse_naive(wall)?;
        let candidates: &[i32] = &match self.rule {
            Rule::Fixed(m) => [m, m],
            // Summer first: an ambiguous time resolves to its first occurrence.
            Rule::EuSummerTime(standard) => [standard + 60, standard],
        };
        for &candidate in candidates {
            let utc = wall_secs - i64::from(candidate) * 60;
            if self.offset_at_utc(utc) == candidate {
                return Ok(candidate);
            }
        }
        Err(format!(
            "'{wall}' is not a real wall time in {} — it falls in the hour skipped at the spring-forward transition",
            self.name
        ))
    }

    fn offset_at_utc(&self, utc_secs: i64) -> i32 {
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

/// `+02:00` / `-0800` / `+05` / `Z` → minutes east of UTC.
fn parse_offset(text: &str) -> Option<i32> {
    if text.eq_ignore_ascii_case("Z") {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
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

fn format_offset(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let abs = minutes.abs();
    format!("{sign}{:02}:{:02}", abs / 60, abs % 60)
}

/// Howard Hinnant's days-from-civil, the same algorithm `src/date.rs` uses.
/// Not delegated to it: that module speaks day counts, this one speaks seconds
/// and needs the pair inline for the offset arithmetic.
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
    let days = days_from_civil(year, month, 31);
    let sunday = days - weekday(days);
    sunday * SECS_PER_DAY + hour_utc * 3_600
}

fn in_eu_summer_time(utc_secs: i64) -> bool {
    let (year, _, _) = civil_from_days(utc_secs.div_euclid(SECS_PER_DAY));
    utc_secs >= last_sunday_utc(year, 3, 1) && utc_secs < last_sunday_utc(year, 10, 1)
}

/// Splits `YYYY-MM-DDTHH:MM[:SS][.fff]` into (date, time), tolerating a space
/// separator. Returns seconds-of-day plus the civil date.
fn split_civil(body: &str, original: &str) -> Result<(i64, u32, u32, i64), String> {
    let (date, time) = body
        .split_once('T')
        .or_else(|| body.split_once(' '))
        .ok_or_else(|| format!("'{original}' has no time component"))?;

    let mut date_parts = date.split('-');
    let year: i64 = next_num(&mut date_parts, original, "year")?;
    let month: u32 = next_num(&mut date_parts, original, "month")?;
    let day: u32 = next_num(&mut date_parts, original, "day")?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("'{original}' has an out-of-range date"));
    }

    // Fractional seconds carry no information this capability stores.
    let time = time.split('.').next().unwrap_or(time);
    let mut time_parts = time.split(':');
    let hour: i64 = next_num(&mut time_parts, original, "hour")?;
    let minute: i64 = next_num(&mut time_parts, original, "minute")?;
    let second: i64 = match time_parts.next() {
        Some(s) => s
            .parse()
            .map_err(|_| format!("'{original}' has a non-numeric second"))?,
        None => 0,
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&second) {
        return Err(format!("'{original}' has an out-of-range time"));
    }
    Ok((year, month, day, hour * 3_600 + minute * 60 + second))
}

/// Parses an RFC 3339 instant **with** an offset (`Z`, `+02:00`, `-0800`) into
/// UTC seconds. A value without one is an error, deliberately.
pub fn parse_rfc3339(text: &str) -> Result<i64, String> {
    let t = text.trim();
    // The offset sign can only appear after the time; the date's own hyphens
    // sit before the 'T', so search from there.
    let time_start = t
        .find(['T', ' '])
        .ok_or_else(|| format!("'{t}' has no time component"))?;
    let marker = t[time_start..]
        .find(['Z', 'z', '+', '-'])
        .map(|i| i + time_start);
    let (body, offset) = match marker {
        Some(i) => (
            &t[..i],
            parse_offset(&t[i..]).ok_or_else(|| format!("'{t}' has an unreadable UTC offset"))?,
        ),
        None => {
            return Err(format!(
                "'{t}' carries no UTC offset — a bare local time cannot be converted without guessing its zone"
            ))
        }
    };
    let (year, month, day, secs_of_day) = split_civil(body, t)?;
    Ok(days_from_civil(year, month, day) * SECS_PER_DAY + secs_of_day - i64::from(offset) * 60)
}

/// Parses a naive `YYYY-MM-DDTHH:MM[:SS]` into seconds since the epoch *as if
/// UTC* — the scalar the offset search compares against. Rejects anything
/// carrying an offset: this side of the boundary is naive by contract.
fn parse_naive(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let time_start = t
        .find(['T', ' '])
        .ok_or_else(|| format!("'{t}' has no time component"))?;
    if t[time_start..].contains(['Z', 'z', '+', '-']) {
        return Err(format!(
            "'{t}' already carries an offset; expected naive local wall time"
        ));
    }
    let (year, month, day, secs_of_day) = split_civil(t, t)?;
    Ok(days_from_civil(year, month, day) * SECS_PER_DAY + secs_of_day)
}

fn format_naive(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(SECS_PER_DAY));
    let rem = secs.rem_euclid(SECS_PER_DAY);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> HomeTimezone {
        HomeTimezone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn a_google_offset_instant_becomes_home_wall_time() {
        // Google emits the event's own offset, not UTC. Both shapes of the
        // same instant have to land on the same wall clock.
        let tz = berlin();
        assert_eq!(
            tz.wall_time("2026-08-14T18:00:00+02:00").unwrap(),
            "2026-08-14T18:00:00"
        );
        assert_eq!(
            tz.wall_time("2026-08-14T16:00:00Z").unwrap(),
            "2026-08-14T18:00:00"
        );
        // A New York event viewed in the configured UTC+2 home zone: 09:00 EDT is 15:00 CEST.
        assert_eq!(
            tz.wall_time("2026-08-14T09:00:00-04:00").unwrap(),
            "2026-08-14T15:00:00"
        );
    }

    #[test]
    fn winter_and_summer_use_different_offsets() {
        let tz = berlin();
        assert_eq!(
            tz.wall_time("2026-01-15T16:00:00Z").unwrap(),
            "2026-01-15T17:00:00"
        );
        assert_eq!(
            tz.wall_time("2026-07-15T16:00:00Z").unwrap(),
            "2026-07-15T18:00:00"
        );
    }

    #[test]
    fn crossing_midnight_moves_the_calendar_day() {
        // Filing this on the wrong day is the failure that actually bites.
        assert_eq!(
            berlin().wall_time("2026-08-14T23:00:00Z").unwrap(),
            "2026-08-15T01:00:00"
        );
    }

    #[test]
    fn import_across_the_spring_forward_boundary() {
        // 2026: forward on 29 Mar at 01:00 UTC. 02:00–03:00 local never happens.
        let tz = berlin();
        assert_eq!(
            tz.wall_time("2026-03-29T00:59:00Z").unwrap(),
            "2026-03-29T01:59:00"
        );
        assert_eq!(
            tz.wall_time("2026-03-29T01:00:00Z").unwrap(),
            "2026-03-29T03:00:00"
        );
    }

    #[test]
    fn import_across_the_autumn_boundary_repeats_an_hour() {
        // 2026: back on 25 Oct at 01:00 UTC. 02:00–03:00 local happens twice,
        // and two distinct instants legitimately map to the same wall time —
        // the exact information naive local storage cannot hold, which is why
        // the original offset stays in `payload`.
        let tz = berlin();
        assert_eq!(
            tz.wall_time("2026-10-25T00:30:00Z").unwrap(),
            "2026-10-25T02:30:00"
        );
        assert_eq!(
            tz.wall_time("2026-10-25T01:30:00Z").unwrap(),
            "2026-10-25T02:30:00"
        );
    }

    #[test]
    fn export_stamps_the_offset_that_wall_time_actually_had() {
        let tz = berlin();
        assert_eq!(
            tz.rfc3339("2026-08-14T18:00:00").unwrap(),
            "2026-08-14T18:00:00+02:00"
        );
        assert_eq!(
            tz.rfc3339("2026-01-15T18:00:00").unwrap(),
            "2026-01-15T18:00:00+01:00"
        );
    }

    #[test]
    fn export_resolves_the_boundaries_it_can_and_refuses_the_one_it_cannot() {
        let tz = berlin();
        // Before the jump: still CET.
        assert_eq!(tz.offset_for_wall("2026-03-29T01:59:00").unwrap(), 60);
        // After it: CEST.
        assert_eq!(tz.offset_for_wall("2026-03-29T03:00:00").unwrap(), 120);
        // Inside it: no such wall time, so no offset to stamp.
        let error = tz.offset_for_wall("2026-03-29T02:30:00").unwrap_err();
        assert!(error.contains("skipped at the spring-forward"), "{error}");
        // The repeated autumn hour resolves to its first occurrence (CEST).
        assert_eq!(tz.offset_for_wall("2026-10-25T02:30:00").unwrap(), 120);
    }

    #[test]
    fn import_and_export_round_trip_through_a_dst_transition() {
        let tz = berlin();
        for instant in [
            "2026-03-29T00:30:00Z", // 01:30 CET
            "2026-03-29T02:00:00Z", // 04:00 CEST
            "2026-06-01T10:00:00Z",
            "2026-12-01T10:00:00Z",
        ] {
            let wall = tz.wall_time(instant).unwrap();
            let back = tz.rfc3339(&wall).unwrap();
            assert_eq!(
                parse_rfc3339(&back).unwrap(),
                parse_rfc3339(instant).unwrap(),
                "{instant} → {wall} → {back} is not the same instant"
            );
        }
    }

    #[test]
    fn the_repeated_hour_is_the_documented_lossy_case() {
        // Round-tripping the *second* occurrence returns the first. This is
        // the cost of naive local storage, named rather than hidden.
        let tz = berlin();
        let wall = tz.wall_time("2026-10-25T01:30:00Z").unwrap();
        let back = tz.rfc3339(&wall).unwrap();
        assert_eq!(back, "2026-10-25T02:30:00+02:00");
        assert_ne!(
            parse_rfc3339(&back).unwrap(),
            parse_rfc3339("2026-10-25T01:30:00Z").unwrap()
        );
    }

    #[test]
    fn a_bare_local_time_is_refused_rather_than_assumed_to_be_utc() {
        let error = berlin().wall_time("2026-08-14T18:00:00").unwrap_err();
        assert!(error.contains("carries no UTC offset"), "{error}");
        assert!(berlin().wall_time("2026-08-14").is_err());
    }

    #[test]
    fn fixed_offsets_and_utc_are_supported_zones() {
        assert_eq!(
            HomeTimezone::parse("UTC")
                .unwrap()
                .wall_time("2026-08-14T16:00:00Z")
                .unwrap(),
            "2026-08-14T16:00:00"
        );
        let india = HomeTimezone::parse("+05:30").unwrap();
        assert_eq!(
            india.wall_time("2026-08-14T16:00:00Z").unwrap(),
            "2026-08-14T21:30:00"
        );
        assert_eq!(
            india.rfc3339("2026-08-14T21:30:00").unwrap(),
            "2026-08-14T21:30:00+05:30"
        );
    }

    #[test]
    fn london_keeps_its_own_standard_offset() {
        let london = HomeTimezone::parse("Europe/London").unwrap();
        assert_eq!(
            london.wall_time("2026-08-14T16:00:00Z").unwrap(),
            "2026-08-14T17:00:00"
        );
        assert_eq!(
            london.rfc3339("2026-01-15T09:00:00").unwrap(),
            "2026-01-15T09:00:00+00:00"
        );
    }

    #[test]
    fn an_unsupported_zone_is_an_error_not_a_guess() {
        let error = HomeTimezone::parse("America/Toronto").unwrap_err();
        assert!(error.contains("unsupported timezone"), "{error}");
        assert!(HomeTimezone::parse("  ").is_err());
    }

    #[test]
    fn offset_parsing_covers_the_shapes_rfc3339_allows() {
        assert_eq!(parse_offset("+02:00"), Some(120));
        assert_eq!(parse_offset("-0430"), Some(-270));
        assert_eq!(parse_offset("+05"), Some(300));
        assert_eq!(parse_offset("Z"), Some(0));
        assert_eq!(parse_offset("+15:00"), None);
        assert_eq!(parse_offset("02:00"), None);
        assert_eq!(format_offset(-270), "-04:30");
        assert_eq!(format_offset(0), "+00:00");
    }
}
