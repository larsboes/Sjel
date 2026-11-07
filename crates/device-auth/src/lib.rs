//! The `sjel-device-auth/v1` protocol between a paired device and the node, written once.
//!
//! A paired device signs every request with its Ed25519 key. The signature covers the bytes from
//! [`signing_message`]. The devices capability rebuilds the same bytes to verify
//! (`crates/devices/src/auth.rs`); the GPUI app builds them to sign
//! (`apps/gpui/core/src/device.rs`). The key itself never enters this crate: a caller passes a signing function.

use sha2::{Digest, Sha256};

#[cfg(feature = "pinned-tls")]
mod pinned;
#[cfg(feature = "pinned-tls")]
pub use pinned::{decode_pin, pinned_tls, PinnedCertificate};

pub const PROTOCOL_VERSION: &str = "sjel-device-auth/v1";

/// The pre-rename protocol string, still accepted: it is part of the bytes a device signs, so a
/// phone paired before the rename cannot be re-interpreted, only re-paired. Dropped when no
/// paired device predates the rename.
pub const LEGACY_PROTOCOL_VERSION: &str = "axon-device-auth/v1";

/// Every protocol string a verifier must accept, current first.
pub const ACCEPTED_PROTOCOL_VERSIONS: [&str; 2] = [PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION];

/// The header names a device sends. `x-axon-*` is still accepted by
/// `sjel_server::normalize_legacy_headers`, so a server that has not restarted since keeps
/// admitting this build.
pub const SENT_HEADERS: [&str; 4] = [
    "x-sjel-device-id",
    "x-sjel-timestamp",
    "x-sjel-nonce",
    "x-sjel-signature",
];

/// The exact bytes a device signs. `path_and_query` is the path the capability sees, not the
/// shell mount used to reach it (`/api/devices/me`, not `/devices/api/devices/me`): see
/// [`signed_path`].
pub fn signing_message(
    device_id: &str,
    timestamp: i64,
    nonce: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Vec<u8> {
    signing_message_for(
        PROTOCOL_VERSION,
        device_id,
        timestamp,
        nonce,
        method,
        path_and_query,
        body,
    )
}

/// The signed bytes for one named protocol version, which is what lets a verifier check a
/// signature made under a legacy string without knowing which one the device used.
pub fn signing_message_for(
    version: &str,
    device_id: &str,
    timestamp: i64,
    nonce: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Vec<u8> {
    format!(
        "{version}\n{device_id}\n{timestamp}\n{nonce}\n{}\n{path_and_query}\n{}\n",
        method.to_ascii_uppercase(),
        hex(&Sha256::digest(body)),
    )
    .into_bytes()
}

/// The four signed headers for one request, in [`SENT_HEADERS`] order. `sign` signs the
/// message with the device key and returns the 64-byte signature.
pub fn signed_headers(
    device_id: &str,
    timestamp: i64,
    method: &str,
    shell_path: &str,
    body: &[u8],
    sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<[(&'static str, String); 4], String> {
    let nonce = new_nonce()?;
    let message = signing_message(
        device_id,
        timestamp,
        &nonce,
        method,
        signed_path(shell_path),
        body,
    );
    let signature = hex(&sign(&message)?);
    let [id, time, once, sig] = SENT_HEADERS;
    Ok([
        (id, device_id.to_owned()),
        (time, timestamp.to_string()),
        (once, nonce),
        (sig, signature),
    ])
}

/// 16 random bytes as 32 hex characters, the nonce shape the registry accepts.
pub fn new_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("secure random source: {error}"))?;
    Ok(hex(&bytes))
}

/// The path a device signs for a request sent to `shell_path`. The shell mounts capabilities at
/// `/<name>/...` and strips that first segment before forwarding, so the device signs the
/// upstream target. `/api/...` is the shell's own and passes unchanged.
pub fn signed_path(shell_path: &str) -> &str {
    if shell_path == "/api" || shell_path.starts_with("/api/") {
        return shell_path;
    }
    shell_path[1..]
        .find('/')
        .map_or("/", |index| &shell_path[index + 1..])
}

/// The requests a device sends before it is registered: claim a code, ask to join, and read one
/// join request's state. Mirrors `unsigned_pairing_route` in `libs/sjel-server/src/auth.rs`,
/// which admits exactly these on the LAN listener.
pub fn unsigned_pairing_request(method: &str, shell_path: &str) -> bool {
    let path = shell_path.split('?').next().unwrap_or(shell_path);
    match method {
        "POST" => path == "/devices/api/pairing/claims" || path == "/devices/api/pairing/requests",
        "GET" => path
            .strip_prefix("/devices/api/pairing/requests/")
            .is_some_and(|id| !id.is_empty() && !id.contains('/')),
        _ => false,
    }
}

/// Whether `host` may serve the LAN listener: `<name>.local` or a private address. The phone
/// talks to nothing else without TLS that a public CA vouches for.
pub fn is_local_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let local_name = host
        .strip_suffix(".local")
        .is_some_and(|name| !name.is_empty() && !name.contains('.'));
    let private = match host.trim_start_matches('[').trim_end_matches(']').parse() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    };
    local_name || private
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("value must be an even-length hexadecimal string".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| "invalid hexadecimal value".to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_message_binds_method_target_and_body() {
        let message = signing_message(
            "dev_test",
            1_700_000_000,
            &"00".repeat(16),
            "get",
            "/api/devices/me",
            b"{}",
        );
        let text = String::from_utf8(message).unwrap();
        assert!(text.starts_with("sjel-device-auth/v1\ndev_test\n1700000000\n"));
        assert!(text.contains("\nGET\n/api/devices/me\n"));
        assert!(text.ends_with('\n'));
        let nonce = "00".repeat(16);
        assert_ne!(
            signing_message("d", 1, &nonce, "GET", "/api/x", b"{}"),
            signing_message("d", 1, &nonce, "GET", "/api/x", b"[]")
        );
    }

    #[test]
    fn signed_headers_sign_the_upstream_path() {
        let mut signed = Vec::new();
        let headers = signed_headers(
            "dev_1",
            1_700_000_000,
            "get",
            "/devices/api/devices/me",
            b"",
            |message| {
                signed = message.to_vec();
                Ok(vec![7; 64])
            },
        )
        .unwrap();
        let text = String::from_utf8(signed).unwrap();
        assert!(text.contains("\nGET\n/api/devices/me\n"));
        assert_eq!(headers[0], ("x-sjel-device-id", "dev_1".to_owned()));
        assert_eq!(headers[2].1.len(), 32);
        assert_eq!(headers[3].1, "07".repeat(64));
    }

    #[test]
    fn a_legacy_version_signs_different_bytes() {
        let current = signing_message("d", 1, &"00".repeat(16), "GET", "/api/x", b"");
        let legacy = signing_message_for(
            LEGACY_PROTOCOL_VERSION,
            "d",
            1,
            &"00".repeat(16),
            "GET",
            "/api/x",
            b"",
        );
        assert_ne!(current, legacy);
        assert!(String::from_utf8(legacy)
            .unwrap()
            .starts_with("axon-device-auth/v1\n"));
    }

    #[test]
    fn signed_path_matches_the_upstream_capability_path() {
        assert_eq!(signed_path("/devices/api/devices/me"), "/api/devices/me");
        assert_eq!(
            signed_path("/interior/api/items/item-1?room=kitchen"),
            "/api/items/item-1?room=kitchen"
        );
        assert_eq!(
            signed_path("/api/suggest?q=Berlin"),
            "/api/suggest?q=Berlin"
        );
        assert_eq!(signed_path("/devices"), "/");
    }

    #[test]
    fn pairing_requests_go_unsigned_and_nothing_else_does() {
        assert!(unsigned_pairing_request(
            "POST",
            "/devices/api/pairing/claims"
        ));
        assert!(unsigned_pairing_request(
            "POST",
            "/devices/api/pairing/requests"
        ));
        assert!(unsigned_pairing_request(
            "GET",
            "/devices/api/pairing/requests/join_1"
        ));
        assert!(!unsigned_pairing_request(
            "GET",
            "/devices/api/pairing/requests"
        ));
        assert!(!unsigned_pairing_request(
            "POST",
            "/devices/api/pairing/requests/join_1/approve"
        ));
        assert!(!unsigned_pairing_request("GET", "/devices/api/devices/me"));
        assert!(!unsigned_pairing_request(
            "GET",
            "/sjel-status/api/sjel-status/health"
        ));
    }

    #[test]
    fn local_hosts_are_local_names_or_private_addresses() {
        for good in ["lars-mac.local", "192.168.1.20", "10.0.0.5", "[fd00::1]"] {
            assert!(is_local_host(good), "{good}");
        }
        for bad in ["sjel.example.com", "8.8.8.8", "a.b.local", "127.0.0.1"] {
            assert!(!is_local_host(bad), "{bad}");
        }
    }

    #[test]
    fn hex_round_trips_and_refuses_ambiguous_values() {
        assert_eq!(decode_hex("00aF").unwrap(), vec![0, 175]);
        assert_eq!(decode_hex(&hex(&[1, 254])).unwrap(), vec![1, 254]);
        assert!(decode_hex("0").is_err());
        assert!(decode_hex("0x00").is_err());
    }
}
