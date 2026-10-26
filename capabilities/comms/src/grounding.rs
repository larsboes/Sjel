//! Does a quoted span support the specific claim attached to it?
//!
//! The general law this encodes was measured on 2026-08-12 (travel PRD §2/X2):
//! asked to extract events with a verbatim quote per date, a model returned
//! five REAL page headings paired with five sequential INVENTED dates -- and a
//! naive "does the quote exist in the document" check passed all five.
//! Requiring the quote to support the date took it to 0/5. A quoted span
//! existing in a document does not verify the claim attached to it; any
//! grounding check has to test the specific assertion.
//!
//! For date claims that check is writable deterministically -- no model, no
//! judgment: the quote either carries date material matching the claimed
//! day and month, or it does not. Plain token scanning, no regex dependency;
//! the token windows accepted are pinned by the tests.

/// The verdict on one (quote, claimed date) pair against its document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateGrounding {
    /// The quote exists in the document AND carries date material matching
    /// the claimed day and month.
    Supported,
    /// The quote does not appear in the document at all.
    QuoteMissing,
    /// The quote is real but carries no date material for this claim -- the
    /// measured failure shape, where the date was invented next to a real
    /// heading.
    Unsupported,
}

/// Whitespace-insensitive, case-insensitive containment.
fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a quoted span appears in a source document after whitespace and case normalization.
pub fn quote_is_grounded(document: &str, quote: &str) -> bool {
    let quote = normalized(quote);
    !quote.is_empty() && normalized(document).contains(&quote)
}

/// The quote reduced to scannable tokens: alphanumeric runs, everything else
/// a separator. "10.08.2026" becomes ["10","08","2026"], "10. August" becomes
/// ["10","august"].
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() {
            current.push(c);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// German and English month names and their common short forms, 1-indexed.
fn month_number(token: &str) -> Option<u32> {
    Some(match token {
        "januar" | "january" | "jan" => 1,
        "februar" | "february" | "feb" => 2,
        "märz" | "maerz" | "march" | "mar" | "mrz" => 3,
        "april" | "apr" => 4,
        "mai" | "may" => 5,
        "juni" | "june" | "jun" => 6,
        "juli" | "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "oktober" | "october" | "okt" | "oct" => 10,
        "november" | "nov" => 11,
        "dezember" | "december" | "dez" | "dec" => 12,
        _ => return None,
    })
}

fn as_number(token: &str) -> Option<u32> {
    let digits = ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|suffix| token.strip_suffix(suffix))
        .unwrap_or(token);
    if digits.len() > 4 {
        return None;
    }
    digits.parse().ok()
}

/// Whether the quote's tokens carry the claimed day+month, in any of the
/// shapes German and English prose actually uses: "10.08." / "10.08.2026" /
/// "2026-08-10" / "10. August" / "August 10". A day and its month may be
/// separated by at most one token ("10 im August" counts, a day three words
/// from its month does not).
fn quote_carries_day_month(quote_tokens: &[String], month: u32, day: u32) -> bool {
    let n = quote_tokens.len();
    for i in 0..n {
        let Some(first) = as_number(&quote_tokens[i]).or_else(|| month_number(&quote_tokens[i]))
        else {
            continue;
        };
        for j in (i + 1)..n.min(i + 3) {
            let second_num = as_number(&quote_tokens[j]);
            let second_month = month_number(&quote_tokens[j]);
            // day month (numeric "10 08", or "10 august")
            if first == day && (second_num == Some(month) || second_month == Some(month)) {
                return true;
            }
            // month day (english "august 10", or ISO's "08 10" tail after the year)
            if (month_number(&quote_tokens[i]) == Some(month) || first == month)
                && second_num == Some(day)
            {
                return true;
            }
        }
    }
    false
}

/// The check itself. `date` is the claimed ISO date (YYYY-MM-DD); a claim
/// whose date cannot even be parsed is nobody's evidence and comes back
/// `Unsupported`.
pub fn date_grounding(document: &str, source_text: &str, date: &str) -> DateGrounding {
    if !quote_is_grounded(document, source_text) {
        return DateGrounding::QuoteMissing;
    }
    let mut parts = date.splitn(3, '-');
    let (Some(_year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return DateGrounding::Unsupported;
    };
    let (Ok(month), Ok(day)) = (month.parse::<u32>(), day.parse::<u32>()) else {
        return DateGrounding::Unsupported;
    };
    if quote_carries_day_month(&tokens(source_text), month, day) {
        DateGrounding::Supported
    } else {
        DateGrounding::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "Stadtfest Programm. Jazz am Rheinufer. Das große Feuerwerk \
        findet am 10. August 2026 statt. Flohmarkt in der Altstadt. \
        Konzertabend: 23.08.2026, Einlass ab 19 Uhr. Museum night coming soon.";

    /// The measured failure shape: a REAL heading as the quote, an invented
    /// date as the claim. The naive existence check passes it; this does not.
    #[test]
    fn a_real_quote_with_an_invented_date_is_unsupported() {
        assert_eq!(
            date_grounding(DOCUMENT, "Jazz am Rheinufer", "2026-08-11"),
            DateGrounding::Unsupported
        );
        assert_eq!(
            date_grounding(DOCUMENT, "Museum night coming soon", "2026-08-12"),
            DateGrounding::Unsupported
        );
    }

    #[test]
    fn a_quote_that_carries_its_date_is_supported_in_both_languages() {
        assert_eq!(
            date_grounding(DOCUMENT, "findet am 10. August 2026 statt", "2026-08-10"),
            DateGrounding::Supported
        );
        assert_eq!(
            date_grounding(DOCUMENT, "Konzertabend: 23.08.2026", "2026-08-23"),
            DateGrounding::Supported
        );
        assert_eq!(
            date_grounding(
                "The gala is on August 10 this year.",
                "on August 10",
                "2026-08-10"
            ),
            DateGrounding::Supported
        );
    }

    #[test]
    fn a_fabricated_quote_is_quote_missing_whatever_its_date_says() {
        assert_eq!(
            date_grounding(DOCUMENT, "am 10. August 2026 im Stadion", "2026-08-10"),
            DateGrounding::QuoteMissing
        );
    }

    #[test]
    fn a_quote_whose_date_material_contradicts_the_claim_is_unsupported() {
        // The quote genuinely carries 23.08; the claim says 24.08.
        assert_eq!(
            date_grounding(DOCUMENT, "Konzertabend: 23.08.2026", "2026-08-24"),
            DateGrounding::Unsupported
        );
    }

    #[test]
    fn containment_survives_whitespace_and_case_but_not_paraphrase() {
        assert_eq!(
            date_grounding(
                DOCUMENT,
                "das GROSSE Feuerwerk findet am 10. August 2026",
                "2026-08-10"
            ),
            DateGrounding::QuoteMissing, // "GROSSE" != "große": paraphrase is not a quote
        );
        assert_eq!(
            date_grounding(DOCUMENT, "  findet   am  10.  August 2026 ", "2026-08-10"),
            DateGrounding::Supported
        );
    }
}
