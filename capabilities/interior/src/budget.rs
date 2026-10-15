//! The purchase order of the open needs, measured against inventory's monthly balance.

pub use sjel_inventory::budget::{monthly_balance, months_to_pay, MonthlyBalance};

/// Urgency of a need, from its `priority`.
///
/// Two axes in one word, both real in the data: a slot says how urgent the need is (`required`,
/// `recommended`, `concept`), a product how far the decision went (`decided`, `candidate`,
/// `deferred`). Both answer what to buy next, so one order is valid; a decided product leads.
fn rank(priority: Option<&str>) -> u8 {
    match priority {
        Some("decided") => 0,
        Some("required") => 1,
        Some("recommended") => 2,
        Some("candidate") => 3,
        Some("concept") => 4,
        // Unknown urgency is not low: after everything ranked, but before the explicitly deferred.
        None => 5,
        Some("deferred") => 6,
        Some(_) => 7,
    }
}

/// The words this order knows. Any other word is reported, not sorted in.
pub const KNOWN_PRIORITIES: &[&str] = &[
    "decided",
    "required",
    "recommended",
    "candidate",
    "concept",
    "deferred",
    // Known, but outside the order: both mean decided against.
    "rejected",
    "replaced",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct PurchaseLine {
    pub id: String,
    pub label: String,
    pub priority: Option<String>,
    /// The price used: `price_cents`, else the low end of the estimate.
    pub price_cents: Option<i64>,
    /// Layouts that already plan this piece. A need a layout builds on ranks before an equally
    /// urgent idea.
    pub in_layouts: Vec<String>,
    /// The total of all lines up to and including this one.
    pub cumulative_cents: i64,
    /// After how many monthly balances this line is affordable. `None` without a positive balance.
    pub affordable_after_months: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PurchaseOrder {
    pub lines: Vec<PurchaseLine>,
    /// Needs without any price. Not in the order: at zero they would rank first.
    pub unpriced: Vec<PurchaseLine>,
    pub balance: Option<MonthlyBalance>,
    /// Priority words from the data that `rank` does not know, reported instead of silently ranked.
    pub unknown_priorities: Vec<String>,
}

/// What first, what next, and when it is affordable. An order from existing fields and a timeline
/// from one measurement; no budget and no advice.
pub fn purchase_order(
    model: &crate::model::Model,
    balance: Option<MonthlyBalance>,
) -> Result<PurchaseOrder, crate::model::ModelError> {
    // Which needs a layout already places, read once.
    let mut in_layouts: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for name in model.layout_names()? {
        let l = model.load_layout(&name)?;
        for it in &l.items {
            in_layouts
                .entry(it.reference.clone())
                .or_default()
                .push(name.clone());
        }
    }

    let mut priced: Vec<PurchaseLine> = Vec::new();
    let mut unpriced: Vec<PurchaseLine> = Vec::new();
    let mut unknown: std::collections::BTreeSet<String> = Default::default();
    for i in model.wishlist() {
        // Both words mean decided against and belong in no purchase order. `replaced` becomes `gone` on
        // import anyway; it is listed so this does not depend on another file.
        if matches!(i.priority.as_deref(), Some("rejected") | Some("replaced")) {
            continue;
        }
        if let Some(p) = i.priority.as_deref() {
            if !KNOWN_PRIORITIES.contains(&p) {
                unknown.insert(p.to_string());
            }
        }
        let price = i.price_cents.or(i.cost_min_cents);
        let line = PurchaseLine {
            id: i.id.clone(),
            label: i.label.clone(),
            priority: i.priority.clone(),
            price_cents: price,
            in_layouts: in_layouts.get(&i.id).cloned().unwrap_or_default(),
            cumulative_cents: 0,
            affordable_after_months: None,
        };
        if price.is_some() {
            priced.push(line);
        } else {
            unpriced.push(line);
        }
    }

    // Urgency, then whether a layout builds on it, then price: price decides between equally urgent
    // lines, not what is urgent.
    priced.sort_by(|a, b| {
        rank(a.priority.as_deref())
            .cmp(&rank(b.priority.as_deref()))
            .then(b.in_layouts.len().cmp(&a.in_layouts.len()))
            .then(a.price_cents.cmp(&b.price_cents))
            .then(a.id.cmp(&b.id))
    });
    unpriced.sort_by(|a, b| {
        rank(a.priority.as_deref())
            .cmp(&rank(b.priority.as_deref()))
            .then(a.id.cmp(&b.id))
    });

    let mut total = 0i64;
    for p in priced.iter_mut() {
        total += p.price_cents.unwrap_or(0);
        p.cumulative_cents = total;
        p.affordable_after_months = balance.as_ref().and_then(|s| months_to_pay(total, s));
    }

    Ok(PurchaseOrder {
        lines: priced,
        unpriced,
        balance,
        unknown_priorities: unknown.into_iter().collect(),
    })
}
