//! Timezone-correct arithmetic for station-local times.
//!
//! bahn.de's journey API serves every stop's times naive -- `2026-08-13T13:57:00`,
//! no offset -- and each stop's time is in that stop's OWN local zone (verified
//! live 2026-08-12: Köln 09:43 CEST -> London 13:57 BST, where wall-clock
//! subtraction yields 4h14m against a true elapsed of 5h14m). Any duration,
//! buffer, or "can I make this connection" computed by subtracting those strings
//! is wrong the moment a leg crosses a zone. This crate is the one place that
//! turns (naive local time, station id) into an unambiguous UTC instant.
//!
//! The zone source is the UIC country prefix: digits 1-2 of the 7-digit
//! station id (EVA number), per the UIC country-code list
//! (https://en.wikipedia.org/wiki/List_of_UIC_country_codes, fetched
//! 2026-08-12; spot-checked live the same day: 8000207 Köln = 80/DE,
//! 8103000 Wien = 81/AT, 7004428 London St. Pancras = 70/GB). Exact for rail,
//! ~30 static rows, and no megabyte timezone-boundary dataset. Countries whose
//! rail networks span multiple zones (Russia, Kazakhstan, ...) are deliberately
//! absent: an unknown prefix returns `None`, never a guessed zone.

use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

/// UIC country prefix -> the IANA zone of that country's rail network.
///
/// Only countries whose rail network sits in ONE zone are listed. The source
/// list is in the crate doc; each row's zone is the country's canonical IANA
/// identifier so DST rules track the real jurisdiction, not a Berlin-alias.
const UIC_ZONES: &[(&str, Tz)] = &[
    ("10", Tz::Europe__Helsinki),   // FI
    ("24", Tz::Europe__Vilnius),    // LT
    ("25", Tz::Europe__Riga),       // LV
    ("26", Tz::Europe__Tallinn),    // EE
    ("41", Tz::Europe__Tirane),     // AL
    ("44", Tz::Europe__Sarajevo),   // BA (Serb Republic entity code)
    ("49", Tz::Europe__Sarajevo),   // BA
    ("50", Tz::Europe__Sarajevo),   // BA (Muslim-Croat Federation entity code)
    ("51", Tz::Europe__Warsaw),     // PL
    ("52", Tz::Europe__Sofia),      // BG
    ("53", Tz::Europe__Bucharest),  // RO
    ("54", Tz::Europe__Prague),     // CZ
    ("55", Tz::Europe__Budapest),   // HU
    ("56", Tz::Europe__Bratislava), // SK
    ("60", Tz::Europe__Dublin),     // IE
    ("62", Tz::Europe__Podgorica),  // ME
    ("65", Tz::Europe__Skopje),     // MK
    ("70", Tz::Europe__London),     // GB
    ("71", Tz::Europe__Madrid),     // ES (mainland; the Canaries have no rail)
    ("72", Tz::Europe__Belgrade),   // RS
    ("73", Tz::Europe__Athens),     // GR
    ("74", Tz::Europe__Stockholm),  // SE
    ("75", Tz::Europe__Istanbul),   // TR
    ("76", Tz::Europe__Oslo),       // NO
    ("78", Tz::Europe__Zagreb),     // HR
    ("79", Tz::Europe__Ljubljana),  // SI
    ("80", Tz::Europe__Berlin),     // DE
    ("81", Tz::Europe__Vienna),     // AT
    ("82", Tz::Europe__Luxembourg), // LU
    ("83", Tz::Europe__Rome),       // IT
    ("84", Tz::Europe__Amsterdam),  // NL
    ("85", Tz::Europe__Zurich),     // CH
    ("86", Tz::Europe__Copenhagen), // DK
    ("87", Tz::Europe__Paris),      // FR
    ("88", Tz::Europe__Brussels),   // BE
    ("94", Tz::Europe__Lisbon),     // PT (mainland; the Azores have no rail)
];

/// The IANA zone a station's local times are expressed in, from its UIC prefix.
///
/// `ext_id` is the plain numeric station id ("8000207"), not the composite
/// `A=1@O=...` lid string. Anything that is not a 7-digit id with a known
/// country prefix returns `None` -- the caller decides what honesty looks like
/// there, but it never receives a guess.
pub fn zone_for_station(ext_id: &str) -> Option<Tz> {
    if ext_id.len() != 7 || !ext_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let prefix = &ext_id[..2];
    UIC_ZONES
        .iter()
        .find(|(p, _)| *p == prefix)
        .map(|(_, tz)| *tz)
}

/// English country name (as flight APIs serve it) -> IANA zone, for countries
/// whose airports all sit in one zone. The flight-search API carries a country
/// name per segment endpoint and nothing better, so this is the honest
/// granularity: multi-zone countries (United States, Russia, Canada, Brazil,
/// ...) are deliberately absent and return `None`. Spain and Portugal ARE
/// listed with their mainland zones because their off-zone airports are all on
/// islands, and those are carried by `AIRPORT_ZONE_EXCEPTIONS` below.
const COUNTRY_ZONES: &[(&str, Tz)] = &[
    ("Albania", Tz::Europe__Tirane),
    ("Austria", Tz::Europe__Vienna),
    ("Belgium", Tz::Europe__Brussels),
    ("Bosnia and Herzegovina", Tz::Europe__Sarajevo),
    ("Bulgaria", Tz::Europe__Sofia),
    ("Croatia", Tz::Europe__Zagreb),
    ("Czech Republic", Tz::Europe__Prague),
    ("Czechia", Tz::Europe__Prague),
    ("Denmark", Tz::Europe__Copenhagen),
    ("Estonia", Tz::Europe__Tallinn),
    ("Finland", Tz::Europe__Helsinki),
    ("France", Tz::Europe__Paris),
    ("Germany", Tz::Europe__Berlin),
    ("Greece", Tz::Europe__Athens),
    ("Hungary", Tz::Europe__Budapest),
    ("Ireland", Tz::Europe__Dublin),
    ("Italy", Tz::Europe__Rome),
    ("Latvia", Tz::Europe__Riga),
    ("Lithuania", Tz::Europe__Vilnius),
    ("Luxembourg", Tz::Europe__Luxembourg),
    ("Montenegro", Tz::Europe__Podgorica),
    ("Netherlands", Tz::Europe__Amsterdam),
    ("North Macedonia", Tz::Europe__Skopje),
    ("Norway", Tz::Europe__Oslo),
    ("Poland", Tz::Europe__Warsaw),
    ("Portugal", Tz::Europe__Lisbon),
    ("Romania", Tz::Europe__Bucharest),
    ("Serbia", Tz::Europe__Belgrade),
    ("Slovakia", Tz::Europe__Bratislava),
    ("Slovenia", Tz::Europe__Ljubljana),
    ("Spain", Tz::Europe__Madrid),
    ("Sweden", Tz::Europe__Stockholm),
    ("Switzerland", Tz::Europe__Zurich),
    ("Turkey", Tz::Europe__Istanbul),
    ("United Kingdom", Tz::Europe__London),
];

/// Airports whose zone differs from their country's mainland zone. All Spanish
/// and Portuguese island airports, which is what makes listing Spain/Portugal
/// in `COUNTRY_ZONES` safe.
const AIRPORT_ZONE_EXCEPTIONS: &[(&str, Tz)] = &[
    // Canary Islands (UTC+0/+1, vs mainland Spain's +1/+2)
    ("ACE", Tz::Atlantic__Canary),
    ("FUE", Tz::Atlantic__Canary),
    ("GMZ", Tz::Atlantic__Canary),
    ("LPA", Tz::Atlantic__Canary),
    ("SPC", Tz::Atlantic__Canary),
    ("TFN", Tz::Atlantic__Canary),
    ("TFS", Tz::Atlantic__Canary),
    ("VDE", Tz::Atlantic__Canary),
    // Azores (UTC-1/+0)
    ("HOR", Tz::Atlantic__Azores),
    ("PDL", Tz::Atlantic__Azores),
    ("PIX", Tz::Atlantic__Azores),
    ("SJZ", Tz::Atlantic__Azores),
    ("TER", Tz::Atlantic__Azores),
    // Madeira (same offsets as Lisbon, its own IANA identity)
    ("FNC", Tz::Atlantic__Madeira),
    ("PXO", Tz::Atlantic__Madeira),
];

/// The IANA zone an airport's local times are expressed in, from its IATA code
/// and the country name the flight API serves next to it. Exceptions first
/// (island airports), then the single-zone country table; anything else --
/// including every airport in a multi-zone country -- returns `None`, never a
/// guess.
pub fn zone_for_airport(iata: &str, country: &str) -> Option<Tz> {
    if let Some((_, tz)) = AIRPORT_ZONE_EXCEPTIONS
        .iter()
        .find(|(code, _)| *code == iata)
    {
        return Some(*tz);
    }
    COUNTRY_ZONES
        .iter()
        .find(|(name, _)| *name == country)
        .map(|(_, tz)| *tz)
}

/// `zone_for_airport`, resolved straight to an RFC 3339 UTC string for a naive
/// airport-local timestamp. `None` when the zone is unknown or the local time
/// does not exist.
pub fn rfc3339_utc_airport(time: &str, iata: &str, country: &str) -> Option<String> {
    let zone = zone_for_airport(iata, country)?;
    let naive = if let Ok(with_offset) = DateTime::parse_from_rfc3339(time) {
        return Some(
            with_offset
                .with_timezone(&Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        );
    } else {
        NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M:%S").ok()?
    };
    match zone.from_local_datetime(&naive) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(
            t.with_timezone(&Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        ),
        LocalResult::None => None,
    }
}

/// A station-local timestamp resolved to a UTC instant.
///
/// Accepts what the wire actually carries: an RFC 3339 string with an offset is
/// respected as-is (never double-shifted), a naive `YYYY-MM-DDTHH:MM:SS` is
/// interpreted in the station's zone. During the autumn DST fold, where a local
/// time exists twice, the earlier instant is taken -- the convention timetables
/// themselves use. A local time that does not exist (the spring-forward gap) or
/// an unknown station prefix yields `None`.
pub fn utc_from_station_local(time: &str, ext_id: &str) -> Option<DateTime<Utc>> {
    if let Ok(with_offset) = DateTime::parse_from_rfc3339(time) {
        return Some(with_offset.with_timezone(&Utc));
    }
    let naive = NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M:%S").ok()?;
    let zone = zone_for_station(ext_id)?;
    match zone.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _later) => Some(earlier.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

/// `utc_from_station_local`, rendered as an RFC 3339 UTC string ("...Z") for
/// fields that travel as JSON.
pub fn rfc3339_utc(time: &str, ext_id: &str) -> Option<String> {
    utc_from_station_local(time, ext_id).map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// True elapsed time from one station-local moment to another, across zones.
pub fn duration_between(
    departure: &str,
    departure_station: &str,
    arrival: &str,
    arrival_station: &str,
) -> Option<chrono::Duration> {
    let dep = utc_from_station_local(departure, departure_station)?;
    let arr = utc_from_station_local(arrival, arrival_station)?;
    Some(arr - dep)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live-measured defect this crate exists to fix: Köln 09:43 CEST to
    /// London St. Pancras 13:57 BST is 5h14m, while naive wall-clock
    /// subtraction says 4h14m. Captured from a real bahn.de response,
    /// 2026-08-12 (ICE 316 + EUR 9135, 2026-08-13).
    #[test]
    fn a_zone_crossing_leg_measures_its_true_elapsed_time() {
        let d = duration_between(
            "2026-08-13T09:43:00",
            "8000207", // Köln Hbf, DE
            "2026-08-13T13:57:00",
            "7004428", // London St. Pancras International, GB
        )
        .expect("both stations carry known UIC prefixes");
        assert_eq!(d.num_seconds(), 18840, "5h14m, not the naive 4h14m");
    }

    #[test]
    fn a_same_zone_leg_matches_wall_clock_subtraction() {
        // Köln -> Wien, same offset all year despite different zones.
        let d = duration_between(
            "2026-08-13T08:53:00",
            "8000207", // Köln Hbf, DE
            "2026-08-13T19:32:00",
            "8103000", // Wien Hbf, AT
        )
        .expect("known prefixes");
        assert_eq!(d.num_minutes(), 639);
    }

    #[test]
    fn an_offset_carrying_time_is_respected_not_double_shifted() {
        let t = utc_from_station_local("2026-08-13T13:57:00+01:00", "7004428").unwrap();
        assert_eq!(t.format("%H:%M").to_string(), "12:57");
        // Even against a wrong or unknown station id: the offset wins.
        let t = utc_from_station_local("2026-08-13T13:57:00+01:00", "0000000").unwrap();
        assert_eq!(t.format("%H:%M").to_string(), "12:57");
    }

    #[test]
    fn an_unknown_prefix_yields_none_never_a_guess() {
        assert_eq!(zone_for_station("2000001"), None); // RU: spans zones, excluded
        assert_eq!(zone_for_station("999"), None); // not a 7-digit EVA id
        assert_eq!(zone_for_station("A=1@O=x"), None); // composite lid, not an id
        assert_eq!(
            utc_from_station_local("2026-08-13T09:43:00", "2000001"),
            None
        );
    }

    /// The flight-side twin of the London rail case, captured live 2026-08-12:
    /// Ryanair FR2353 CGN 08:15 CEST -> STN 08:35 BST, durationSeconds 4800.
    /// Naive subtraction says 20 minutes; the UTC pair says 80, matching the
    /// API's own duration field.
    #[test]
    fn a_flight_segment_resolves_via_country_with_island_exceptions() {
        let dep = rfc3339_utc_airport("2026-08-20T08:15:00", "CGN", "Germany").unwrap();
        let arr = rfc3339_utc_airport("2026-08-20T08:35:00", "STN", "United Kingdom").unwrap();
        assert_eq!(dep, "2026-08-20T06:15:00Z");
        assert_eq!(arr, "2026-08-20T07:35:00Z");

        // An island airport wins over its country's mainland zone.
        assert_eq!(zone_for_airport("LPA", "Spain"), Some(Tz::Atlantic__Canary));
        assert_eq!(zone_for_airport("MAD", "Spain"), Some(Tz::Europe__Madrid));
        // A multi-zone country resolves to nothing, never a guess.
        assert_eq!(zone_for_airport("JFK", "United States"), None);
    }

    #[test]
    fn dst_boundaries_resolve_by_rule_not_by_fixed_offset() {
        // 2026-03-29 02:30 does not exist in Berlin (spring-forward gap).
        assert_eq!(
            utc_from_station_local("2026-03-29T02:30:00", "8000207"),
            None
        );
        // 2026-10-25 02:30 exists twice; the earlier instant (CEST, 00:30Z) wins.
        let folded = utc_from_station_local("2026-10-25T02:30:00", "8000207").unwrap();
        assert_eq!(folded.format("%H:%M").to_string(), "00:30");
        // London and Berlin change on the same date; the offset gap holds either side.
        let d = duration_between(
            "2026-10-24T09:00:00",
            "8000207",
            "2026-10-24T13:00:00",
            "7004428",
        )
        .unwrap();
        assert_eq!(d.num_hours(), 5);
    }
}
