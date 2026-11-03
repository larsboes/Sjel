//! Minting the tokens a primary authenticates, and the key pair they are signed with.
//!
//! `sqld` refuses a client it cannot authenticate once `--auth-jwt-key-file` is set. That file
//! holds the **public** half of an Ed25519 key pair, base64url-encoded without padding; the
//! private half stays in the operating system's keychain and never appears in a tracked file, a
//! log line or an argument vector.
//!
//! The claims are `sqld`'s, not a choice made here. `libsql-server` decodes a JWT into
//! `Token { id, a, p, exp }` with `jsonwebtoken` pinned to `Algorithm::EdDSA`
//! (`libsql-server/src/auth/user_auth_strategies/jwt.rs`, tag `libsql-server-v0.24.32`): `a` is
//! `"ro"` or `"rw"`, `id` names a namespace and may be absent on a single-namespace server, `p`
//! carries fine-grained scopes and is unused here, and `exp` is unix seconds. An absent `a`
//! defaults to write, which is why this module never omits it.
//!
//! Measured against `sqld 0.24.32` on 2026-10-21: a replica carrying a token minted here pulls and
//! writes, and the same replica without one is refused.

use std::error::Error as StdError;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair as _};

/// What a fallible operation in this crate returns.
///
/// Deliberately the same shape `sjel-turso` uses: callers re-export these types beside the crate's
/// own, and a second error vocabulary would make `?` at the boundary need a conversion that says
/// nothing.
pub type Error = Box<dyn StdError + Send + Sync>;

/// The result of a fallible operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// What a minted token lets its holder do on the primary.
///
/// `sqld`'s own word for these is `Permission`: `"ro"` satisfies reads alone and `"rw"` satisfies
/// reads as well, so a capability that only reads should be given [`Access::Read`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Reads only (`"ro"`).
    Read,
    /// Reads and writes (`"rw"`).
    Write,
}

impl Access {
    /// The claim `sqld` parses.
    fn claim(self) -> &'static str {
        match self {
            Self::Read => "ro",
            Self::Write => "rw",
        }
    }
}

/// The signing half of a primary's JWT key pair, and the only place a token comes from.
///
/// Deliberately not `Clone`: one owner, and the PKCS#8 bytes leave only through [`Self::pkcs8`],
/// which is what the keychain stores.
pub struct PrimaryKey {
    pair: Ed25519KeyPair,
    /// The document the pair was parsed from, kept because `ring` signs from a parsed key and does
    /// not offer to serialize one back: the keychain needs these bytes.
    pkcs8: Vec<u8>,
}

impl std::fmt::Debug for PrimaryKey {
    /// Prints the public half only. A `Debug` that printed the private key would put it in a panic
    /// message and from there into a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrimaryKey")
            .field("public", &self.public_key_base64url())
            .finish_non_exhaustive()
    }
}

impl PrimaryKey {
    /// Generate a fresh pair from the operating system's randomness.
    pub fn generate() -> Result<Self> {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .map_err(|_| Error::from("the system random source refused to generate a key"))?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    /// Rebuild a key from the PKCS#8 bytes the keychain holds.
    pub fn from_pkcs8(der: &[u8]) -> Result<Self> {
        let pair = Ed25519KeyPair::from_pkcs8(der)
            .map_err(|_| Error::from("those bytes are not an Ed25519 PKCS#8 private key"))?;
        Ok(Self {
            pair,
            pkcs8: der.to_vec(),
        })
    }

    /// The bytes to store in the keychain: the PKCS#8 document holding the private key.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// What `sqld --auth-jwt-key-file` reads: the 32-byte public key, base64url without padding.
    pub fn public_key_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.pair.public_key().as_ref())
    }

    /// Mint a token that expires at `expires_at` (unix seconds).
    ///
    /// `namespace` is a single-namespace server's `id` claim; pass `None` for `sqld` started with
    /// `--db-path`, where there is nothing to scope the token to and `sqld` treats an absent `id`
    /// as "any namespace this server has".
    pub fn mint(&self, namespace: Option<&str>, access: Access, expires_at: u64) -> Result<String> {
        let header = serde_json::json!({"alg": "EdDSA", "typ": "JWT"});
        let mut claims = serde_json::json!({
            "a": access.claim(),
            "exp": expires_at,
        });
        if let Some(namespace) = namespace {
            claims["id"] = serde_json::Value::String(namespace.to_owned());
        }
        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature = self.pair.sign(signing_input.as_bytes());
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }

    /// Mint a token that expires `days` from now.
    ///
    /// An expiry is the point of the claim: a token that never expires cannot be revoked without
    /// regenerating the key pair. `sqld` accepts a token with no `exp` at all, which is why the
    /// choice is made here rather than left to the caller.
    pub fn mint_for_days(
        &self,
        namespace: Option<&str>,
        access: Access,
        days: u64,
    ) -> Result<String> {
        self.mint(namespace, access, unix_now().saturating_add(days * 86_400))
    }
}

/// The clock a token is stamped against.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token is three base64url segments, and the first says EdDSA — the only algorithm `sqld`
    /// will decode.
    #[test]
    fn a_token_is_three_segments_with_an_eddsa_header() {
        let key = PrimaryKey::generate().unwrap();
        let token = key.mint_for_days(None, Access::Write, 30).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "not a JWT: {token}");

        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["typ"], "JWT");
    }

    /// The claims carry what `sqld` reads: the access, an expiry, and the namespace only when one
    /// was asked for.
    #[test]
    fn the_claims_carry_the_access_an_expiry_and_an_optional_namespace() {
        let key = PrimaryKey::generate().unwrap();
        let token = key
            .mint(Some("default"), Access::Read, 1_700_000_000)
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(token.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["a"], "ro");
        assert_eq!(claims["exp"], 1_700_000_000);
        assert_eq!(claims["id"], "default");
        assert!(claims.get("p").is_none(), "fine-grained scopes are unused");

        let unscoped = key.mint(None, Access::Write, 1_700_000_001).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(unscoped.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["a"], "rw");
        assert!(
            claims.get("id").is_none(),
            "a single-namespace primary needs no id claim"
        );
    }

    /// The signature covers the first two segments and verifies with the public half — the half
    /// `sqld` is given.
    #[test]
    fn the_signature_verifies_with_the_public_half_that_sqld_is_given() {
        let key = PrimaryKey::generate().unwrap();
        let token = key.mint(None, Access::Write, 1_700_000_002).unwrap();
        let (signing_input, signature) = token.rsplit_once('.').unwrap();
        let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();

        let public = URL_SAFE_NO_PAD.decode(key.public_key_base64url()).unwrap();
        assert_eq!(public.len(), 32, "an Ed25519 public key is 32 bytes");
        let verifier = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &public);
        verifier
            .verify(signing_input.as_bytes(), &signature)
            .expect("the signature must verify with the published public key");

        // And one whose signing input was altered does not.
        let mut altered = signing_input.to_owned();
        let last = altered.pop().unwrap();
        altered.push(if last == 'A' { 'B' } else { 'A' });
        assert!(
            verifier.verify(altered.as_bytes(), &signature).is_err(),
            "an altered token must not verify"
        );
    }

    /// A key survives the round trip through the bytes the keychain stores, and keeps signing
    /// tokens the original public half verifies.
    #[test]
    fn a_key_rebuilt_from_its_keychain_bytes_still_signs() {
        let key = PrimaryKey::generate().unwrap();
        let restored = PrimaryKey::from_pkcs8(key.pkcs8()).unwrap();
        assert_eq!(restored.public_key_base64url(), key.public_key_base64url());
        assert_eq!(restored.pkcs8(), key.pkcs8());

        let token = restored.mint(None, Access::Write, 1_700_000_003).unwrap();
        let (signing_input, signature) = token.rsplit_once('.').unwrap();
        let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
        let public = URL_SAFE_NO_PAD.decode(key.public_key_base64url()).unwrap();
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public)
            .verify(signing_input.as_bytes(), &signature)
            .expect("a restored key signs tokens the original public key verifies");
    }

    /// `Debug` is printed in panics and logs, so it must not carry the private key.
    #[test]
    fn debug_prints_the_public_half_only() {
        let key = PrimaryKey::generate().unwrap();
        let printed = format!("{key:?}");
        assert!(printed.contains(&key.public_key_base64url()));
        let private = URL_SAFE_NO_PAD.encode(key.pkcs8());
        assert!(
            !printed.contains(&private),
            "Debug must not print the private key: {printed}"
        );
    }

    /// Garbage is refused with a message naming what was wrong, rather than silently minting a new
    /// key and leaving the caller with a token no primary accepts.
    #[test]
    fn bytes_that_are_not_a_key_are_refused() {
        let err = PrimaryKey::from_pkcs8(b"not a pkcs8 document").unwrap_err();
        assert!(err.to_string().contains("Ed25519 PKCS#8"), "{err}");
    }
}
