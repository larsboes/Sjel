//! Booking confirmations read without a model: one reader per sender, tried in order.
//!
//! A reader recognises its own sender and returns every leg it finds, or `None`, so the next
//! reader and finally the local model can try. A reader keeps only its sender's layout; a shape
//! two readers share moves here.

use super::Event;

mod deutsche_bahn;
mod eurowings;

/// Reads one sender's confirmation, or answers `None` for any other mail.
type Reader = fn(&str) -> Option<Vec<Event>>;

/// Every reader. A new sender is one file and one line here.
const READERS: &[Reader] = &[deutsche_bahn::legs, eurowings::legs];

/// The legs of the first reader that recognises `body`. A leg without a date is kept with
/// `date: None`, so the caller records `date_missing` instead of asking the model to guess.
pub(super) fn legs(body: &str) -> Option<Vec<Event>> {
    READERS.iter().find_map(|read| read(body))
}

/// `25.10.2026 21:14 Uhr` as `2026-10-25`: a German date at the start of a line.
fn iso_date(line: &str) -> Option<String> {
    let date = line.split_whitespace().next()?;
    let mut parts = date.split('.');
    let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    (parts.next().is_none() && digits(d, 2) && digits(m, 2) && digits(y, 4))
        .then(|| format!("{y}-{m}-{d}"))
}

/// One leg. `date_line` is quoted as the evidence, so grounding checks the date against it.
fn leg(from: &str, to: &str, from_city: String, city: String, date_line: Option<&str>) -> Event {
    let date = date_line.and_then(iso_date);
    let evidence = match date_line {
        Some(line) if date.is_some() => line.to_string(),
        _ => format!("{from} {to}"),
    };
    Event {
        title: format!("{from} → {to}"),
        date,
        location: Some(to.to_string()),
        city: Some(city),
        from_city: Some(from_city),
        evidence,
    }
}

/// Airports by the name a confirmation prints, and the cities they serve. An airport that serves
/// two cities names both, so Calendar's home test, which reads each comma segment, matches either.
/// ponytail: only the airports seen in mail; add a row when a confirmation names a new one.
const AIRPORTS: &[(&str, &str)] = &[("Köln-Bonn", "Köln, Bonn"), ("Gatwick Airport", "London")];

/// The city an airport serves. An unknown airport keeps its name, less "Airport".
fn airport_city(name: &str) -> String {
    AIRPORTS
        .iter()
        .find(|(airport, _)| airport.eq_ignore_ascii_case(name))
        .map(|(_, city)| city.to_string())
        .unwrap_or_else(|| name.trim_end_matches("Airport").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_read_only_at_the_start_of_a_line() {
        assert_eq!(
            iso_date("25.10.2026 21:14 Uhr").as_deref(),
            Some("2026-10-25")
        );
        assert_eq!(iso_date("Tag der Buchung: 23.10.2026"), None);
        assert_eq!(iso_date("5.1.2026"), None);
    }

    #[test]
    fn an_airport_for_two_cities_names_both() {
        assert_eq!(airport_city("Köln-Bonn"), "Köln, Bonn");
        assert_eq!(airport_city("Gatwick Airport"), "London");
        assert_eq!(airport_city("Palma Airport"), "Palma");
    }

    #[test]
    fn the_first_reader_that_knows_the_sender_answers() {
        assert!(legs("A newsletter with no booking in it").is_none());
    }
}
