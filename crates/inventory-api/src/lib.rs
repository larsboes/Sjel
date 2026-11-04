//! Typed API contract, wire models, and route constants for the Sjel inventory capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-inventory`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_INVENTORY: &str = "/api/inventory";
pub const ROUTE_ITEM_REFS: &str = "/api/item-refs/{reference}";
pub const ROUTE_MAIL_PROPOSALS: &str = "/api/mail-proposals";
pub const ROUTE_MAIL_PROPOSAL_DECISION: &str = "/api/mail-proposals/{id}/decision";
pub const ROUTE_MAIL_MATCH_SNAPSHOT: &str = "/api/items/{id}/mail-match-snapshot";
pub const ROUTE_ITEMS: &str = "/api/items";
pub const ROUTE_ITEM_BY_ID: &str = "/api/items/{id}";
pub const ROUTE_ITEM_STATE: &str = "/api/items/{id}/state";
pub const ROUTE_WISHLIST: &str = "/api/wishlist";
pub const ROUTE_WISH_GROUPS: &str = "/api/wish-groups";
pub const ROUTE_WISH_GROUP_BY_ID: &str = "/api/wish-groups/{id}";
pub const ROUTE_WISH_GROUP_ITEMS: &str = "/api/wish-groups/{id}/items";
pub const ROUTE_WISH_GROUP_SPECS: &str = "/api/wish-groups/{id}/specs";
pub const ROUTE_WISH_GROUP_NEEDS: &str = "/api/wish-groups/{id}/needs";
pub const ROUTE_WISH_GROUP_PRICES: &str = "/api/wish-groups/{id}/prices";
pub const ROUTE_WISH_COMPARISON: &str = "/api/wish-comparison";
pub const ROUTE_LINKS: &str = "/api/links";

// ============================================================================
// Enums
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

// ============================================================================
// Item & Inventory Models
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InventoryRow {
    pub item: InventoryItem,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct InventoryStateChange {
    pub id: String,
    pub state: InventoryState,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InventoryStateHistoryEntry {
    pub state: String,
    pub since: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct InventoryItemWrite {
    pub id: String,
    pub ok: bool,
    pub revision: i64,
}

// ============================================================================
// Mail Proposals
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
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

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MailProposals {
    pub proposals: Vec<MailProposal>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MailMatchSnapshot {
    pub item_id: String,
    pub title: String,
    pub item_revision: i64,
    pub state: Option<String>,
    pub state_id: Option<i64>,
}

// ============================================================================
// Wishlist & Wish Groups
// ============================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MonthlyBalance {
    pub median_cents: i64,
    pub months: usize,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WishGroup {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub alternatives: bool,
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
    pub fn item_ids(&self) -> Vec<String> {
        self.items.iter().map(|item| item.id.clone()).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|item| item.id == id)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WishGroups {
    #[serde(default)]
    pub wish_groups: Vec<WishGroup>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Budget {
    pub low_cents: i64,
    pub high_cents: i64,
    pub unpriced: usize,
}

// ============================================================================
// Interior Spatial Impact
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InteriorImpactVerdict {
    pub pass: bool,
    #[serde(default)]
    pub hard: Vec<String>,
    #[serde(default)]
    pub soft: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InteriorImpactChange {
    pub layout: String,
    pub before: InteriorImpactVerdict,
    pub after: InteriorImpactVerdict,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InteriorItemImpact {
    pub item: String,
    pub layouts: usize,
    pub passing_before: usize,
    pub passing_after: usize,
    #[serde(default)]
    pub changed: Vec<InteriorImpactChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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

// ============================================================================
// Wish Comparison Models
// ============================================================================

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Spec {
    pub key: String,
    pub value: f64,
    #[serde(default)]
    pub unit: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct PriceObservation {
    pub price_cents: i64,
    pub source: String,
    #[serde(default)]
    pub configuration: String,
    pub observed_at: String,
    #[serde(default)]
    pub alert: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub cost_max_cents: Option<i64>,
    #[serde(default)]
    pub specs: Vec<Spec>,
    #[serde(default)]
    pub observations: Vec<PriceObservation>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GroupComparison {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub needs: Vec<Spec>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Score {
    pub key: String,
    pub unit: String,
    pub cents_per_unit: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Fit {
    Meets,
    Short { key: String, have: f64, need: f64 },
    Unknown { key: String },
}

impl Candidate {
    pub fn spec(&self, key: &str) -> Option<f64> {
        self.specs.iter().find(|s| s.key == key).map(|s| s.value)
    }

    /// The newest observation. With several sources on one day, the cheapest of that day wins:
    /// it is the price one can pay.
    pub fn latest(&self) -> Option<&PriceObservation> {
        let day = self.observations.last()?.observed_at.get(..10)?;
        self.observations
            .iter()
            .filter(|o| o.observed_at.get(..10) == Some(day))
            .min_by_key(|o| o.price_cents)
    }

    /// The candidate's limit is reached when its latest price is at or below `cost_max_cents`.
    pub fn within_limit(&self) -> Option<bool> {
        Some(self.latest()?.price_cents <= self.cost_max_cents?)
    }

    /// One score per positive spec, in spec order. None without a price.
    pub fn scores(&self) -> Vec<Score> {
        let Some(latest) = self.latest() else {
            return Vec::new();
        };
        self.specs
            .iter()
            .filter(|s| s.value > 0.0)
            .map(|s| Score {
                key: s.key.clone(),
                unit: s.unit.clone(),
                cents_per_unit: latest.price_cents as f64 / s.value,
            })
            .collect()
    }

    /// One answer per need. A missing spec is unknown, not short.
    pub fn fits(&self, needs: &[Spec]) -> Vec<Fit> {
        needs
            .iter()
            .map(|need| match self.spec(&need.key) {
                None => Fit::Unknown {
                    key: need.key.clone(),
                },
                Some(have) if have >= need.value => Fit::Meets,
                Some(have) => Fit::Short {
                    key: need.key.clone(),
                    have,
                    need: need.value,
                },
            })
            .collect()
    }
}

impl GroupComparison {
    /// Spec keys at least two candidates share: only those can be compared on an axis.
    pub fn axis_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .candidates
            .iter()
            .flat_map(|c| c.specs.iter().map(|s| s.key.clone()))
            .collect();
        keys.sort();
        let mut shared = Vec::new();
        for window in keys.chunk_by(|a, b| a == b) {
            if window.len() >= 2 {
                shared.push(window[0].clone());
            }
        }
        shared
    }

    /// The axis to open with: the first need that is an axis key, else the first axis key.
    pub fn default_axis(&self) -> Option<String> {
        let keys = self.axis_keys();
        self.needs
            .iter()
            .find(|n| keys.contains(&n.key))
            .map(|n| n.key.clone())
            .or_else(|| keys.into_iter().next())
    }

    pub fn need(&self, key: &str) -> Option<f64> {
        self.needs.iter().find(|n| n.key == key).map(|n| n.value)
    }

    pub fn unit_of(&self, key: &str) -> String {
        self.needs
            .iter()
            .chain(self.candidates.iter().flat_map(|c| &c.specs))
            .find(|s| s.key == key && !s.unit.is_empty())
            .map(|s| s.unit.clone())
            .unwrap_or_default()
    }

    /// Whether there is anything to draw: two candidates with a price, or one with a history.
    pub fn is_comparable(&self) -> bool {
        let priced = self
            .candidates
            .iter()
            .filter(|c| !c.observations.is_empty())
            .count();
        priced >= 2 || self.candidates.iter().any(|c| c.observations.len() >= 2)
    }
}

// ============================================================================
// Request Mutation Models
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SaveWishGroupRequest {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub item_ids: Option<Vec<String>>,
    #[serde(default)]
    pub alternatives: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SetWishGroupItemsRequest {
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SetSpecsRequest {
    pub specs: Vec<Spec>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SetNeedsRequest {
    pub needs: Vec<Spec>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RecordPriceRequest {
    pub price_cents: i64,
    pub source: String,
    #[serde(default)]
    pub configuration: String,
    #[serde(default)]
    pub observed_at: Option<String>,
}
