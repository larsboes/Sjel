//! Compare the open wishlist with the monthly balance reported by Finance.

pub use inventory_api::MonthlyBalance;

/// Fetch the optional balance from Finance. Call from a CLI or blocking worker.
pub fn monthly_balance() -> Option<MonthlyBalance> {
    let base_url =
        sjel_config::env_var("SJEL_FINANCE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".into());
    match crate::client::read_json(&base_url, "/api/monthly-balance", "finance-monthly-balance") {
        Ok(balance) => balance,
        Err(_) => {
            eprintln!("Finance monthly balance unavailable");
            None
        }
    }
}

/// An open wishlist can only be funded from a positive balance.
pub fn months_to_pay(open_cents: i64, balance: &MonthlyBalance) -> Option<f64> {
    if balance.median_cents <= 0 {
        return None;
    }
    Some(open_cents as f64 / balance.median_cents as f64)
}
