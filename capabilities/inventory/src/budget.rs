//! What the open wishlist costs, measured against what is left at the end of a month.
//!
//! Read straight from `finance_transaction_projection` in the shared store: finance requires an
//! inbound token over HTTP, and the number sits in the same file.
//!
//! There is no budget here. Nobody set a ceiling; this reports one measurement and leaves the
//! judgement to the reader.

use rusqlite::Connection;

type Error = Box<dyn std::error::Error>;

/// Completed months in the window. Twelve, so one yearly cycle (insurance, holidays, utility
/// settlements) is inside it once.
pub const WINDOW_MONTHS: usize = 12;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonthlyBalance {
    /// Median of income minus expenses per month, in cents. Negative means nothing is left.
    pub median_cents: i64,
    /// How many completed months were actually measured.
    pub months: usize,
    /// First and last month in the window, as `YYYY-MM`.
    pub from: String,
    pub to: String,
}

/// The monthly balance from finance's projection, or `None` when finance has written nothing on
/// this machine.
///
/// Transfers are left out: a move between own accounts is neither income nor expense. The current
/// month is left out because it is incomplete. The median, not the mean, so one month with a
/// yearly payment does not move the answer by hundreds of euros.
pub fn monthly_balance(conn: &Connection) -> Result<Option<MonthlyBalance>, Error> {
    let present: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master
          WHERE type='table' AND name='finance_transaction_projection'",
        [],
        |row| row.get(0),
    )?;
    if present == 0 {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT substr(booked_at, 1, 7) AS month,
                sum(CASE WHEN kind = 'income'  THEN amount_cents ELSE 0 END)
              - sum(CASE WHEN kind = 'expense' THEN amount_cents ELSE 0 END) AS balance
           FROM finance_transaction_projection
          WHERE kind IN ('income', 'expense')
            AND substr(booked_at, 1, 7) < strftime('%Y-%m', 'now')
          GROUP BY month
          ORDER BY month DESC
          LIMIT ?1",
    )?;
    let rows = stmt.query_map([WINDOW_MONTHS as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut months: Vec<(String, i64)> = rows.collect::<Result<_, _>>()?;
    if months.is_empty() {
        return Ok(None);
    }
    months.sort_by(|a, b| a.0.cmp(&b.0));
    let from = months.first().unwrap().0.clone();
    let to = months.last().unwrap().0.clone();

    let mut balances: Vec<i64> = months.iter().map(|(_, b)| *b).collect();
    balances.sort_unstable();
    let n = balances.len();
    let median_cents = if n % 2 == 1 {
        balances[n / 2]
    } else {
        (balances[n / 2 - 1] + balances[n / 2]) / 2
    };

    Ok(Some(MonthlyBalance {
        median_cents,
        months: n,
        from,
        to,
    }))
}

/// How many monthly balances the open wishlist costs.
///
/// `None` when the median is zero or negative: nothing accrues from a balance that is not
/// positive, and "-3.2 months" would be a number that answers nothing.
pub fn months_to_pay(open_cents: i64, balance: &MonthlyBalance) -> Option<f64> {
    if balance.median_cents <= 0 {
        return None;
    }
    Some(open_cents as f64 / balance.median_cents as f64)
}
