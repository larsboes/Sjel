//! The one identity of a reviewed import candidate.
//!
//! A candidate fingerprint is the SHA-256 of the tuple that makes an imported
//! row that row and no other: when it was booked, what it moved, what it said,
//! and which account it moved on. `capabilities/finance` mints it at import
//! time and writes it into the journal as the `source-id` tag; the projection
//! carries it; `capabilities/places` re-derives it from the raw export to hang
//! a venue on a transaction it never stored (`capabilities/places/README.md`,
//! D2).
//!
//! # Why this is a crate and not a copy
//!
//! It was two copies until 2026-08-28 — `capabilities/finance/src/import.rs`
//! and `capabilities/places/src/backfill.rs` — and one of them wrote so:
//! "The algorithm is a deliberate copy". Measured that day against the live
//! store, 263 rows of `places_transaction_places` resolve only while the two
//! agree byte for byte. A divergence would not raise an error. It would
//! silently stop matching, and the backfill would report the rows as unmatched
//! and move on, because reporting an unmatched row is its honest-failure rule.
//!
//! So the hash lives once. `places` may not depend on `finance` (a capability
//! reaching into another capability's crate is the boundary this repo does not
//! cross), and the shape belongs to neither of them alone, so it is a lib both
//! depend on instead.
//!
//! # The format is frozen
//!
//! Every byte below is load-bearing: the separator, the field order, the
//! decimal-free integer cents, the `csv-occurrence` literal. Changing any of
//! them orphans every live link and every `source-id` already written into the
//! canonical journal. This crate has no migration path by design — a new
//! identity scheme would be a new function with a new name, not an edit here.

use sha2::{Digest, Sha256};

/// The separator between fields. `0xff` is never a byte of well-formed UTF-8,
/// so no field content can imitate it and no two distinct tuples can collide by
/// running their fields together. It terminates every field, including the
/// last, so a trailing empty field still changes the hash.
const FIELD_END: u8 = 0xff;

/// The primitive: SHA-256 over the parts, each terminated by [`FIELD_END`],
/// lowercase hex.
///
/// Public because the holdings ledger in
/// `capabilities/finance/src/investment.rs` identifies its rows with the same
/// primitive over a different tuple. It is deliberately not the interface for
/// the candidate identity — use [`CandidateKey`] for that, so the field order
/// is stated in one place instead of at each call.
pub fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([FIELD_END]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

/// What makes an imported row that row.
///
/// The two callers differ only in how much of it they know, and each states its
/// own case in the fields rather than in the argument order:
///
/// - `finance` reads all six from the CSV mapping, so `currency` may be any
///   declared code and `source_reference` is `Some` whenever the bank gave one.
/// - `places` re-derives from the raw American Express export, where every
///   measured row has an empty reference cell and an EUR currency cell, so it
///   passes `currency: "EUR"` and `source_reference: None`. Those are the two
///   assumptions its parity rests on, and a row that breaks either one hashes
///   differently and is reported unmatched rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateKey<'a> {
    /// The booking date, already normalized to `YYYY-MM-DD`.
    pub booked_at: &'a str,
    /// Signed minor units. Hashed as its decimal digits, so the identity does
    /// not depend on how any caller chose to format money.
    pub amount_cents: i64,
    /// The three-letter code, uppercase.
    pub currency: &'a str,
    /// The bank's text, whitespace runs already collapsed to single spaces.
    pub description: &'a str,
    /// The bank's own reference for the row, when it gives one. `None` hashes
    /// identically to an empty reference, which is what an absent one is.
    pub source_reference: Option<&'a str>,
    /// The account the row moves on, in the journal's own account naming.
    pub source_account: &'a str,
}

impl CandidateKey<'_> {
    /// The identity of the row itself.
    pub fn fingerprint(&self) -> String {
        let amount = self.amount_cents.to_string();
        digest(&[
            self.booked_at,
            &amount,
            self.currency,
            self.description,
            self.source_reference.unwrap_or(""),
            self.source_account,
        ])
    }

    /// The identity of the `occurrence`-th row that shares this tuple within
    /// one export file, counting from zero.
    ///
    /// Rows with no bank reference are not distinguishable by content: two
    /// identical coffees on one day are two real charges, and dropping the
    /// second would lose money from the ledger. So the first keeps the plain
    /// fingerprint — which is why importing a file that has no repeats is
    /// unaffected by this rule existing — and each later one is folded again
    /// with its ordinal. See [`repeated`].
    pub fn repeated_fingerprint(&self, occurrence: usize) -> String {
        repeated(&self.fingerprint(), occurrence)
    }
}

/// The repetition rule applied to an already-computed fingerprint, for callers
/// that count occurrences of the base hash itself.
///
/// `occurrence == 0` returns `base` unchanged, so the common row is identified
/// by its own content and nothing else.
pub fn repeated(base: &str, occurrence: usize) -> String {
    if occurrence == 0 {
        return base.to_string();
    }
    let ordinal = occurrence.to_string();
    digest(&["csv-occurrence", base, &ordinal])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden vectors. The expected values were computed from the two
    /// pre-extraction implementations at 58e9037 and are frozen: they are the
    /// hashes already written into the canonical journal and the live link
    /// table, so a change that moves them is a change that orphans real data.
    ///
    /// The inputs are synthetic. The live proof runs against the real store in
    /// `capabilities/places/tests/live_fingerprint.rs`, which reads its golden
    /// values at run time rather than publishing anyone's transactions here.
    #[test]
    fn the_candidate_identity_is_frozen() {
        let key = CandidateKey {
            booked_at: "2026-08-02",
            amount_cents: -1234,
            currency: "EUR",
            description: "Example market",
            source_reference: Some("row-1"),
            source_account: "liabilities:card:example",
        };
        assert_eq!(
            key.fingerprint(),
            "5f68ea6e5f8f745645382dffe6168be74c0906dcd2f6fc587ab584aa50a6815a"
        );

        // The reference-less shape, which is the one 263 live links rest on,
        // and its first repeat.
        let no_reference = CandidateKey {
            description: "Coffee",
            amount_cents: -250,
            source_reference: None,
            ..key
        };
        assert_eq!(
            no_reference.fingerprint(),
            "f2ba65bcaaef8775d9ef392f4bffbbc987287912a903a07ea9e6abde3ee2f5d6"
        );
        assert_eq!(
            no_reference.repeated_fingerprint(1),
            "96f48d68515fc725b1f7eb8ea2e76f99261e0949d01d4f3e8054d0f7e03c4326"
        );
    }

    /// The shape `places` passes: no reference, currency stated rather than
    /// read. It has to be a separate vector because an absent reference is the
    /// case 263 live links are built on.
    #[test]
    fn an_absent_reference_hashes_as_an_empty_one() {
        let common = |source_reference| CandidateKey {
            booked_at: "2026-08-02",
            amount_cents: -1234,
            currency: "EUR",
            description: "Example market",
            source_reference,
            source_account: "liabilities:card:example",
        };
        assert_eq!(
            common(None).fingerprint(),
            common(Some("")).fingerprint(),
            "an absent reference and an empty one are the same absence"
        );
    }

    /// The first of a repeated tuple is not folded, so a file with no repeats
    /// produces exactly the fingerprints it did before the rule existed.
    #[test]
    fn the_first_occurrence_is_the_plain_fingerprint() {
        let key = CandidateKey {
            booked_at: "2026-08-02",
            amount_cents: -250,
            currency: "EUR",
            description: "Coffee",
            source_reference: None,
            source_account: "liabilities:card:example",
        };
        assert_eq!(key.repeated_fingerprint(0), key.fingerprint());
        assert_eq!(repeated(&key.fingerprint(), 0), key.fingerprint());
    }

    /// Each later repeat gets its own identity, and the ordinal is part of the
    /// hash rather than a suffix on it — a suffix would be forgeable by a
    /// description that happened to end the same way.
    #[test]
    fn later_occurrences_are_distinct_and_stay_64_hex() {
        let key = CandidateKey {
            booked_at: "2026-08-02",
            amount_cents: -250,
            currency: "EUR",
            description: "Coffee",
            source_reference: None,
            source_account: "liabilities:card:example",
        };
        let ids: Vec<String> = (0..4).map(|n| key.repeated_fingerprint(n)).collect();
        for id in &ids {
            assert_eq!(id.len(), 64);
            assert!(id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        }
        let distinct: std::collections::BTreeSet<&String> = ids.iter().collect();
        assert_eq!(distinct.len(), ids.len(), "a repeat may not collide");
    }

    /// The terminator, not a joiner. Without a terminator on the last field
    /// these two tuples would hash the same, and a description could absorb an
    /// account name.
    #[test]
    fn fields_cannot_run_together() {
        assert_ne!(digest(&["ab", "c"]), digest(&["a", "bc"]));
        assert_ne!(digest(&["a", ""]), digest(&["a"]));
        assert_ne!(digest(&[]), digest(&[""]));
    }

    /// The amount is hashed as digits, so the identity does not move when a
    /// caller changes how it renders money.
    #[test]
    fn the_amount_is_hashed_as_integer_cents() {
        let key = |amount_cents| CandidateKey {
            booked_at: "2026-08-02",
            amount_cents,
            currency: "EUR",
            description: "Example",
            source_reference: None,
            source_account: "assets:bank:example",
        };
        // A sign flip is a different transaction, so it is a different identity.
        assert_ne!(key(-1234).fingerprint(), key(1234).fingerprint());
        // And the extremes still produce one well-formed identity each.
        assert_eq!(key(i64::MIN).fingerprint().len(), 64);
        assert_ne!(key(i64::MIN).fingerprint(), key(i64::MAX).fingerprint());
    }
}
