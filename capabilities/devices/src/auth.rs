//! Signed requests from registered devices (`axon-device-auth/v1`).
//!
//! The signature covers the device id, request target, timestamp, nonce and body digest. The
//! registry stores each accepted nonce, so a valid signature cannot be replayed inside the
//! timestamp window.

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

pub const PROTOCOL_VERSION: &str = "axon-device-auth/v1";
// `x-sjel-*` since 2026-10-01 (ISA ISC-28). The gate renames a legacy `x-axon-*` header to these
// names before anything reads it (`sjel_server::normalize_legacy_headers`), so a paired phone
// that still sends the old names keeps working. The protocol string above is unchanged: it is
// part of what the device signs.
pub const DEVICE_ID_HEADER: &str = "x-sjel-device-id";
pub const TIMESTAMP_HEADER: &str = "x-sjel-timestamp";
pub const NONCE_HEADER: &str = "x-sjel-nonce";
pub const SIGNATURE_HEADER: &str = "x-sjel-signature";
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5 * 60;
pub const NONCE_TTL_SECONDS: i64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub device_id: String,
    pub timestamp: i64,
    pub nonce: String,
    pub signature: Vec<u8>,
}

impl SignedRequest {
    pub fn from_headers(headers: &HeaderMap) -> Result<Self, String> {
        let required = |name: &str| {
            if headers.get_all(name).iter().count() != 1 {
                return Err(format!("{name} header must occur exactly once"));
            }
            headers
                .get(name)
                .ok_or_else(|| format!("missing {name} header"))?
                .to_str()
                .map_err(|_| format!("{name} header is not valid ASCII"))
        };

        let device_id = required(DEVICE_ID_HEADER)?.to_string();
        if device_id.is_empty() || device_id.len() > 128 {
            return Err("device id has an invalid length".into());
        }
        let timestamp = required(TIMESTAMP_HEADER)?
            .parse::<i64>()
            .map_err(|_| "timestamp must be a Unix timestamp in seconds".to_string())?;
        let nonce = required(NONCE_HEADER)?.to_string();
        if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("nonce must be 16 bytes encoded as 32 hexadecimal characters".into());
        }
        let signature = decode_hex(required(SIGNATURE_HEADER)?)?;
        if signature.len() != 64 {
            return Err("signature must be 64 bytes encoded as 128 hexadecimal characters".into());
        }
        Ok(Self {
            device_id,
            timestamp,
            nonce,
            signature,
        })
    }
}

/// The exact bytes signed by a device. `path_and_query` is the path seen by the capability, not
/// the shell mount used to reach it (for example `/api/devices/me`, not `/devices/api/devices/me`).
pub fn signing_message(
    request: &SignedRequest,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Vec<u8> {
    let body_digest = Sha256::digest(body);
    let body_digest = hex(&body_digest);
    format!(
        "{PROTOCOL_VERSION}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        request.device_id,
        request.timestamp,
        request.nonce,
        method.to_ascii_uppercase(),
        path_and_query,
        body_digest,
    )
    .into_bytes()
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("value must be an even-length hexadecimal string".into());
    }
    let bytes = value.as_bytes();
    (0..bytes.len())
        .step_by(2)
        .map(|index| {
            let high = (bytes[index] as char)
                .to_digit(16)
                .ok_or_else(|| "invalid hexadecimal value".to_string())?;
            let low = (bytes[index + 1] as char)
                .to_digit(16)
                .ok_or_else(|| "invalid hexadecimal value".to_string())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_message_binds_method_target_and_body() {
        let request = SignedRequest {
            device_id: "dev_test".into(),
            timestamp: 1_700_000_000,
            nonce: "00".repeat(16),
            signature: vec![],
        };
        let message = signing_message(&request, "get", "/api/devices/me", b"{}");
        let text = String::from_utf8(message).unwrap();
        assert!(text.starts_with("axon-device-auth/v1\ndev_test\n1700000000\n"));
        assert!(text.contains("\nGET\n/api/devices/me\n"));
        assert!(text.ends_with('\n'));
        assert_ne!(
            signing_message(&request, "GET", "/api/devices/me", b"{}"),
            signing_message(&request, "GET", "/api/devices/me", b"[]")
        );
    }

    #[test]
    fn hex_parser_refuses_ambiguous_values() {
        assert_eq!(decode_hex("00aF").unwrap(), vec![0, 175]);
        assert!(decode_hex("0").is_err());
        assert!(decode_hex("0x00").is_err());
    }
}
