//! Pure wire models, contracts, and route constants for the Sjel rail punctuality capability.
//!
//! Shared across server capability (`sjel-punctuality`) and consumer capabilities (`sjel-transit`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const DEFAULT_PUNCTUALITY_PORT: u16 = 8085;

pub const ROUTE_PUNCTUALITY_HEALTH: &str = "/health";
pub const ROUTE_PUNCTUALITY_ROUTES: &str = "/routes";
pub const ROUTE_PUNCTUALITY_LOOKUP: &str = "/lookup";
pub const ROUTE_PUNCTUALITY_STATIONS: &str = "/stations";

pub const MIN_SAMPLE: i64 = 30;
pub const MAX_LOOKUP_STOPS: usize = 200;

// ============================================================================
// Stop Query and Lookup Models
// ============================================================================

/// One stop to look up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StopQuery {
    pub eva: String,
    pub train_type: String,
    pub hour: u8,
    #[serde(default)]
    pub weekend: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least_minutes: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LookupBody {
    pub stops: Vec<StopQuery>,
}

/// What one stop's history says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct StopStats {
    #[serde(default)]
    pub eva: String,
    #[serde(default)]
    pub station_name: Option<String>,
    pub train_type: String,
    pub hour: i16,
    #[serde(default)]
    pub weekend: bool,
    #[serde(default)]
    pub n: i64,
    #[serde(default)]
    pub mean_delay: f32,
    #[serde(default)]
    pub p50: i16,
    #[serde(default)]
    pub p90: i16,
    #[serde(default)]
    pub share_late_6: f32,
    #[serde(default)]
    pub cancel_rate: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_delay_at_least: Option<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least_minutes: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LookupResponse {
    pub stats: Vec<Option<StopStats>>,
    pub min_sample: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StationResult {
    pub eva: String,
    pub station_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoverageInfo {
    pub from_month: String,
    pub to_month: String,
    pub cells: i32,
    pub last_ingest_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HealthResponse {
    pub ok: bool,
    pub capability: String,
    #[serde(default)]
    pub coverage: Option<CoverageInfo>,
}

// ============================================================================
// EVA Normalization
// ============================================================================

/// The dataset writes EVA numbers zero-padded to eight digits (`08000044`); HAFAS, and
/// therefore `crates/transit`, returns them unpadded (`8000044`).
pub fn normalize_eva(eva: &str) -> String {
    let digits = eva.trim_start_matches('0');
    format!("{digits:0>8}")
}

/// Strip leading zeroes from an EVA code for lookups against unpadded HAFAS keys.
pub fn unpad_eva(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('0');
    if trimmed.is_empty() {
        raw.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_eva() {
        assert_eq!(normalize_eva("8000044"), "08000044");
        assert_eq!(normalize_eva("08000044"), "08000044");
        assert_eq!(normalize_eva("008000044"), "08000044");
    }

    #[test]
    fn test_stop_query_serde() {
        let query = StopQuery {
            eva: "8000044".into(),
            train_type: "ICE".into(),
            hour: 14,
            weekend: false,
            at_least_minutes: Some(10),
        };
        let serialized = serde_json::to_string(&query).unwrap();
        let deserialized: StopQuery = serde_json::from_str(&serialized).unwrap();
        assert_eq!(query, deserialized);
    }
}
