//! Finance projections available to the local Places capability.

use serde::{Deserialize, Serialize};

pub const EXPENSES: &str = "/places/expenses";
pub const CANDIDATES: &str = "/places/candidates";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Expense {
    pub source_id: Option<String>,
    pub description: String,
    pub amount_cents: i64,
    pub booked_at: String,
    pub category: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Candidate {
    pub fingerprint: String,
    pub street: Option<String>,
    pub city: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}
