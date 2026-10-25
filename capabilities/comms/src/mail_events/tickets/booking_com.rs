//! Booking.com stays. Two layouts carry one: the confirmation ("Your booking in <city> is
//! confirmed.", "<hotel> is expecting you", `Check-in <date>` on one line) and the receipt
//! (`Property name`, `Check-in` and their values on the next line, the city in the address).
//! Dates are English, day and month in either order.
//!
//! The reader answers for every Booking.com mail. A message from the property or an invoice
//! names no stay of its own, and a model reading one made "Check-in" and "Check-out" events in
//! a city called after the hotel. So such a mail has no events.

use super::{booking_key, iso_date_english, Event};

pub(super) fn legs(body: &str) -> Option<Vec<Event>> {
    if !body.contains("Booking.com") {
        return None;
    }
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    Some(stay(&lines).into_iter().collect())
}

fn stay(lines: &[&str]) -> Option<Event> {
    let (check_in_line, check_in) = value(lines, "Check-in")?;
    let (_, check_out) = value(lines, "Check-out")?;
    let hotel = lines
        .iter()
        .find_map(|l| l.split_once(" is expecting you").map(|(name, _)| name))
        .map(|name| name.trim_start_matches("[checkmark.png]").trim())
        .or_else(|| value(lines, "Property name").map(|(_, name)| name))
        .filter(|name| !name.is_empty())?;
    let reference = value(lines, "Confirmation:").or_else(|| value(lines, "Booking number"));
    Some(Event {
        title: format!("Stay at {hotel}"),
        date: Some(iso_date_english(check_in)?),
        end_date: Some(iso_date_english(check_out)?),
        location: Some(hotel.to_string()),
        city: city(lines).map(str::to_string),
        from_city: None,
        evidence: check_in_line.to_string(),
        booking_key: reference.map(|(_, r)| booking_key("booking.com", r)),
    })
}

/// The value of `label`: the rest of its line, or the next line when the label stands alone.
/// Returns the line the value was read from, for the grounding quote.
fn value<'a>(lines: &[&'a str], label: &str) -> Option<(&'a str, &'a str)> {
    lines.iter().enumerate().find_map(|(i, line)| {
        if *line == label {
            lines.get(i + 1).map(|next| (*next, *next))
        } else {
            line.strip_prefix(label)
                .filter(|rest| rest.starts_with(' '))
                .map(|rest| (*line, rest.trim()))
        }
    })
}

/// "Your booking in Hamburg is confirmed." or, in a receipt, the `Stuttgart, Germany` line of
/// the property address.
fn city<'a>(lines: &[&'a str]) -> Option<&'a str> {
    lines
        .iter()
        .find_map(|l| {
            let (_, rest) = l.split_once("Your booking in ")?;
            rest.split_once(" is confirmed")
                .map(|(city, _)| city.trim())
        })
        .or_else(|| {
            let at = lines.iter().position(|l| *l == "Property address")?;
            lines[at + 1..]
                .iter()
                .take(4)
                .find(|l| l.contains(", ") && !l.chars().any(|c| c.is_ascii_digit()))
                .and_then(|l| l.split(", ").next())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic, in the layout of a real confirmation.
    const CONFIRMATION: &str =
        "Booking.com\n   Confirmation: 1234567890\n   PIN: 0000 (Confidential)\n\n\
        Thanks Max! Your booking in Hamburg is confirmed.\n\n\
           [checkmark.png] Beispiel Hotel is expecting you on Thu 15 Oct\n\
        Reservation details\n\n\
           Check-in Thursday, 15 October 2026 (15:00 - 20:00)\n\
           Check-out Sunday, 18 October 2026 (until 11:00)\n\
           Your reservation 3 nights, Economy Double Room\n";

    /// Synthetic, in the layout of a real receipt for the same booking.
    const RECEIPT: &str = "Booking.com\nThis is your receipt\nBooking details\nBooking number\n\
        1234567890\nProperty name\nBeispiel Hotel\nProperty address\nMusterweg 1\n\
        Hamburg, Germany\n20000\nCheck-in\nThursday, October 15, 2026\nCheck-out\n\
        Sunday, October 18, 2026\nAmount paid on Sep 30, 2026\n";

    #[test]
    fn a_stay_runs_from_check_in_to_check_out() {
        let stay = &legs(CONFIRMATION).unwrap()[0];
        assert_eq!(stay.title, "Stay at Beispiel Hotel");
        assert_eq!(stay.date.as_deref(), Some("2026-10-15"));
        assert_eq!(stay.end_date.as_deref(), Some("2026-10-18"));
        assert_eq!(stay.city.as_deref(), Some("Hamburg"));
        assert!(stay
            .booking_key
            .as_deref()
            .is_some_and(|k| !k.contains("1234567890")));
        assert_eq!(
            crate::grounding::date_grounding(CONFIRMATION, &stay.evidence, "2026-10-15"),
            crate::grounding::DateGrounding::Supported
        );
    }

    #[test]
    fn the_receipt_is_the_same_stay() {
        let (confirmation, receipt) = (&legs(CONFIRMATION).unwrap()[0], &legs(RECEIPT).unwrap()[0]);
        assert_eq!(receipt.title, confirmation.title);
        assert_eq!(receipt.date, confirmation.date);
        assert_eq!(receipt.end_date, confirmation.end_date);
        assert_eq!(receipt.city.as_deref(), Some("Hamburg"));
        assert_eq!(receipt.booking_key, confirmation.booking_key);
        assert_eq!(
            crate::grounding::date_grounding(RECEIPT, &receipt.evidence, "2026-10-15"),
            crate::grounding::DateGrounding::Supported
        );
    }

    #[test]
    fn a_message_from_the_property_has_no_events() {
        let message =
            "Booking.com\nYou have a message from Beispiel Hotel\nCheck-in is from 3 pm.\n";
        assert_eq!(legs(message).map(|events| events.len()), Some(0));
        assert!(legs("A newsletter").is_none());
    }
}
