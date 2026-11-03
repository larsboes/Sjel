//! What a trip was meant to cost, what was committed to, and what was actually
//! paid — joined on read, stored nowhere.
//!
//! Three sources, three grains, and the whole point is that they are kept apart:
//!
//! - **Intent** is `trips_plans.budget_cents`, in the plan's own currency.
//! - **Committed** is `booking.amount_cents`, and `stay.amount_cents` for a stay
//!   that carries evidence of a booking: integer minor units with an ISO code
//!   beside them. A stay copied from a search result has a price and no booking,
//!   and counting it put EUR 707 of "booked" on a plan with nothing booked
//!   (Berlin, 2026-09-25).
//! - **Offered** is `option_set` and `transport` prices, which are plain floats
//!   with no currency field anywhere in the schema. They are reported in their
//!   own block labelled offered-not-paid and are NEVER summed into the committed
//!   total: adding a float of unknown currency to an integer minor-unit sum
//!   produces a number that is wrong in a way nobody can see.
//! - **Actual** is finance's four per-trip figures, carried whole.
//!
//! Every rule here is a pure function over a `PlanDetails` and a
//! `Result<TripSpending, Unreachable>`, so all of it is testable with no network
//! and no database.

use serde::Serialize;
use serde_json::Value;

use crate::finance_client::{TripSpending, Unreachable};
use crate::store::PlanDetails;

/// Finance's four figures, or four nulls and a reason. Never a zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectedOptions {
    /// A float, because the schema says a float. Null when nothing is priced.
    pub total: Option<f64>,
    /// Always null, and declared rather than omitted: `optionSetPayload` and
    /// `transportPayload` carry no currency field at all, so the unit of this
    /// number is genuinely unknown.
    pub currency: Option<String>,
    pub priced_items: usize,
    pub note: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ByCurrency {
    pub currency: String,
    pub booked_cents: i64,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unattributed {
    pub booked_cents: Option<i64>,
    pub currency: Option<String>,
    pub reason: Option<String>,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub source: &'static str,
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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

const OFFERED_NOT_PAID: &str = "offered, not paid; these prices are floats with no currency \
     (schemas/trip-plan.schema.json optionSetPayload, transportPayload)";

/// Whether an item's price is money committed rather than money offered.
///
/// A `booking` is always a purchase made elsewhere. A `stay` is one only when it
/// says so: `booked: true`, or the provider's `order_ref`. Both item types carry
/// an integer minor-unit price with an ISO code.
fn is_committed(item: &crate::store::PlanItem) -> bool {
    match item.item_type.as_str() {
        "booking" => true,
        "stay" => {
            item.payload.get("booked").and_then(Value::as_bool) == Some(true)
                || item
                    .payload
                    .get("order_ref")
                    .and_then(Value::as_str)
                    .is_some_and(|reference| !reference.trim().is_empty())
        }
        _ => false,
    }
}

fn cents(payload: &Value) -> Option<i64> {
    payload.get("amount_cents")?.as_i64()
}

fn currency_of(payload: &Value) -> Option<String> {
    payload
        .get("currency")?
        .as_str()
        .map(|code| code.trim().to_uppercase())
        .filter(|code| !code.is_empty())
}

/// What one priced item says its unit is: its own currency, else the plan's.
///
/// One function, so the total, the per-stage rows and the unattributed row all
/// read the same answer. They used to disagree: the total skipped an item whose
/// unit it could not resolve while the stage rows added it anyway.
fn unit_of(payload: &Value, plan_currency: Option<&String>) -> Option<String> {
    currency_of(payload).or_else(|| plan_currency.cloned())
}

const NO_UNIT: &str = "a priced item carries no currency and the plan carries none either";

/// A running total that refuses to add two units.
///
/// The module docstring's rule — adding a figure of unknown currency to an
/// integer minor-unit sum produces a number that is wrong in a way nobody can
/// see — applies to a stage row exactly as it applies to the headline. The first
/// item that disagrees, or that carries no unit at all, turns the total into
/// `None` and records why.
#[derive(Debug, Default)]
struct Tally {
    cents: i64,
    currency: Option<String>,
    reason: Option<String>,
    item_count: usize,
}

impl Tally {
    fn add(&mut self, amount: i64, unit: Option<String>) {
        self.item_count += 1;
        let Some(unit) = unit else {
            self.refuse(NO_UNIT.to_string());
            return;
        };
        match &self.currency {
            None => {
                self.currency = Some(unit);
                self.cents += amount;
            }
            Some(held) if *held == unit => self.cents += amount,
            Some(held) => self.refuse(format!(
                "prices are in {held} and {unit}; two units are never added"
            )),
        }
    }

    /// First reason wins: it names the disagreement a reader can act on, and a
    /// later one adds no information.
    fn refuse(&mut self, reason: String) {
        if self.reason.is_none() {
            self.reason = Some(reason);
        }
    }

    fn total(&self) -> Option<i64> {
        self.reason.is_none().then_some(self.cents)
    }

    /// The unit the total is in, and none once the total is refused: a currency
    /// beside a null figure would read as a claim about a number nobody has.
    fn unit(&self) -> Option<String> {
        self.reason
            .is_none()
            .then(|| self.currency.clone())
            .flatten()
    }
}

/// Roll one plan up. Pure: the caller decides whether finance was asked.
pub fn roll_up(details: &PlanDetails, spending: Result<TripSpending, Unreachable>) -> CostRollup {
    let plan = &details.plan;

    // ── committed prices, grouped by their own stated currency ──────────────
    let mut by_currency: Vec<ByCurrency> = Vec::new();
    // A price whose unit cannot be resolved is counted, not dropped. Dropping it
    // is how the headline answered a confident 0 for a plan that had money on it.
    let mut unitless_items = 0_usize;
    for item in details.items.iter().filter(|item| is_committed(item)) {
        let Some(amount) = cents(&item.payload) else {
            continue;
        };
        // A price with no currency is attributed to the plan's currency when the
        // plan has one, and is otherwise unusable — never silently added to a
        // sum in a different unit.
        let Some(code) = unit_of(&item.payload, plan.currency.as_ref()) else {
            unitless_items += 1;
            continue;
        };
        match by_currency.iter_mut().find(|row| row.currency == code) {
            Some(row) => {
                row.booked_cents += amount;
                row.item_count += 1;
            }
            None => by_currency.push(ByCurrency {
                currency: code,
                booked_cents: amount,
                item_count: 1,
            }),
        }
    }
    by_currency.sort_by(|a, b| a.currency.cmp(&b.currency));

    let mut refusals: Vec<String> = Vec::new();
    if by_currency.len() > 1 {
        refusals.push(format!(
            "bookings are in {} currencies; they are reported per currency rather than added",
            by_currency.len()
        ));
    }
    if unitless_items > 0 {
        refusals.push(if unitless_items == 1 {
            format!("{NO_UNIT}, so it is not in this total")
        } else {
            format!("{unitless_items} priced items carry no currency and the plan carries none either, so they are not in this total")
        });
    }
    let (booked_cents, booked_reason) = if refusals.is_empty() {
        // Zero committed IS a measurement when nothing priced is on the plan.
        (
            Some(by_currency.first().map_or(0, |row| row.booked_cents)),
            None,
        )
    } else {
        (None, Some(refusals.join("; ")))
    };

    // The trip's currency: the plan's own if it has one, else the single
    // currency its bookings agree on. Two disagreeing currencies is null.
    let currency = plan
        .currency
        .clone()
        .or_else(|| (by_currency.len() == 1).then(|| by_currency[0].currency.clone()));

    // ── offered prices, reported and never summed in ─────────────────────────
    let mut offered_total = 0.0_f64;
    let mut priced_items = 0_usize;
    for item in &details.items {
        let prices: Vec<f64> = match item.item_type.as_str() {
            "option_set" => item
                .payload
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        // The chosen option when one is marked, every option
                        // otherwise would double-count, so an unchosen set
                        // contributes nothing to the total and only to the count.
                        .filter(|option| {
                            option.get("chosen").and_then(Value::as_bool) == Some(true)
                        })
                        .filter_map(|option| option.get("total_price").and_then(Value::as_f64))
                        .collect()
                })
                .unwrap_or_default(),
            "transport" => item
                .payload
                .get("journey")
                .and_then(|journey| journey.get("total_price"))
                .and_then(Value::as_f64)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        for price in prices {
            offered_total += price;
            priced_items += 1;
        }
    }

    // ── attribution to a stage ───────────────────────────────────────────────
    let mut stage_tallies: Vec<(&crate::store::TripStage, Tally)> = plan
        .stages
        .iter()
        .map(|stage| (stage, Tally::default()))
        .collect();
    let mut unattributed_tally = Tally::default();
    for item in details.items.iter().filter(|item| is_committed(item)) {
        let Some(amount) = cents(&item.payload) else {
            continue;
        };
        let unit = unit_of(&item.payload, plan.currency.as_ref());
        // Two bindings, in order, and no third. A date-based guess is
        // deliberately absent: a stay that spans three stages has no single
        // right answer, and inventing one hides the gap this block exists to
        // show.
        let stage_id = item
            .payload
            .get("stage_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                plan.stages
                    .iter()
                    .find(|stage| {
                        stage.selected_option_id.as_deref() == Some(item.external_id.as_str())
                    })
                    .map(|stage| stage.id.clone())
            });
        match stage_id
            .and_then(|id| stage_tallies.iter_mut().find(|(stage, _)| stage.id == id))
            .map(|(_, tally)| tally)
        {
            Some(tally) => tally.add(amount, unit),
            None => unattributed_tally.add(amount, unit),
        }
    }
    let by_stage: Vec<ByStage> = stage_tallies
        .into_iter()
        .map(|(stage, tally)| ByStage {
            stage_id: stage.id.clone(),
            sequence: stage.sequence,
            origin: stage.origin.name.clone(),
            destination: stage.destination.name.clone(),
            booked_cents: tally.total(),
            currency: tally.unit(),
            reason: tally.reason.clone(),
            item_count: tally.item_count,
        })
        .collect();
    let unattributed = Unattributed {
        booked_cents: unattributed_tally.total(),
        currency: unattributed_tally.unit(),
        reason: unattributed_tally.reason.clone(),
        item_count: unattributed_tally.item_count,
    };

    // ── actuals, or four nulls and a reason ──────────────────────────────────
    let unknown_actuals = |reason: String| {
        (
            Actuals {
                ok: false,
                reason: Some(reason.clone()),
                currency: None,
                personal_cents: None,
                gross_cash_outflow_cents: None,
                reimbursed_cents: None,
                outstanding_cents: None,
                posting_count: None,
            },
            Source {
                source: "finance",
                ok: false,
                reason: Some(reason),
            },
        )
    };
    let (actuals, finance_source) = match spending {
        // Finance answering in another unit than the plan is not an answer to
        // this plan's question. Relabelling those figures with the plan's
        // currency is the one thing this module exists to refuse.
        Ok(spending)
            if currency
                .as_deref()
                .zip(spending.currency.as_deref())
                .is_some_and(|(plan_code, finance_code)| {
                    !plan_code.eq_ignore_ascii_case(finance_code)
                }) =>
        {
            let plan_code = currency.clone().unwrap_or_default();
            let finance_code = spending.currency.clone().unwrap_or_default();
            unknown_actuals(format!(
                "finance reports this trip in {finance_code} and the plan is in {plan_code}; the figures are not comparable"
            ))
        }
        Ok(spending) => (
            Actuals {
                ok: true,
                reason: None,
                currency: spending.currency.clone(),
                personal_cents: Some(spending.personal_spending_cents),
                gross_cash_outflow_cents: Some(spending.gross_cash_outflow_cents),
                reimbursed_cents: Some(spending.reimbursed_cents),
                outstanding_cents: Some(spending.outstanding_cents),
                posting_count: Some(spending.expense_posting_count),
            },
            Source {
                source: "finance",
                ok: true,
                reason: None,
            },
        ),
        Err(Unreachable { reason }) => unknown_actuals(reason),
    };

    CostRollup {
        plan_id: plan.id.clone(),
        currency,
        planned_cents: plan.budget_cents,
        booked_cents,
        booked_reason,
        actuals,
        selected_options: SelectedOptions {
            total: (priced_items > 0).then_some(offered_total),
            currency: None,
            priced_items,
            note: OFFERED_NOT_PAID,
        },
        by_currency,
        by_stage,
        unattributed,
        sources: vec![
            Source {
                source: "trips",
                ok: true,
                reason: None,
            },
            finance_source,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PlaceKind, PlaceRef, PlanItem, StageStatus, TripPlan, TripStage};
    use serde_json::json;

    fn place(name: &str) -> PlaceRef {
        PlaceRef {
            id: format!("place:{name}"),
            name: name.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn stage(id: &str, sequence: usize, selected: Option<&str>) -> TripStage {
        TripStage {
            id: id.into(),
            sequence,
            origin: place("Origin"),
            destination: place("Destination"),
            date: None,
            transport_modes: Vec::new(),
            travelers: Vec::new(),
            status: StageStatus::Planning,
            selected_option_id: selected.map(str::to_string),
            branch_note: None,
        }
    }

    fn item(item_type: &str, external_id: &str, payload: Value) -> PlanItem {
        PlanItem {
            id: format!("trip:item:{external_id}"),
            plan_id: "trip:plan:1".into(),
            item_type: item_type.into(),
            day: None,
            external_id: external_id.into(),
            title: "synthetic".into(),
            payload,
            created_at: "0".into(),
        }
    }

    fn details(
        currency: Option<&str>,
        budget: Option<i64>,
        stages: Vec<TripStage>,
        items: Vec<PlanItem>,
    ) -> PlanDetails {
        PlanDetails {
            plan: TripPlan {
                id: "trip:plan:1".into(),
                title: "Synthetic".into(),
                origin: place("Origin"),
                destinations: vec![place("Destination")],
                date_start: "2026-01-01".into(),
                date_end: "2026-01-07".into(),
                interests: String::new(),
                status: "saved".into(),
                travelers: Vec::new(),
                transport_modes: Vec::new(),
                stages,
                cover_image_url: None,
                source: None,
                created_at: "0".into(),
                updated_at: "0".into(),
                budget_cents: budget,
                currency: currency.map(str::to_string),
            },
            items,
            retrospective: None,
        }
    }

    fn spending() -> TripSpending {
        TripSpending {
            trip_id: "trip:plan:1".into(),
            personal_spending_cents: 100,
            gross_cash_outflow_cents: 250,
            reimbursed_cents: 150,
            outstanding_cents: 0,
            expense_posting_count: 4,
            currency: Some("EUR".into()),
        }
    }

    #[test]
    fn the_roll_up_reports_unknown_actuals_when_finance_is_unreachable() {
        let rolled = roll_up(
            &details(Some("EUR"), Some(90_000), Vec::new(), Vec::new()),
            Err(Unreachable {
                reason: "finance is not reachable".into(),
            }),
        );
        assert!(!rolled.actuals.ok);
        assert!(rolled.actuals.reason.is_some());
        // Explicitly not zero: a zero here reads as "nothing was spent".
        assert_eq!(rolled.actuals.personal_cents, None);
        assert_eq!(rolled.actuals.gross_cash_outflow_cents, None);
        assert_eq!(rolled.actuals.reimbursed_cents, None);
        assert_eq!(rolled.actuals.outstanding_cents, None);
        assert_eq!(rolled.actuals.posting_count, None);
        let finance = rolled
            .sources
            .iter()
            .find(|source| source.source == "finance")
            .unwrap();
        assert!(!finance.ok && finance.reason.is_some());
        // The intent still answers: it comes from the plan, not from finance.
        assert_eq!(rolled.planned_cents, Some(90_000));
    }

    #[test]
    fn the_four_finance_figures_are_carried_not_summed() {
        let rolled = roll_up(
            &details(Some("EUR"), None, Vec::new(), Vec::new()),
            Ok(spending()),
        );
        assert_eq!(rolled.actuals.personal_cents, Some(100));
        assert_eq!(rolled.actuals.gross_cash_outflow_cents, Some(250));
        assert_eq!(rolled.actuals.reimbursed_cents, Some(150));
        assert_eq!(rolled.actuals.outstanding_cents, Some(0));
        assert_eq!(rolled.actuals.posting_count, Some(4));
        // No combined total anywhere in the body.
        let body = serde_json::to_value(&rolled).unwrap();
        assert!(body.get("spent_cents").is_none());
        for value in [350, 500, 250 + 100] {
            assert!(
                !serde_json::to_string(&body)
                    .unwrap()
                    .contains(&value.to_string()),
                "a combined total ({value}) reached the wire"
            );
        }
    }

    /// Two units are never added — and the rule holds at every grain, not only
    /// at the headline. The stage rows and the unattributed row used to add them
    /// anyway, under a headline that refused to: this fixture now carries the
    /// stages whose absence let that through.
    #[test]
    fn mixed_currencies_are_not_summed() {
        let rolled = roll_up(
            &details(
                None,
                None,
                vec![stage("stage:1", 0, None)],
                vec![
                    item(
                        "booking",
                        "b1",
                        json!({"provider":"p","order_ref":"r","amount_cents":12_000,"currency":"EUR","stage_id":"stage:1"}),
                    ),
                    item(
                        "stay",
                        "s1",
                        json!({"check_in":"2026-01-01","check_out":"2026-01-03","latitude":0,"longitude":0,"amount_cents":9_000,"currency":"USD","stage_id":"stage:1","booked":true}),
                    ),
                    // Bound to no stage, and in two units again.
                    item(
                        "booking",
                        "b2",
                        json!({"provider":"p","order_ref":"r","amount_cents":4_000,"currency":"USD"}),
                    ),
                    item(
                        "booking",
                        "b3",
                        json!({"provider":"p","order_ref":"r","amount_cents":1_000,"currency":"EUR"}),
                    ),
                ],
            ),
            Ok(spending()),
        );
        assert_eq!(rolled.booked_cents, None, "two units must never be added");
        assert!(rolled.booked_reason.is_some());
        assert_eq!(rolled.by_currency.len(), 2);
        assert_eq!(rolled.by_currency[0].currency, "EUR");
        assert_eq!(rolled.by_currency[0].booked_cents, 13_000);
        assert_eq!(rolled.by_currency[1].currency, "USD");
        assert_eq!(rolled.by_currency[1].booked_cents, 13_000);
        assert_eq!(rolled.currency, None, "no single currency to report");

        // 12000 EUR + 9000 USD is not 21000 of anything.
        assert_eq!(rolled.by_stage[0].booked_cents, None);
        assert_eq!(rolled.by_stage[0].currency, None);
        assert_eq!(rolled.by_stage[0].item_count, 2);
        let stage_reason = rolled.by_stage[0].reason.clone().expect("a stated reason");
        assert!(
            stage_reason.contains("EUR") && stage_reason.contains("USD"),
            "the reason names both units: {stage_reason}"
        );
        assert_eq!(rolled.unattributed.booked_cents, None);
        assert_eq!(rolled.unattributed.currency, None);
        assert_eq!(rolled.unattributed.item_count, 2);
        assert!(rolled.unattributed.reason.is_some());

        // The added figure must not reach the wire under any name.
        let body = serde_json::to_string(&serde_json::to_value(&rolled).unwrap()).unwrap();
        for wrong in [21_000, 5_000, 26_000] {
            assert!(
                !body.contains(&wrong.to_string()),
                "an added cross-currency figure ({wrong}) reached the wire"
            );
        }
    }

    /// A booking that names no currency on a plan that names none either is
    /// money in an unknown unit. It used to fall out of the headline (leaving a
    /// confident `0` with no reason) and into the stage rows at the same time.
    #[test]
    fn a_priced_item_with_no_currency_refuses_the_total_rather_than_reporting_zero() {
        let rolled = roll_up(
            &details(
                None,
                None,
                vec![stage("stage:1", 0, None)],
                vec![item(
                    "booking",
                    "b1",
                    json!({"provider":"p","order_ref":"r","amount_cents":45_000,"stage_id":"stage:1"}),
                )],
            ),
            Ok(spending()),
        );
        assert_eq!(
            rolled.booked_cents, None,
            "a 0 here reads as 'nothing booked'"
        );
        let reason = rolled.booked_reason.clone().expect("a stated reason");
        assert!(
            reason.contains("no currency"),
            "the reason names the gap: {reason}"
        );
        assert!(rolled.by_currency.is_empty());
        assert_eq!(rolled.by_stage[0].booked_cents, None);
        assert_eq!(rolled.by_stage[0].item_count, 1);
        assert!(rolled.by_stage[0].reason.is_some());
    }

    /// The same item on a plan that DOES carry a currency is denominated by the
    /// plan and counts normally — the inheritance rule the retrospective relies on.
    #[test]
    fn a_priced_item_with_no_currency_inherits_the_plans() {
        let rolled = roll_up(
            &details(
                Some("EUR"),
                None,
                vec![stage("stage:1", 0, None)],
                vec![item(
                    "booking",
                    "b1",
                    json!({"provider":"p","order_ref":"r","amount_cents":45_000,"stage_id":"stage:1"}),
                )],
            ),
            Ok(spending()),
        );
        assert_eq!(rolled.booked_cents, Some(45_000));
        assert_eq!(rolled.booked_reason, None);
        assert_eq!(rolled.by_stage[0].booked_cents, Some(45_000));
        assert_eq!(rolled.by_stage[0].currency.as_deref(), Some("EUR"));
    }

    /// A figure in another unit is not this plan's figure. The card renders the
    /// actuals under a currency, so relabelling them would be a false claim about
    /// somebody else's numbers.
    #[test]
    fn finance_answering_in_another_currency_is_unknown_rather_than_relabelled() {
        let rolled = roll_up(
            &details(Some("GBP"), None, Vec::new(), Vec::new()),
            Ok(spending()),
        );
        assert!(!rolled.actuals.ok);
        assert_eq!(rolled.actuals.personal_cents, None);
        assert_eq!(rolled.actuals.currency, None);
        let reason = rolled.actuals.reason.clone().expect("a stated reason");
        assert!(
            reason.contains("EUR") && reason.contains("GBP"),
            "the reason names both units: {reason}"
        );
    }

    /// Finance stating no currency is not a disagreement: the figures are carried
    /// and the card shows them without inventing a unit for them.
    #[test]
    fn finance_stating_no_currency_still_answers() {
        let mut answer = spending();
        answer.currency = None;
        let rolled = roll_up(
            &details(Some("EUR"), None, Vec::new(), Vec::new()),
            Ok(answer),
        );
        assert!(rolled.actuals.ok);
        assert_eq!(rolled.actuals.personal_cents, Some(100));
        assert_eq!(rolled.actuals.currency, None);
    }

    #[test]
    fn option_prices_never_reach_booked_cents() {
        let rolled = roll_up(
            &details(
                Some("EUR"),
                None,
                Vec::new(),
                vec![item(
                    "option_set",
                    "o1",
                    json!({
                        "query": {"from": "a", "to": "b"},
                        "options": [
                            {"id": "x", "total_price": 39.99, "chosen": true},
                            {"id": "y", "total_price": 71.50}
                        ]
                    }),
                )],
            ),
            Ok(spending()),
        );
        assert_eq!(rolled.booked_cents, Some(0));
        assert!(rolled.by_currency.is_empty());
        assert_eq!(rolled.selected_options.priced_items, 1);
        assert_eq!(rolled.selected_options.total, Some(39.99));
        assert_eq!(
            rolled.selected_options.currency, None,
            "the schema gives these floats no currency, so neither does this"
        );
    }

    #[test]
    fn a_booking_without_a_stage_binding_lands_in_unattributed() {
        let rolled = roll_up(
            &details(
                Some("EUR"),
                None,
                vec![
                    stage("stage:1", 0, Some("chosen-option")),
                    stage("stage:2", 1, None),
                ],
                vec![
                    // Bound by the explicit stage_id.
                    item(
                        "booking",
                        "b1",
                        json!({"provider":"p","order_ref":"r","amount_cents":5_000,"currency":"EUR","stage_id":"stage:2"}),
                    ),
                    // Bound by external_id matching a stage's selected option.
                    item(
                        "booking",
                        "chosen-option",
                        json!({"provider":"p","order_ref":"r","amount_cents":3_000,"currency":"EUR"}),
                    ),
                    // Bound by nothing. Never guessed at by date.
                    item(
                        "booking",
                        "b3",
                        json!({"provider":"p","order_ref":"r","amount_cents":700,"currency":"EUR"}),
                    ),
                ],
            ),
            Ok(spending()),
        );
        assert_eq!(rolled.booked_cents, Some(8_700));
        assert_eq!(rolled.by_stage[0].booked_cents, Some(3_000));
        assert_eq!(rolled.by_stage[0].currency.as_deref(), Some("EUR"));
        assert_eq!(rolled.by_stage[1].booked_cents, Some(5_000));
        assert_eq!(rolled.unattributed.booked_cents, Some(700));
        assert_eq!(rolled.unattributed.item_count, 1);
    }

    #[test]
    fn a_plan_with_nothing_on_it_reports_zero_committed_and_no_intent() {
        let rolled = roll_up(&details(None, None, Vec::new(), Vec::new()), Ok(spending()));
        // Zero committed IS a measurement: nothing has been booked.
        assert_eq!(rolled.booked_cents, Some(0));
        // No budget is not a zero budget.
        assert_eq!(rolled.planned_cents, None);
        assert_eq!(rolled.selected_options.total, None);
    }

    /// A stay copied from a search result is a candidate, not a purchase. The
    /// Berlin plan reported EUR 707.35 booked on 2026-09-25 with nothing booked.
    #[test]
    fn a_stay_counts_as_booked_only_with_evidence_of_a_booking() {
        let stay = |id: &str, extra: Value| {
            let mut payload = json!({"check_in":"2026-10-07","check_out":"2026-10-13","latitude":52.5,"longitude":13.4,"amount_cents":70_735,"currency":"EUR"});
            payload
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            item("stay", id, payload)
        };
        let candidate = roll_up(
            &details(Some("EUR"), None, Vec::new(), vec![stay("s1", json!({}))]),
            Ok(spending()),
        );
        assert_eq!(candidate.booked_cents, Some(0));

        for evidence in [json!({"booked": true}), json!({"order_ref": "4711"})] {
            let booked = roll_up(
                &details(Some("EUR"), None, Vec::new(), vec![stay("s1", evidence)]),
                Ok(spending()),
            );
            assert_eq!(booked.booked_cents, Some(70_735));
        }
        let blank = roll_up(
            &details(
                Some("EUR"),
                None,
                Vec::new(),
                vec![stay("s1", json!({"order_ref": " "}))],
            ),
            Ok(spending()),
        );
        assert_eq!(blank.booked_cents, Some(0));
    }
}
