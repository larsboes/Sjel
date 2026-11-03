//! A TLS client that accepts exactly one certificate, identified by its SHA-256.
//!
//! The node's LAN listener has a self-signed certificate (`libs/sjel-server/src/lan.rs`), so no
//! chain or host name can be checked; the pin replaces both. The handshake signature is still
//! verified with the provider's own functions, which proves the peer holds the pinned key.

use std::sync::Arc;

use sha2::{Digest, Sha256};

/// Parses a 64-character hex SHA-256 certificate pin.
pub fn decode_pin(pin: &str) -> Result<[u8; 32], String> {
    let bytes = crate::decode_hex(pin.trim())
        .map_err(|_| "certificate pin must be 64 hexadecimal characters".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "certificate pin must be 64 hexadecimal characters".into())
}

#[derive(Debug)]
pub struct PinnedCertificate {
    pin: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedCertificate {
    pub fn new(pin: [u8; 32], provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        Self { pin, provider }
    }
}

impl rustls::client::danger::ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if Sha256::digest(end_entity.as_ref()).as_slice() == self.pin {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the certificate is not the one this device paired with".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The client config for the LAN listener: TLS that trusts exactly the pinned certificate.
pub fn pinned_tls(pin: [u8; 32]) -> Result<rustls::ClientConfig, String> {
    // Named, not the process default: `builder()` panics when a tree enables two providers.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("tls versions: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCertificate::new(pin, provider)))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pin is the whole check: the certificate with that SHA-256 passes, any other fails,
    /// whatever name it claims.
    #[test]
    fn the_pinned_verifier_accepts_one_certificate_only() {
        use rustls::client::danger::ServerCertVerifier;
        let cert = rustls::pki_types::CertificateDer::from(b"the paired certificate".to_vec());
        let other = rustls::pki_types::CertificateDer::from(b"another certificate".to_vec());
        let pin: [u8; 32] = Sha256::digest(cert.as_ref()).into();
        let verifier =
            PinnedCertificate::new(pin, Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
        let name = rustls::pki_types::ServerName::try_from("anything.local").unwrap();
        let now = rustls::pki_types::UnixTime::now();
        assert!(verifier
            .verify_server_cert(&cert, &[], &name, &[], now)
            .is_ok());
        assert!(verifier
            .verify_server_cert(&other, &[], &name, &[], now)
            .is_err());
        assert_eq!(decode_pin(&crate::hex(&pin)).unwrap(), pin);
        assert!(decode_pin("abc").is_err());
        assert!(pinned_tls(pin).is_ok());
    }
}
