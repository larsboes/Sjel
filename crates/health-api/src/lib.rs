//! Route constants and wire models for the Sjel health capability. The capability
//! (`sjel-health`) and its clients, such as the iOS app's HealthKit sync, share them.

use serde::{Deserialize, Serialize};

pub const ROUTE_METRICS: &str = "/api/metrics";
pub const ROUTE_DAYS: &str = "/api/days";
pub const ROUTE_SUMMARY: &str = "/api/summary";
pub const ROUTE_IMPORTS: &str = "/api/imports";

/// Every route constant above. The server test checks each one against its manifest.
pub const ALL_ROUTES: [&str; 4] = [ROUTE_METRICS, ROUTE_DAYS, ROUTE_SUMMARY, ROUTE_IMPORTS];

/// The path prefix under which the status proxy serves these routes.
pub const PROXY_PREFIX: &str = "/health";

/// The most values one upload may carry: ten years of every metric.
pub const MAX_UPLOAD_VALUES: usize = 3_660 * 32;

/// One daily value as a client sends it. The capability fills in the unit from its catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UploadValue {
    /// A metric key from `GET /api/metrics` or the capability README, for example `steps`.
    pub metric: String,
    /// The local date, `YYYY-MM-DD`.
    pub day: String,
    /// The value in the catalogue's unit (km, kcal, kg, bpm, ms, %, hours).
    pub value: f64,
}

/// The most raw sleep samples one upload may carry: about a month of minute-level stages.
pub const MAX_UPLOAD_SLEEP: usize = 50_000;

/// One raw sleep sample. The capability folds samples into one night per day with the same rule
/// it applies to an Apple Health export, so the phone and the export agree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SleepSample {
    /// The app or device that recorded it, for example `Apple Watch`.
    pub source: String,
    /// Local time with offset, `YYYY-MM-DD HH:MM:SS +HHMM`.
    pub start: String,
    pub end: String,
    /// `InBed`, `Awake`, `AsleepCore`, `AsleepDeep`, `AsleepREM` or `AsleepUnspecified`.
    pub stage: String,
}

/// `POST /api/days`: daily values from one device. Each value replaces the stored value of the
/// same metric and day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DaysUpload {
    /// Who measured: shown as the value's source, for example `HealthKit`.
    pub source: String,
    #[serde(default)]
    pub values: Vec<UploadValue>,
    /// Raw sleep samples. Send them from noon of the day before `from`, so the first night is
    /// whole.
    #[serde(default)]
    pub sleep: Vec<SleepSample>,
    /// The first day the upload covers, `YYYY-MM-DD`. A night before it is not stored.
    #[serde(default)]
    pub from: Option<String>,
}
