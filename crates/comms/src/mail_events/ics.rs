//! Calendar attachments as text the readers and grounding already understand.
//!
//! Each `VEVENT` becomes a block appended to the mail body: a [`MARKER`] line with the summary,
//! then the start and, if present, the end as `dd.mm.yyyy HH:MM` lines. A reader dates a leg from
//! the block, and grounding finds the quoted line in the same document the reader saw.
//! iCalendar: RFC 5545 (line folding §3.1, text escapes §3.3.11, DATE-TIME forms §3.3.5).

pub(super) const MARKER: &str = "Kalenderanhang:";

/// Every event in `calendar` as a block of lines. Events without a readable start are dropped.
pub(super) fn render(calendar: &str) -> String {
    let mut out = String::new();
    let mut event: Option<(Option<String>, Option<String>, Option<String>)> = None;
    for line in unfold(calendar) {
        let (name, value) = match line.split_once(':') {
            Some((head, value)) => (head.split(';').next().unwrap_or(head), value),
            None => continue,
        };
        match (name.to_ascii_uppercase().as_str(), event.as_mut()) {
            ("BEGIN", _) if value.eq_ignore_ascii_case("VEVENT") => {
                event = Some((None, None, None));
            }
            ("END", Some(_)) if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some((summary, Some(start), end)) = event.take() {
                    out.push_str(&format!(
                        "{MARKER} {}\n{start}\n",
                        summary.unwrap_or_default()
                    ));
                    if let Some(end) = end {
                        out.push_str(&format!("{end}\n"));
                    }
                }
            }
            ("SUMMARY", Some((summary, _, _))) => *summary = Some(unescape(value)),
            ("DTSTART", Some((_, start, _))) => *start = local_line(value),
            ("DTEND", Some((_, _, end))) => *end = local_line(value),
            _ => {}
        }
    }
    out
}

/// Joins folded lines: a line that starts with a space or tab continues the one before.
fn unfold(calendar: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in calendar.lines() {
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

fn unescape(value: &str) -> String {
    value
        .replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
        .trim()
        .to_string()
}

/// `20261110T081500` (local or `TZID`) as `10.11.2026 08:15`, `20261110` as `10.11.2026`.
/// A UTC value (`…Z`) keeps only its date, and only when the hour leaves the date unchanged
/// in every European offset.
/// ponytail: no timezone database; UTC times lose the time of day. Convert with a tz crate when
/// a sender sends only UTC.
fn local_line(value: &str) -> Option<String> {
    let value = value.trim();
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let date = value.get(..8).filter(|d| digits(d))?;
    let date = format!("{}.{}.{}", &date[6..8], &date[4..6], &date[..4]);
    let Some(time) = value.get(8..) else {
        return Some(date);
    };
    if time.is_empty() {
        return Some(date);
    }
    let (time, utc) = match time.strip_suffix('Z') {
        Some(time) => (time, true),
        None => (time, false),
    };
    let time = time
        .strip_prefix('T')
        .filter(|t| t.len() >= 4 && digits(t))?;
    let hour: u8 = time[..2].parse().ok()?;
    if utc {
        return (3..=20).contains(&hour).then_some(date);
    }
    Some(format!("{date} {}:{}", &time[..2], &time[2..4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic, in the shape of a rail ticket's calendar attachment.
    const TICKET: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n\
        DTSTART;TZID=Europe/Berlin:20261110T081500\r\nDTEND;TZID=Europe/Berlin:20261110T093000\r\n\
        SUMMARY:Bonn Hbf -> Berlin\r\n  Hbf\\, ICE 100\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn an_event_becomes_a_marker_and_two_local_lines() {
        assert_eq!(
            render(TICKET),
            "Kalenderanhang: Bonn Hbf -> Berlin Hbf, ICE 100\n10.11.2026 08:15\n10.11.2026 09:30\n"
        );
    }

    #[test]
    fn dates_utc_and_unreadable_starts() {
        assert_eq!(local_line("20261110").as_deref(), Some("10.11.2026"));
        assert_eq!(
            local_line("20261110T120000Z").as_deref(),
            Some("10.11.2026")
        );
        assert_eq!(local_line("20261110T230000Z"), None);
        assert_eq!(local_line("soon"), None);
        let no_start = "BEGIN:VEVENT\nSUMMARY:x\nEND:VEVENT\n";
        assert_eq!(render(no_start), "");
    }
}
