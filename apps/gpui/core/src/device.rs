//! A paired device's way into Sjel (`capabilities/devices/README.md`, "Guided pairing").
//!
//! The Mac's Devices panel shows a QR code with the node's LAN listener and a one-time code. The
//! device claims the code with its public key and from then on signs every request
//! (`libs/device-auth`) over TLS pinned to the node's certificate. The key never enters this
//! module: the caller passes a [`Signer`] and the public key.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Signs the bytes it gets with the device key and returns the 64-byte signature.
pub type Signer = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync>;

const QR_KIND: &str = "sjel-pair/1";
const CLAIM_PATH: &str = "/devices/api/pairing/claims";
const JOIN_PATH: &str = "/devices/api/pairing/requests";
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the Mac's pairing QR code carries (`pairingQrPayload` in `dashboard/src/lib/devices.ts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingQr {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub challenge_id: String,
    pub code: String,
}

/// Reads a scanned code with the rules of `parsePairingQr` in `dashboard/src/lib/devices.ts`.
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
    let host_ok = (1..=63).contains(&raw.h.len())
        && raw
            .h
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
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
    Ok((
        format!("https://{}.local:{}", qr.host, qr.port),
        qr.fingerprint,
    ))
}

/// The certificate half of the code both screens show while a join waits: the first 16 hex
/// characters in four groups (`comparisonCode` in `dashboard/src/lib/connection/transports.ts`).
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

/// A device's request to join, as the node reports it (`JoinRequest` in
/// `dashboard/src/lib/devices.ts`).
#[derive(Clone, Debug, Deserialize)]
pub struct JoinRequest {
    pub id: String,
    /// Six digits from this device's key, as the Mac shows them: `482 913`.
    pub key_code: String,
    /// `pending`, `approved`, `denied` or `expired`.
    pub status: String,
    pub device: Option<DeviceRecord>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceRecord {
    pub id: String,
}

/// Asks the node to register this device's key. The owner allows or denies it on the Mac.
pub async fn request_join(
    base: &str,
    pin: &str,
    label: &str,
    public_key: &str,
) -> Result<JoinRequest, String> {
    let body = serde_json::json!({
        "label": label,
        "platform": "ios",
        "algorithm": "ed25519",
        "public_key": public_key,
    });
    // Unsigned: the key is not registered yet (`device_auth::unsigned_pairing_request`).
    let response = pinned_client(pin)?
        .post(format!("{base}{JOIN_PATH}"))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("The Mac did not answer: {error}"))?;
    read(response).await
}

/// The current state of one join request.
pub async fn join_state(base: &str, pin: &str, id: &str) -> Result<JoinRequest, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("The Mac answered with a request id this app cannot use.".into());
    }
    let response = pinned_client(pin)?
        .get(format!("{base}{JOIN_PATH}/{id}"))
        .send()
        .await
        .map_err(|error| format!("The Mac did not answer: {error}"))?;
    read(response).await
}

/// The node this device paired with. Nothing in it is secret: the key stays with the platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// `https://<name>.local:<port>`, the LAN listener.
    pub base: String,
    /// SHA-256 of the listener's certificate, 64 hex characters.
    pub pin: String,
    pub device_id: String,
}

/// Claims the QR code's one-time code for this device's public key, and returns the node.
pub async fn claim(qr: &PairingQr, label: &str, public_key: &str) -> Result<Node, String> {
    #[derive(Deserialize)]
    struct Device {
        id: String,
    }
    let base = format!("https://{}.local:{}", qr.host, qr.port);
    let client = pinned_client(&qr.fingerprint)?;
    let body = serde_json::json!({
        "challenge_id": qr.challenge_id,
        "code": qr.code,
        "label": label,
        "platform": "ios",
        "algorithm": "ed25519",
        "public_key": public_key,
    });
    // One of the three requests a device sends unsigned (`device_auth::unsigned_pairing_request`).
    let response = client
        .post(format!("{base}{CLAIM_PATH}"))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("The Mac did not answer: {error}"))?;
    let device: Device = read(response).await?;
    Ok(Node {
        base,
        pin: qr.fingerprint.clone(),
        device_id: device.id,
    })
}

/// Signed requests to the node, as the paired device.
pub struct DeviceClient {
    node: Node,
    client: reqwest::Client,
    sign: Signer,
}

impl DeviceClient {
    pub fn new(node: Node, sign: Signer) -> Result<Self, String> {
        let client = pinned_client(&node.pin)?;
        Ok(Self { node, client, sign })
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    /// A signed GET of `shell_path`, for example `/api/sjel-status/health`.
    pub async fn get<T: DeserializeOwned>(&self, shell_path: &str) -> Result<T, String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "the clock is before 1970".to_string())?
            .as_secs() as i64;
        let sign = self.sign.clone();
        let headers = device_auth::signed_headers(
            &self.node.device_id,
            timestamp,
            "GET",
            shell_path,
            b"",
            |m| sign(m),
        )?;
        let mut request = self
            .client
            .request(Method::GET, format!("{}{shell_path}", self.node.base));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("The Mac did not answer: {error}"))?;
        read(response).await
    }
}

/// A client that trusts exactly the certificate with this SHA-256, and talks only to a local host.
fn pinned_client(pin: &str) -> Result<reqwest::Client, String> {
    let tls = device_auth::pinned_tls(device_auth::decode_pin(pin)?)?;
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("TLS client: {error}"))
}

async fn read<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .map_err(|_| "The Mac answered with something this app cannot read.".into());
    }
    let error = response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
    Err(error.unwrap_or_else(|| format!("The Mac refused the request ({status}).")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr(h: &str, p: u32, f: &str) -> String {
        format!(r#"{{"k":"sjel-pair/1","h":"{h}","p":{p},"f":"{f}","i":"ch_1","c":"123456"}}"#)
    }

    #[test]
    fn reads_the_code_the_dashboard_draws() {
        let pin = "ab".repeat(32);
        let parsed = parse_pairing_qr(&qr("lars-mac", 8443, &pin)).unwrap();
        assert_eq!(parsed.host, "lars-mac");
        assert_eq!(parsed.port, 8443);
        assert_eq!(parsed.code, "123456");
    }

    #[test]
    fn join_target_takes_the_public_half_of_the_code() {
        let pin = "ab".repeat(32);
        let (base, got) = parse_join_target(&format!("lars-mac 8443 {pin}")).unwrap();
        assert_eq!(base, "https://lars-mac.local:8443");
        assert_eq!(got, pin);
        assert!(parse_join_target(&format!("evil.example 8443 {pin}")).is_err());
        assert!(parse_join_target("lars-mac 8443").is_err());
        assert_eq!(comparison_code(&pin), "ABAB ABAB ABAB ABAB");
    }

    #[test]
    fn refuses_foreign_and_damaged_codes() {
        let pin = "ab".repeat(32);
        assert!(parse_pairing_qr("https://example.com").is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 8443, &pin).replace("sjel-pair/1", "x")).is_err());
        // A dot would let the host leave `.local`.
        assert!(parse_pairing_qr(&qr("evil.example", 8443, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 0, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 70000, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 8443, &"AB".repeat(32))).is_err());
    }
}
