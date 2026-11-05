//! Pure wire models and contracts for Sjel device pairing, challenges, and revocation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROTOCOL_VERSION: &str = "axon-pairing/v1";
pub const QR_KIND: &str = "sjel-pair/1";

pub const ROUTE_DEVICES: &str = "/api/devices";
pub const ROUTE_DEVICES_ME: &str = "/api/devices/me";
pub const ROUTE_DEVICE_REVOKE: &str = "/api/devices/{id}/revoke";
pub const ROUTE_PAIRING_CHALLENGES: &str = "/api/pairing/challenges";
pub const ROUTE_PAIRING_CLAIMS: &str = "/api/pairing/claims";
pub const ROUTE_PAIRING_REQUESTS: &str = "/api/pairing/requests";
pub const ROUTE_PAIRING_REQUEST: &str = "/api/pairing/requests/{id}";
pub const ROUTE_PAIRING_APPROVE: &str = "/api/pairing/requests/{id}/approve";
pub const ROUTE_PAIRING_DENY: &str = "/api/pairing/requests/{id}/deny";

pub const PROXY_ROUTE_DEVICES: &str = "/devices/api/devices";
pub const PROXY_ROUTE_DEVICES_ME: &str = "/devices/api/devices/me";
pub const PROXY_ROUTE_PAIRING_CHALLENGES: &str = "/devices/api/pairing/challenges";
pub const PROXY_ROUTE_PAIRING_CLAIMS: &str = "/devices/api/pairing/claims";
pub const PROXY_ROUTE_PAIRING_REQUESTS: &str = "/devices/api/pairing/requests";

fn default_protocol_version() -> String {
    PROTOCOL_VERSION.to_string()
}

/// One paired device registered with the local node.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_seen_at: Option<i64>,
    #[serde(default)]
    pub revoked_at: Option<i64>,
}

impl Device {
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

pub type PairedDevice = Device;

/// A phone's request to join, waiting for the owner to allow or deny it on the node.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PairingRequest {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub platform: String,
    /// Six digits derived from the requesting key (`key_code`). The phone shows the same digits,
    /// so the owner can tell their phone's request from anyone else's.
    #[serde(default)]
    pub key_code: String,
    /// `pending`, `approved`, `denied` or `expired`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

pub type SetupJoinRequest = PairingRequest;

/// Minimal reference to a registered device.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct DeviceRecord {
    pub id: String,
}

/// A device's request to join as reported back to the device.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct JoinResponse {
    pub id: String,
    #[serde(default)]
    pub key_code: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub device: Option<DeviceRecord>,
}

pub type JoinStatus = JoinResponse;

/// A one-time code for one new device, minted with `POST /devices/api/pairing/challenges`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PairingChallenge {
    #[serde(default = "default_protocol_version")]
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

/// Payload for claiming a one-time pairing challenge.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ClaimRequest {
    pub challenge_id: String,
    pub code: String,
    pub label: String,
    pub platform: String,
    pub algorithm: String,
    /// Hex-encoded 32-byte Ed25519 public key. The private key never crosses this API.
    pub public_key: String,
}

/// Payload for requesting a join directly.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct JoinRequest {
    pub label: String,
    pub platform: String,
    pub algorithm: String,
    /// Hex-encoded 32-byte Ed25519 public key. The private key never crosses this API.
    pub public_key: String,
}

/// List of paired devices via `GET /devices/api/devices`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct DevicesResp {
    #[serde(default)]
    pub devices: Vec<Device>,
}

/// List of pending join requests via `GET /devices/api/pairing/requests`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct JoinsResp {
    #[serde(default)]
    pub requests: Vec<PairingRequest>,
}

/// What the GPUI app's pairing QR code carries.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PairingQr {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub challenge_id: String,
    pub code: String,
}

/// Writes what [`parse_pairing_qr`] reads. The two live together so a field cannot be renamed on
/// one side only: the Mac draws this and the phone refuses anything the parser would not accept.
pub fn pairing_qr_payload(qr: &PairingQr) -> String {
    serde_json::json!({
        "k": QR_KIND,
        "h": qr.host,
        "p": qr.port,
        "f": qr.fingerprint,
        "i": qr.challenge_id,
        "c": qr.code,
    })
    .to_string()
}

/// Reads a scanned code. A code may carry a dotted hostname, including the Mac's tailnet address.
pub fn parse_pairing_qr(content: &str) -> Result<PairingQr, String> {
    #[derive(Deserialize)]
    struct Raw {
        k: String,
        h: String,
        p: u32,
        f: String,
        i: String,
        c: String,
    }
    let raw: Raw = serde_json::from_str(content)
        .map_err(|_| "That is not a Sjel pairing code.".to_string())?;
    if raw.k != QR_KIND {
        return Err("That is not a Sjel pairing code.".into());
    }
    let host_ok = host_ok(&raw.h);
    let fingerprint_ok = raw.f.len() == 64
        && raw
            .f
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let port = u16::try_from(raw.p).ok().filter(|p| *p > 0);
    match port {
        Some(port) if host_ok && fingerprint_ok && !raw.i.is_empty() && !raw.c.is_empty() => {
            Ok(PairingQr {
                host: raw.h,
                port,
                fingerprint: raw.f,
                challenge_id: raw.i,
                code: raw.c,
            })
        }
        _ => Err("This pairing code is damaged. Show a new one on the Mac.".into()),
    }
}

/// A DNS name or an IPv4 literal: at most 253 characters, dot-separated labels of 1 to 63 from
/// `[A-Za-z0-9-]`. No scheme, path, port or space fits, so the host cannot leave the base URL.
pub fn host_ok(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// The listener's base URL for a host from a pairing code. A name without a dot is the Mac's
/// Bonjour name and gets `.local`; a dotted name or an IP address, such as the Mac's tailnet
/// address, is used as given.
pub fn node_base(host: &str, port: u16) -> String {
    if host.contains('.') {
        format!("https://{host}:{port}")
    } else {
        format!("https://{host}.local:{port}")
    }
}

/// The Mac's LAN listener without a code: `<name> <port> <fingerprint>`, the parts of the QR
/// code that are not secret, under the same rules. Returns the base URL and the pin.
pub fn parse_join_target(text: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [host, port, fingerprint] = parts[..] else {
        return Err("Expected `<mac name> <port> <fingerprint>`.".into());
    };
    let qr = serde_json::json!({
        "k": QR_KIND, "h": host, "p": port.parse::<u32>().unwrap_or(0), "f": fingerprint,
        "i": "-", "c": "-",
    });
    let qr = parse_pairing_qr(&qr.to_string())?;
    Ok((node_base(&qr.host, qr.port), qr.fingerprint))
}

/// The certificate half of the code both screens show while a join waits: the first 16 hex
/// characters in four groups shown by the GPUI app.
pub fn comparison_code(fingerprint: &str) -> String {
    let head = fingerprint
        .get(..16)
        .unwrap_or(fingerprint)
        .to_ascii_uppercase();
    head.as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Six digits from the SHA-256 of a public key, shown as `482 913`. The phone computes the same
/// value from its own key. It tells requests apart; it
/// is not a secret. The TLS pin the phone compares in the same step is what stops a relay.
pub fn key_code(public_key: &str) -> String {
    let digest = Sha256::digest(
        format!(
            "sjel-pairing-key/v1:{}",
            public_key.trim().to_ascii_lowercase()
        )
        .as_bytes(),
    );
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    let digits = format!("{value:06}");
    format!("{} {}", &digits[..3], &digits[3..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_roundtrip() {
        let qr = PairingQr {
            host: "lars-mac".into(),
            port: 8443,
            fingerprint: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            challenge_id: "pair_1234".into(),
            code: "482913".into(),
        };
        let payload = pairing_qr_payload(&qr);
        let parsed = parse_pairing_qr(&payload).expect("should parse");
        assert_eq!(parsed, qr);
    }

    #[test]
    fn key_code_stability() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let code = key_code(key);
        assert_eq!(code.len(), 7); // "XXX XXX"
        assert_eq!(code, key_code(key));
    }

    #[test]
    fn comparison_code_formatting() {
        let fp = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let comp = comparison_code(fp);
        assert_eq!(comp, "0123 4567 89AB CDEF");
    }
}
