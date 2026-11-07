//! A paired device's way into Sjel (`crates/devices/README.md`, "Guided pairing").
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

pub use devices_api::{
    comparison_code, node_base, pairing_qr_payload, parse_join_target, parse_pairing_qr,
    DeviceRecord, JoinResponse as JoinRequest, PairingQr, PROXY_ROUTE_PAIRING_CLAIMS as CLAIM_PATH,
    PROXY_ROUTE_PAIRING_REQUESTS as JOIN_PATH, QR_KIND,
};

const TIMEOUT: Duration = Duration::from_secs(15);

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
    /// The listener: `https://<name>.local:<port>` on the same Wi-Fi, or
    /// `https://<address>:<port>` for a dotted name or IP address such as the tailnet's
    /// ([`node_base`]).
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
    let base = node_base(&qr.host, qr.port);
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
    stream_client: reqwest::Client,
    sign: Signer,
}

impl DeviceClient {
    pub fn new(node: Node, sign: Signer) -> Result<Self, String> {
        let client = pinned_client(&node.pin)?;
        let stream_client = pinned_stream_client(&node.pin)?;
        Ok(Self {
            node,
            client,
            stream_client,
            sign,
        })
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    /// A signed GET of `shell_path`, for example `/api/sjel-status/health`.
    pub async fn get<T: DeserializeOwned>(&self, shell_path: &str) -> Result<T, String> {
        let (status, body) = self.signed(Method::GET, shell_path, &[]).await?;
        decode(status, &body)
    }

    /// A signed GET for a non-JSON capability asset, such as a USDZ archive.
    pub async fn get_bytes(&self, shell_path: &str) -> Result<Vec<u8>, String> {
        let (status, body) = self.signed(Method::GET, shell_path, &[]).await?;
        accepted(status, body)
    }

    /// A signed request whose answer has no body to read, such as `204 No Content`.
    pub async fn send(&self, method: Method, shell_path: &str, body: &[u8]) -> Result<(), String> {
        let (status, body) = self.signed(method, shell_path, body).await?;
        accepted(status, body).map(|_| ())
    }

    /// A signed event stream from the paired Mac. Its total body has no timeout: callers stop it
    /// when the run's terminal event arrives or when they detach from the view.
    pub async fn streaming_get(&self, shell_path: &str) -> Result<reqwest::Response, String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "the clock is before 1970".to_string())?
            .as_secs() as i64;
        let sign = self.sign.clone();
        let headers = device_auth::signed_headers(
            &self.node.device_id,
            timestamp,
            Method::GET.as_str(),
            shell_path,
            &[],
            |message| sign(message),
        )?;
        let mut request = self
            .stream_client
            .get(format!("{}{shell_path}", self.node.base));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("The Mac did not answer: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("The Mac refused the stream ({status}): {body}"));
        }
        Ok(response)
    }

    /// A signed POST to `shell_path` with `body`.
    pub async fn post<T: DeserializeOwned>(
        &self,
        shell_path: &str,
        body: &[u8],
    ) -> Result<T, String> {
        let (status, body) = self.signed(Method::POST, shell_path, body).await?;
        decode(status, &body)
    }

    /// A signed PUT to `shell_path` with `body`.
    pub async fn put<T: DeserializeOwned>(
        &self,
        shell_path: &str,
        body: &[u8],
    ) -> Result<T, String> {
        let (status, body) = self.signed(Method::PUT, shell_path, body).await?;
        decode(status, &body)
    }

    /// One signed request, answered with the status and body as the Mac sent them.
    ///
    /// The status is returned rather than turned into an error because a caller can read what a
    /// refusal carries: a stale save answers `409` with the content that is there now, and a sync
    /// replays the edit over it (`core/src/sync.rs`).
    pub async fn signed(
        &self,
        method: Method,
        shell_path: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "the clock is before 1970".to_string())?
            .as_secs() as i64;
        let sign = self.sign.clone();
        let headers = device_auth::signed_headers(
            &self.node.device_id,
            timestamp,
            method.as_str(),
            shell_path,
            body,
            |m| sign(m),
        )?;
        let mut request = self
            .client
            .request(method, format!("{}{shell_path}", self.node.base));
        if !body.is_empty() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = if body.is_empty() {
            request
        } else {
            request.body(body.to_vec())
        }
        .send()
        .await
        .map_err(|error| format!("The Mac did not answer: {error}"))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("The Mac's answer stopped part way: {error}"))?
            .to_vec();
        Ok((status, body))
    }
}

/// A client that trusts exactly the certificate with this SHA-256, and talks only to a local host.
fn pinned_client(pin: &str) -> Result<reqwest::Client, String> {
    pinned_client_with_timeout(pin, Some(TIMEOUT))
}

fn pinned_stream_client(pin: &str) -> Result<reqwest::Client, String> {
    pinned_client_with_timeout(pin, None)
}

fn pinned_client_with_timeout(
    pin: &str,
    timeout: Option<Duration>,
) -> Result<reqwest::Client, String> {
    let tls = device_auth::pinned_tls(device_auth::decode_pin(pin)?)?;
    let builder = reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none());
    let builder = match timeout {
        Some(timeout) => builder.timeout(timeout),
        None => builder,
    };
    builder
        .build()
        .map_err(|error| format!("TLS client: {error}"))
}

/// Reads one unsigned answer: the pairing requests, before there is a device key to sign with.
async fn read<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("The Mac's answer stopped part way: {error}"))?;
    decode(status, &body)
}

fn accepted(status: u16, body: Vec<u8>) -> Result<Vec<u8>, String> {
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let error = crate::shell::error_text(&body);
    Err(error.unwrap_or_else(|| format!("The Mac refused the request ({status}).")))
}

fn decode<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T, String> {
    if (200..300).contains(&status) {
        return serde_json::from_slice(body)
            .map_err(|_| "The Mac answered with something this app cannot read.".into());
    }
    let error = crate::shell::error_text(body);
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
    fn what_the_mac_writes_is_what_the_phone_reads() {
        let written = pairing_qr_payload(&PairingQr {
            host: "lars-mac".into(),
            port: 8443,
            fingerprint: "ab".repeat(32),
            challenge_id: "pair_1234".into(),
            code: "482913abcd".into(),
        });
        let parsed = parse_pairing_qr(&written).expect("the Mac's own code must parse");
        assert_eq!(parsed.host, "lars-mac");
        assert_eq!(parsed.port, 8443);
        assert_eq!(parsed.challenge_id, "pair_1234");
        assert_eq!(parsed.code, "482913abcd");
    }

    #[test]
    fn join_target_takes_the_public_half_of_the_code() {
        let pin = "ab".repeat(32);
        let (base, got) = parse_join_target(&format!("lars-mac 8443 {pin}")).unwrap();
        assert_eq!(base, "https://lars-mac.local:8443");
        assert_eq!(got, pin);
        let (base, _) = parse_join_target(&format!("100.116.104.41 8443 {pin}")).unwrap();
        assert_eq!(base, "https://100.116.104.41:8443");
        assert!(parse_join_target(&format!("http://x 8443 {pin}")).is_err());
        assert!(parse_join_target("lars-mac 8443").is_err());
        assert_eq!(comparison_code(&pin), "ABAB ABAB ABAB ABAB");
    }

    #[test]
    fn refuses_foreign_and_damaged_codes() {
        let pin = "ab".repeat(32);
        assert!(parse_pairing_qr("https://example.com").is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 8443, &pin).replace("sjel-pair/1", "x")).is_err());
        // A host is a name or an address, never part of a URL.
        for host in [
            "evil/example",
            "http://x",
            "a b",
            "",
            ".lars",
            "lars..mac",
            "lars:9",
        ] {
            assert!(parse_pairing_qr(&qr(host, 8443, &pin)).is_err(), "{host}");
        }
        assert!(parse_pairing_qr(&qr(&"a".repeat(64), 8443, &pin)).is_err());
        assert!(parse_pairing_qr(&qr(&["a"; 128].join("."), 8443, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 0, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 70000, &pin)).is_err());
        assert!(parse_pairing_qr(&qr("lars-mac", 8443, &"AB".repeat(32))).is_err());
    }

    #[test]
    fn a_dotted_host_is_used_as_given_and_a_bare_name_gets_local() {
        let pin = "ab".repeat(32);
        for host in ["100.116.104.41", "lars-mac.example.ts.net"] {
            let parsed = parse_pairing_qr(&qr(host, 8443, &pin)).expect(host);
            assert_eq!(
                node_base(&parsed.host, parsed.port),
                format!("https://{host}:8443")
            );
        }
        assert_eq!(
            node_base("Lars-Mac-2", 8443),
            "https://Lars-Mac-2.local:8443"
        );
    }

    #[test]
    fn an_answer_with_no_body_is_accepted_and_a_refusal_keeps_its_reason() {
        // `204 No Content` is what the terminal's input, resize and DELETE routes answer.
        assert_eq!(accepted(204, Vec::new()), Ok(Vec::new()));
        assert!(decode::<serde_json::Value>(204, &[]).is_err());
        assert_eq!(
            accepted(404, br#"{"error":"no such terminal"}"#.to_vec()),
            Err("no such terminal".to_string())
        );
    }
}
