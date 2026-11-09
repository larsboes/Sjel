//! Typed API contract, wire models, and route constants for the Sjel trips capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-trips`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod cost;

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_PLANS: &str = "/api/plans";
pub const ROUTE_PLAN_BY_ID: &str = "/api/plans/{id}";
pub const ROUTE_PLAN_COST: &str = "/api/plans/{id}/cost";
pub const ROUTE_PLAN_ITEMS: &str = "/api/plans/{id}/items";
pub const ROUTE_PLAN_ITEM_BY_ID: &str = "/api/plans/{plan_id}/items/{item_id}";
pub const ROUTE_PLAN_ITEM_FIELDS: &str = "/api/plans/{plan_id}/items/{item_id}/fields";
pub const ROUTE_PLAN_RESOLVE_PLACES: &str = "/api/plans/{id}/resolve-places";
pub const ROUTE_PLAN_RETROSPECTIVE: &str = "/api/plans/{id}/retrospective";
pub const ROUTE_PLAN_OUTCOME: &str = "/api/plans/{id}/outcome";
pub const ROUTE_BASES: &str = "/api/bases";
pub const ROUTE_PEOPLE_OPEN: &str = "/api/people/open";
pub const ROUTE_PEOPLE_DECIDE: &str = "/api/people/decide";
pub const ROUTE_LINKS: &str = "/api/links";

// ============================================================================
// Core Trip and Place Types
// ============================================================================

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Address,
    Airport,
    City,
    #[default]
    Station,
    Venue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct PlaceRef {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: PlaceKind,
    #[serde(default)]
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl PlaceRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            name: name.into(),
            kind: PlaceKind::default(),
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    pub fn with_coords(mut self, lat: f64, lon: f64) -> Self {
        self.latitude = Some(lat);
        self.longitude = Some(lon);
        self
    }

    pub fn coordinate(&self) -> Option<places_api::Coordinate> {
        places_api::Coordinate::new(self.latitude?, self.longitude?)
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    Bike,
    Bus,
    Car,
    Ferry,
    Flight,
    #[default]
    Train,
    Walk,
}

impl TransportMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Bike => "bike",
            Self::Bus => "bus",
            Self::Car => "car",
            Self::Ferry => "ferry",
            Self::Flight => "flight",
            Self::Train => "train",
            Self::Walk => "walk",
        }
    }
}

impl std::fmt::Display for TransportMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Booked,
    Completed,
    OptionSelected,
    #[default]
    Planning,
    Open,
}

impl StageStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Booked => "booked",
            Self::Completed => "completed",
            Self::OptionSelected => "option_selected",
            Self::Planning => "planning",
            Self::Open => "open",
        }
    }
}

impl std::fmt::Display for StageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct TripStage {
    #[serde(default)]
    pub id: String,
    pub sequence: usize,
    pub origin: PlaceRef,
    pub destination: PlaceRef,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub status: StageStatus,
    #[serde(default)]
    pub selected_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct PlanSource {
    pub kind: String,
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlan {
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub stages: Vec<TripStage>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub source: Option<PlanSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct TripPlan {
    pub id: String,
    pub title: String,
    pub origin: PlaceRef,
    pub destinations: Vec<PlaceRef>,
    pub date_start: String,
    pub date_end: String,
    #[serde(default)]
    pub interests: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub travelers: Vec<String>,
    #[serde(default)]
    pub transport_modes: Vec<TransportMode>,
    #[serde(default)]
    pub stages: Vec<TripStage>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub source: Option<PlanSource>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub budget_cents: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
}

impl TripPlan {
    /// Connections follow explicitly saved stages; they are not turn-by-turn routes.
    pub fn connections(&self) -> Vec<(places_api::Coordinate, places_api::Coordinate)> {
        self.stages
            .iter()
            .filter_map(|stage| Some((stage.origin.coordinate()?, stage.destination.coordinate()?)))
            .collect()
    }

    pub fn places(&self) -> impl Iterator<Item = &PlaceRef> {
        let mut seen = std::collections::HashSet::new();
        std::iter::once(&self.origin)
            .chain(self.destinations.iter())
            .chain(self.stages.iter().flat_map(|s| [&s.origin, &s.destination]))
            .filter(move |place| {
                seen.insert((
                    place.name.as_str(),
                    place.latitude.map(f64::to_bits),
                    place.longitude.map(f64::to_bits),
                ))
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePlanItem {
    pub item_type: String,
    pub day: Option<String>,
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct PlanItem {
    pub id: String,
    #[serde(default)]
    pub plan_id: String,
    pub item_type: String,
    pub day: Option<String>,
    #[serde(default)]
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub created_at: String,
}

impl PlanItem {
    /// Only fields understood by the itinerary are presented as item properties.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        [
            ("status", "Status"),
            ("time", "Time"),
            ("check_in", "Check-in"),
            ("check_out", "Check-out"),
            ("address", "Address"),
            ("provider", "Provider"),
            ("order_ref", "Reference"),
            ("free_cancellation_until", "Free cancellation until"),
            ("notes", "Notes"),
        ]
        .into_iter()
        .filter_map(|(key, label)| {
            self.payload
                .get(key)?
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| (label, s.to_owned()))
        })
        .collect()
    }

    /// Returns the provider's nonempty booking reference.
    pub fn booking_code(&self) -> Option<&str> {
        self.payload
            .get("order_ref")
            .or_else(|| self.payload.get("booking_code"))
            .or_else(|| self.payload.get("reference"))
            .or_else(|| self.payload.get("pnr"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Returns an HTTP source URL that carries no embedded credentials.
    pub fn source_url(&self) -> Option<url::Url> {
        let raw = self.payload.get("url")?.as_str()?;
        let parsed = url::Url::parse(raw).ok()?;
        (matches!(parsed.scheme(), "http" | "https")
            && parsed.username().is_empty()
            && parsed.password().is_none())
        .then_some(parsed)
    }

    /// Returns a positive candidate rank.
    pub fn stay_rank(&self) -> Option<i64> {
        self.payload
            .get("rank")
            .and_then(serde_json::Value::as_i64)
            .filter(|&rank| rank >= 1)
    }

    /// Whether the stay records a booking or a provider reference.
    pub fn is_stay_booked(&self) -> bool {
        self.payload
            .get("booked")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            || self.booking_code().is_some()
    }

    /// Formats the integer minor-unit price with its stated currency.
    pub fn price_label(&self) -> Option<String> {
        let cents = self
            .payload
            .get("amount_cents")
            .and_then(serde_json::Value::as_i64)?;
        let currency = self
            .payload
            .get("currency")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("EUR");
        let amount = cents.unsigned_abs();
        let sign = if cents < 0 { "-" } else { "" };
        Some(format!(
            "{currency} {sign}{}.{:02}",
            amount / 100,
            amount % 100
        ))
    }

    /// Formats a provider rating to one decimal place.
    pub fn rating_label(&self) -> Option<String> {
        let rating = self
            .payload
            .get("rating")
            .and_then(serde_json::Value::as_f64)?;
        Some(format!("{rating:.1}"))
    }

    /// Whether a packing or gear item is marked as packed.
    pub fn is_packed(&self) -> bool {
        self.payload
            .get("packed")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            || self
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("packed")
    }

    /// Packing category or gear classification.
    pub fn packing_category(&self) -> Option<&str> {
        self.payload
            .get("category")
            .or_else(|| self.payload.get("pack_location"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Weight in grams if known.
    pub fn weight_grams(&self) -> Option<i64> {
        self.payload
            .get("weight_g")
            .or_else(|| self.payload.get("weight"))
            .and_then(serde_json::Value::as_i64)
            .filter(|&w| w > 0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Retrospective {
    pub plan_id: String,
    pub cost_cents: Option<i64>,
    pub currency: Option<String>,
    pub again: String,
    pub change_note: String,
    pub filled_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct PlanDetails {
    #[serde(flatten)]
    pub plan: TripPlan,
    pub items: Vec<PlanItem>,
    #[serde(default)]
    pub retrospective: Option<Retrospective>,
}

// ============================================================================
// Place Resolution
// ============================================================================

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ResolvePlacesRequest {
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ResolvePlacesResult {
    pub plan_id: String,
    pub updated_at: String,
    pub resolved: Vec<ResolvedPlace>,
    pub unresolved: Vec<UnresolvedPlace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ResolvedPlace {
    pub place_id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct UnresolvedPlace {
    pub place_id: String,
    pub name: String,
    pub reason: String,
}

// ============================================================================
// Plan Item Mutations
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct SetItemDay {
    pub day: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, schemars::JsonSchema)]
pub struct EditItemPayload {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Map<String, Value>,
}

// ============================================================================
// Shared URL / Query Helpers
// ============================================================================

pub use places_api::haversine_km;

/// Percent-encode a query value or path segment. Place names carry spaces, commas, umlauts.
pub fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencode() {
        assert_eq!(urlencode("Köln Hbf"), "K%C3%B6ln%20Hbf");
        assert_eq!(urlencode("Paris-Nord"), "Paris-Nord");
    }

    #[test]
    fn test_place_ref_coordinate() {
        let p = PlaceRef {
            id: "1".into(),
            name: "Berlin".into(),
            kind: PlaceKind::Station,
            address: None,
            latitude: Some(52.52),
            longitude: Some(13.405),
        };
        assert!(p.coordinate().is_some());
        let coord = p.coordinate().unwrap();
        assert!((coord.latitude - 52.52).abs() < 1e-4);
    }

    #[test]
    fn test_plan_item_helpers() {
        let item = PlanItem {
            id: "stay-1".into(),
            plan_id: "plan-1".into(),
            item_type: "stay".into(),
            day: Some("2026-06-01".into()),
            external_id: "ext-1".into(),
            title: "Hotel Central".into(),
            payload: serde_json::json!({
                "amount_cents": 12550,
                "currency": "EUR",
                "rank": 1,
                "booked": true,
                "url": "https://example.com/hotel",
                "order_ref": "REF123"
            }),
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        assert_eq!(item.booking_code(), Some("REF123"));
        assert!(item.is_stay_booked());
        assert_eq!(item.stay_rank(), Some(1));
        assert_eq!(item.price_label(), Some("EUR 125.50".into()));
        assert_eq!(
            item.source_url().map(|u| u.to_string()),
            Some("https://example.com/hotel".into())
        );
        let mut refund = item.clone();
        refund.payload["amount_cents"] = serde_json::json!(-50);
        assert_eq!(refund.price_label().as_deref(), Some("EUR -0.50"));
        refund.payload["amount_cents"] = serde_json::json!(i64::MIN);
        assert!(refund.price_label().unwrap().starts_with("EUR -"));
    }
}
