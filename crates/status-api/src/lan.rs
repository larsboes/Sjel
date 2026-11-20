//! Local-network listener contract for device pairing.

use serde::{Deserialize, Serialize};

pub const ROUTE_STATUS_LAN: &str = "/api/sjel-status/lan";

/// The node's Same-Wi-Fi listener (`crates/sjel-status/src/lan.rs`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
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
