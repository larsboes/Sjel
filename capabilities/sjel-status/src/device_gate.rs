//! The shell's device verifier: a paired device's signature admits a request (PRD Q119).
//!
//! The protocol and the registry belong to `capabilities/devices`; this file only adapts
//! `DevicesStore::authenticate_scoped` to `sjel_server::DeviceVerifier`. The nonce is consumed in
//! the `shell` scope, so a request the shell admits to `/devices/api/devices/me` is not refused
//! there as a replay.

use axum::http::HeaderMap;
use devices::auth::SignedRequest;
use devices::store::DevicesStore;
use sjel_server::DeviceVerifier;

const NONCE_SCOPE: &str = "shell";

pub(crate) struct RegistryVerifier {
    store: DevicesStore,
}

impl RegistryVerifier {
    /// The registry in the deployment's shared database, or `None` when it cannot be opened.
    /// Without it the shell admits on the tailnet identity and the token only, as before.
    pub(crate) fn open() -> Option<Self> {
        match DevicesStore::open(&sjel_config::database_path()) {
            Ok(store) => Some(Self { store }),
            Err(error) => {
                eprintln!(
                    "[sjel-status] device registry unavailable, no device-key admission: {error}"
                );
                None
            }
        }
    }
}

impl DeviceVerifier for RegistryVerifier {
    fn verify(
        &self,
        method: &str,
        signed_path: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<String, String> {
        let request = SignedRequest::from_headers(headers)?;
        self.store
            .authenticate_scoped(NONCE_SCOPE, &request, method, signed_path, body)
            .map(|device| device.id)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::auth::{self, SignedRequest};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn signed_headers(
        key: &Ed25519KeyPair,
        device_id: &str,
        nonce: &str,
        path: &str,
        body: &[u8],
    ) -> HeaderMap {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut request = SignedRequest {
            device_id: device_id.into(),
            timestamp,
            nonce: nonce.into(),
            signature: Vec::new(),
        };
        request.signature = key
            .sign(&auth::signing_message(&request, "POST", path, body))
            .as_ref()
            .to_vec();
        let mut headers = HeaderMap::new();
        headers.insert(auth::DEVICE_ID_HEADER, device_id.parse().unwrap());
        headers.insert(
            auth::TIMESTAMP_HEADER,
            timestamp.to_string().parse().unwrap(),
        );
        headers.insert(auth::NONCE_HEADER, nonce.parse().unwrap());
        headers.insert(
            auth::SIGNATURE_HEADER,
            auth::hex(&request.signature).parse().unwrap(),
        );
        headers
    }

    /// The real registry and a real key: a paired device is admitted once, a replay and a
    /// changed body are refused. The shell's signed path is `sjel_server::device_signed_path`.
    #[test]
    fn a_paired_device_is_admitted_once_on_its_signature() {
        let dir =
            std::env::temp_dir().join(format!("sjel-status-device-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = DevicesStore::open(&dir.join("axon.db")).unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[5u8; 32]).unwrap();
        let challenge = store.create_challenge().unwrap();
        let device = store
            .claim(
                &challenge.challenge_id,
                &challenge.code,
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                auth::hex(key.public_key().as_ref()),
            )
            .unwrap();
        let verifier = RegistryVerifier { store };
        let path = sjel_server::device_signed_path("/interior/api/items?room=k");
        let headers = signed_headers(&key, &device.id, &"0a".repeat(16), path, b"lamp");

        assert_eq!(
            verifier.verify("POST", path, &headers, b"lamp"),
            Ok(device.id.clone())
        );
        assert!(
            verifier.verify("POST", path, &headers, b"lamp").is_err(),
            "replay"
        );
        let other = signed_headers(&key, &device.id, &"0b".repeat(16), path, b"lamp");
        assert!(
            verifier.verify("POST", path, &other, b"sofa").is_err(),
            "changed body"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
