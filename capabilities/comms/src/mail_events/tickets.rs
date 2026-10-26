//! Booking confirmations read without a model: one reader per sender, tried in order.
//!
//! A reader recognises its own sender and returns every leg it finds, or `None`, so the next
//! reader and finally the local model can try. A reader keeps only its sender's layout; a shape
//! two readers share moves here.

use super::Event;

mod booking_com;
mod deutsche_bahn;
mod eurowings;

/// Reads one sender's confirmation, or answers `None` for any other mail.
type Reader = fn(&str) -> Option<Vec<Event>>;

/// Every reader. A new sender is one file and one line here.
const READERS: &[Reader] = &[deutsche_bahn::legs, eurowings::legs, booking_com::legs];

/// The legs of the first reader that recognises `body`. A leg without a date is kept with
/// `date: None`, so the caller records `date_missing` instead of asking the model to guess.
pub(super) fn legs(body: &str) -> Option<Vec<Event>> {
    READERS.iter().find_map(|read| read(body))
}

/// Reads an invitation with explicit `Event details`, `Date`, `Time` and `Location` labels.
pub(super) fn explicit_invitation(body: &str, title: Option<&str>) -> Option<Vec<Event>> {
    if !body.to_ascii_lowercase().contains("event details") {
        return None;
    }
    let field = |name: &str| {
        body.lines().map(str::trim).find_map(|line| {
            let (label, value) = line.split_once(':')?;
            label.eq_ignore_ascii_case(name).then_some(value.trim())
        })
    };
    let date_line = field("Date")?;
    let time_line = field("Time")?;
    let location = field("Location")?;
    let date = iso_date_english(date_line)?;
    let (start_time, end_time) = time_range(time_line)?;
    let evidence = format!("Date: {date_line}\nTime: {time_line}");
    let city = postal_city(location);
    let title = title
        .map(invitation_title)
        .filter(|title| !title.is_empty())
        .or_else(|| city.as_ref().map(|city| format!("Event in {city}")))?;
    Some(vec![Event {
        title,
        date: Some(date.clone()),
        starts_at: Some(format!("{date}T{start_time}")),
        ends_at: Some(format!("{date}T{end_time}")),
        location: Some(location.to_owned()),
        agenda: Vec::new(),
        city,
        from_city: None,
        evidence,
        end_date: None,
        booking_key: None,
    }])
}

fn invitation_title(subject: &str) -> String {
    let subject = subject.trim();
    let subject = ["Fw:", "Fwd:", "Re:"]
        .iter()
        .find_map(|prefix| subject.strip_prefix(prefix))
        .unwrap_or(subject)
        .trim();
    let subject = subject
        .strip_prefix("See you next week at ")
        .unwrap_or(subject);
    subject.trim().trim_end_matches([' ', '.', '!']).to_owned()
}

fn time_range(value: &str) -> Option<(&str, &str)> {
    let (start, end) = value
        .split_once(" - ")
        .or_else(|| value.split_once(" – "))
        .or_else(|| value.split_once(" — "))?;
    [start.trim(), end.trim()]
        .iter()
        .all(|time| {
            time.len() == 5
                && time.as_bytes()[2] == b':'
                && time.bytes().enumerate().all(|(index, byte)| {
                    if index == 2 {
                        byte == b':'
                    } else {
                        byte.is_ascii_digit()
                    }
                })
                && time[..2].parse::<u8>().is_ok_and(|hour| hour < 24)
                && time[3..].parse::<u8>().is_ok_and(|minute| minute < 60)
        })
        .then_some((start.trim(), end.trim()))
}

fn postal_city(location: &str) -> Option<String> {
    location.split(',').find_map(|part| {
        let mut words = part.split_whitespace();
        let postal_code = words.next()?;
        (matches!(postal_code.len(), 4..=6)
            && postal_code.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| words.collect::<Vec<_>>().join(" "))
        .filter(|city| !city.is_empty())
    })
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
        starts_at: None,
        ends_at: None,
        location: Some(to.to_string()),
        agenda: Vec::new(),
        city: Some(city),
        from_city: Some(from_city),
        evidence,
        end_date: None,
        booking_key: None,
    }
}

/// Parses a German ticket's `dd.mm.yyyy HH:MM` local timestamp.
pub(super) fn local_timestamp(line: Option<&str>) -> Option<String> {
    let line = line?;
    let mut parts = line.split_whitespace();
    let date = iso_date(parts.next()?)?;
    let time = parts.next()?;
    let bytes = time.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            if index == 2 {
                *byte == b':'
            } else {
                byte.is_ascii_digit()
            }
        })
        || time[..2].parse::<u8>().ok().is_none_or(|hour| hour >= 24)
        || time[3..]
            .parse::<u8>()
            .ok()
            .is_none_or(|minute| minute >= 60)
    {
        return None;
    }
    Some(format!("{date}T{time}"))
}

/// SHA-256 of `sender` and `reference`, so a booking is recognised again without its reference
/// being stored.
fn booking_key(sender: &str, reference: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("{sender}\0{}", reference.trim()).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// `Thursday, 15 October 2026 (15:00 - 20:00)` or `Thursday, October 8, 2026 (from 3:00 PM)`
/// as `2026-10-15`: an English date, day and month in either order, anything in brackets ignored.
fn iso_date_english(text: &str) -> Option<String> {
    let text = text.split('(').next()?;
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .collect();
    let month = words.iter().find_map(|w| {
        MONTHS
            .iter()
            .position(|m| m.eq_ignore_ascii_case(w))
            .map(|i| i + 1)
    })?;
    let numbers: Vec<u32> = words
        .iter()
        .filter_map(|word| {
            ["st", "nd", "rd", "th"]
                .iter()
                .find_map(|suffix| word.strip_suffix(suffix))
                .unwrap_or(word)
                .parse()
                .ok()
        })
        .collect();
    let year = numbers.iter().find(|n| **n >= 1000)?;
    let day = numbers.iter().find(|n| (1..=31).contains(*n))?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
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
    fn an_english_date_reads_in_either_order() {
        let uk = "Thursday, 15 October 2026 (15:00 - 20:00)";
        let us = "Thursday, October 8, 2026 (from 3:00 PM)";
        assert_eq!(iso_date_english(uk).as_deref(), Some("2026-10-15"));
        assert_eq!(iso_date_english(us).as_deref(), Some("2026-10-08"));
        assert_eq!(iso_date_english("sometime soon"), None);
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

    #[test]
    fn an_explicit_invitation_keeps_its_date_and_time_range() {
        let body = "Event details\nDate: Thursday, 15 October 2026\nTime: 15:00 - 20:00\n\
                    Location: Messe Berlin, 14055 Berlin\n";
        let events = explicit_invitation(body, Some("Fw: See you next week at OpenAI DevDay"))
            .expect("a labelled invitation is an event");
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.title, "OpenAI DevDay");
        assert_eq!(event.date.as_deref(), Some("2026-10-15"));
        assert_eq!(event.starts_at.as_deref(), Some("2026-10-15T15:00"));
        assert_eq!(event.ends_at.as_deref(), Some("2026-10-15T20:00"));
        assert_eq!(event.city.as_deref(), Some("Berlin"));
        assert!(event.evidence.contains("15:00 - 20:00"));
        assert!(event.agenda.is_empty());
    }

    #[test]
    fn an_invitation_without_the_labels_stays_with_the_model() {
        let no_time = "Event details\nDate: 15 October 2026\nLocation: Berlin";
        assert!(explicit_invitation(no_time, Some("Fw: An invitation")).is_none());
        let not_a_range = "Event details\nDate: 15 October 2026\nTime: 15:00\nLocation: Berlin";
        assert!(explicit_invitation(not_a_range, Some("Fw: An invitation")).is_none());
        let unlabelled = "Date: 15 October 2026\nTime: 15:00 - 20:00";
        assert!(explicit_invitation(unlabelled, Some("Fw: An invitation")).is_none());
    }

    #[test]
    fn a_subject_without_a_name_falls_back_to_the_location() {
        let body = "Event details\nDate: 15 October 2026\nTime: 15:00 - 20:00\n\
                    Location: Messe Berlin, 14055 Berlin\n";
        let events = explicit_invitation(body, Some("Fw:  ")).expect("the labels still date it");
        assert_eq!(events[0].title, "Event in Berlin");
    }
}
