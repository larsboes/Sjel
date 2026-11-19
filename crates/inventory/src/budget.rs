//! Compare the open wishlist with the monthly balance reported by Finance.

use std::io::Read;
use std::time::Duration;

pub use inventory_api::MonthlyBalance;

type Error = Box<dyn std::error::Error>;

/// Fetch the optional balance from Finance. Call from a CLI or blocking worker.
pub fn monthly_balance() -> Option<MonthlyBalance> {
    let base_url = sjel_config::env_var("SJEL_FINANCE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8090".into());
    match fetch_monthly_balance(&base_url) {
        Ok(balance) => balance,
        Err(_) => {
            eprintln!("Finance monthly balance unavailable");
            None
        }
    }
}

fn fetch_monthly_balance(base_url: &str) -> Result<Option<MonthlyBalance>, Error> {
    let client = sjel_http::loopback_builder(
        sjel_http::Purpose::new("finance-monthly-balance"),
        Duration::from_secs(1),
    )
    .build()?;
    let mut url = client.get(base_url).build()?.url().clone();
    if url.host_str() == Some("localhost") {
        url.set_host(Some("127.0.0.1"))?;
    }
    if !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Finance URL must use a loopback address".into());
    }
    url.set_path("/api/monthly-balance");
    url.set_query(None);
    url.set_fragment(None);
    let request = sjel_server::InboundAuth::with_loopback_auth(client.get(url.clone()), url.as_str());
    let response = request.send()?.error_for_status()?;
    if !response.status().is_success() {
        return Err("unexpected Finance response status".into());
    }
    Ok(serde_json::from_reader(response.take(4096))?)
}

/// An open wishlist can only be funded from a positive balance.
pub fn months_to_pay(open_cents: i64, balance: &MonthlyBalance) -> Option<f64> {
    if balance.median_cents <= 0 {
        return None;
    }
    Some(open_cents as f64 / balance.median_cents as f64)
}
