//! Inventory, household assets, wishlist budget, and interior spatial layout models.

use serde::{Deserialize, Serialize};

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
            if q.is_empty() {
                return true;
            }
            row.item.label.to_lowercase().contains(&q)
                || row.item.id.to_lowercase().contains(&q)
                || row
                    .item
                    .notes
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
                || row
                    .item
                    .category
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
