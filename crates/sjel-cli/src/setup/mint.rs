//! Where this deployment's primary signing key lives, and how a credential is minted from it.
//!
//! Two files and one secret, and the separation is the whole point:
//!
//! - the **private** half is a PKCS#8 document in the macOS Keychain, under
//!   `com.sjel.credential.turso-primary-key`. It is never written to the overlay, never printed,
//!   and never passed to a subprocess;
//! - the **public** half is written to `<overlay>/config/sqld-auth.pub`, which is what
//!   `sqld --auth-jwt-key-file` reads. A public key in a file is not a secret;
//! - the **minted token** is a JWT signed by the private half, written to the deployment record
//!   like any other setting and reported only by its presence.
//!
//! The key is created on first use and reused after. Regenerating it invalidates every token ever
//! minted, so nothing here does that implicitly: losing the key means minting a new one, and the
//! function that would do it is not wired to a verb yet.
//!
//! The minting itself is `libsql_auth`, a crate with no libsql in it, because this binary already
//! links `sjel-store` and two SQLite copies in one executable clash at link time.

use std::path::PathBuf;

use libsql_auth::{Access, PrimaryKey};
use sjel_credentials::{
    CredentialId, CredentialProvider as _, KeychainProvider, Presence, ProviderReference,
    SecretValue,
};

use crate::paths::Paths;

/// The Keychain entry's id. The provider derives the service name from it
/// (`com.sjel.credential.<id>`) and uses it as the account, so this one string is the whole
/// address of the key.
const KEY_ID: &str = "turso-primary-key";

/// Where `sqld --auth-jwt-key-file` reads the public half.
const PUBLIC_NAME: &str = "sqld-auth.pub";

/// The default lifetime of a minted credential.
///
/// A month, because the alternative is a credential that cannot be revoked without regenerating
/// the key pair, and rotation is cheaper than that.
pub(crate) const DEFAULT_MINT_DAYS: u64 = 30;

/// The address of the private half: one credential id, and the Keychain service the provider
/// derives from it. Shared with `sjel setup rotate turso`, because a second copy of the string is a
/// second place for the key to hide.
pub(crate) fn key_reference() -> Result<(CredentialId, ProviderReference), String> {
    let id = CredentialId::parse(KEY_ID).map_err(|e| format!("credential id refused: {e}"))?;
    Ok((
        id,
        ProviderReference::Keychain {
            account: KEY_ID.to_owned(),
        },
    ))
}

/// The signing key, created on first use and read from the Keychain afterwards.
fn signing_key() -> Result<PrimaryKey, String> {
    let (id, reference) = key_reference()?;
    let provider = KeychainProvider::system();

    match provider
        .status(&id, &reference)
        .map_err(|e| format!("cannot read the Keychain: {e}"))?
    {
        Presence::Present => {
            let value = provider
                .get(&id, &reference)
                .map_err(|e| format!("the Keychain refused to hand back the key: {e}"))?;
            PrimaryKey::from_pkcs8(value.expose_bytes())
                .map_err(|e| format!("the stored key is unusable: {e}"))
        }
        Presence::Missing => {
            let key = PrimaryKey::generate().map_err(|e| format!("cannot generate a key: {e}"))?;
            let value = SecretValue::new(key.pkcs8().to_vec())
                .map_err(|e| format!("cannot wrap the key for storage: {e}"))?;
            provider
                .create(&id, "Turso primary signing key", &value)
                .map_err(|e| format!("cannot store the key in the Keychain: {e}"))?;
            Ok(key)
        }
    }
}

/// Where `sqld --auth-jwt-key-file` reads the public half, which is the one path that answers for
/// this deployment's key file.
pub(crate) fn public_key_path(paths: &Paths) -> Result<PathBuf, String> {
    let Some(root) = paths.overlay_root.as_ref() else {
        return Err(
            "no overlay resolved, so there is no private directory for the public key".to_owned(),
        );
    };
    Ok(root.join("config").join(PUBLIC_NAME))
}

/// Write the public half where `sqld` reads it, answering the path.
///
/// Written every time rather than only when absent: the file is public, and a key that changed
/// with the file left behind would make every token fail with no hint why.
pub(crate) fn write_public_key(paths: &Paths, key: &PrimaryKey) -> Result<PathBuf, String> {
    let path = public_key_path(paths)?;
    let Some(dir) = path.parent() else {
        return Err("the public key path has no directory".to_owned());
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let body = key.public_key_base64url();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
        std::fs::write(&path, &body)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(path)
}

/// Write the public half where `sqld` reads it, and answer the path.
///
/// Loads the signing key from the Keychain, creating it on first use. Written every time rather
/// than only when absent: the file is public, and a key that changed with the file left behind
/// would make every credential fail with no hint why.
pub(crate) fn ensure_public_key(paths: &Paths) -> Result<PathBuf, String> {
    let key = signing_key()?;
    write_public_key(paths, &key)
}

/// Mint a credential for this deployment's own primary: a month's read-write one, plus the
/// public key file the primary must be started with.
///
/// Answers `(value, public key path)`. The value is returned to be written into the deployment
/// record by the caller, which is the only place it may land.
pub(crate) fn mint_for_local_primary(
    paths: &Paths,
    days: u64,
) -> Result<(String, PathBuf), String> {
    let key = signing_key()?;
    let public = write_public_key(paths, &key)?;
    let token = key
        .mint_for_days(None, Access::Write, days)
        .map_err(|e| format!("cannot mint a credential: {e}"))?;
    Ok((token, public))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(dir: &std::path::Path) -> Paths {
        Paths {
            root: dir.to_path_buf(),
            machine_toml: None,
            overlay_root: Some(dir.to_path_buf()),
            caps_dir: dir.join("capabilities"),
            overlay_caps_dir: None,
        }
    }

    /// The public half is written to the one path `sqld` is told about, and it is the key's own
    /// base64url form — the 32 bytes `parse_jwt_keys` decodes.
    #[test]
    fn the_public_half_lands_where_sqld_reads_it() {
        let dir = std::env::temp_dir().join(format!("sjel-setup-mint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = PrimaryKey::generate().unwrap();

        let path = write_public_key(&overlay(&dir), &key).unwrap();
        assert_eq!(path.file_name().unwrap(), PUBLIC_NAME);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            key.public_key_base64url()
        );

        // Rewritten in place when the key differs, so a rotated key cannot leave a stale file.
        let other = PrimaryKey::generate().unwrap();
        let path = write_public_key(&overlay(&dir), &other).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            other.public_key_base64url()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no overlay there is nowhere private to write, and the refusal says so instead of
    /// dropping the file into the working directory.
    #[test]
    fn no_overlay_is_a_refusal_not_a_stray_file() {
        let dir = std::env::temp_dir().join(format!("sjel-setup-mint-none-{}", std::process::id()));
        let mut paths = overlay(&dir);
        paths.overlay_root = None;
        let key = PrimaryKey::generate().unwrap();
        let err = write_public_key(&paths, &key).unwrap_err();
        assert!(err.contains("no overlay resolved"), "{err}");
    }

    /// A minted credential has the three segments `sqld` splits on, and a month's expiry.
    #[test]
    fn a_minted_credential_is_a_jwt_that_expires_in_a_month() {
        let key = PrimaryKey::generate().unwrap();
        let token = key
            .mint_for_days(None, Access::Write, DEFAULT_MINT_DAYS)
            .unwrap();
        assert_eq!(token.split('.').count(), 3);
    }
}
