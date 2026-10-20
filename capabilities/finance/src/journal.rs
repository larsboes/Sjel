//! Axon's own journal parser.
//!
//! The journal FORMAT stays hledger-compatible (Principle 8: the file is
//! human-readable and opens in hledger any day). Only the runtime dependency is
//! gone: this module reads the plaintext journal directly into the same
//! [`JournalTransaction`] shape the shell-out adapter used to return, so no
//! caller changed when the engine did.
//!
//! Two journals were measured on 2026-08-28 and both parse here:
//!
//! * the live journal (6,697 lines, 1,339 transactions) — a `decimal-mark .`
//!   and a `commodity` directive, then 1,339 blocks of exactly the five line
//!   shapes `import.rs`'s writer emits, plus one hand-written opening balance
//!   whose source-id is `opening_<hex>` rather than plain hex;
//! * `schemas/finance-journal.example` — the published format contract, which
//!   is deliberately richer: top-level comments, `account` and `P` directives,
//!   a comma decimal mark with `.` digit groups, unmarked (no `*`) transactions,
//!   three-posting transactions, transaction-level tag comments that
//!   `analytics::project` reads, and a commodity amount with an `@` unit cost.
//!
//! Anything outside those measured shapes is refused with its line number
//! rather than skipped. A silently dropped posting is a wrong balance, and a
//! wrong balance in a ledger is worse than a loud refusal.

use crate::accounting::{Amount, JournalTransaction, Posting};
use std::collections::BTreeMap;

/// Working precision for balance arithmetic. Every amount is widened to this
/// scale in `i128` before it is summed, so postings of unequal precision add
/// exactly and no binary floating point is involved anywhere in this module.
/// The measured journals use scale 2; an amount needing more than this is
/// refused rather than rounded.
const WORKING_SCALE: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalError {
    /// 1-based line number in the journal the refusal points at.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for JournalError {}

type Result<T> = std::result::Result<T, JournalError>;

fn refuse<T>(line: usize, message: impl Into<String>) -> Result<T> {
    Err(JournalError {
        line,
        message: message.into(),
    })
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Parse a journal into the transactions the projection is built from.
///
/// Transactions carry a 1-based `index` in file order — the same identity
/// hledger's `tindex` had, which the projection's row ids are formatted from —
/// and are returned ordered by date, stably, so equal dates keep file order.
pub fn parse(text: &str) -> Result<Vec<JournalTransaction>> {
    let mut transactions = Parser::new().run(text)?;
    transactions.sort_by(|left, right| {
        left.date
            .cmp(&right.date)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(transactions)
}

/// Validate a journal without keeping the parse.
///
/// This is what the old `hledger check` guarded, stated in Axon's own terms:
/// every line is a shape this parser recognizes, every date is a real calendar
/// date, every amount parses, every transaction balances (with at most one
/// amount inferred), and every `source-id` tag carries a value. A journal that
/// passes here is a journal the projection can be rebuilt from.
pub fn validate(text: &str) -> Result<()> {
    Parser::new().run(text).map(|_| ())
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// An amount as written, before balancing. `cost` is the `@` unit price or `@@`
/// total price; it is what the posting contributes to the balance, in the cost's
/// commodity, which is how a commodity purchase balances against cash.
#[derive(Debug, Clone)]
struct ParsedAmount {
    commodity: String,
    mantissa: i128,
    scale: u32,
    cost: Option<Box<Cost>>,
}

#[derive(Debug, Clone)]
struct Cost {
    total: bool,
    amount: ParsedAmount,
}

#[derive(Debug)]
struct RawPosting {
    line: usize,
    account: String,
    amount: Option<ParsedAmount>,
    tags: Vec<(String, String)>,
}

#[derive(Debug)]
struct RawTransaction {
    line: usize,
    date: String,
    description: String,
    tags: Vec<(String, String)>,
    postings: Vec<RawPosting>,
}

struct Parser {
    decimal_mark: Option<char>,
    pending: Option<RawTransaction>,
    finished: Vec<JournalTransaction>,
    next_index: u64,
}

impl Parser {
    fn new() -> Self {
        Self {
            decimal_mark: None,
            pending: None,
            finished: Vec::new(),
            next_index: 1,
        }
    }

    fn run(mut self, text: &str) -> Result<Vec<JournalTransaction>> {
        for (offset, raw_line) in text.lines().enumerate() {
            let number = offset + 1;
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            if line.trim().is_empty() {
                self.close_transaction()?;
                continue;
            }
            if line.starts_with([' ', '\t']) {
                self.indented(number, line)?;
            } else {
                self.close_transaction()?;
                self.top_level(number, line)?;
            }
        }
        self.close_transaction()?;
        Ok(self.finished)
    }

    // -- top-level lines ---------------------------------------------------

    fn top_level(&mut self, number: usize, line: &str) -> Result<()> {
        if line.starts_with(';') || line.starts_with('#') {
            return Ok(());
        }
        if line.starts_with(|c: char| c.is_ascii_digit()) {
            return self.open_transaction(number, line);
        }
        let (keyword, rest) = split_keyword(line);
        match keyword {
            "decimal-mark" => {
                let mark = rest.trim();
                match mark {
                    "." => self.decimal_mark = Some('.'),
                    "," => self.decimal_mark = Some(','),
                    _ => {
                        return refuse(
                            number,
                            format!("decimal-mark must be '.' or ',', found {mark:?}"),
                        )
                    }
                }
                Ok(())
            }
            "commodity" => {
                // The directive declares a display style. Axon reads only the
                // decimal mark out of it, and only when no decimal-mark
                // directive has already said so.
                if self.decimal_mark.is_none() {
                    if let Some(mark) = decimal_mark_of_sample(rest) {
                        self.decimal_mark = Some(mark);
                    }
                }
                Ok(())
            }
            // Declared for the reader's benefit; plain validation never
            // required an account to be declared, so neither does this.
            "account" | "payee" | "tag" => Ok(()),
            // A market price. Nothing consumes prices now that the valuation
            // report is gone, but the shape is in the published fixture, so it
            // parses rather than refusing a file Axon itself publishes.
            "P" => Ok(()),
            other => refuse(
                number,
                format!(
                    "unrecognized directive {other:?}; Axon's parser handles \
                     decimal-mark, commodity, account, payee, tag and P"
                ),
            ),
        }
    }

    fn open_transaction(&mut self, number: usize, line: &str) -> Result<()> {
        let (body, comment) = split_comment(line);
        let body = body.trim_end();
        let (date_text, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
        if date_text.contains('=') {
            return refuse(
                number,
                "a secondary date is not a shape Axon writes or has measured; \
                 remove it or extend the parser deliberately",
            );
        }
        let date = normalize_date(date_text).ok_or_else(|| JournalError {
            line: number,
            message: format!("{date_text:?} is not a YYYY-MM-DD calendar date"),
        })?;
        let mut rest = rest.trim_start();
        // Status mark. Axon's writer always emits `*`; the published fixture
        // leaves it off, and both are valid hledger.
        if let Some(tail) = rest.strip_prefix('*').or_else(|| rest.strip_prefix('!')) {
            if tail.is_empty() || tail.starts_with(char::is_whitespace) {
                rest = tail.trim_start();
            }
        }
        // A transaction code, `(1234)`, immediately after the status. hledger
        // keeps it out of the description, so this parser drops it the same way
        // rather than letting the two disagree about what a description is.
        if let Some(tail) = rest.strip_prefix('(') {
            if let Some((_code, after)) = tail.split_once(')') {
                rest = after.trim_start();
            }
        }
        self.pending = Some(RawTransaction {
            line: number,
            date,
            description: rest.trim().to_string(),
            tags: parse_comment_tags(comment),
            postings: Vec::new(),
        });
        Ok(())
    }

    // -- indented lines ----------------------------------------------------

    fn indented(&mut self, number: usize, line: &str) -> Result<()> {
        let trimmed = line.trim();
        let Some(transaction) = self.pending.as_mut() else {
            return refuse(
                number,
                "indented line outside a transaction; a posting or comment must \
                 follow a transaction header",
            );
        };
        if let Some(comment) = trimmed.strip_prefix(';') {
            let tags = parse_comment_tags(Some(comment));
            // hledger attaches a comment to the posting above it, and to the
            // transaction when no posting has been read yet. Axon's writer puts
            // `; source-id:` under the last posting, which is exactly why the
            // id is read back off a posting rather than the transaction.
            match transaction.postings.last_mut() {
                Some(posting) => posting.tags.extend(tags),
                None => transaction.tags.extend(tags),
            }
            return Ok(());
        }
        let (account_text, amount_text) = split_posting(trimmed);
        let account = account_text.trim();
        if account.is_empty() {
            return refuse(number, "posting has no account name");
        }
        let amount = match amount_text.trim() {
            "" => None,
            text => Some(parse_amount(
                number,
                text,
                self.decimal_mark.unwrap_or('.'),
            )?),
        };
        transaction.postings.push(RawPosting {
            line: number,
            account: account.to_string(),
            amount,
            tags: Vec::new(),
        });
        Ok(())
    }

    // -- closing -----------------------------------------------------------

    fn close_transaction(&mut self) -> Result<()> {
        let Some(raw) = self.pending.take() else {
            return Ok(());
        };
        if raw.postings.is_empty() {
            return refuse(raw.line, "transaction has no postings");
        }
        let postings = balance(&raw)?;
        let tags: BTreeMap<String, String> = raw.tags.iter().cloned().collect();
        // Identity resolution, unchanged from the adapter this replaces: the
        // transaction's own tags win, otherwise the first posting that carries
        // a source-id supplies it.
        let source_id = tags.get("source-id").cloned().or_else(|| {
            raw.postings.iter().find_map(|posting| {
                posting
                    .tags
                    .iter()
                    .find(|(key, _)| key == "source-id")
                    .map(|(_, value)| value.clone())
            })
        });
        if source_id.as_deref() == Some("") {
            return refuse(
                raw.line,
                "source-id tag has no value; a transaction Axon wrote is identified by it",
            );
        }
        self.finished.push(JournalTransaction {
            index: self.next_index,
            date: raw.date,
            description: raw.description,
            source_id,
            tags,
            postings,
        });
        self.next_index += 1;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Balancing
// ---------------------------------------------------------------------------

/// Sum the written amounts per commodity, infer the one that may be left out,
/// and refuse a transaction whose postings do not cancel.
fn balance(raw: &RawTransaction) -> Result<Vec<Posting>> {
    let mut totals: BTreeMap<String, i128> = BTreeMap::new();
    let mut scales: BTreeMap<String, u32> = BTreeMap::new();
    let mut elided: Option<usize> = None;

    for (index, posting) in raw.postings.iter().enumerate() {
        let Some(amount) = &posting.amount else {
            if elided.is_some() {
                return refuse(
                    posting.line,
                    "a second posting leaves its amount out; hledger infers at \
                     most one per transaction and so does Axon",
                );
            }
            elided = Some(index);
            continue;
        };
        let (commodity, value, scale) = contribution(posting.line, amount)?;
        *totals.entry(commodity.clone()).or_default() += value;
        let entry = scales.entry(commodity).or_default();
        *entry = (*entry).max(scale);
    }
    totals.retain(|_, value| *value != 0);

    let mut postings: Vec<Posting> = raw
        .postings
        .iter()
        .map(|posting| Posting {
            account: posting.account.clone(),
            amounts: posting
                .amount
                .as_ref()
                .map(|amount| {
                    vec![Amount {
                        commodity: amount.commodity.clone(),
                        mantissa: amount.mantissa as i64,
                        scale: amount.scale,
                    }]
                })
                .unwrap_or_default(),
        })
        .collect();

    match elided {
        Some(index) => {
            let mut inferred = Vec::new();
            for (commodity, value) in &totals {
                let scale = scales.get(commodity).copied().unwrap_or(0);
                inferred.push(to_amount(
                    raw.postings[index].line,
                    commodity,
                    -*value,
                    scale,
                )?);
            }
            postings[index].amounts = inferred;
        }
        None => {
            if let Some((commodity, value)) = totals.iter().next() {
                let scale = scales.get(commodity).copied().unwrap_or(2);
                let residual = to_amount(raw.line, commodity, *value, scale)
                    .map(|amount| format!("{} {}", render(&amount), amount.commodity))
                    .unwrap_or_else(|_| commodity.clone());
                return refuse(
                    raw.line,
                    format!("transaction does not balance; {residual} is left over"),
                );
            }
        }
    }
    Ok(postings)
}

/// What one written amount adds to the balance: its cost when it has one, so a
/// share purchase weighs against cash in euros rather than in shares.
fn contribution(line: usize, amount: &ParsedAmount) -> Result<(String, i128, u32)> {
    let Some(cost) = &amount.cost else {
        return Ok((
            amount.commodity.clone(),
            widen(line, amount.mantissa, amount.scale)?,
            amount.scale,
        ));
    };
    if cost.amount.cost.is_some() {
        return refuse(line, "a cost may not itself carry a cost");
    }
    let (mantissa, scale) = if cost.total {
        // `@@` states the total directly; it is written unsigned and takes the
        // sign of the quantity it prices.
        let sign = if amount.mantissa < 0 { -1 } else { 1 };
        (sign * cost.amount.mantissa.abs(), cost.amount.scale)
    } else {
        (
            amount.mantissa * cost.amount.mantissa,
            amount.scale + cost.amount.scale,
        )
    };
    Ok((
        cost.amount.commodity.clone(),
        widen(line, mantissa, scale)?,
        scale,
    ))
}

fn widen(line: usize, mantissa: i128, scale: u32) -> Result<i128> {
    if scale > WORKING_SCALE {
        return refuse(
            line,
            format!("amount has more than {WORKING_SCALE} decimal places"),
        );
    }
    Ok(mantissa * 10_i128.pow(WORKING_SCALE - scale))
}

/// Narrow a working-scale value back to a written amount, keeping at least the
/// precision the transaction's other postings used and growing it only when the
/// value would not divide evenly.
fn to_amount(line: usize, commodity: &str, value: i128, preferred_scale: u32) -> Result<Amount> {
    for scale in preferred_scale..=WORKING_SCALE {
        let divisor = 10_i128.pow(WORKING_SCALE - scale);
        if value % divisor == 0 {
            let mantissa = value / divisor;
            return i64::try_from(mantissa)
                .map(|mantissa| Amount {
                    commodity: commodity.to_string(),
                    mantissa,
                    scale,
                })
                .map_err(|_| JournalError {
                    line,
                    message: format!("{commodity} amount is too large for the ledger"),
                });
        }
    }
    refuse(
        line,
        format!("{commodity} amount needs more than {WORKING_SCALE} decimal places"),
    )
}

fn render(amount: &Amount) -> String {
    if amount.scale == 0 {
        return amount.mantissa.to_string();
    }
    let sign = if amount.mantissa < 0 { "-" } else { "" };
    let magnitude = amount.mantissa.unsigned_abs();
    let divisor = 10_u64.pow(amount.scale);
    format!(
        "{sign}{}.{:0width$}",
        magnitude / divisor,
        magnitude % divisor,
        width = amount.scale as usize
    )
}

// ---------------------------------------------------------------------------
// Line-level parsing
// ---------------------------------------------------------------------------

fn split_keyword(line: &str) -> (&str, &str) {
    line.split_once(char::is_whitespace).unwrap_or((line, ""))
}

/// Split a line at its comment marker. Descriptions Axon writes never contain a
/// `;` — `import::sanitize_description` replaces it — so the first one wins.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    match line.split_once(';') {
        Some((body, comment)) => (body, Some(comment)),
        None => (line, None),
    }
}

/// hledger separates a posting's account from its amount by two or more spaces
/// or a tab, which is what lets an account name contain a single space.
fn split_posting(trimmed: &str) -> (&str, &str) {
    let (body, _comment) = split_comment(trimmed);
    let bytes = body.as_bytes();
    for index in 0..bytes.len() {
        let two_spaces = bytes[index] == b' ' && bytes.get(index + 1) == Some(&b' ');
        if two_spaces || bytes[index] == b'\t' {
            return (&body[..index], &body[index..]);
        }
    }
    (body, "")
}

fn parse_comment_tags(comment: Option<&str>) -> Vec<(String, String)> {
    let Some(comment) = comment else {
        return Vec::new();
    };
    let mut tags = Vec::new();
    for segment in comment.split(',') {
        let Some((left, right)) = segment.split_once(':') else {
            continue;
        };
        // hledger's rule: the tag name is the word immediately before the colon.
        let Some(key) = left.split_whitespace().next_back() else {
            continue;
        };
        tags.push((key.to_string(), right.trim().to_string()));
    }
    tags
}

/// `-45.99 EUR`, `EUR -45.99`, `10 ACME @ 42,50 EUR`, `10 ACME @@ 425,00 EUR`.
fn parse_amount(line: usize, text: &str, decimal_mark: char) -> Result<ParsedAmount> {
    let (body, cost) = match text.split_once("@@") {
        Some((body, price)) => (body, Some((true, price))),
        None => match text.split_once('@') {
            Some((body, price)) => (body, Some((false, price))),
            None => (text, None),
        },
    };
    let mut amount = parse_simple_amount(line, body.trim(), decimal_mark)?;
    if let Some((total, price)) = cost {
        let price = parse_simple_amount(line, price.trim(), decimal_mark)?;
        amount.cost = Some(Box::new(Cost {
            total,
            amount: price,
        }));
    }
    Ok(amount)
}

fn parse_simple_amount(line: usize, text: &str, decimal_mark: char) -> Result<ParsedAmount> {
    if text.is_empty() {
        return refuse(line, "amount is empty");
    }
    // A commodity symbol may lead or trail the number. Split at the boundary
    // between the numeric run (sign, digits, separators) and everything else.
    let numeric = |c: char| c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == '+';
    let leading_number = text.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+');
    let (number_text, commodity) = if leading_number {
        let end = text.find(|c: char| !numeric(c)).unwrap_or(text.len());
        (&text[..end], text[end..].trim())
    } else {
        let start = text
            .find(|c: char| c.is_ascii_digit() || c == '-' || c == '+')
            .ok_or_else(|| JournalError {
                line,
                message: format!("{text:?} has no number"),
            })?;
        (&text[start..], text[..start].trim())
    };
    let commodity = commodity.trim_matches('"').trim();
    if commodity.is_empty() {
        return refuse(
            line,
            format!("{text:?} has no commodity; Axon's journal names one on every amount"),
        );
    }
    let (mantissa, scale) = parse_number(line, number_text.trim(), decimal_mark)?;
    Ok(ParsedAmount {
        commodity: commodity.to_string(),
        mantissa,
        scale,
        cost: None,
    })
}

/// Digits, optional digit-group separators, and at most one decimal mark.
fn parse_number(line: usize, text: &str, decimal_mark: char) -> Result<(i128, u32)> {
    let group_mark = if decimal_mark == '.' { ',' } else { '.' };
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut whole = String::new();
    let mut fraction: Option<String> = None;
    for character in digits.chars() {
        if character.is_ascii_digit() {
            match &mut fraction {
                Some(fraction) => fraction.push(character),
                None => whole.push(character),
            }
        } else if character == group_mark {
            if fraction.is_some() {
                return refuse(
                    line,
                    format!("{text:?} groups digits after its decimal mark"),
                );
            }
        } else if character == decimal_mark {
            if fraction.is_some() {
                return refuse(line, format!("{text:?} has two decimal marks"));
            }
            fraction = Some(String::new());
        } else {
            return refuse(line, format!("{text:?} is not a number"));
        }
    }
    if whole.is_empty() && fraction.as_ref().is_none_or(String::is_empty) {
        return refuse(line, format!("{text:?} is not a number"));
    }
    let fraction = fraction.unwrap_or_default();
    let scale = u32::try_from(fraction.len()).unwrap_or(u32::MAX);
    if scale > WORKING_SCALE {
        return refuse(
            line,
            format!("{text:?} has more than {WORKING_SCALE} decimal places"),
        );
    }
    let combined = format!("{whole}{fraction}");
    let magnitude: i128 = combined.parse().map_err(|_| JournalError {
        line,
        message: format!("{text:?} is not a number this ledger can hold"),
    })?;
    Ok((if negative { -magnitude } else { magnitude }, scale))
}

/// The decimal mark declared by a `commodity` sample such as `1,000.00 EUR`:
/// the last of the two marks to appear is the decimal one.
fn decimal_mark_of_sample(sample: &str) -> Option<char> {
    sample.chars().rfind(|c| *c == '.' || *c == ',')
}

/// `YYYY-MM-DD`, or the `/` and `.` separators hledger also accepts, normalized
/// to the one shape every consumer downstream compares as a string.
fn normalize_date(text: &str) -> Option<String> {
    let separator = ['-', '/', '.'].into_iter().find(|s| text.contains(*s))?;
    let mut parts = text.split(separator);
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{render_journal_entry, CandidateState, TransactionCandidate};

    /// The two header lines the live journal opens with, which every journal
    /// Axon writes into starts from (`import.rs`'s own test writes the same).
    const HEADER: &str = "decimal-mark .\ncommodity 1,000.00 EUR\n";

    fn parse_ok(text: &str) -> Vec<JournalTransaction> {
        parse(text).unwrap_or_else(|error| panic!("journal should parse: {error}"))
    }

    fn cents(posting: &Posting) -> i64 {
        posting.amounts[0].minor_units(2).expect("EUR minor units")
    }

    // -- golden: the live journal's measured shapes -------------------------

    /// Measured 2026-08-28 against the live journal: two directives, then 1,339
    /// blocks of exactly these five line shapes, one of which is the
    /// hand-written opening balance whose source-id is not plain hex.
    #[test]
    fn every_line_shape_measured_in_the_live_journal_parses() {
        let journal = format!(
            "{HEADER}\n\
             2025-07-31 * opening balance\n    \
             assets:bank:checking-2  1643.38 EUR\n    \
             equity:opening-balances\n    \
             ; source-id: opening_61ffae0a4143c012a4a5e398115444e363d0464096a1765372f0fbd7042ba13a\n\
             \n\
             2026-08-03 * AXA Versicherung · Haftpflicht 45,99. EUR · Lastschrift\n    \
             assets:bank:checking-2  -45.99 EUR\n    \
             expenses:uncategorized\n    \
             ; source-id: 1b7f0a2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8\n"
        );
        let transactions = parse_ok(&journal);
        assert_eq!(transactions.len(), 2);

        let opening = &transactions[0];
        assert_eq!(opening.index, 1);
        assert_eq!(opening.date, "2025-07-31");
        assert_eq!(opening.description, "opening balance");
        // The hand edit's non-hex source-id survives; refusing it would refuse
        // the journal's own first transaction.
        assert_eq!(
            opening.source_id.as_deref(),
            Some("opening_61ffae0a4143c012a4a5e398115444e363d0464096a1765372f0fbd7042ba13a")
        );
        assert_eq!(cents(&opening.postings[0]), 164_338);
        // The elided posting is inferred, which is the whole reason the
        // projection can classify a two-line posting pair at all.
        assert_eq!(cents(&opening.postings[1]), -164_338);

        let expense = &transactions[1];
        assert_eq!(expense.index, 2);
        // A description keeps its UTF-8, its commas and its embedded periods.
        assert!(expense.description.contains("· Haftpflicht 45,99. EUR ·"));
        assert_eq!(cents(&expense.postings[0]), -4_599);
        assert_eq!(cents(&expense.postings[1]), 4_599);
        assert_eq!(expense.postings[1].account, "expenses:uncategorized");
        // Axon's writer puts the id under the last posting, so it is read back
        // off a posting rather than the transaction. `tags` stays empty, which
        // is what the shell-out adapter returned for these too.
        assert!(expense.tags.is_empty());
    }

    // -- golden: the published format contract ------------------------------

    fn published_fixture() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/finance-journal.example");
        std::fs::read_to_string(path).expect("the published fixture is part of the repository")
    }

    /// `schemas/finance-journal.example` is the format Axon publishes. It is
    /// deliberately richer than the live journal, and every shape in it has to
    /// parse or the repository documents a format its own reader refuses.
    #[test]
    fn the_published_fixture_parses_every_shape_it_documents() {
        let transactions = parse_ok(&published_fixture());
        assert_eq!(transactions.len(), 7);

        // A comma decimal mark with `.` digit groups, and a three-posting
        // transaction whose third amount is inferred.
        let opening = &transactions[0];
        assert_eq!(opening.date, "2026-01-01");
        assert_eq!(cents(&opening.postings[0]), 150_000);
        assert_eq!(cents(&opening.postings[1]), 500_000);
        assert_eq!(opening.postings[2].account, "equity:opening-balances");
        assert_eq!(cents(&opening.postings[2]), -650_000);

        // An unmarked transaction: no `*`, which the writer always emits but
        // the fixture (and a human editing in hledger) does not.
        assert_eq!(transactions[1].description, "employer");

        // A commodity amount with an `@` unit cost. The share count stays in
        // ACME; the cash side balances in EUR at 10 x 42,50.
        let purchase = &transactions[3];
        assert_eq!(purchase.description, "buy ACME");
        assert_eq!(purchase.postings[0].amounts[0].commodity, "ACME");
        assert_eq!(purchase.postings[0].amounts[0].mantissa, 10);
        assert_eq!(purchase.postings[1].account, "assets:broker:cash");
        assert_eq!(cents(&purchase.postings[1]), -42_500);

        // Transaction-level tags, which `analytics::project` reads for trip and
        // shared-expense attribution.
        let dinner = &transactions[4];
        assert_eq!(dinner.description, "group dinner");
        assert_eq!(
            dinner.tags.get("axon-purpose").map(String::as_str),
            Some("trip")
        );
        assert_eq!(
            dinner.tags.get("axon-shared-cents").map(String::as_str),
            Some("3000")
        );
        assert_eq!(
            dinner.tags.get("axon-trip-id").map(String::as_str),
            Some("trip:synthetic")
        );
        assert_eq!(dinner.source_id.as_deref(), Some("synthetic-expense"));

        let repayment = &transactions[6];
        assert_eq!(
            repayment
                .tags
                .get("axon-reimbursement-for")
                .map(String::as_str),
            Some("synthetic-expense")
        );
        assert_eq!(repayment.source_id.as_deref(), Some("synthetic-repayment"));
    }

    /// The end-to-end proof that replacing the engine did not move a number:
    /// these are the exact dashboard figures the shell-out adapter's own test
    /// asserted against this same fixture.
    #[test]
    fn the_published_fixture_projects_the_same_dashboard_as_before() {
        let transactions = parse_ok(&published_fixture());
        let projection = crate::analytics::project(&transactions, "EUR");
        let dashboard =
            crate::analytics::dashboard(&projection, &crate::analytics::AnalyticsFilter::default());
        assert_eq!(dashboard.summary.income_cents, 120_000);
        assert_eq!(dashboard.summary.personal_spending_cents, 3_640);
        assert_eq!(dashboard.summary.gross_cash_outflow_cents, 6_640);
        assert_eq!(dashboard.summary.reimbursement_received_cents, 2_000);
        assert_eq!(dashboard.summary.personal_result_cents, 116_360);
        let shared = dashboard.shared_expenses.first().unwrap();
        assert_eq!(shared.source_id, "synthetic-expense");
        assert_eq!(shared.trip_id.as_deref(), Some("trip:synthetic"));
        assert_eq!(shared.outstanding_cents, 1_000);
    }

    // -- property: the writer's output is always readable -------------------

    fn candidate(
        booked_at: &str,
        description: &str,
        amount_cents: i64,
        currency: &str,
        source_account: &str,
    ) -> TransactionCandidate {
        TransactionCandidate {
            id: "candidate_x".into(),
            fingerprint: format!("{:064x}", amount_cents.unsigned_abs()),
            booked_at: booked_at.into(),
            description: description.into(),
            amount_cents,
            currency: currency.into(),
            source_account: source_account.into(),
            source_reference: None,
            proposed_account: "expenses:food".into(),
            confidence_basis_points: 0,
            state: CandidateState::Pending,
            location_street: None,
            location_postal_code: None,
            location_city: None,
            location_country: None,
            location_mcc: None,
        }
    }

    /// Round-trip property: anything `render_journal_entry` writes, this parser
    /// reads back with the date, description, signed amount and source-id
    /// intact, and with the elided posting inferred to the exact opposite.
    ///
    /// The sweep is deterministic rather than randomized so a failure names one
    /// reproducible case. It covers both signs, magnitudes from one cent to
    /// eight figures, every month-end, and the descriptions the real export
    /// produces — UTF-8 separators, quotes, parentheses and a leading `(` that
    /// a code-parsing reader could mistake for a transaction code.
    #[test]
    fn the_writers_output_always_round_trips_through_the_parser() {
        let descriptions = [
            "REWE SAGT DANKE",
            "AMERICAN EXPRESS EUROPE S.A. (Germany branch) · 01KX53SNQ8HP3S20QGRPGDJPFM",
            "PAYPAL *UNIQLOEUROP 4029357733",
            "AXA · Haftpflicht 45,99. EUR · Lastschrift",
            "(opening) parenthesised leading token",
            "a  description   with   runs",
            "Ünïcödé Ätzend",
            "*",
        ];
        let magnitudes: [i64; 8] = [1, 7, 99, 100, 4_599, 164_338, 1_000_000, 99_999_999];
        let days = ["01", "15", "28", "29", "30", "31"];
        let months = ["01", "02", "04", "12"];
        // Every source account, paired with the outflow and inflow account the
        // writer will actually accept for it, so both signs are exercised
        // against all three rather than filtered away.
        let accounts = [
            (
                "liabilities:card:amex",
                "expenses:food:groceries",
                "income:refunds",
            ),
            (
                "assets:broker:trade-republic:cash",
                "expenses:uncategorized",
                "income:investments:dividends",
            ),
            (
                "assets:bank:checking-2",
                "expenses:housing:rent",
                "income:salary",
            ),
        ];

        let mut checked = 0_usize;
        for (index, description) in descriptions.iter().enumerate() {
            for (magnitude_index, magnitude) in magnitudes.iter().enumerate() {
                for sign in [-1_i64, 1] {
                    let month = months[(index + magnitude_index) % months.len()];
                    let day = days[(index + magnitude_index) % days.len()];
                    if month == "02" && day > "28" {
                        continue; // 2026 is not a leap year; the writer never emits it either.
                    }
                    if month == "04" && day == "31" {
                        continue;
                    }
                    let (source_account, outflow, inflow) =
                        accounts[magnitude_index % accounts.len()];
                    // The writer refuses sign/account pairs the ledger rejects,
                    // so ask it for the pairing it will actually produce.
                    let account = if sign < 0 { outflow } else { inflow };
                    let amount_cents = sign * magnitude;
                    let booked_at = format!("2026-{month}-{day}");
                    let candidate =
                        candidate(&booked_at, description, amount_cents, "EUR", source_account);
                    let entry = render_journal_entry(&candidate, account)
                        .expect("the writer accepts this pairing");
                    let journal = format!("{HEADER}{entry}");

                    let transactions = parse(&journal).unwrap_or_else(|error| {
                        panic!("writer output must parse ({booked_at}, {amount_cents}, {description:?}): {error}")
                    });
                    assert_eq!(transactions.len(), 1);
                    let transaction = &transactions[0];
                    assert_eq!(transaction.date, booked_at);
                    assert_eq!(
                        transaction.source_id.as_deref(),
                        Some(candidate.fingerprint.as_str())
                    );
                    assert_eq!(transaction.postings.len(), 2);
                    assert_eq!(transaction.postings[0].account, source_account);
                    assert_eq!(transaction.postings[1].account, account);
                    assert_eq!(cents(&transaction.postings[0]), amount_cents);
                    assert_eq!(cents(&transaction.postings[1]), -amount_cents);
                    // The parser reads back exactly the text the writer wrote:
                    // the sanitizer is the only thing allowed to change a
                    // description, and it runs before the line exists.
                    assert_eq!(
                        transaction.description,
                        crate::import::sanitize_description(description),
                        "description round-trip for {description:?}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 100,
            "the sweep must actually cover cases: {checked}"
        );
    }

    /// Appending is how the journal grows, so a file of many appended entries
    /// has to parse as many transactions with distinct indices.
    #[test]
    fn appended_entries_accumulate_without_running_together() {
        let mut journal = HEADER.to_string();
        for index in 0..50_i64 {
            let candidate = candidate(
                "2026-05-04",
                &format!("entry {index}"),
                -(index + 1) * 13,
                "EUR",
                "liabilities:card:amex",
            );
            journal.push_str(&render_journal_entry(&candidate, "expenses:food").unwrap());
        }
        let transactions = parse_ok(&journal);
        assert_eq!(transactions.len(), 50);
        let indices: std::collections::BTreeSet<u64> =
            transactions.iter().map(|t| t.index).collect();
        assert_eq!(indices.len(), 50);
        assert_eq!(cents(&transactions[49].postings[0]), -650);
    }

    // -- refusals: every unrecognized shape names its line -------------------

    fn refusal(journal: &str) -> JournalError {
        parse(journal).expect_err("this journal must be refused")
    }

    #[test]
    fn an_unbalanced_transaction_is_refused_with_its_line() {
        let error = refusal(
            "decimal-mark .\n\n2026-01-02 * skewed\n    assets:bank:a  10.00 EUR\n    expenses:food  3.00 EUR\n",
        );
        assert_eq!(error.line, 3);
        assert!(error.message.contains("does not balance"), "{error}");
        assert!(error.message.contains("13.00 EUR"), "{error}");
    }

    #[test]
    fn two_elided_amounts_are_refused_rather_than_guessed() {
        let error = refusal(
            "2026-01-02 * ambiguous\n    assets:bank:a  10.00 EUR\n    expenses:food\n    expenses:other\n",
        );
        assert_eq!(error.line, 4);
        assert!(error.message.contains("at most one"), "{error}");
    }

    #[test]
    fn an_impossible_date_is_refused() {
        let error =
            refusal("2026-02-30 * nonexistent\n    assets:bank:a  1.00 EUR\n    expenses:food\n");
        assert_eq!(error.line, 1);
        assert!(error.message.contains("calendar date"), "{error}");
    }

    #[test]
    fn an_unknown_directive_is_refused_rather_than_skipped() {
        // `include` is the shape that would silently hide a whole second file.
        let error = refusal("decimal-mark .\ninclude other.journal\n");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("unrecognized directive"), "{error}");
    }

    #[test]
    fn a_posting_outside_a_transaction_is_refused() {
        let error = refusal("decimal-mark .\n    assets:bank:a  1.00 EUR\n");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("outside a transaction"), "{error}");
    }

    #[test]
    fn an_amount_without_a_commodity_is_refused() {
        let error = refusal("2026-01-02 * bare\n    assets:bank:a  10.00\n    expenses:food\n");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("no commodity"), "{error}");
    }

    #[test]
    fn an_empty_source_id_is_refused() {
        let error = refusal(
            "2026-01-02 * anonymous\n    assets:bank:a  -1.00 EUR\n    expenses:food\n    ; source-id:\n",
        );
        assert_eq!(error.line, 1);
        assert!(error.message.contains("source-id"), "{error}");
    }

    // -- units ---------------------------------------------------------------

    #[test]
    fn numbers_follow_the_declared_decimal_mark() {
        assert_eq!(parse_number(1, "1.500,00", ',').unwrap(), (150_000, 2));
        assert_eq!(parse_number(1, "1,000.00", '.').unwrap(), (100_000, 2));
        assert_eq!(parse_number(1, "-45.99", '.').unwrap(), (-4_599, 2));
        assert_eq!(parse_number(1, "10", '.').unwrap(), (10, 0));
        assert!(parse_number(1, "1.2.3", '.').is_err());
    }

    #[test]
    fn a_commodity_sample_declares_its_decimal_mark() {
        assert_eq!(decimal_mark_of_sample("1,000.00 EUR"), Some('.'));
        assert_eq!(decimal_mark_of_sample("1.000,00 EUR"), Some(','));
    }

    #[test]
    fn transactions_come_back_in_date_order_keeping_file_order_within_a_date() {
        let journal = "2026-03-01 * later\n    assets:bank:a  -1.00 EUR\n    expenses:food\n\
                       \n2026-01-01 * earlier\n    assets:bank:a  -2.00 EUR\n    expenses:food\n\
                       \n2026-01-01 * same day, written second\n    assets:bank:a  -3.00 EUR\n    expenses:food\n";
        let transactions = parse_ok(journal);
        let dates: Vec<&str> = transactions.iter().map(|t| t.date.as_str()).collect();
        assert_eq!(dates, ["2026-01-01", "2026-01-01", "2026-03-01"]);
        // The index stays the file-order identity the projection's row ids are
        // built from, so ordering the report never renumbers a transaction.
        let indices: Vec<u64> = transactions.iter().map(|t| t.index).collect();
        assert_eq!(indices, [2, 3, 1]);
    }
}
