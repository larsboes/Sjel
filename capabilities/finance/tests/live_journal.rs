//! Read-only proof that the own parser reads a real journal.
//!
//! The journal this is aimed at is private: it lives in the active overlay and
//! is canonical financial truth. So the path is never written down here. Point
//! `SJEL_FINANCE_JOURNAL` at a journal and the checks run against it; leave it
//! unset and they report why they did nothing, which is what happens in CI and
//! on any machine that does not hold the file.
//!
//! Nothing in this file opens the journal for writing, creates a temporary copy
//! or names an account. It reads, counts, and prints the counts.

use std::collections::BTreeSet;

fn journal() -> Option<(std::path::PathBuf, String)> {
    let path = axon_config::env_var_os("SJEL_FINANCE_JOURNAL")?;
    let path = std::path::PathBuf::from(path);
    if !path.is_file() {
        eprintln!("SJEL_FINANCE_JOURNAL is set but names no file; skipping");
        return None;
    }
    let text = std::fs::read_to_string(&path).expect("the journal must be readable");
    Some((path, text))
}

/// The whole live journal parses, every transaction keeps an identity, and the
/// identities are distinct. A source-id lost here is a venue link in `places`
/// that stops resolving, so the count is asserted rather than eyeballed.
#[test]
fn the_live_journal_parses_with_every_source_id_intact() {
    let Some((path, text)) = journal() else {
        eprintln!("set SJEL_FINANCE_JOURNAL to run the live-journal proof; skipping");
        return;
    };

    let transactions = match finance::journal::parse(&text) {
        Ok(transactions) => transactions,
        // The refusal names a line number, which is the point of it. Surface
        // that rather than a bare parse failure.
        Err(error) => panic!("the live journal must parse: {error}"),
    };

    let lines = text.lines().count();
    let written = text
        .lines()
        .filter(|line| line.trim_start().starts_with("; source-id:"))
        .count();
    let recovered = transactions
        .iter()
        .filter(|transaction| transaction.source_id.is_some())
        .count();
    let distinct: BTreeSet<&str> = transactions
        .iter()
        .filter_map(|transaction| transaction.source_id.as_deref())
        .collect();

    println!("journal file:        {}", path.display());
    println!("lines:               {lines}");
    println!("transactions parsed: {}", transactions.len());
    println!("source-id comments:  {written}");
    println!("source-ids returned: {recovered}");
    println!("source-ids distinct: {}", distinct.len());

    assert_eq!(
        recovered, written,
        "every written source-id must come back out of the parser"
    );
    assert_eq!(
        recovered,
        transactions.len(),
        "every transaction in this journal carries an identity"
    );
    assert_eq!(
        distinct.len(),
        recovered,
        "source-ids identify a transaction, so they cannot repeat"
    );

    // Balance is the property the old `hledger check` existed to guard. Parsing
    // already enforced it; assert the postings really came back so a future
    // change cannot quietly return empty transactions and still pass.
    assert!(
        transactions
            .iter()
            .all(|transaction| transaction.postings.len() >= 2),
        "double-entry means at least two postings"
    );
}

/// `validate` is what the check endpoint answers with, so it runs over the same
/// real file rather than only over fixtures.
#[test]
fn the_live_journal_validates() {
    let Some((_, text)) = journal() else {
        eprintln!("set SJEL_FINANCE_JOURNAL to run the live-journal proof; skipping");
        return;
    };
    if let Err(error) = finance::journal::validate(&text) {
        panic!("the live journal must validate: {error}");
    }
    println!("validate: ok");
}
