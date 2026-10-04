//! Signed browser sessions: one credential the shell mints and every capability verifies.
//!
//! ## Why the session is an identity here and not shell state
//!
//! A browser at `127.0.0.1` can carry neither the deployment token nor a tailnet identity. The
//! shell (sjel-status) mints a session for it after the menu-bar app trades the Keychain token
//! for a single-use ticket (ISA ISC-45). That session was shell-local: a table in the shared
//! store, read by one `SessionVerifier`. So a capability serving its own panel — soundscape,
//! whose browser loads `:8088` directly and carries no token — answered `401` to the panel and
//! to every request the panel made, because nothing there could verify the cookie.
//!
//! The fix is to make the session verifiable the way the other two identities are: from the
//! request and a key already at startup, with no I/O and no cross-capability table read. The
//! token is `HMAC-SHA256(deployment token, payload)`, so there is no second secret, rotating the
//! deployment token revokes every session at once, and `InboundAuth::resolve` installs the
//! verifier for every capability — one gate, one comparison, as the crate's first paragraph
//! requires.
//!
//! ## The trade, stated
//!
//! Any holder of the deployment token can mint a session. It already holds the master
//! credential, so this is not a new privilege — but it is why that token stays out of the
//! browser and out of logs. The cost of statelessness is that a single session cannot be revoked
//! on its own: `logout` clears the cookie, and an already-stolen copy stays valid until it
//! expires. Rotating the deployment token is the revocation that exists.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;

use super::auth::SessionVerifier;

type HmacSha256 = Hmac<Sha256>;

/// How long a session lives without use. The principal's ruling, 2026-10-01: 30 days, renewed
/// while it is used.
pub const SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;
/// A used session is renewed at most this often, so a page load is not a cookie write per request.
const RENEW_AFTER_SECONDS: i64 = 60 * 60;
/// The token's format tag, so a future change can be told from this one.
const PREFIX: &str = "v1";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy)]
struct Claims {
    iat: i64,
}

/// The signed-session verifier and minter, keyed by the deployment token.
pub struct SignedSessions {
    key: Vec<u8>,
}

impl SignedSessions {
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        Self { key: key.into() }
    }

    /// The deployment's own token as the signing key, so no second secret exists.
    pub fn from_deployment() -> Option<Self> {
        super::auth::deployment_token().map(|token| Self::new(token.into_bytes()))
    }

    /// A new session, expiring `SESSION_TTL_SECONDS` from now.
    pub fn mint(&self) -> Result<String, String> {
        self.mint_at(now())
    }

    fn mint_at(&self, at: i64) -> Result<String, String> {
        let payload = serde_json::json!({
            "sub": "operator",
            "iat": at,
            "exp": at + SESSION_TTL_SECONDS,
        })
        .to_string();
        let signed = format!("{PREFIX}.{}", hex_encode(payload.as_bytes()));
        let mac = self.mac(signed.as_bytes())?;
        Ok(format!("{signed}.{}", hex_encode(&mac)))
    }

    fn mac(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut mac = HmacSha256::new_from_slice(&self.key).map_err(|e| e.to_string())?;
        mac.update(data);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    /// The payload of a token that is well-formed, correctly signed and unexpired.
    fn claims(&self, token: &str) -> Option<Claims> {
        let mut parts = token.split('.');
        let (Some(prefix), Some(payload_hex), Some(mac_hex), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        if prefix != PREFIX {
            return None;
        }
        let signed = format!("{PREFIX}.{payload_hex}");
        let given = hex_decode(mac_hex)?;
        // `verify_slice` is the constant-time comparison; recomputing over the same bytes is how
        // a tampered payload or MAC is refused rather than parsed.
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(signed.as_bytes());
        mac.verify_slice(&given).ok()?;

        let payload = hex_decode(payload_hex)?;
        let value: serde_json::Value = serde_json::from_slice(&payload).ok()?;
        let iat = value.get("iat")?.as_i64()?;
        let exp = value.get("exp")?.as_i64()?;
        if now() >= exp {
            return None;
        }
        Some(Claims { iat })
    }
}

impl SessionVerifier for SignedSessions {
    fn verify(&self, session: &str) -> bool {
        self.claims(session).is_some()
    }

    /// Slide the expiry of a session that is being used, without a store: the shell and every
    /// capability hold the same key, so any of them can re-issue.
    fn renew(&self, session: &str) -> Option<String> {
        let claims = self.claims(session)?;
        (now() - claims.iat >= RENEW_AFTER_SECONDS)
            .then(|| self.mint().ok())
            .flatten()
    }
}

/// The `Set-Cookie` value for a session. `HttpOnly` and `SameSite=Strict`, so a page on another
/// site can neither read it nor make the browser send it. Not `Secure`, because the listener is
/// plain HTTP on loopback.
pub fn session_cookie_header(value: &str, max_age: i64) -> String {
    format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}",
        super::auth::SESSION_COOKIE
    )
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions() -> SignedSessions {
        SignedSessions::new(b"deployment-token".to_vec())
    }

    #[test]
    fn a_minted_session_verifies_and_carries_its_issue_time() {
        let s = sessions();
        let token = s.mint().unwrap();
        assert!(s.verify(&token));
        assert!(token.starts_with("v1."));
        assert_eq!(s.claims(&token).unwrap().iat, now());
    }

    #[test]
    fn another_key_refuses_it() {
        let token = sessions().mint().unwrap();
        assert!(!SignedSessions::new(b"other".to_vec()).verify(&token));
    }

    #[test]
    fn a_tampered_payload_or_mac_is_refused() {
        let s = sessions();
        let token = s.mint().unwrap();
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();

        // A rewritten payload with the original MAC.
        parts[1] = hex_encode(br#"{"sub":"operator","iat":1,"exp":9999999999}"#);
        assert!(!s.verify(&parts.join(".")));

        // A rewritten MAC.
        parts[1] = hex_encode(br#"{"sub":"operator","iat":1,"exp":9999999999}"#);
        parts[2] = "00".repeat(32);
        assert!(!s.verify(&parts.join(".")));

        // Not a token at all.
        assert!(!s.verify(""));
        assert!(!s.verify("v1.deadbeef"));
        assert!(!s.verify("v2.00.00"));
    }

    #[test]
    fn an_expired_session_is_refused() {
        let s = sessions();
        let old = s.mint_at(now() - SESSION_TTL_SECONDS - 10).unwrap();
        assert!(!s.verify(&old));
    }

    #[test]
    fn a_used_session_is_renewed_but_not_on_every_request() {
        let s = sessions();
        let fresh = s.mint().unwrap();
        assert!(
            s.renew(&fresh).is_none(),
            "a fresh session is not re-issued"
        );

        let old = s.mint_at(now() - RENEW_AFTER_SECONDS - 10).unwrap();
        let renewed = s.renew(&old).expect("an hour-old session slides");
        assert!(s.verify(&renewed));
        assert_eq!(s.claims(&renewed).unwrap().iat, now());
    }

    #[test]
    fn the_cookie_is_http_only_and_same_site_strict() {
        let header = session_cookie_header("abc", SESSION_TTL_SECONDS);
        assert!(header.starts_with("sjel_session=abc;"), "{header}");
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Strict"));
        assert!(header.contains(&format!("Max-Age={SESSION_TTL_SECONDS}")));
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        assert_eq!(hex_decode(&hex_encode(b"hello")).unwrap(), b"hello");
        assert!(hex_decode("abc").is_none(), "odd length");
        assert!(hex_decode("zz").is_none(), "not hex");
    }
}
