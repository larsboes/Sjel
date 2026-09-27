//! The local-network listener: a TLS port on every interface that admits paired devices only.
//!
//! PRD Q119 makes the device key the trust root, so a phone on the same Wi-Fi reaches Sjel with no
//! tailnet and no account. Two things replace what `tailscale serve` provided:
//!
//! - **Encryption and the server's identity.** A self-signed certificate made on first start and
//!   kept in the overlay. The phone pins its SHA-256 fingerprint when it pairs, so the certificate
//!   needs no CA and its host name does not matter.
//! - **Who may call.** The gate runs in [`crate::auth::LanPolicy::DevicesOnly`]: a request needs a
//!   valid device signature. The one exception is the pairing claim, which a device sends before
//!   it has a registered key; it is protected by the one-time code instead
//!   (`capabilities/devices`, ten minutes, 50 bits).
//!
//! Opt-in per deployment: `SJEL_LAN_PORT` (or its earlier name `SJEL_LAN_PORT`) in
//! `<overlay>/config/deployment.env`.

use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// The Bonjour service type the Mac advertises and the phone browses for.
pub const SERVICE_TYPE: &str = "_sjel._tcp";

const PORT_KEY: &str = "SJEL_LAN_PORT";
const CERT_FILE: &str = "lan-cert.der";
const KEY_FILE: &str = "lan-key.der";

/// The deployment's LAN port, or `None` when the listener is not enabled.
pub fn deployment_port() -> Option<u16> {
    let body = std::fs::read_to_string(sjel_config::overlay_config("deployment.env")?).ok()?;
    sjel_config::deployment_value(&body, PORT_KEY)?.parse().ok()
}

/// The listener's certificate and key, and the fingerprint a phone pins.
pub struct LanIdentity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    /// Lowercase hex SHA-256 of the DER certificate.
    pub fingerprint: String,
}

impl LanIdentity {
    /// Loads the identity from `dir`, or creates it there on first start.
    ///
    /// The key file is written `0600`. It is C3 by §6.1 and stays in the overlay; a new key means
    /// every paired phone must pin again, so it is made once and kept.
    pub fn load_or_create(dir: &Path, host_name: &str) -> Result<Self, String> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        if cert_path.exists() && key_path.exists() {
            let cert_der = std::fs::read(&cert_path)
                .map_err(|e| format!("read {}: {e}", cert_path.display()))?;
            let key_der = std::fs::read(&key_path)
                .map_err(|e| format!("read {}: {e}", key_path.display()))?;
            return Ok(Self::from_der(cert_der, key_der));
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let names = vec![format!("{host_name}.local"), "localhost".to_string()];
        let certified =
            rcgen::generate_simple_self_signed(names).map_err(|e| format!("certificate: {e}"))?;
        let cert_der = certified.cert.der().to_vec();
        let key_der = certified.signing_key.serialize_der();
        write_private(&key_path, &key_der)?;
        std::fs::write(&cert_path, &cert_der)
            .map_err(|e| format!("write {}: {e}", cert_path.display()))?;
        Ok(Self::from_der(cert_der, key_der))
    }

    fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let fingerprint = fingerprint(&cert_der);
        Self {
            cert_der,
            key_der,
            fingerprint,
        }
    }

    fn server_config(&self) -> Result<rustls::ServerConfig, String> {
        let cert = rustls::pki_types::CertificateDer::from(self.cert_der.clone());
        let key = rustls::pki_types::PrivateKeyDer::try_from(self.key_der.clone())
            .map_err(|e| format!("private key: {e}"))?;
        // The provider is named rather than taken from the process default: `builder()` panics
        // when a tree enables two providers, and whether it does depends on the binary.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("tls versions: {e}"))?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|e| format!("tls config: {e}"))?;
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(config)
    }
}

/// Lowercase hex SHA-256 of a DER certificate: what the phone compares on every connection.
pub fn fingerprint(cert_der: &[u8]) -> String {
    Sha256::digest(cert_der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Serves `router` over TLS on every interface, behind a devices-only gate. Never returns on
/// success; exits with one line on a configuration the listener must not run with.
pub async fn serve_lan(
    name: &str,
    port: u16,
    router: axum::Router,
    auth: crate::InboundAuth,
    identity: &LanIdentity,
) {
    if !auth.admits_devices() {
        eprintln!("{name}: refusing the LAN listener without a device verifier; it admits paired devices only");
        std::process::exit(1);
    }
    let config = match identity.server_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{name}: LAN listener: {error}");
            std::process::exit(1);
        }
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let app = crate::auth::authenticated(router, auth.lan_devices_only());
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("{name}: cannot bind LAN {addr}: {error}");
            std::process::exit(1);
        }
    };
    println!("{name} LAN listener on {addr} (TLS, paired devices only)");
    loop {
        let Ok((tcp, _peer)) = listener.accept().await else {
            continue;
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            // A failed handshake is a scanner or a phone that pinned another key: nothing to log.
            let Ok(tls) = acceptor.accept(tcp).await else {
                return;
            };
            let service = hyper_util::service::TowerToHyperService::new(app);
            let _ =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(hyper_util::rt::TokioIo::new(tls), service)
                    .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("axon-lan-identity-{}", std::process::id()));
        let first = LanIdentity::load_or_create(&dir, "test-mac").unwrap();
        let second = LanIdentity::load_or_create(&dir, "test-mac").unwrap();
        assert_eq!(
            first.fingerprint, second.fingerprint,
            "a new key would unpin every phone"
        );
        assert_eq!(first.fingerprint.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(first.server_config().is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct OnlyGood;
    impl crate::DeviceVerifier for OnlyGood {
        fn verify(
            &self,
            _method: &str,
            _signed_path: &str,
            headers: &axum::http::HeaderMap,
            _body: &[u8],
        ) -> Result<String, String> {
            match headers
                .get(crate::DEVICE_SIGNATURE_HEADER)
                .map(|v| v.as_bytes())
            {
                Some(b"good") => Ok("dev_phone".into()),
                _ => Err("signature is invalid".into()),
            }
        }
    }

    /// Over real TLS: an unsigned request is refused even though the deployment has no token,
    /// the pairing claim gets through unsigned, a signed request gets through, and a forged
    /// tailnet identity never reaches a handler.
    #[tokio::test]
    async fn the_lan_listener_admits_paired_devices_and_the_pairing_claim_only() {
        use axum::routing::{get, post};
        let dir = std::env::temp_dir().join(format!("axon-lan-serve-{}", std::process::id()));
        let identity = LanIdentity::load_or_create(&dir, "test-mac").unwrap();
        let router = axum::Router::new()
            .route(
                "/interior/api/items",
                get(|headers: axum::http::HeaderMap| async move {
                    headers.contains_key("tailscale-user-login").to_string()
                }),
            )
            .route(crate::PAIRING_CLAIM_PATH, post(|| async { "claimed" }));
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let auth = crate::InboundAuth::with_token(None).with_device_verifier(Arc::new(OnlyGood));
        tokio::spawn(async move { serve_lan("test", port, router, auth, &identity).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The test trusts the self-signed certificate by name only; the phone pins the key.
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let base = format!("https://127.0.0.1:{port}");
        let unsigned = client
            .get(format!("{base}/interior/api/items"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            unsigned.status(),
            401,
            "no token configured must not mean open on the LAN"
        );
        let claim = client
            .post(format!("{base}{}", crate::PAIRING_CLAIM_PATH))
            .send()
            .await
            .unwrap();
        assert_eq!(claim.text().await.unwrap(), "claimed");
        let signed = client
            .get(format!("{base}/interior/api/items"))
            .header(crate::DEVICE_SIGNATURE_HEADER, "good")
            .header("tailscale-user-login", "someone@example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(signed.status(), 200);
        assert_eq!(
            signed.text().await.unwrap(),
            "false",
            "the forged identity header was removed"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
