//! Eurowings booking confirmations. Under "Flugdaten" each flight is `Flug:`, a `dd.mm.yyyy`
//! line, the flight number, then `Abflug`, the time, `Uhr`, the airport, and `Ankunft`, the time,
//! `Uhr`, the airport. Times are local.

use super::{airport_city, booking_key, iso_date, leg, Event};

pub(super) fn legs(body: &str) -> Option<Vec<Event>> {
    if !body.contains("Eurowings") {
        return None;
    }
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    // The reference is the line after "Booking Confirmation".
    let key = lines
        .iter()
        .position(|l| *l == "Booking Confirmation")
        .and_then(|at| lines.get(at + 1))
        .filter(|reference| !reference.is_empty())
        .map(|reference| booking_key("eurowings.com", reference));
    let mut events = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if *line != "Flug:" {
            continue;
        }
        let Some(date_line) = lines.get(i + 1).filter(|l| iso_date(l).is_some()) else {
            continue;
        };
        let from = airport_after(&lines[i..], "Abflug")?;
        let to = airport_after(&lines[i..], "Ankunft")?;
        events.push(Event {
            booking_key: key.clone(),
            ..leg(
                from,
                to,
                airport_city(from),
                airport_city(to),
                Some(date_line),
            )
        });
    }
    (!events.is_empty()).then_some(events)
}

/// The airport after the first `label` line: the first line past the time and `Uhr`.
fn airport_after<'a>(lines: &[&'a str], label: &str) -> Option<&'a str> {
    let at = lines.iter().position(|l| *l == label)?;
    lines[at + 1..]
        .iter()
        .copied()
        .find(|l| !l.is_empty() && *l != "Uhr" && !is_time(l))
}

/// `06:50`.
fn is_time(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 2 || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic, in the layout of a real confirmation: an outward and a return flight.
    const BOOKING: &str = "Booking Confirmation\nABCDEF\nBuchungsbestätigung\nHallo Max Muster,\n\
        herzlichen Dank für deine Buchung bei Eurowings.\nTag der Buchung: 01.11.2026 10:00\n\
        Flugdaten (Zeiten sind Ortszeiten)\nFlug:\n05.12.2026\n| Flugnummer:\nEW\n470\n\
        ( BASIC \\\nE\n)\nAbflug\n06:50\nUhr\nKöln-Bonn\nAnkunft\n07:10\nUhr\nGatwick Airport\n\
        Flug:\n08.12.2026\n| Flugnummer:\nEW\n471\n( BASIC \\\nT\n)\nAbflug\n07:55\nUhr\n\
        Gatwick Airport\nAnkunft\n10:15\nUhr\nKöln-Bonn\nFluggäste:\n";

    #[test]
    fn an_outward_and_a_return_flight_are_two_legs() {
        let legs = legs(BOOKING).unwrap();
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0].title, "Köln-Bonn → Gatwick Airport");
        assert_eq!(legs[0].date.as_deref(), Some("2026-12-05"));
        assert_eq!(legs[0].city.as_deref(), Some("London"));
        assert_eq!(legs[1].date.as_deref(), Some("2026-12-08"));
        assert_eq!(legs[1].city.as_deref(), Some("Köln, Bonn"));
        assert_eq!(legs[1].from_city.as_deref(), Some("London"));
        assert_eq!(
            crate::grounding::date_grounding(BOOKING, &legs[1].evidence, "2026-12-08"),
            crate::grounding::DateGrounding::Supported
        );
    }

    #[test]
    fn the_booking_date_is_not_a_flight() {
        let only_booking = "Eurowings\nTag der Buchung: 01.11.2026 10:00\n";
        assert!(legs(only_booking).is_none());
    }
}
