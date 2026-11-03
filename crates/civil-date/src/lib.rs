//! Howard Hinnant's civil-from-days pair, written once.
//!
//! Public domain, from "chrono-Compatible Low-Level Date Algorithms"
//! (<https://howardhinnant.github.io/date_algorithms.html>). Four capabilities had
//! copied it: `capabilities/places/src/lib.rs`, `capabilities/finance/src/clock.rs`,
//! `capabilities/calendar/src/date.rs` and `capabilities/trips/src/windows.rs`, with
//! a fifth copy inlined inside `capabilities/trips/src/intent.rs`. Three of the five
//! were the same twelve lines to the character.
//!
//! Every one of them explained itself the same way — no date dependency for a
//! conversion that is a dozen lines of integer arithmetic — so this crate has no
//! dependencies either. A shared home that arrived with chrono and chrono-tz would
//! have undone the property those five authors were buying.
//!
//! # Two scales, and the bridge between them
//!
//! Hinnant's algorithm counts days from a **proleptic year 0**, not from Unix.
//! `capabilities/trips/src/windows.rs` calls that a *day number* and records what
//! happens when the two are mixed: a raw Unix day count fed to the inverse returns a
//! well-formed date that is nearly two thousand years wrong, so nothing downstream
//! can tell it went wrong. [`UNIX_EPOCH_DAY`] is the only bridge, and every name here
//! says which scale it is on. Mixing them now needs ignoring the function's name.

/// The day number of 1970-01-01 on the proleptic-year-0 scale.
///
/// Anything converting between a Unix day count and a [`day_number_of_iso`] result
/// adds or subtracts this. `capabilities/trips/src/windows.rs` named the constant
/// after a copy that did not, and was off by 1,969 years while still returning a
/// well-formed date.
pub const UNIX_EPOCH_DAY: i64 = 719_468;

/// A civil date as days since 1970-01-01, proleptic Gregorian.
///
/// No validation: February 30th converts to March 2nd, the way the algorithm
/// defines it. [`unix_day_of_iso`] range-checks its input; a caller that needs
/// "is this a real date" round-trips through [`unix_day_to_ymd`], which is what
/// `capabilities/calendar/src/date.rs::parse_date` does.
pub fn ymd_to_unix_day(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i64 + 9) % 12; // [0, 11], March = 0
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - UNIX_EPOCH_DAY
}

/// The inverse of [`ymd_to_unix_day`].
pub fn unix_day_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + UNIX_EPOCH_DAY;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// Days since 1970-01-01 as `YYYY-MM-DD`.
pub fn iso_of_unix_day(days: i64) -> String {
    let (y, m, d) = unix_day_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` as days since 1970-01-01, or `None` when the string is not shaped
/// like a date.
///
/// Shape, not calendar: the month must be 1..=12 and the day 1..=31, so `2026-13-01`
/// is refused and `2026-02-30` is not. That is what `capabilities/places` has always
/// done here, and `capabilities/finance/src/clock.rs` gets the stricter answer by
/// running its own `valid_iso_date` first. A caller wanting the strict form without
/// finance's helper round-trips through [`unix_day_to_ymd`].
///
/// The day field is read two characters wide, so a trailing wall time
/// (`2026-08-08T10:00`) parses as its date rather than failing. Removing that would
/// change what `places::days_from_civil` accepts.
pub fn unix_day_of_iso(iso: &str) -> Option<i64> {
    let mut parts = iso.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(ymd_to_unix_day(year, month, day))
}

/// Days since proleptic year 0 as `YYYY-MM-DD`.
///
/// The scale `capabilities/trips/src/windows.rs` does its window arithmetic on.
pub fn iso_of_day_number(z: i64) -> String {
    iso_of_unix_day(z - UNIX_EPOCH_DAY)
}

/// `YYYY-MM-DD` as days since proleptic year 0, with [`unix_day_of_iso`]'s shape check.
pub fn day_number_of_iso(iso: &str) -> Option<i64> {
    Some(unix_day_of_iso(iso)? + UNIX_EPOCH_DAY)
}

/// Today, UTC, as days since 1970-01-01.
fn today_unix_day_at(now: std::time::SystemTime) -> i64 {
    now.duration_since(std::time::UNIX_EPOCH)
        .map(|since| (since.as_secs() as i64).div_euclid(86_400))
        .unwrap_or(0)
}

/// Today, UTC, as days since 1970-01-01.
///
/// UTC rather than the deployment's home zone: every caller uses this at day
/// granularity for a billing date, a materialization horizon or a search window, and
/// each of them says in its own words that a timezone database would be bought for a
/// boundary case it does not have.
pub fn today_unix_day() -> i64 {
    today_unix_day_at(std::time::SystemTime::now())
}

/// Today, UTC, as `YYYY-MM-DD`.
pub fn today() -> String {
    iso_of_unix_day(today_unix_day())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known days every copy of this algorithm asserted, pooled. The 1970,
    /// 2024-02-29 and 2026-08-08 rows come from finance and places; the 2024-01-01
    /// row from finance alone.
    #[test]
    fn the_date_conversion_matches_known_days() {
        assert_eq!(iso_of_unix_day(0), "1970-01-01");
        assert_eq!(iso_of_unix_day(19_723), "2024-01-01");
        // 2024 was a leap year; the day after 02-28 is 02-29, not 03-01.
        assert_eq!(iso_of_unix_day(19_782), "2024-02-29");
        assert_eq!(iso_of_unix_day(20_673), "2026-08-08");
        assert_eq!(unix_day_of_iso("1970-01-01"), Some(0));
        assert_eq!(unix_day_of_iso("2024-02-29"), Some(19_782));
        assert_eq!(unix_day_of_iso("2026-08-08"), Some(20_673));
    }

    #[test]
    fn the_two_directions_are_inverses() {
        for day in [0i64, 19_782, 20_673, -1, 25_000] {
            assert_eq!(unix_day_of_iso(&iso_of_unix_day(day)), Some(day));
        }
        assert_eq!(unix_day_of_iso("2026-13-01"), None);
        assert_eq!(unix_day_of_iso("not a date"), None);
    }

    /// From `capabilities/calendar/src/date.rs`, which asserted the tuple form.
    #[test]
    fn epoch_is_zero_and_the_tuple_form_agrees_with_the_string_form() {
        assert_eq!(ymd_to_unix_day(1970, 1, 1), 0);
        assert_eq!(unix_day_to_ymd(0), (1970, 1, 1));
        for day in [-146_097i64, -1, 0, 19_782, 20_673, 146_097] {
            let (y, m, d) = unix_day_to_ymd(day);
            assert_eq!(ymd_to_unix_day(y, m, d), day);
            assert_eq!(iso_of_unix_day(day), format!("{y:04}-{m:02}-{d:02}"));
        }
    }

    /// From `capabilities/calendar/src/date.rs`: steps of 97 days cross every
    /// February 29th and every century boundary without an exhaustive loop.
    #[test]
    fn civil_round_trips_across_leap_boundaries() {
        let mut day = -400_000i64;
        while day < 400_000 {
            let (y, m, d) = unix_day_to_ymd(day);
            assert_eq!(ymd_to_unix_day(y, m, d), day, "{y:04}-{m:02}-{d:02}");
            day += 97;
        }
    }

    /// From `capabilities/trips/src/windows.rs`. The trap the named constant exists
    /// to prevent: the year-0 inverse on a raw Unix day count returns a well-formed
    /// date that is nearly two thousand years wrong.
    #[test]
    fn the_two_scales_do_not_agree_and_the_constant_is_the_bridge() {
        assert_eq!(iso_of_day_number(UNIX_EPOCH_DAY), "1970-01-01");
        assert_eq!(day_number_of_iso("1970-01-01"), Some(UNIX_EPOCH_DAY));
        assert_eq!(iso_of_day_number(UNIX_EPOCH_DAY + 20_697), "2026-09-01");
        assert_eq!(iso_of_day_number(20_697), "0056-10-30");
    }

    /// The shape check is a shape check. February 30th is arithmetic, not a refusal;
    /// `capabilities/finance/src/clock.rs` layers `valid_iso_date` on top for the
    /// stricter answer.
    #[test]
    fn the_shape_check_refuses_an_impossible_month_but_not_an_impossible_day() {
        assert_eq!(unix_day_of_iso("2026-13-01"), None);
        assert_eq!(unix_day_of_iso("2026-00-01"), None);
        assert_eq!(unix_day_of_iso("2026-01-32"), None);
        assert_eq!(unix_day_of_iso("2026-01-00"), None);
        assert_eq!(unix_day_of_iso("2026-02-30"), unix_day_of_iso("2026-03-02"));
    }

    /// `places::days_from_civil` accepted a trailing wall time, and its store passes
    /// one. Reading the day two characters wide is what allows it.
    #[test]
    fn a_trailing_wall_time_parses_as_its_date() {
        assert_eq!(unix_day_of_iso("2026-08-08T10:30"), Some(20_673));
    }

    #[test]
    fn today_is_an_iso_date_on_the_right_scale() {
        let value = today();
        assert_eq!(value.len(), 10, "{value}");
        let day = unix_day_of_iso(&value).expect("today parses back");
        assert_eq!(day, today_unix_day());
        // Anything between 2020 and 2100: this asserts the epoch, not the clock.
        assert!(
            day > unix_day_of_iso("2020-01-01").unwrap()
                && day < unix_day_of_iso("2100-01-01").unwrap(),
            "today() returned {value}, which is off the wall clock's scale"
        );
    }

    #[test]
    fn a_clock_before_the_epoch_is_day_zero_rather_than_a_panic() {
        // `duration_since(UNIX_EPOCH)` fails on a clock set before 1970. Every copy
        // this replaced fell back to 0 rather than unwrapping, and so does this.
        let before = std::time::UNIX_EPOCH - std::time::Duration::from_secs(1);
        assert_eq!(today_unix_day_at(before), 0);
    }
}
