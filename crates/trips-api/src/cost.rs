//! Trip cost response models shared by capability and clients.

use serde::{Deserialize, Serialize};

/// Finance's four figures, or four nulls and a reason. Never a zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actuals {
    pub ok: bool,
    pub reason: Option<String>,
    /// The unit finance stated for these five figures, and null when it stated
    /// none. Carried rather than assumed: the card used to render them under the
    /// PLAN's currency, which is a claim about somebody else's numbers.
    pub currency: Option<String>,
    pub personal_cents: Option<i64>,
    pub gross_cash_outflow_cents: Option<i64>,
    pub reimbursed_cents: Option<i64>,
    pub outstanding_cents: Option<i64>,
    pub posting_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedOptions {
    /// A float, because the schema says a float. Null when nothing is priced.
    pub total: Option<f64>,
    /// Always null, and declared rather than omitted: `optionSetPayload` and
    /// `transportPayload` carry no currency field at all, so the unit of this
    /// number is genuinely unknown.
    pub currency: Option<String>,
    pub priced_items: usize,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByCurrency {
    pub currency: String,
    pub booked_cents: i64,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByStage {
    pub stage_id: String,
    pub sequence: usize,
    pub origin: String,
    pub destination: String,
    /// Null, exactly like the top-level total, when the items on this stage do
    /// not agree on a currency. `currency` then carries none and `reason` says
    /// which disagreement it was.
    pub booked_cents: Option<i64>,
    pub currency: Option<String>,
    pub reason: Option<String>,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unattributed {
    pub booked_cents: Option<i64>,
    pub currency: Option<String>,
    pub reason: Option<String>,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub source: String,
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostRollup {
    pub plan_id: String,
    pub currency: Option<String>,
    pub planned_cents: Option<i64>,
    /// Null rather than a sum when the bookings are in more than one currency;
    /// `booked_reason` then says so and `by_currency` carries one row each.
    pub booked_cents: Option<i64>,
    pub booked_reason: Option<String>,
    pub actuals: Actuals,
    pub selected_options: SelectedOptions,
    pub by_currency: Vec<ByCurrency>,
    pub by_stage: Vec<ByStage>,
    pub unattributed: Unattributed,
    pub sources: Vec<Source>,
}
