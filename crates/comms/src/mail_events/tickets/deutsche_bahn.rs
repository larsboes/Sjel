//! Deutsche Bahn booking confirmations, read without a model.
//!
//! The confirmation lists each leg under "Leistungen" / "Services" as `von|from`, the station,
//! an optional `dd.mm.yyyy hh:mm` line, then `nach|to`, the station and its optional date.
//! A booking without a fixed train (Flexpreis) has no date lines.

use super::{booking_key, iso_date, leg, local_timestamp, Event};

pub(super) fn legs(body: &str) -> Option<Vec<Event>> {
    if !body.contains("Deutsche Bahn") {
        return None;
    }
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let start = lines
        .iter()
        .position(|l| *l == "Leistungen" || *l == "Services")?;
    // "Auftragsnummer 230843307334" / "order number 628236251080".
    let key = lines.iter().find_map(|l| {
        let (_, rest) = l
            .split_once("Auftragsnummer ")
            .or_else(|| l.split_once("order number "))?;
        rest.split_whitespace()
            .next()
            .map(|reference| booking_key("deutschebahn.com", reference))
    });
    let block = &lines[start + 1..];
    let mut events = Vec::new();
    let mut i = 0;
    while i < block.len() {
        if !matches!(block[i], "von" | "from") {
            i += 1;
            continue;
        }
        let (from, from_date, next) = stop(block, i + 1)?;
        if !matches!(block.get(next), Some(&("nach" | "to"))) {
            return None;
        }
        let (to, to_date, next) = stop(block, next + 1)?;
        let mut event = leg(
            from,
            to,
            city_of_station(from),
            city_of_station(to),
            from_date.or(to_date),
        );
        event.starts_at = local_timestamp(from_date);
        event.ends_at = local_timestamp(to_date);
        events.push(Event {
            booking_key: key.clone(),
            ..event
        });
        i = next;
    }
    (!events.is_empty()).then_some(events)
}

/// A station at `at` and the date line after it, if any, plus the index after both.
fn stop<'a>(lines: &[&'a str], at: usize) -> Option<(&'a str, Option<&'a str>, usize)> {
    let station = lines.get(at)?.trim_end_matches(',').trim();
    if station.is_empty() {
        return None;
    }
    match lines.get(at + 1).filter(|l| iso_date(l).is_some()) {
        Some(line) => Some((station, Some(*line), at + 2)),
        None => Some((station, None, at + 1)),
    }
}

/// `Berlin Hbf` is Berlin, `Frankfurt(Main)Hbf` is Frankfurt: Calendar compares cities.
/// ponytail: strips "Hbf" and a bracketed suffix only; `Köln Messe/Deutz` stays as written.
fn city_of_station(station: &str) -> String {
    let name = station.split('(').next().unwrap_or(station);
    let name = name.trim().trim_end_matches("Hbf").trim();
    if name.is_empty() {
        station.to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic, in the layout of a real confirmation: the English one with dates.
    const ENGLISH: &str = "Here is your booking confirmation.\nHello Mr Max Muster,\n\
        thank you for your booking with Deutsche Bahn.\nThe order with the order number 100000000000 \
        was booked on 01.11.2026 at 10:00 as follows:\nServices\nSuper Sparpreis, 2. Klasse\nfrom\n\
        Bonn Hbf,\n06.11.2026 21:14\nto\nBerlin Hbf,\n07.11.2026 06:15\n\
        The total amount is paid using the following payment methods:\nTotal amount:\n10,00 EUR\n";

    /// The German one for a ticket without a fixed train: no date lines at all.
    const GERMAN_UNDATED: &str =
        "Hier Ihre Buchungsbestätigung.\nvielen Dank für Ihre Buchung bei \
        der Deutschen Bahn.\nLeistungen\nFlexpreis, 2. Klasse\nvon\nHamburg Hbf\nnach\nKöln Hbf\n\
        Die Zahlung des Gesamtpreises erfolgt über folgende Zahlungsmittel:\n© Deutsche Bahn AG\n";

    #[test]
    fn a_dated_leg_reads_its_departure_date_and_cities() {
        let legs = legs(ENGLISH).unwrap();
        assert_eq!(legs.len(), 1);
        let leg = &legs[0];
        assert_eq!(leg.title, "Bonn Hbf → Berlin Hbf");
        assert_eq!(leg.date.as_deref(), Some("2026-11-06"));
        assert_eq!(leg.city.as_deref(), Some("Berlin"));
        assert_eq!(leg.from_city.as_deref(), Some("Bonn"));
        assert_eq!(
            crate::grounding::date_grounding(ENGLISH, &leg.evidence, "2026-11-06"),
            crate::grounding::DateGrounding::Supported
        );
    }

    #[test]
    fn an_undated_leg_is_kept_without_a_date() {
        let legs = legs(GERMAN_UNDATED).unwrap();
        assert_eq!(legs[0].title, "Hamburg Hbf → Köln Hbf");
        assert_eq!(legs[0].date, None);
        assert_eq!(legs[0].city.as_deref(), Some("Köln"));
    }

    #[test]
    fn a_return_booking_is_two_legs() {
        let body = ENGLISH.replace(
            "07.11.2026 06:15\n",
            "07.11.2026 06:15\nfrom\nBerlin Hbf,\n09.11.2026 18:00\nto\nBonn Hbf,\n09.11.2026 23:30\n",
        );
        let legs = legs(&body).unwrap();
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[1].date.as_deref(), Some("2026-11-09"));
        assert_eq!(legs[1].city.as_deref(), Some("Bonn"));
        assert_eq!(legs[1].from_city.as_deref(), Some("Berlin"));
    }

    #[test]
    fn other_mail_goes_to_the_model() {
        assert!(legs("Eurowings Buchungsbestätigung\nFlug:\n05.12.2026").is_none());
        assert!(legs("Deutsche Bahn newsletter\nNeuer Fahrplan").is_none());
        assert_eq!(city_of_station("Frankfurt(Main)Hbf"), "Frankfurt");
    }
}
