//! Pure wire models and contracts for Sjel machine setup, component probes, plans, and LAN listener status.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ROUTE_STATUS_LAN: &str = "/api/sjel-status/lan";
pub const ROUTE_STATUS_SETUP: &str = "/api/sjel-status/setup";
pub const ROUTE_STATUS_SETUP_VERIFY: &str = "/api/sjel-status/setup/verify";
pub const ROUTE_STATUS_SETUP_APPLY: &str = "/api/sjel-status/setup/apply";

/// Status mark for a machine probe. Ordered by how much attention it asks for when sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProbeMark {
    #[default]
    Ok,
    Missing,
    Warn,
    Blocked,
}

impl ProbeMark {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Warn => "warn",
            Self::Blocked => "blocked",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Missing => "✗",
            Self::Warn => "!",
            Self::Blocked => "·",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Self::Ok => '✓',
            Self::Missing => '✗',
            Self::Warn => '!',
            Self::Blocked => '·',
        }
    }

    pub fn human_label(self) -> &'static str {
        match self {
            Self::Ok => "in place",
            Self::Missing => "missing",
            Self::Warn => "check it",
            Self::Blocked => "needs you",
        }
    }
}

impl PartialEq<&str> for ProbeMark {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ProbeMark> for &str {
    fn eq(&self, other: &ProbeMark) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<String> for ProbeMark {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<ProbeMark> for String {
    fn eq(&self, other: &ProbeMark) -> bool {
        self.as_str() == other.as_str()
    }
}

impl From<&str> for ProbeMark {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "ok" => Self::Ok,
            "missing" => Self::Missing,
            "warn" => Self::Warn,
            _ => Self::Blocked,
        }
    }
}

impl From<String> for ProbeMark {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

pub type Mark = ProbeMark;

/// One probe reported by `sjel setup plan` or `sjel setup verify`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupProbe {
    pub id: String,
    pub label: String,
    pub mark: ProbeMark,
    pub detail: String,
}

impl SetupProbe {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        mark: ProbeMark,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            mark,
            detail: detail.into(),
        }
    }

    pub fn parsed_mark(&self) -> ProbeMark {
        self.mark
    }
}

pub type Probe = SetupProbe;

/// One actionable step in a setup plan.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupStep {
    pub label: String,
    pub detail: String,
    /// True when the probes say this is already done, so `apply` skips it.
    pub done: bool,
}

pub type Step = SetupStep;

/// Setup plan for a component (e.g. `turso` local replica).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupPlan {
    pub component: String,
    pub purpose: String,
    pub probes: Vec<SetupProbe>,
    pub steps: Vec<SetupStep>,
}

impl SetupPlan {
    /// Number of incomplete steps this apply would execute.
    pub fn applyable_steps(&self) -> usize {
        self.steps.iter().filter(|s| !s.done).count()
    }
}

pub type Plan = SetupPlan;

/// Response payload from `GET /api/sjel-status/setup` and CLI plans JSON.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupPlans {
    pub components: Vec<SetupPlan>,
}

pub type PlansJson = SetupPlans;

/// Probes verification for one component.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupComponentVerification {
    pub component: String,
    pub probes: Vec<SetupProbe>,
}

pub type VerifyJson = SetupComponentVerification;

/// Response payload from `GET /api/sjel-status/setup/verify` and CLI verifies JSON.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupVerification {
    pub components: Vec<SetupComponentVerification>,
}

pub type VerifiesJson = SetupVerification;

/// Result of applying setup steps for one component.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupComponentApplied {
    pub component: String,
    pub applied: Vec<String>,
    #[serde(default)]
    pub verify: Vec<SetupProbe>,
}

pub type ApplyJson = SetupComponentApplied;

/// Response payload from `POST /api/sjel-status/setup/apply` and CLI applies JSON.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupApplied {
    pub components: Vec<SetupComponentApplied>,
}

pub type AppliesJson = SetupApplied;

/// Request payload for `POST /api/sjel-status/setup/apply`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct SetupApplyRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    /// The primary's credential. Write-only: piped to tool stdin, never logged or returned.
    #[serde(default, skip_serializing)]
    pub token: Option<String>,
}

/// The node's Same-Wi-Fi listener (`capabilities/sjel-status/src/lan.rs`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct LanInfo {
    pub port: u16,
    pub host: String,
    pub fingerprint: String,
    #[serde(default)]
    pub service: String,
    /// This Mac's tailnet IPv4 address, when Tailscale runs.
    #[serde(default)]
    pub tailnet: Option<String>,
}

impl LanInfo {
    /// The hosts a pairing code can carry, tailnet first.
    pub fn pairing_hosts(&self) -> Vec<String> {
        self.tailnet
            .iter()
            .chain(std::iter::once(&self.host))
            .cloned()
            .collect()
    }
}

/// What `GET /api/sjel-status/lan` answers: `{"enabled": true, "lan": {...}}`, or `enabled: false`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct LanStatus {
    pub enabled: bool,
    #[serde(default)]
    pub lan: Option<LanInfo>,
}

impl LanStatus {
    pub fn listener(&self) -> Option<&LanInfo> {
        if self.enabled {
            self.lan.as_ref()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_glyphs_and_symbols() {
        assert_eq!(ProbeMark::Ok.glyph(), '✓');
        assert_eq!(ProbeMark::Missing.glyph(), '✗');
        assert_eq!(ProbeMark::Warn.glyph(), '!');
        assert_eq!(ProbeMark::Blocked.glyph(), '·');

        assert_eq!(ProbeMark::Ok.symbol(), "✓");
        assert_eq!(ProbeMark::Missing.symbol(), "✗");
    }

    #[test]
    fn mark_equality_with_str() {
        assert_eq!(ProbeMark::Ok, "ok");
        assert_eq!("missing", ProbeMark::Missing);
        assert_ne!(ProbeMark::Warn, "ok");
    }

    #[test]
    fn serde_probe_mark() {
        let json = serde_json::to_string(&ProbeMark::Ok).unwrap();
        assert_eq!(json, "\"ok\"");
        let parsed: ProbeMark = serde_json::from_str("\"warn\"").unwrap();
        assert_eq!(parsed, ProbeMark::Warn);
    }
}
