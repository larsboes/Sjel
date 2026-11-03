//! The monthly balance against invented bookings. Real numbers stay in the overlay; this checks
//! which month and which booking counts, and what happens when nothing is left.

use rusqlite::Connection;
use sjel_inventory::budget::{monthly_balance, months_to_pay};

/// An empty database with finance's projection table, only the columns this query reads.
fn db_with_finance() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory");
    conn.execute_batch(
        "CREATE TABLE finance_transaction_projection (
            id TEXT PRIMARY KEY, booked_at TEXT, kind TEXT, amount_cents INTEGER
         );",
    )
    .unwrap();
    conn
}

/// Euro in, cents out; clippy's `inconsistent_digit_grouping` forbids writing `100_00`.
fn eur(amount: i64) -> i64 {
    amount * 100
}

fn book(conn: &Connection, id: &str, day: &str, kind: &str, cents: i64) {
    conn.execute(
        "INSERT INTO finance_transaction_projection (id, booked_at, kind, amount_cents)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, day, kind, cents],
    )
    .unwrap();
}

/// Without finance there is no number, which is `None` and not zero.
#[test]
fn without_finance_there_is_no_balance() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(monthly_balance(&conn).unwrap().is_none());
}

#[test]
fn an_empty_projection_gives_no_balance_either() {
    let conn = db_with_finance();
    assert!(monthly_balance(&conn).unwrap().is_none());
}

/// Eleven months at +100 and one at -5000: the mean would be negative, the median is 100.
#[test]
fn one_outlier_does_not_tip_the_median() {
    let conn = db_with_finance();
    for m in 1..=11 {
        book(
            &conn,
            &format!("e{m}"),
            &format!("2025-{m:02}-15"),
            "income",
            eur(100),
        );
    }
    book(&conn, "crash", "2025-12-15", "expense", eur(5000));

    let b = monthly_balance(&conn).unwrap().expect("twelve months");
    assert_eq!(b.months, 12);
    assert_eq!(b.median_cents, eur(100));
}

/// A transfer between own accounts is neither income nor expense.
#[test]
fn a_transfer_does_not_count() {
    let conn = db_with_finance();
    book(&conn, "a", "2025-05-01", "income", eur(200));
    book(&conn, "b", "2025-05-02", "expense", eur(50));
    book(&conn, "c", "2025-05-03", "transfer", eur(900));

    let b = monthly_balance(&conn).unwrap().unwrap();
    assert_eq!(b.months, 1);
    assert_eq!(b.median_cents, eur(150));
}

/// The current month is incomplete and is no measurement.
#[test]
fn the_current_month_does_not_count() {
    let conn = db_with_finance();
    let today: String = conn
        .query_row("SELECT strftime('%Y-%m-15','now')", [], |r| r.get(0))
        .unwrap();
    book(&conn, "now", &today, "expense", eur(9999));
    assert!(
        monthly_balance(&conn).unwrap().is_none(),
        "only the current month has bookings, so nothing completed is left to measure"
    );

    book(&conn, "before", "2025-03-10", "income", eur(300));
    let b = monthly_balance(&conn).unwrap().unwrap();
    assert_eq!((b.months, b.median_cents), (1, eur(300)));
    assert_eq!((b.from.as_str(), b.to.as_str()), ("2025-03", "2025-03"));
}

#[test]
fn a_negative_balance_gives_no_month_count() {
    let conn = db_with_finance();
    book(&conn, "a", "2025-05-01", "income", eur(100));
    book(&conn, "b", "2025-05-02", "expense", eur(400));
    let b = monthly_balance(&conn).unwrap().unwrap();
    assert_eq!(b.median_cents, -eur(300));
    assert_eq!(months_to_pay(eur(1000), &b), None);
}

#[test]
fn a_positive_balance_gives_the_month_count() {
    let conn = db_with_finance();
    book(&conn, "a", "2025-05-01", "income", eur(500));
    book(&conn, "b", "2025-05-02", "expense", eur(250));
    let b = monthly_balance(&conn).unwrap().unwrap();
    assert_eq!(b.median_cents, eur(250));
    assert_eq!(months_to_pay(eur(1000), &b), Some(4.0));
}
