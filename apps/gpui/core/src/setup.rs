//! Data models and pure reductions for Sjel Machine Setup & Device Pairing (`/setup`).
//!
//! Replaces the dashboard's `/setup` route. Backed by `capabilities/sjel-status` (`/api/sjel-status/setup*`,
//! `/api/sjel-status/lan`) and `capabilities/devices` (`/api/devices`, `/api/pairing/*`).

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

/// Status mark for a machine probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeMark {
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

    pub fn human_label(self) -> &'static str {
        match self {
            Self::Ok => "in place",
            Self::Missing => "missing",
            Self::Warn => "check it",
            Self::Blocked => "needs you",
        }
    }
}

/// One probe reported by `sjel setup plan` or `sjel setup verify`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupProbe {
    pub id: String,
    pub label: String,
    pub mark: String,
    pub detail: String,
}

impl SetupProbe {
    pub fn parsed_mark(&self) -> ProbeMark {
        match self.mark.as_str() {
            "ok" => ProbeMark::Ok,
            "missing" => ProbeMark::Missing,
            "warn" => ProbeMark::Warn,
            _ => ProbeMark::Blocked,
        }
    }
}

/// One actionable step in a setup plan.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupStep {
    pub label: String,
    pub detail: String,
    pub done: bool,
}

/// Setup plan for a component (e.g. `turso` local replica).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
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

/// Response payload from `GET /api/sjel-status/setup`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupPlans {
    pub components: Vec<SetupPlan>,
}

/// Probes verification for one component.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupComponentVerification {
    pub component: String,
    pub probes: Vec<SetupProbe>,
}

/// Response payload from `GET /api/sjel-status/setup/verify`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupVerification {
    pub components: Vec<SetupComponentVerification>,
}

/// Request payload for `POST /api/sjel-status/setup/apply`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupApplyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Result of applying setup steps for one component.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupComponentApplied {
    pub component: String,
    pub applied: Vec<String>,
    #[serde(default)]
    pub verify: Vec<SetupProbe>,
}

/// Response payload from `POST /api/sjel-status/setup/apply`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupApplied {
    pub components: Vec<SetupComponentApplied>,
}

/// One paired device registered with the local node, as `GET /devices/api/devices` answers it
/// (`Device` in `capabilities/devices/src/store.rs`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub public_key: String,
    /// SHA-256 of the public key: what the phone pinned and what a person can compare.
    #[serde(default)]
    pub fingerprint: String,
    /// `active` or `revoked`, decided at the node.
    #[serde(default)]
    pub status: String,
    /// Epoch seconds. The capability records when the key was registered, not `paired_at`.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_seen_at: Option<i64>,
    #[serde(default)]
    pub revoked_at: Option<i64>,
}

impl PairedDevice {
    /// Whether this key still opens the node. A revoked key keeps a row here: the record of what
    /// was once trusted is worth reading after it stops being trusted.
    pub fn is_active(&self) -> bool {
        self.status != "revoked" && self.revoked_at.is_none()
    }

    /// The name to draw: the label the person gave the phone, or its id when they gave none.
    pub fn name(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// Pending join request from a phone or companion device, as `GET /devices/api/pairing/requests`
/// answers it (`PairingRequest` in `capabilities/devices/src/store.rs`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupJoinRequest {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub platform: String,
    /// Six digits derived from the requesting key. The phone shows the same digits, so the owner
    /// can tell their phone's request from anyone else's.
    #[serde(default)]
    pub key_code: String,
    /// `pending`, `approved`, `denied` or `expired`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: i64,
}

/// The node's Same-Wi-Fi listener (`capabilities/sjel-status/src/lan.rs`). Nothing in it is secret:
/// the fingerprint is advertised in the Bonjour TXT record, and it is what a phone pins.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct LanInfo {
    pub port: u16,
    pub host: String,
    pub fingerprint: String,
    #[serde(default)]
    pub service: String,
}

/// What `GET /api/sjel-status/lan` answers: `{"enabled": true, "lan": {...}}`, or `enabled: false`
/// and no listener when this deployment opted out.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct LanStatus {
    pub enabled: bool,
    #[serde(default)]
    pub lan: Option<LanInfo>,
}

impl LanStatus {
    /// The listener, when this node runs one.
    pub fn listener(&self) -> Option<&LanInfo> {
        if self.enabled {
            self.lan.as_ref()
        } else {
            None
        }
    }
}

/// A one-time code for one new device, minted with `POST /devices/api/pairing/challenges`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairingChallenge {
    #[serde(default)]
    pub protocol_version: String,
    pub challenge_id: String,
    /// Typed by hand when the camera cannot be used.
    pub code: String,
    /// Epoch seconds: ten minutes after it was minted.
    pub expires_at: i64,
    /// The `axon-pairing/v1` JSON the challenge carries, for a paste on the phone.
    #[serde(default)]
    pub qr_payload: String,
}

/// Full combined view model for the `/setup` native page.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetupView {
    pub plans: Vec<SetupPlan>,
    #[serde(default)]
    pub devices: Vec<PairedDevice>,
    #[serde(default)]
    pub pending_joins: Vec<SetupJoinRequest>,
    /// The listener a phone can reach, or `None` when this node has none.
    #[serde(default)]
    pub lan: Option<LanInfo>,
    /// Why the listener could not be read, kept apart from `lan: None` so a page never reports a
    /// failed read as "Same Wi-Fi is off".
    #[serde(default)]
    pub lan_error: Option<String>,
}

// ── Pairing code ───────────────────────────────────────────────────────────

/// The code a phone scans: the address, the certificate pin and the one-time code in one string,
/// so a single scan finds the Mac, pins it and claims the code (`device::PairingQr`).
pub fn pairing_payload(lan: &LanInfo, challenge: &PairingChallenge) -> String {
    crate::device::pairing_qr_payload(&crate::device::PairingQr {
        host: lan.host.clone(),
        port: lan.port,
        fingerprint: lan.fingerprint.clone(),
        challenge_id: challenge.challenge_id.clone(),
        code: challenge.code.clone(),
    })
}

/// The dark modules of a QR code, row-major, with its width in modules. The page paints them;
/// nothing here knows about pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QrMatrix {
    pub width: usize,
    pub dark: Vec<bool>,
}

impl QrMatrix {
    /// Whether the module at column `x`, row `y` is dark. Out of range reads light, so a paint
    /// pass that overruns draws nothing rather than panicking mid-frame.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        if x >= self.width {
            return false;
        }
        self.dark.get(y * self.width + x).copied().unwrap_or(false)
    }
}

/// Encodes `payload` for display. A payload too long for a QR code is the only failure.
pub fn qr_matrix(payload: &str) -> Result<QrMatrix, String> {
    let code = qrcode::QrCode::new(payload.as_bytes())
        .map_err(|_| "This pairing payload is too long to draw as a code.".to_string())?;
    let width = code.width();
    let dark = code
        .to_colors()
        .into_iter()
        .map(|module| module == qrcode::Color::Dark)
        .collect();
    Ok(QrMatrix { width, dark })
}

// ── Times and codes ────────────────────────────────────────────────────────

/// `12:04` in the given zone. A pairing screen names a time of day, never a date: the code lives
/// for ten minutes.
pub fn clock_in<T: TimeZone>(zone: &T, secs: i64) -> String {
    match zone.timestamp_opt(secs, 0).single() {
        Some(at) => at.naive_local().format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// [`clock_in`] in this machine's own zone: a code expires this afternoon, not on another planet.
pub fn clock(secs: i64) -> String {
    clock_in(&Local, secs)
}

/// `24 Oct 2026 12:04` in the given zone, for a record that is read long after it was written.
pub fn stamp_in<T: TimeZone>(zone: &T, secs: i64) -> String {
    match zone.timestamp_opt(secs, 0).single() {
        Some(at) => at.naive_local().format("%d %b %Y %H:%M").to_string(),
        None => String::new(),
    }
}

/// [`stamp_in`] in this machine's own zone, and `Never` for a device the node has not seen.
pub fn stamp(secs: Option<i64>) -> String {
    match secs {
        Some(secs) => stamp_in(&Local, secs),
        None => "Never".to_string(),
    }
}

/// How long a code has left, in the words the person needs: the countdown, or that it is gone.
pub fn minutes_left(expires_at: i64, now: i64) -> String {
    let left = expires_at - now;
    if left <= 0 {
        return "expired".to_string();
    }
    if left < 60 {
        return "under a minute left".to_string();
    }
    format!("{} min left", left / 60)
}

/// The certificate pin in a form a person can read aloud and compare with the phone's screen:
/// first eight hex digits, an ellipsis, then the last four.
pub fn short_pin(fingerprint: &str) -> String {
    if fingerprint.len() <= 12 {
        return fingerprint.to_string();
    }
    format!(
        "{}…{}",
        &fingerprint[..8],
        &fingerprint[fingerprint.len() - 4..]
    )
}

/// The devices still trusted, and the ones a revoke left behind.
pub fn partition_devices(devices: &[PairedDevice]) -> (Vec<PairedDevice>, Vec<PairedDevice>) {
    devices
        .iter()
        .cloned()
        .partition(|device| device.is_active())
}

/// The joins still waiting for a decision: an approved or expired row is history, not a prompt.
pub fn waiting_joins(joins: &[SetupJoinRequest]) -> Vec<SetupJoinRequest> {
    joins
        .iter()
        .filter(|join| join.status.is_empty() || join.status == "pending")
        .cloned()
        .collect()
}

/// `Some(instant)` for one instant, for a test that must not read the machine's zone.
#[cfg(test)]
fn at(secs: i64) -> chrono::DateTime<chrono::FixedOffset> {
    chrono::FixedOffset::east_opt(2 * 3600)
        .expect("a two-hour offset is real")
        .timestamp_opt(secs, 0)
        .single()
        .expect("a real instant")
}

// ── Pure Reductions ────────────────────────────────────────────────────────

/// Merge fresh verification probes into existing setup plans.
pub fn merge_verification_probes(
    plans: &[SetupPlan],
    verification: &SetupVerification,
) -> Vec<SetupPlan> {
    plans
        .iter()
        .map(|plan| {
            if let Some(v) = verification
                .components
                .iter()
                .find(|c| c.component == plan.component)
            {
                let mut updated = plan.clone();
                updated.probes = v.probes.clone();
                updated
            } else {
                plan.clone()
            }
        })
        .collect()
}

/// Check if apply form fields contain any input to write.
pub fn has_apply_input(url: &str, token: &str, interval: &str) -> bool {
    !url.trim().is_empty()
        || !token.trim().is_empty()
        || (!interval.trim().is_empty() && interval.trim().parse::<u64>().unwrap_or(0) > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_mark_helpers() {
        assert_eq!(ProbeMark::Ok.symbol(), "✓");
        assert_eq!(ProbeMark::Missing.symbol(), "✗");
        assert_eq!(ProbeMark::Warn.symbol(), "!");
        assert_eq!(ProbeMark::Blocked.symbol(), "·");

        assert_eq!(ProbeMark::Ok.human_label(), "in place");
        assert_eq!(ProbeMark::Missing.human_label(), "missing");
        assert_eq!(ProbeMark::Warn.human_label(), "check it");
        assert_eq!(ProbeMark::Blocked.human_label(), "needs you");
    }

    #[test]
    fn test_applyable_steps_count() {
        let plan = SetupPlan {
            component: "turso".to_string(),
            purpose: "replica".to_string(),
            probes: vec![],
            steps: vec![
                SetupStep {
                    label: "step 1".into(),
                    detail: "done".into(),
                    done: true,
                },
                SetupStep {
                    label: "step 2".into(),
                    detail: "pending".into(),
                    done: false,
                },
                SetupStep {
                    label: "step 3".into(),
                    detail: "pending".into(),
                    done: false,
                },
            ],
        };
        assert_eq!(plan.applyable_steps(), 2);
    }

    #[test]
    fn test_has_apply_input() {
        assert!(!has_apply_input("", "", ""));
        assert!(!has_apply_input("   ", "", "0"));
        assert!(has_apply_input("libsql://primary", "", ""));
        assert!(has_apply_input("", "secret-token", ""));
        assert!(has_apply_input("", "", "60"));
    }

    #[test]
    fn test_merge_verification_probes() {
        let plans = vec![SetupPlan {
            component: "turso".into(),
            purpose: "replica".into(),
            probes: vec![SetupProbe {
                id: "record".into(),
                label: "deployment record".into(),
                mark: "missing".into(),
                detail: "no record".into(),
            }],
            steps: vec![],
        }];

        let verification = SetupVerification {
            components: vec![SetupComponentVerification {
                component: "turso".into(),
                probes: vec![SetupProbe {
                    id: "record".into(),
                    label: "deployment record".into(),
                    mark: "ok".into(),
                    detail: "verified".into(),
                }],
            }],
        };

        let merged = merge_verification_probes(&plans, &verification);
        assert_eq!(merged[0].probes[0].mark, "ok");
        assert_eq!(merged[0].probes[0].detail, "verified");
    }

    #[test]
    fn test_deserialize_live_setup_plan() {
        let json = r#"{
            "components": [
                {
                    "component": "turso",
                    "purpose": "follow a remote Turso primary as a local replica",
                    "probes": [
                        {
                            "id": "record",
                            "label": "deployment record",
                            "mark": "ok",
                            "detail": "/config/deployment.env"
                        }
                    ],
                    "steps": [
                        {
                            "label": "record SJEL_TURSO_URL",
                            "detail": "already recorded",
                            "done": true
                        }
                    ]
                }
            ]
        }"#;

        let plans: SetupPlans = serde_json::from_str(json).unwrap();
        assert_eq!(plans.components.len(), 1);
        assert_eq!(plans.components[0].component, "turso");
        assert_eq!(plans.components[0].probes[0].parsed_mark(), ProbeMark::Ok);
        assert!(plans.components[0].steps[0].done);
    }

    /// The shape `GET /devices/api/devices` really answers (`Device` in the capability's store).
    fn live_device(status: &str) -> PairedDevice {
        let json = format!(
            r#"{{
                "id": "dev_01",
                "label": "iPhone",
                "platform": "iOS",
                "algorithm": "ed25519",
                "public_key": "abc",
                "fingerprint": "{}",
                "status": "{status}",
                "created_at": 1761300000,
                "last_seen_at": 1761303600,
                "revoked_at": {}
            }}"#,
            "ab".repeat(32),
            if status == "revoked" {
                "1761304000"
            } else {
                "null"
            },
        );
        serde_json::from_str(&json).expect("the capability's own device shape")
    }

    #[test]
    fn a_device_reads_the_capabilitys_own_fields() {
        let device = live_device("active");
        assert_eq!(device.name(), "iPhone");
        assert_eq!(device.platform, "iOS");
        // The capability records `created_at`; a page that asked for `paired_at` drew a blank date.
        assert_eq!(device.created_at, 1_761_300_000);
        assert!(device.is_active());
        assert_eq!(stamp(Some(1_761_303_600)).is_empty(), false);
        assert_eq!(stamp(None), "Never");

        let revoked = live_device("revoked");
        assert!(!revoked.is_active());
    }

    #[test]
    fn a_revoked_device_is_history_not_trust() {
        let devices = vec![live_device("active"), live_device("revoked")];
        let (active, revoked) = partition_devices(&devices);
        assert_eq!(active.len(), 1);
        assert_eq!(revoked.len(), 1);
        assert!(active[0].is_active());
        // A revoked key still carries its revocation time, which is what the row explains.
        assert!(revoked[0].revoked_at.is_some());
    }

    #[test]
    fn only_a_pending_join_asks_for_a_decision() {
        let join = |status: &str| SetupJoinRequest {
            id: format!("join_{status}"),
            key_code: "482 913".into(),
            status: status.into(),
            ..Default::default()
        };
        let waiting =
            waiting_joins(&[join("pending"), join("approved"), join("expired"), join("")]);
        assert_eq!(waiting.len(), 2);
        assert!(waiting.iter().all(|j| j.status != "approved"));
    }

    #[test]
    fn the_listener_is_only_there_when_it_is_on() {
        let off: LanStatus = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
        assert!(off.listener().is_none());

        let on: LanStatus = serde_json::from_str(
            r#"{"enabled": true, "lan": {"port": 8443, "host": "lars-mac",
                 "fingerprint": "abababababababababababababababababababababababababababababababab",
                 "service": "_sjel._tcp"}}"#,
        )
        .unwrap();
        let lan = on.listener().expect("a listener when the node runs one");
        assert_eq!(lan.host, "lars-mac");
        assert_eq!(lan.port, 8443);
    }

    #[test]
    fn the_code_the_mac_draws_is_the_code_the_phone_parses() {
        let lan = LanInfo {
            port: 8443,
            host: "lars-mac".into(),
            fingerprint: "ab".repeat(32),
            service: "_sjel._tcp".into(),
        };
        let challenge: PairingChallenge = serde_json::from_str(
            r#"{"protocol_version": "axon-pairing/v1", "challenge_id": "pair_01",
                "code": "482913abcd", "expires_at": 1761303600,
                "qr_payload": "{}"}"#,
        )
        .unwrap();
        let payload = pairing_payload(&lan, &challenge);
        let parsed = crate::device::parse_pairing_qr(&payload).expect("its own code must parse");
        assert_eq!(parsed.host, "lars-mac");
        assert_eq!(parsed.port, 8443);
        assert_eq!(parsed.challenge_id, "pair_01");
        assert_eq!(parsed.code, "482913abcd");
    }

    #[test]
    fn the_matrix_is_square_and_reads_light_past_its_edge() {
        let matrix = qr_matrix("{\"k\":\"sjel-pair/1\"}").expect("a short payload encodes");
        assert!(matrix.width >= 21);
        assert_eq!(matrix.dark.len(), matrix.width * matrix.width);
        assert!(matrix.dark.iter().any(|dark| *dark));
        // A paint pass that overruns the grid draws nothing rather than panicking mid-frame.
        assert!(!matrix.is_dark(matrix.width, 0));
        assert!(!matrix.is_dark(0, matrix.width));
    }

    #[test]
    fn a_countdown_says_what_a_person_can_act_on() {
        let now = 1_761_303_000;
        assert_eq!(minutes_left(now + 600, now), "10 min left");
        assert_eq!(minutes_left(now + 90, now), "1 min left");
        assert_eq!(minutes_left(now + 59, now), "under a minute left");
        assert_eq!(minutes_left(now, now), "expired");
        assert_eq!(minutes_left(now - 5, now), "expired");
    }

    #[test]
    fn a_pin_is_short_enough_to_compare_by_eye() {
        let pin = "0123456789abcdef".repeat(4);
        assert_eq!(short_pin(&pin), "01234567…cdef");
        assert_eq!(short_pin("abc"), "abc");
    }

    #[test]
    fn times_are_read_in_the_zone_the_reader_is_in() {
        // 2026-10-24 10:00:00 UTC, read two hours east: the clock and the date both move.
        let secs = at(1_792_836_000).timestamp();
        assert_eq!(
            clock_in(&chrono::FixedOffset::east_opt(2 * 3600).unwrap(), secs),
            "12:00"
        );
        assert_eq!(
            clock_in(&chrono::FixedOffset::east_opt(0).unwrap(), secs),
            "10:00"
        );
        assert_eq!(
            stamp_in(&chrono::FixedOffset::east_opt(2 * 3600).unwrap(), secs),
            "24 Oct 2026 12:00"
        );
    }
}
