//! Data models and pure reductions for Sjel Machine Setup & Device Pairing (`/setup`).
//!
//! Replaces the dashboard's `/setup` route. Backed by `capabilities/sjel-status` (`/api/sjel-status/setup*`,
//! `/api/sjel-status/lan`) and `capabilities/devices` (`/api/devices`, `/api/pairing/*`).

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

pub use devices_api::{
    Device as PairedDevice, PairingChallenge, PairingRequest as SetupJoinRequest,
};
pub use setup_api::*;

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
/// so a single scan finds the Mac, pins it and claims the code (`device::PairingQr`). `host` is
/// one of [`LanInfo::pairing_hosts`].
pub fn pairing_payload(lan: &LanInfo, host: &str, challenge: &PairingChallenge) -> String {
    crate::device::pairing_qr_payload(&crate::device::PairingQr {
        host: host.to_string(),
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
        assert!(!stamp(Some(1_761_303_600)).is_empty());
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
            tailnet: Some("100.116.104.41".into()),
        };
        assert_eq!(lan.pairing_hosts(), ["100.116.104.41", "lars-mac"]);
        let challenge: PairingChallenge = serde_json::from_str(
            r#"{"protocol_version": "sjel-pairing/v1", "challenge_id": "pair_01",
                "code": "482913abcd", "expires_at": 1761303600,
                "qr_payload": "{}"}"#,
        )
        .unwrap();
        let payload = pairing_payload(&lan, "lars-mac", &challenge);
        let parsed = crate::device::parse_pairing_qr(&payload).expect("its own code must parse");
        assert_eq!(parsed.host, "lars-mac");
        let tailnet = pairing_payload(&lan, "100.116.104.41", &challenge);
        let parsed = crate::device::parse_pairing_qr(&tailnet).expect("a tailnet code must parse");
        assert_eq!(parsed.host, "100.116.104.41");
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
