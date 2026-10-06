//! What finance answers on `GET /api/links` (libs/links/ISA.md, LNK-7).
//!
//! Finance holds one kind of reference: the `axon-trip-id` tag, projected as
//! `TransactionRow::trip_id`. A transaction's linkable id is `fin:tx:<source_id>`, because
//! `source_id` is the fingerprint that survives a projection rebuild and the row `id` is a journal
//! position that does not (`analytics.rs`, the `transaction_{index}_…` format). A tagged row with
//! no `source_id` is counted in `unlinkable`, never dropped in silence.
//!
//! Every currency is answered. The dashboard scopes to one; a trip abroad is the case where that
//! would hide rows.

use std::collections::BTreeMap;

use sjel_links::{Link, Links, TypedId};

use crate::analytics::{TransactionKind, TransactionRow};

/// The id kinds finance answers for. Declared again as `links_to` in `service.toml`; the test
/// below holds the two equal.
pub const LINKS_TO: &[&str] = &["trip:plan"];

/// The rows that reference `target`, one link per transaction. A transaction with several
/// expense postings is several rows with one `source_id`; their amounts are summed per currency.
pub fn links(rows: &[TransactionRow], target: &TypedId<'_>) -> Result<Links, String> {
    if !LINKS_TO.contains(&target.kind) {
        return Err(format!(
            "finance holds no references to `{}`; it answers {LINKS_TO:?}",
            target.kind
        ));
    }
    let mut unlinkable = 0;
    // source_id -> (first row, signed cents per currency). BTreeMap keeps the order stable.
    let mut grouped: BTreeMap<&str, (&TransactionRow, BTreeMap<&str, i64>)> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| row.trip_id.as_deref() == Some(target.id))
    {
        let Some(source_id) = row.source_id.as_deref() else {
            unlinkable += 1;
            continue;
        };
        let signed = match row.kind {
            TransactionKind::Expense => -row.amount_cents,
            _ => row.amount_cents,
        };
        let entry = grouped
            .entry(source_id)
            .or_insert_with(|| (row, BTreeMap::new()));
        *entry.1.entry(row.currency.as_str()).or_default() += signed;
    }
    let mut links: Vec<Link> = grouped
        .into_iter()
        .map(|(source_id, (row, sums))| Link {
            id: format!("fin:tx:{source_id}"),
            kind: "fin:tx".into(),
            title: row.description.clone(),
            at: Some(row.date.clone()),
            meta: Some(
                sums.iter()
                    .map(|(currency, cents)| amount(*cents, currency))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
            via: "trip_id",
        })
        .collect();
    links.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    Ok(Links {
        to: target.id.to_owned(),
        links,
        unlinkable,
    })
}

/// `−49.90 EUR`. The minus is U+2212, as the dashboard prints it.
fn amount(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "−" } else { "+" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02} {currency}", abs / 100, abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source_id: Option<&str>, trip: &str, cents: i64, currency: &str) -> TransactionRow {
        TransactionRow {
            id: "transaction_1_0_eur".into(),
            date: "2026-10-07".into(),
            description: "DB Fernverkehr".into(),
            kind: TransactionKind::Expense,
            account: "assets:bank".into(),
            category: "expenses:travel:rail".into(),
            amount_cents: cents,
            currency: currency.into(),
            source_id: source_id.map(Into::into),
            purpose: None,
            trip_id: Some(trip.into()),
            cash_amount_cents: cents,
            shared_cents: 0,
            reimbursement_for: None,
        }
    }

    fn to(id: &str) -> TypedId<'_> {
        TypedId::parse(id).unwrap()
    }

    #[test]
    fn postings_of_one_transaction_are_one_link_and_every_currency_counts() {
        let rows = [
            row(Some("abc"), "trip:plan:1", 4990, "EUR"),
            row(Some("abc"), "trip:plan:1", 1000, "EUR"),
            row(Some("def"), "trip:plan:1", 2500, "CHF"),
            row(Some("ghi"), "trip:plan:2", 9999, "EUR"),
        ];
        let answer = links(&rows, &to("trip:plan:1")).unwrap();
        let ids: Vec<_> = answer.links.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["fin:tx:abc", "fin:tx:def"]);
        assert_eq!(answer.links[0].meta.as_deref(), Some("−59.90 EUR"));
        assert_eq!(answer.links[1].meta.as_deref(), Some("−25.00 CHF"));
        assert_eq!(answer.unlinkable, 0);
    }

    #[test]
    fn a_row_without_source_id_is_counted_not_dropped() {
        let rows = [row(None, "trip:plan:1", 100, "EUR")];
        let answer = links(&rows, &to("trip:plan:1")).unwrap();
        assert!(answer.links.is_empty());
        assert_eq!(answer.unlinkable, 1);
    }

    #[test]
    fn an_undeclared_kind_is_refused() {
        assert!(links(&[], &to("ent:1")).is_err());
    }

    #[test]
    fn links_to_matches_the_manifest() {
        let manifest = include_str!("../service.toml");
        let line = manifest
            .lines()
            .find(|line| line.starts_with("links_to"))
            .expect("service.toml declares links_to");
        for kind in LINKS_TO {
            assert!(
                line.contains(&format!("\"{kind}\"")),
                "{kind} missing from {line}"
            );
        }
    }
}
