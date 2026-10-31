//! Inventory, household assets, wishlist budget, and interior spatial layout models.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryState {
    Owned,
    Wanted,
    Gone,
}

impl InventoryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Wanted => "wanted",
            Self::Gone => "gone",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MailProposal {
    pub id: i64,
    pub source_id: String,
    pub line_id: String,
    pub event: String,
    pub title: String,
    pub evidence: String,
    pub data_class: String,
    pub order_ref: Option<String>,
    pub version: i64,
    pub decision: Option<String>,
    pub item_id: Option<String>,
}

impl MailProposal {
    pub fn gmail_url(&self) -> Option<String> {
        let mut parts = self.source_id.split(':');
        let (Some("mail"), Some("v1"), Some(thread), Some(message), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return None;
        };
        let valid = |id: &str| {
            !id.is_empty()
                && id.len() <= 120
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        };
        if !valid(thread) || !valid(message) {
            return None;
        }
        Some(format!("https://mail.google.com/mail/u/0/#all/{thread}"))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MailProposals {
    pub proposals: Vec<MailProposal>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MailMatchSnapshot {
    pub item_id: String,
    pub title: String,
    pub item_revision: i64,
    pub state: Option<String>,
    pub state_id: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NewInventoryItem {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub state: InventoryState,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InventoryStateChange {
    pub id: String,
    pub state: InventoryState,
    pub changed: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InventoryItemWrite {
    pub id: String,
    pub ok: bool,
    pub revision: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InventoryItem {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub depth: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub count: Option<i32>,
    #[serde(default)]
    pub price_cents: Option<i64>,
    #[serde(default)]
    pub cost_min_cents: Option<i64>,
    #[serde(default)]
    pub cost_max_cents: Option<i64>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventoryRow {
    pub item: InventoryItem,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonthlyBalance {
    pub median_cents: i64,
    pub months: usize,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventoryWishlist {
    pub items: Vec<InventoryItem>,
    pub total_low_cents: i64,
    pub total_high_cents: i64,
    pub unpriced_count: usize,
    #[serde(default)]
    pub monthly_balance: Option<MonthlyBalance>,
    #[serde(default)]
    pub months_to_pay: Option<f64>,
}

/// One wish group as `GET /api/wish-groups` returns it: the constraint that bounds a set of
/// wishes, its member items, and what they cost.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct WishGroup {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub items: Vec<InventoryItem>,
    #[serde(default)]
    pub items_count: usize,
    #[serde(default)]
    pub total_low_cents: i64,
    #[serde(default)]
    pub total_high_cents: i64,
}

impl WishGroup {
    /// The members as ids, in the order the capability answered with: what `PUT
    /// /api/wish-groups/{id}/items` takes back.
    pub fn item_ids(&self) -> Vec<String> {
        self.items.iter().map(|item| item.id.clone()).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|item| item.id == id)
    }
}

/// The envelope `GET /api/wish-groups` answers with.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WishGroups {
    #[serde(default)]
    pub wish_groups: Vec<WishGroup>,
}

/// What a set of wishes costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub low_cents: i64,
    pub high_cents: i64,
    /// Wishes with no price and no estimate. They count as zero, so the total is a floor.
    pub unpriced: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteriorLayoutSummary {
    pub id: String,
    pub name: String,
    pub pass: bool,
    #[serde(default)]
    pub hard: usize,
    #[serde(default)]
    pub soft: usize,
    #[serde(default)]
    pub occupied_m2: Option<f64>,
}

/// Format price or budget cents as a formatted currency string.
pub fn format_cents(cents: Option<i64>) -> String {
    let Some(c) = cents else {
        return "—".to_string();
    };
    let eur = c as f64 / 100.0;
    format!("{:.2} €", eur)
}

/// Format width, depth, and height into concise dimension label.
pub fn format_dimensions(w: Option<i32>, d: Option<i32>, h: Option<i32>) -> String {
    match (w, d, h) {
        (Some(w), Some(d), Some(h)) => format!("{w} × {d} × {h} cm"),
        (Some(w), Some(d), None) => format!("{w} × {d} cm"),
        _ => "—".to_string(),
    }
}

/// Determine the grouping or category for an inventory piece.
pub fn group_of(item: &InventoryItem) -> String {
    if let Some(cat) = &item.category {
        if !cat.trim().is_empty() {
            return cat.to_lowercase();
        }
    }
    match item.kind.as_str() {
        "slot" => "needs".to_string(),
        "gear" => "gear".to_string(),
        _ => "furniture".to_string(),
    }
}

/// Whether an entry answers a lowercase search text: its name, its id, its note or its category.
pub fn matches_query(item: &InventoryItem, q: &str) -> bool {
    q.is_empty()
        || item.label.to_lowercase().contains(q)
        || item.id.to_lowercase().contains(q)
        || item
            .notes
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains(q)
        || item
            .category
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains(q)
}

/// Filter inventory rows by state, category, and text search.
pub fn filter_inventory(
    rows: &[InventoryRow],
    query: &str,
    category: Option<&str>,
    state_filter: Option<&str>,
) -> Vec<InventoryRow> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            if let Some(expected_state) = state_filter {
                if row.state.as_deref().unwrap_or("") != expected_state {
                    return false;
                }
            }
            if let Some(cat) = category {
                if group_of(&row.item) != cat.to_lowercase() {
                    return false;
                }
            }
            matches_query(&row.item, &q)
        })
        .cloned()
        .collect()
}

/// The open wishes a wishlist view shows: one group's members when a group is chosen, otherwise all
/// of them, narrowed by the search text. Order stays the wishlist's.
///
/// A group keeps the members that were bought, and the wishlist is what knows which are still open,
/// so the membership is joined against `items` rather than drawn from the group's own list.
pub fn wishlist_view<'a>(
    items: &'a [InventoryItem],
    group: Option<&WishGroup>,
    query: &str,
) -> Vec<&'a InventoryItem> {
    let q = query.trim().to_lowercase();
    items
        .iter()
        .filter(|item| group.is_none_or(|group| group.contains(&item.id)))
        .filter(|item| matches_query(item, &q))
        .collect()
}

/// What these wishes cost, in the rule `GET /api/wishlist` totals with: a product's price, else the
/// low and the high end of a need's estimate.
pub fn budget_of<'a>(items: impl IntoIterator<Item = &'a InventoryItem>) -> Budget {
    let mut budget = Budget::default();
    for item in items {
        budget.low_cents += item.price_cents.or(item.cost_min_cents).unwrap_or(0);
        budget.high_cents += item.price_cents.or(item.cost_max_cents).unwrap_or(0);
        if item.price_cents.is_none() && item.cost_min_cents.is_none() {
            budget.unpriced += 1;
        }
    }
    budget
}

/// How many monthly surpluses these wishes take, in the rule `budget::months_to_pay` applies on the
/// capability side: a surplus that is not positive funds nothing.
pub fn months_to_pay(open_cents: i64, balance: &MonthlyBalance) -> Option<f64> {
    if balance.median_cents <= 0 {
        return None;
    }
    Some(open_cents as f64 / balance.median_cents as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_source_url_only_uses_validated_references() {
        let proposal = |source: &str| {
            serde_json::from_value::<MailProposal>(serde_json::json!({
                "id": 1, "source_id": source, "line_id": "1", "event": "delivered",
                "title": "item", "evidence": "exact", "data_class": "c2", "order_ref": null,
                "version": 1, "decision": null, "item_id": null
            }))
            .unwrap()
        };
        assert_eq!(
            proposal("mail:v1:thread-1:message_2")
                .gmail_url()
                .as_deref(),
            Some("https://mail.google.com/mail/u/0/#all/thread-1")
        );
        for bad in [
            "mail:v1:../bad:message",
            "mail:v1:thread:message:extra",
            "mail:v1:thread:",
            "other:v1:thread:message",
        ] {
            assert!(proposal(bad).gmail_url().is_none());
        }
    }

    #[test]
    fn test_format_cents() {
        assert_eq!(format_cents(None), "—");
        assert_eq!(format_cents(Some(0)), "0.00 €");
        assert_eq!(format_cents(Some(45000)), "450.00 €");
        assert_eq!(format_cents(Some(199)), "1.99 €");
    }

    #[test]
    fn test_format_dimensions() {
        assert_eq!(format_dimensions(None, None, None), "—");
        assert_eq!(format_dimensions(Some(120), Some(80), None), "120 × 80 cm");
        assert_eq!(
            format_dimensions(Some(160), Some(80), Some(72)),
            "160 × 80 × 72 cm"
        );
    }

    #[test]
    fn test_group_and_filtering() {
        let item1 = InventoryItem {
            id: "table".into(),
            label: "Dining Table".into(),
            kind: "piece".into(),
            category: Some("furniture".into()),
            ..Default::default()
        };
        let item2 = InventoryItem {
            id: "backpack".into(),
            label: "Hiking Pack".into(),
            kind: "gear".into(),
            category: None,
            ..Default::default()
        };
        let rows = vec![
            InventoryRow {
                item: item1,
                state: Some("owned".into()),
            },
            InventoryRow {
                item: item2,
                state: Some("wanted".into()),
            },
        ];

        let filtered = filter_inventory(&rows, "table", None, None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].item.id, "table");

        let owned = filter_inventory(&rows, "", None, Some("owned"));
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].item.id, "table");

        let gear = filter_inventory(&rows, "", Some("gear"), None);
        assert_eq!(gear.len(), 1);
        assert_eq!(gear[0].item.id, "backpack");
    }

    fn wish(id: &str, price_cents: Option<i64>, cost: Option<(i64, i64)>) -> InventoryItem {
        InventoryItem {
            id: id.into(),
            label: id.into(),
            kind: "piece".into(),
            price_cents,
            cost_min_cents: cost.map(|c| c.0),
            cost_max_cents: cost.map(|c| c.1),
            ..Default::default()
        }
    }

    #[test]
    fn a_group_narrows_the_wishlist_and_keeps_its_order() {
        let items = vec![
            wish("bath-mat", None, None),
            wish("hook", None, None),
            wish("pflanze", None, None),
        ];
        let group = WishGroup {
            id: "bad".into(),
            title: "Bad".into(),
            items: vec![
                wish("hook", None, None),
                wish("bath-mat", None, None),
                wish("bought", None, None),
            ],
            items_count: 3,
            ..Default::default()
        };

        let shown = wishlist_view(&items, Some(&group), "");
        let ids: Vec<&str> = shown.iter().map(|item| item.id.as_str()).collect();
        // The group's third member is no longer open, so it is not shown.
        assert_eq!(ids, ["bath-mat", "hook"]);

        let all = wishlist_view(&items, None, "PfLan");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "pflanze");
    }

    #[test]
    fn a_budget_is_a_floor_and_says_how_many_are_unpriced() {
        let items = vec![
            wish("mat", Some(3990), None),
            wish("shelf", None, Some((5000, 9000))),
            wish("plant", None, None),
        ];
        assert_eq!(
            budget_of(&items),
            Budget {
                low_cents: 8990,
                high_cents: 12990,
                unpriced: 1
            }
        );
        assert_eq!(budget_of(&items[..0]), Budget::default());
    }

    #[test]
    fn a_group_hands_back_the_ids_it_was_given() {
        let group = WishGroup {
            id: "bad".into(),
            title: "Bad".into(),
            items: vec![wish("b", None, None), wish("a", None, None)],
            ..Default::default()
        };
        assert_eq!(group.item_ids(), ["b", "a"]);
        assert!(group.contains("a"));
        assert!(!group.contains("c"));
    }

    #[test]
    fn only_a_positive_surplus_funds_anything() {
        let balance = |median_cents| MonthlyBalance {
            median_cents,
            months: 3,
            from: "2026-08".into(),
            to: "2026-10".into(),
        };
        assert_eq!(months_to_pay(20000, &balance(10000)), Some(2.0));
        assert_eq!(months_to_pay(20000, &balance(0)), None);
        assert_eq!(months_to_pay(20000, &balance(-5000)), None);
    }
}
