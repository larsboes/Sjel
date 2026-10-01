//! Pure-Rust PDF/email/regex parsing of German rail ticket confirmations.
//! Ported close to as-is -- this part of the original service was already
//! well-built (10 unit tests, clean error handling) and needed no DB, no
//! network, and no redaction (no personal data in the parsing logic itself).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedLeg {
    pub origin_name: String,
    pub destination_name: String,
    pub train_number: Option<String>,
    pub departure_time: String,
    pub arrival_time: String,
    pub price: Option<f64>,
    /// `Some("flight")` when the airline path produced this leg; absent on
    /// rail parses, so stored rail extractions read back unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTicket {
    pub source_file: String,
    pub text_preview: String,
    pub legs: Vec<ExtractedLeg>,
    pub confirmation_number: Option<String>,
    pub meta: ExtractionMeta,
    /// Whether this parse produced at least one leg to review. Not "the file was
    /// readable": a confirmation whose stations did not parse is readable and
    /// useless, and those are the same value to any caller that only checks a
    /// boolean.
    pub ok: bool,
    /// The fields this parse could not find, by name.
    #[serde(default)]
    pub missing: Vec<String>,
    /// Which reader produced the text, so a reader of the reply can tell a
    /// layout-aware parse from a flattened one without guessing.
    #[serde(default = "default_backend")]
    pub backend: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionMeta {
    pub trains_found: Vec<String>,
    pub stations: StationPair,
    pub has_date: bool,
    pub has_price: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationPair {
    pub origin: Option<String>,
    pub destination: Option<String>,
}

static TRAIN_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(ICE\s*\d+)").unwrap(),
        Regex::new(r"(?i)(IC\s*\d+)").unwrap(),
        Regex::new(r"(?i)(EC\s*\d+)").unwrap(),
        Regex::new(r"(?i)(RE\s*\d+)").unwrap(),
        Regex::new(r"(?i)(RB\s*\d+)").unwrap(),
        Regex::new(r"(?i)(IRE\s*\d+)").unwrap(),
        Regex::new(r"(?i)\bS\s*(\d+)").unwrap(),
        Regex::new(r"(?i)(FLX\s*\d+)").unwrap(),
        Regex::new(r"(?i)(NJ\s*\d+)").unwrap(),
        Regex::new(r"(?i)(EN\s*\d+)").unwrap(),
    ]
});

static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{2})\.(\d{2})\.(\d{4})").unwrap());
static ISO_DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap());
static TIME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{2}):(\d{2})").unwrap());

static CONFIRMATION_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(?:Buchungsnummer|Buchungscode|Auftragsnummer)\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
        Regex::new(r"(?i)(?:Booking\s*.?code|Booking\s*.?number|Confirmation|Reference)\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
        Regex::new(r"(?i)Ticket\s*[:\-]?\s*([A-Z0-9\-]{6,30})").unwrap(),
    ]
});

static VON_NACH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:von|from|ab)\s+([A-Za-zÄÖÜäöüß\s.\-()]+?)\s+(?:nach|to|an|um)\s+([A-Za-zÄÖÜäöüß\s.\-()]+?)(?:\s+(?:am|um|ab|\d)|$)").unwrap()
});
// `Von` was missing here while `Nach` was already in TO_RE below, so a German
// confirmation laid out as "Von: ... / Nach: ..." parsed its destination and lost
// its origin -- and with one station missing, `parse_ticket_text` builds no legs
// at all. German rail confirmations are this parser's entire subject, so the one
// label it could not read was the one it most needed.
static FROM_TO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:From|Von|Ab|Start|Origin)\s*:\s*(.+)$").unwrap());
static TO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:To|Nach|End|Destination|Ziel)\s*:\s*(.+)$").unwrap());

/// One row of a journey table: a date, a departure time, a station, an arrival
/// time, a station, and a train.
///
/// This exists because no reader recovers that table as a table. `pdf_extract`
/// flattens it into a line; xberg's native PDF path produces the same run-on
/// line; its OCR path emits one cell per line. The row *shape* survives all
/// three, so matching the shape works where matching the layout does not.
///
/// It is also the only thing that tells data from a header. Reading stations
/// with `VON_NACH_RE` on flattened table text produced two legs running from
/// "Bahnhof" to "Bahnhof Zug Gleis" -- the header row -- and reported success.
static TABLE_ROW_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (\d{2}\.\d{2}\.\d{4})      \s+   # date
        (\d{2}:\d{2})                \s+   # departure
        ([A-Za-zÄÖÜäöüß][A-Za-zÄÖÜäöüß0-9\s.\-/()]{1,40}?) \s+
        (\d{2}:\d{2})                \s+   # arrival
        ([A-Za-zÄÖÜäöüß][A-Za-zÄÖÜäöüß0-9\s.\-/()]{1,40}?) \s+
        ((?:ICE|IC|EC|RE|RB|IRE|FLX|NJ|EN|S)\s*\d+)   # train
        ",
    )
    .unwrap()
});

static PRICE_RE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(?:Preis|Price|Total|Sum|Betrag)\s*[:\-]?\s*(\d+[.,]\d{2})").unwrap(),
        Regex::new(r"(\d+[.,]\d{2})\s*[€€]").unwrap(),
        Regex::new(r"[€€]\s*(\d+[.,]\d{2})").unwrap(),
        Regex::new(r"(\d+[.,]\d{2})\s*EUR").unwrap(),
    ]
});

/// Reads a ticket with the configured backend and parses it.
///
/// Which reader runs is config, not a hardcoded match on the file extension:
/// see `crate::document`. That is also where the image path now lives, so this
/// function no longer refuses one on principle.
pub fn extract_from_bytes(bytes: &[u8], file_name: &str) -> Result<ExtractedTicket, String> {
    let config = crate::config::Config::load();
    extract_with_config(bytes, file_name, &config)
}

pub fn extract_with_config(
    bytes: &[u8],
    file_name: &str,
    config: &crate::config::Config,
) -> Result<ExtractedTicket, String> {
    let document = crate::document::read(bytes, file_name, config)?;
    // Parse from the richest representation the backend produced.
    //
    // Not a preference, a measured requirement: xberg's plain-text mode
    // reorders a PDF's content, putting the dates and times of a journey table
    // twenty lines away from the station names, which leaves the row pattern
    // nothing contiguous to match. Its Markdown mode keeps reading order. The
    // builtin reader has no Markdown at all and its text is already in order,
    // so `unwrap_or` is the whole compatibility story.
    let source = document.markdown.as_deref().unwrap_or(&document.text);
    let mut ticket = parse_ticket_text(source, file_name);
    ticket.backend = document.producer;
    Ok(ticket)
}

pub(crate) fn extract_pdf_text(
    bytes: &[u8],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let text = pdf_extract::extract_text_from_mem(bytes)?;
    Ok(text)
}

const HTML_TEXT_WIDTH: usize = 10_000;

pub(crate) fn extract_email_text(
    bytes: &[u8],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = mailparse::parse_mail(bytes)?;
    let ct = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("content-type"))
        .and_then(|h| {
            let val = h.get_value();
            if val.is_empty() {
                None
            } else {
                Some(val)
            }
        });

    let is_html = ct.as_deref().is_some_and(|v| v.contains("text/html"));

    let body = if is_html {
        let html = parsed.get_body()?;
        // Wide enough that no booking line wraps: the parsers below match a station pair or a
        // date and time within one line. html2text 0.17 refuses the width 0 that 0.6 accepted.
        html2text::from_read(html.as_bytes(), HTML_TEXT_WIDTH)?
    } else {
        parsed.get_body()?
    };

    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("subject"))
        .map(|h| h.get_value());

    let from = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("from"))
        .map(|h| h.get_value());

    let mut text = String::new();
    if let Some(s) = &subject {
        text.push_str(&format!("Subject: {s}\n"));
    }
    if let Some(f) = &from {
        text.push_str(&format!("From: {f}\n"));
    }
    text.push_str(&body);
    Ok(text)
}

fn parse_ticket_text(text: &str, file_name: &str) -> ExtractedTicket {
    let text_preview = if text.len() > 500 {
        format!("{}...", &text[..500])
    } else {
        text.to_string()
    };

    let trains = parse_trains(text);
    let stations = parse_stations(text);
    let (departure, arrival) = parse_date_time(text);
    let confirmation = parse_confirmation(text);
    let price = parse_price(text);

    // An airline confirmation is its own genre and is checked first: its
    // markers are unambiguous, and its flight lines would otherwise feed the
    // rail fallback one useless origin/destination pair. A journey table,
    // when there is one, beats every other rail signal.
    let airline_legs = parse_airline_legs(text, price);
    let table_legs = parse_table_rows(text);

    let legs = if !airline_legs.is_empty() {
        airline_legs
    } else if !table_legs.is_empty() {
        table_legs
    } else if !trains.is_empty() && stations.origin.is_some() && stations.destination.is_some() {
        let default_time = format!("{}T00:00:00", chrono_now_date());
        let dep = departure.as_deref().unwrap_or(&default_time).to_string();
        let arr = arrival.as_deref().unwrap_or(&dep).to_string();

        trains
            .iter()
            .map(|train| ExtractedLeg {
                origin_name: stations.origin.clone().unwrap_or_default(),
                destination_name: stations.destination.clone().unwrap_or_default(),
                train_number: Some(train.clone()),
                departure_time: dep.clone(),
                arrival_time: arr.clone(),
                price,
                mode: None,
            })
            .collect()
    } else {
        vec![]
    };

    // `ok` was the literal `true`, so a parse that found a train number and
    // nothing else reported success with an empty leg list. A realistic DB
    // confirmation does exactly that: on a 2026-08-11 check, the order number and
    // `ICE 1513` parsed while both station names did not, and the reply still
    // said ok. A caller cannot tell that apart from a ticket with no legs.
    //
    // `ok` now means "there is at least one leg here to review". `missing` says
    // what a reviewer has to supply, because "not ok" with no reason sends them
    // back to the PDF to work out which half failed.
    let ok = !legs.is_empty();
    let missing = missing_fields(&trains, &stations, departure.is_some(), price.is_some());

    ExtractedTicket {
        source_file: file_name.to_string(),
        text_preview,
        legs,
        confirmation_number: confirmation,
        meta: ExtractionMeta {
            trains_found: trains,
            stations,
            has_date: departure.is_some(),
            has_price: price.is_some(),
        },
        ok,
        missing,
        backend: default_backend(),
        error: None,
    }
}

fn default_backend() -> &'static str {
    "builtin"
}

/// What a parse could not find, named for whoever has to fill it in.
fn missing_fields(
    trains: &[String],
    stations: &StationPair,
    has_date: bool,
    has_price: bool,
) -> Vec<String> {
    let mut missing = Vec::new();
    if trains.is_empty() {
        missing.push("trains".to_string());
    }
    if stations.origin.is_none() {
        missing.push("origin".to_string());
    }
    if stations.destination.is_none() {
        missing.push("destination".to_string());
    }
    if !has_date {
        missing.push("date".to_string());
    }
    if !has_price {
        missing.push("price".to_string());
    }
    missing
}

/// Airline-confirmation markers. The whole airline path only runs when one of
/// these identifies the document, so a rail confirmation can never trip the
/// looser flight patterns below.
static AIRLINE_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:eurowings|ryanair|lufthansa|easyjet|wizz\s*air|condor)\b").unwrap()
});

/// One flight header as Eurowings renders it: a date, then the flight number,
/// then the fare family. Matched on flattened text because every HTML reader
/// flattens the layout differently while the label words survive them all
/// (structure verified against a real Buchungsbestätigung, 2026-08-12).
static FLIGHT_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        Flug:?\s*(\d{2})\.(\d{2})\.(\d{4})        # leg date
        [\s|]* Flugnummer:?\s*
        ([A-Z]{1,3})\s*(\d{2,4})                  # carrier + number
    ",
    )
    .unwrap()
});

/// The departure/arrival pair under each flight header: times are local
/// ("Zeiten sind Ortszeiten"), stations are city names.
static FLIGHT_TIMES_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        Abflug\s*(\d{2}:\d{2})\s*Uhr\s*
        ([A-Za-zÄÖÜäöüß][A-Za-zÄÖÜäöüß\s.\-]{1,30}?)
        [\s|]+ Ankunft\s*(\d{2}:\d{2})\s*Uhr\s*
        ([A-Za-zÄÖÜäöüß][A-Za-zÄÖÜäöüß\s.\-]{1,30}?)
        (?:[\s|]|$)
    ",
    )
    .unwrap()
});

/// Reads flight legs off an airline booking confirmation.
///
/// Flight headers and their departure/arrival pairs appear in document order,
/// so they are zipped by position. The arrival is stamped with the leg's own
/// date: this format prints no arrival date, so an overnight arrival would be
/// off by a day -- a property of the source document, not recoverable here.
fn parse_airline_legs(text: &str, price: Option<f64>) -> Vec<ExtractedLeg> {
    if !AIRLINE_MARKER_RE.is_match(text) {
        return Vec::new();
    }
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let headers: Vec<_> = FLIGHT_LINE_RE.captures_iter(&flattened).collect();
    let times: Vec<_> = FLIGHT_TIMES_RE.captures_iter(&flattened).collect();
    headers
        .iter()
        .zip(times.iter())
        .map(|(header, time)| {
            let (day, month, year) = (&header[1], &header[2], &header[3]);
            let flight = format!("{} {}", &header[4], &header[5]);
            let date = format!("{year}-{month}-{day}");
            ExtractedLeg {
                origin_name: time[2].trim().to_string(),
                destination_name: time[4].trim().to_string(),
                train_number: Some(flight),
                departure_time: format!("{date}T{}:00", &time[1]),
                arrival_time: format!("{date}T{}:00", &time[3]),
                price,
                mode: Some("flight".to_string()),
            }
        })
        .collect()
}

/// Reads legs off a journey table, one row at a time.
///
/// Rows are matched on a whole-document basis rather than line by line, because
/// the readers disagree about where the lines are: one puts a whole table on a
/// single line, another puts one cell per line. Collapsing whitespace first
/// makes both look the same to the row pattern.
fn parse_table_rows(text: &str) -> Vec<ExtractedLeg> {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    TABLE_ROW_RE
        .captures_iter(&flattened)
        .map(|row| {
            let date = iso_date(&row[1]);
            ExtractedLeg {
                origin_name: row[3].trim().to_string(),
                destination_name: row[5].trim().to_string(),
                train_number: Some(row[6].split_whitespace().collect::<Vec<_>>().join(" ")),
                departure_time: format!("{date}T{}:00", &row[2]),
                arrival_time: format!("{date}T{}:00", &row[4]),
                price: None,
                mode: None,
            }
        })
        .collect()
}

/// `DD.MM.YYYY` to `YYYY-MM-DD`.
fn iso_date(german: &str) -> String {
    let mut parts = german.split('.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(d), Some(m), Some(y)) => format!("{y}-{m}-{d}"),
        _ => german.to_string(),
    }
}

fn parse_trains(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for re in TRAIN_RE.iter() {
        for cap in re.captures_iter(text) {
            let t = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if !t.is_empty() {
                found.push(t.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

fn parse_stations(text: &str) -> StationPair {
    let mut origin: Option<String> = None;
    let mut destination: Option<String> = None;

    if let Some(cap) = VON_NACH_RE.captures(text) {
        if origin.is_none() {
            origin = cap.get(1).map(|m| m.as_str().trim().to_string());
        }
        if destination.is_none() {
            destination = cap.get(2).map(|m| m.as_str().trim().to_string());
        }
    }

    for line in text.lines() {
        if let Some(cap) = FROM_TO_RE.captures(line) {
            if origin.is_none() {
                origin = cap.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
        if let Some(cap) = TO_RE.captures(line) {
            if destination.is_none() {
                destination = cap.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
    }

    StationPair {
        origin,
        destination,
    }
}

fn parse_date_time(text: &str) -> (Option<String>, Option<String>) {
    let dates: Vec<String> = DATE_RE
        .captures_iter(text)
        .map(|c| format!("{}-{}-{}", &c[3], &c[2], &c[1]))
        .chain(
            ISO_DATE_RE
                .captures_iter(text)
                .map(|c| format!("{}-{}-{}", &c[1], &c[2], &c[3])),
        )
        .collect();

    let times: Vec<String> = TIME_RE
        .captures_iter(text)
        .map(|c| format!("{}:{}", &c[1], &c[2]))
        .collect();

    let departure = dates.first().map(|d| {
        let t = times
            .first()
            .map(|t| format!("T{t}:00"))
            .unwrap_or_else(|| "T00:00:00".into());
        format!("{d}{t}")
    });

    let arrival = if dates.len() > 1 && times.len() > 1 {
        Some(format!("{}T{}:00", dates[1], times[1]))
    } else if times.len() > 1 {
        dates.first().map(|d| format!("{}T{}:00", d, times[1]))
    } else {
        None
    };

    (departure, arrival)
}

fn parse_confirmation(text: &str) -> Option<String> {
    for re in CONFIRMATION_RE.iter() {
        if let Some(cap) = re.captures(text) {
            let val = cap.get(1).map(|m| m.as_str().trim().to_string());
            if val.as_ref().is_some_and(|v| !v.is_empty()) {
                return val;
            }
        }
    }
    None
}

fn parse_price(text: &str) -> Option<f64> {
    for re in PRICE_RE.iter() {
        if let Some(cap) = re.captures(text) {
            let raw = cap.get(1).map(|m| m.as_str().replace(',', "."));
            if let Some(Ok(v)) = raw.map(|s| s.parse::<f64>()) {
                if v > 0.0 {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn chrono_now_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let y = 1970 + (days * 400 + 365) / 146097;
    format!("{y}-01-01")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_html_booking_mail_keeps_each_line_whole() {
        let eml = "Content-Type: text/html; charset=utf-8\r\nSubject: Ihre Buchung\r\n\r\n\
            <html><body><p>ICE 123 von Berlin Hbf nach M&uuml;nchen Hbf</p>\
            <p>am 15.07.2026 um 08:30, an 14:15</p></body></html>";
        let text = extract_email_text(eml.as_bytes()).unwrap();
        assert!(
            text.contains("ICE 123 von Berlin Hbf nach München Hbf"),
            "{text}"
        );
        assert!(text.contains("am 15.07.2026 um 08:30, an 14:15"), "{text}");
    }

    #[test]
    fn parse_train_numbers() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf";
        let trains = parse_trains(text);
        assert_eq!(trains, vec!["ICE 123"]);
    }

    #[test]
    fn parse_ic_number() {
        let text = "Your booking: IC 2044";
        let trains = parse_trains(text);
        assert!(trains.contains(&"IC 2044".to_string()));
    }

    #[test]
    fn parse_von_nach() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf";
        let s = parse_stations(text);
        assert_eq!(s.origin.as_deref(), Some("Berlin Hbf"));
        assert_eq!(s.destination.as_deref(), Some("München Hbf"));
    }

    #[test]
    fn parse_from_to_labels() {
        let text = "From: Hamburg Hbf\nTo: Frankfurt(Main)Hbf";
        let s = parse_stations(text);
        assert_eq!(s.origin.as_deref(), Some("Hamburg Hbf"));
        assert_eq!(s.destination.as_deref(), Some("Frankfurt(Main)Hbf"));
    }

    #[test]
    fn parse_date_and_times() {
        let text = "am 15.07.2026 um 08:30, an 14:15";
        let (dep, arr) = parse_date_time(text);
        assert_eq!(dep.as_deref(), Some("2026-07-15T08:30:00"));
        assert_eq!(arr.as_deref(), Some("2026-07-15T14:15:00"));
    }

    #[test]
    fn parse_iso_date() {
        let text = "Date: 2026-07-20";
        let (dep, _) = parse_date_time(text);
        assert!(dep.as_deref() == Some("2026-07-20T00:00:00"));
    }

    /// The exact text that exposed this: a realistic DB confirmation whose order
    /// number, train and price all parse while neither station does. It used to
    /// come back `ok: true` with an empty leg list, which no caller can tell
    /// apart from a ticket that legitimately has no legs.
    #[test]
    fn a_parse_with_no_legs_is_not_ok_and_says_what_is_missing() {
        let text = "Deutsche Bahn - Ihre Buchung\n\
                    Auftragsnummer: XJ4K7Q\n\
                    Hinfahrt am 25.08.2026\n\
                    ICE 1513  Bonn Hbf  ab 10:07\n\
                    Frankfurt(Main)Hbf an 11:49\n\
                    Preis: 68,47 EUR\n";
        let result = parse_ticket_text(text, "ticket.txt");

        assert!(
            result.legs.is_empty(),
            "precondition: this text yields no legs"
        );
        assert!(!result.ok, "a parse with no legs must not report ok");
        assert_eq!(result.confirmation_number.as_deref(), Some("XJ4K7Q"));
        assert_eq!(result.meta.trains_found, vec!["ICE 1513"]);
        assert!(result.meta.has_price, "the price does parse");

        // The reviewer is told which half failed rather than being sent back to
        // the PDF to work it out.
        assert!(result.missing.contains(&"origin".to_string()));
        assert!(result.missing.contains(&"destination".to_string()));
        assert!(!result.missing.contains(&"trains".to_string()));
        assert!(!result.missing.contains(&"price".to_string()));
    }

    /// The other direction, and the `Von:` fix at the same time. This exact
    /// layout used to lose its origin, because `From|Ab|Start|Origin` did not
    /// include the German label while `Nach` was already accepted for the
    /// destination. One missing station means no legs at all.
    #[test]
    fn a_german_labelled_ticket_parses_both_stations_and_stays_ok() {
        let text = "Von: Bonn Hbf\nNach: Frankfurt(Main)Hbf\n\
                    ICE 1513\nam 25.08.2026 um 10:07\nPreis: 68,47 EUR\n";
        let result = parse_ticket_text(text, "ticket.txt");
        assert_eq!(result.meta.stations.origin.as_deref(), Some("Bonn Hbf"));
        assert_eq!(
            result.meta.stations.destination.as_deref(),
            Some("Frankfurt(Main)Hbf")
        );
        assert!(!result.legs.is_empty());
        assert!(result.ok);
        assert!(
            result.missing.is_empty(),
            "nothing should be missing, got: {:?}",
            result.missing
        );
    }

    /// `Ziel:` is the other German label a confirmation uses for a destination.
    #[test]
    fn ziel_is_read_as_a_destination_label() {
        let s = parse_stations("Von: Köln Hbf\nZiel: Hamburg Hbf");
        assert_eq!(s.origin.as_deref(), Some("Köln Hbf"));
        assert_eq!(s.destination.as_deref(), Some("Hamburg Hbf"));
    }

    /// The bug this whole change exists for, with the real flattened text.
    ///
    /// `pdf_extract` turns a journey table into these lines. Reading stations
    /// with the von/nach patterns produced two legs from "Bahnhof" to "Bahnhof
    /// Zug Gleis" -- the header row -- and reported ok with nothing missing.
    #[test]
    fn a_journey_table_yields_its_own_rows_not_the_header() {
        let text = "Auftragsnummer  XJ4K7Q2\n\
                    Datum Ab Bahnhof An Bahnhof Zug Gleis\n\
                    25.08.2026 10:07 Bonn Hbf 10:31 Siegburg/Bonn RB 66 2\n\
                    25.08.2026 10:48 Siegburg/Bonn 11:49 Frankfurt(Main)Hbf ICE 1513 4\n\
                    Gesamtpreis 73,97 EUR\n";
        let result = parse_ticket_text(text, "ticket.pdf");

        assert_eq!(result.legs.len(), 2, "one leg per table row");
        assert_eq!(result.legs[0].origin_name, "Bonn Hbf");
        assert_eq!(result.legs[0].destination_name, "Siegburg/Bonn");
        assert_eq!(result.legs[0].train_number.as_deref(), Some("RB 66"));
        assert_eq!(result.legs[0].departure_time, "2026-08-25T10:07:00");
        assert_eq!(result.legs[0].arrival_time, "2026-08-25T10:31:00");
        assert_eq!(result.legs[1].origin_name, "Siegburg/Bonn");
        assert_eq!(result.legs[1].destination_name, "Frankfurt(Main)Hbf");
        assert_eq!(result.legs[1].train_number.as_deref(), Some("ICE 1513"));

        // The header must never become a station.
        for leg in &result.legs {
            assert!(
                !leg.origin_name.contains("Bahnhof Zug"),
                "read the header as a station: {leg:?}"
            );
            assert_ne!(leg.origin_name, "Bahnhof");
            assert_ne!(leg.destination_name, "Bahnhof");
        }
        assert!(result.ok);
    }

    /// The same rows as xberg's OCR path emits them: one cell per line. The row
    /// shape survives; the line breaks do not, which is why matching runs over
    /// collapsed whitespace rather than line by line.
    #[test]
    fn one_cell_per_line_parses_to_the_same_legs() {
        let text = "Datum\nAb\nBahnhof\nAn\nBahnhof\nZug\nGleis\n\
                    25.08.2026\n10:07\nBonn Hbf\n10:31\nSiegburg/Bonn\nRB 66\n2\n\
                    25.08.2026\n10:48\nSiegburg/Bonn\n11:49\nFrankfurt(Main)Hbf\nICE 1513\n4\n";
        let result = parse_ticket_text(text, "ticket.png");
        assert_eq!(result.legs.len(), 2);
        assert_eq!(result.legs[0].origin_name, "Bonn Hbf");
        assert_eq!(result.legs[1].destination_name, "Frankfurt(Main)Hbf");
    }

    /// A ticket with no table still parses the old way, so the fallback is not
    /// dead code and the German-label fix still holds.
    #[test]
    fn a_ticket_without_a_table_still_uses_the_labelled_stations() {
        let text = "Von: Bonn Hbf\nNach: Frankfurt(Main)Hbf\n\
                    ICE 1513\nam 25.08.2026 um 10:07\nPreis: 68,47 EUR\n";
        let result = parse_ticket_text(text, "ticket.txt");
        assert_eq!(result.legs.len(), 1);
        assert_eq!(result.legs[0].origin_name, "Bonn Hbf");
        assert!(result.ok);
    }

    /// An airline confirmation parses into flight legs. The fixture mirrors a
    /// real Eurowings Buchungsbestätigung's structure (verified 2026-08-12)
    /// with every personal value fabricated: the label words and layout are
    /// the airline's, the data is not anyone's.
    #[test]
    fn an_airline_confirmation_becomes_flight_legs() {
        let text = "Booking Confirmation ABC123 Passenger Receipt Buchungsbestätigung \
            Hallo Max Mustermann, herzlichen Dank für deine Buchung bei Eurowings. \
            Dein Buchungscode für den Check-in: ABC123 \
            Flugdaten (Zeiten sind Ortszeiten) \
            Flug: 03.11.2026 | Flugnummer: EW 538 (BASIC X ) \
            | Abflug 07:15 Uhr Köln-Bonn | | Ankunft 09:40 Uhr Valencia \
            Flug: 07.11.2026 | Flugnummer: EW 539 (BASIC E ) \
            | Abflug 14:55 Uhr Valencia | | Ankunft 17:30 Uhr Köln-Bonn \
            Gast 1 : Herr Max Mustermann \
            Köln-Bonn ( CGN ) - Valencia ( VLC ) (BASIC) \
            Gesamtpreis | 111.11 € |";
        let ticket = parse_ticket_text(text, "confirmation.eml");

        assert!(ticket.ok);
        assert_eq!(ticket.legs.len(), 2);
        let out = &ticket.legs[0];
        assert_eq!(out.train_number.as_deref(), Some("EW 538"));
        assert_eq!(out.origin_name, "Köln-Bonn");
        assert_eq!(out.destination_name, "Valencia");
        assert_eq!(out.departure_time, "2026-11-03T07:15:00");
        assert_eq!(out.arrival_time, "2026-11-03T09:40:00");
        assert_eq!(out.mode.as_deref(), Some("flight"));
        let back = &ticket.legs[1];
        assert_eq!(back.train_number.as_deref(), Some("EW 539"));
        assert_eq!(back.departure_time, "2026-11-07T14:55:00");
        assert_eq!(ticket.confirmation_number.as_deref(), Some("ABC123"));
        assert_eq!(back.price, Some(111.11));
    }

    /// The airline path never runs on a rail confirmation: no marker, no
    /// flight parsing, even if a stray "Flug" word appears.
    #[test]
    fn a_rail_confirmation_stays_on_the_rail_path() {
        let text = "Ihre Buchung Auftragsnummer: XY123456 \
            26.11.2026 08:53 Köln Hbf 10:27 Berlin Hbf ICE 848 \
            Flug zum Sparpreis gibt es hier nicht. Summe: 49,99 €";
        let ticket = parse_ticket_text(text, "db.pdf");
        assert_eq!(ticket.legs.len(), 1);
        assert_eq!(ticket.legs[0].mode, None);
        assert_eq!(ticket.legs[0].train_number.as_deref(), Some("ICE 848"));
    }

    #[test]
    fn parse_confirmation_code() {
        let text = "Buchungsnummer: ABC123DEF";
        assert_eq!(parse_confirmation(text).as_deref(), Some("ABC123DEF"));
    }

    #[test]
    fn parse_booking_code() {
        let text = "Booking code: BKG-7X9K2M";
        assert_eq!(parse_confirmation(text).as_deref(), Some("BKG-7X9K2M"));
    }

    #[test]
    fn parse_price_de() {
        let text = "Preis: 89,90 EUR";
        assert!((parse_price(text).unwrap() - 89.90).abs() < 0.01);
    }

    #[test]
    fn parse_price_en() {
        let text = "Price: 72,50 EUR";
        assert!((parse_price(text).unwrap() - 72.50).abs() < 0.01);
    }

    #[test]
    fn end_to_end_text() {
        let text = "ICE 123 von Berlin Hbf nach München Hbf am 15.07.2026 um 08:30, an 14:15. Preis: 89,90 EUR. Buchungsnummer: ABC123DEF";
        let result = parse_ticket_text(text, "ticket.txt");
        assert!(result.ok);
        assert_eq!(result.legs.len(), 1);
        assert_eq!(result.legs[0].origin_name, "Berlin Hbf");
        assert_eq!(result.legs[0].destination_name, "München Hbf");
        assert_eq!(result.legs[0].train_number.as_deref(), Some("ICE 123"));
        assert_eq!(result.legs[0].departure_time, "2026-07-15T08:30:00");
        assert!(result.confirmation_number.as_deref() == Some("ABC123DEF"));
    }

    #[test]
    fn end_to_end_ic_ticket() {
        let text = "Your booking: IC 2044\nFrom: Hamburg Hbf\nTo: Frankfurt(Main)Hbf\nDate: 2026-07-20\nDeparture: 09:45\nArrival: 14:30\nPrice: 72,50 EUR\nBooking code: BKG-7X9K2M";
        let result = parse_ticket_text(text, "ticket.txt");
        assert!(result.ok);
        assert_eq!(result.legs.len(), 1);
        assert_eq!(result.legs[0].origin_name, "Hamburg Hbf");
        assert!((result.legs[0].price.unwrap_or(0.0) - 72.50).abs() < 0.01);
        assert_eq!(result.confirmation_number.as_deref(), Some("BKG-7X9K2M"));
    }
}
