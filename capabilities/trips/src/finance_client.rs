//! The one call trips makes to finance, and the one place its failure is
//! described.
//!
//! Separate from `cost.rs` and from the handler on purpose. `flight_when`
//! degrades an unreachable calendar into "every day free" with
//! `.unwrap_or_default()` and says nothing in the body — a guess that looks like
//! a measurement. Money must not degrade that way, so the failure has a type
//! (`Unreachable`) carrying a reason, and nothing in this file can produce a
//! zero by accident. There is no `unwrap_or_default` here.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where finance-server listens. Mirrors `calendar_base_url()` in `server.rs`,
/// and this is now the THIRD capability hardcoding a sibling's port, which
/// strengthens the case for the spine mechanism that comment defers
/// (service-runner exporting declared siblings' ports).
pub fn finance_base_url() -> String {
    sjel_config::env_var("SJEL_FINANCE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

/// Long enough for a loopback query, short enough that a stopped finance never
/// makes the trip view feel broken.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Finance's four per-trip figures plus its posting count, carried whole.
///
/// All six fields, not one flattened total. `reimbursed` and `outstanding` are
/// filled only from shared expenses, so on a trip with friends the four numbers
/// differ and that difference IS the shared-cost surface. A single
/// `spent_cents` answers "was it worth it" wrongly.
///
/// Field names are finance's own (`capabilities/finance/src/analytics.rs`,
/// `TripSpendingSummary`), read at the TOP LEVEL of the response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TripSpending {
    pub trip_id: String,
    pub personal_spending_cents: i64,
    pub gross_cash_outflow_cents: i64,
    pub reimbursed_cents: i64,
    pub outstanding_cents: i64,
    pub expense_posting_count: usize,
    /// The unit finance says those five figures are in.
    ///
    /// Optional because the route is finance's to write and may not echo it;
    /// absent means "finance did not say", which is not the same as "the same
    /// currency as the plan". `cost::roll_up` refuses to relabel a figure whose
    /// stated unit disagrees with the plan's.
    #[serde(default)]
    pub currency: Option<String>,
}

/// Why the actuals are unknown, in words a reader can act on.
///
/// A type rather than an `Option`, because "finance is not running" and "no
/// transaction is tagged to this trip" are different answers and the card has to
/// say which one it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreachable {
    pub reason: String,
}

impl Unreachable {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Ask finance what one trip actually cost.
///
/// Every failure — a transport error, a timeout, a non-2xx, a 404 because the
/// route does not exist yet, or a body that is not the six documented fields —
/// yields the same `Unreachable` with a reason. It never yields zeroes.
pub fn trip_spending(plan_id: &str) -> Result<TripSpending, Unreachable> {
    trip_spending_at(&finance_base_url(), plan_id)
}

/// The same call with the endpoint supplied, so a test never touches process
/// env.
///
/// `places::climate::fetch_normals` takes its URL the same way, for the reason
/// `capabilities/places/src/geocode.rs:555-557` records: tests mutating one
/// process-global variable under cargo's thread-parallel harness is a flake, not
/// a test.
pub fn trip_spending_at(base_url: &str, plan_id: &str) -> Result<TripSpending, Unreachable> {
    let url = format!("{}/api/trips/{}/spending", base_url, urlencode(plan_id));
    let client = sjel_http::client(sjel_http::Purpose::new("trips-finance"), TIMEOUT)
        .map_err(|error| Unreachable::new(format!("finance client: {error}")))?;
    let mut request = client.get(&url);
    // The inbound gate is on every route except /health and /ready, so without
    // the token a gated finance reads as "not running" — the one wrong answer
    // this must not give. Resolved per request, so rotating the token file needs
    // no restart here (the sjel-status precedent).
    if let Some(bearer) = sjel_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header(reqwest::header::AUTHORIZATION, bearer);
    }
    // `without_url` before the message is built: reqwest prints the full request
    // URL in its Display, and this reason is served in the cost body and printed
    // verbatim in the card's footer. Same rule, same reason, as
    // `capabilities/places/src/climate.rs` applies to the provider error there.
    let response = request.send().map_err(|error| {
        Unreachable::new(format!("finance is not reachable: {}", error.without_url()))
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(Unreachable::new(format!(
            "finance answered {status} for this trip's spending"
        )));
    }
    response.json::<TripSpending>().map_err(|_| {
        Unreachable::new("finance answered an unexpected shape for this trip's spending")
    })
}

/// A plan id is `trip:plan:<hex>`, so the colons need escaping and nothing else
/// does. Hand-rolled rather than a dependency, and deliberately conservative:
/// anything outside the unreserved set is percent-encoded.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_id_survives_the_path_intact() {
        assert_eq!(urlencode("trip:plan:18d285e1"), "trip%3Aplan%3A18d285e1");
        assert_eq!(urlencode("plain-id_1.0~x"), "plain-id_1.0~x");
    }

    /// Port 1 on loopback: nothing listens, the connection is refused. The
    /// contract is that this is an `Err` with a reason, never a `TripSpending`
    /// of zeroes.
    #[test]
    fn an_unreachable_finance_is_an_error_with_a_reason_and_never_a_zero() {
        // The endpoint is a parameter, not an env var: this test runs beside
        // sixty others in one process.
        let error = trip_spending_at("http://127.0.0.1:1", "trip:plan:synthetic")
            .expect_err("a refused connection must not become zeroes");
        assert!(
            error.reason.contains("not reachable"),
            "the reason must be actionable: {}",
            error.reason
        );
        // The reason is served in the cost body and printed in the card footer,
        // so it carries no request URL: no scheme, no host, no plan id.
        for leaked in ["http", "127.0.0.1", "synthetic"] {
            assert!(
                !error.reason.contains(leaked),
                "the request URL leaked into the reason ({leaked}): {}",
                error.reason
            );
        }
    }
}
