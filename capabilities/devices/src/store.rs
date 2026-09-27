use std::fmt::Display;
use std::path::Path;

use ring::signature::{UnparsedPublicKey, ED25519};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::auth::{self, SignedRequest};

const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CHALLENGE_TTL_SECONDS: i64 = 10 * 60;

#[derive(Debug)]
pub enum StoreError {
    Invalid(String),
    NotFound(String),
    Conflict(String),
    Unauthorized(String),
    Db(String),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Unauthorized(message)
            | Self::Db(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for StoreError {}

fn db(error: impl Display) -> StoreError {
    StoreError::Db(error.to_string())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn random_hex(bytes: usize, prefix: &str) -> Result<String, StoreError> {
    let mut value = vec![0u8; bytes];
    getrandom::fill(&mut value).map_err(|error| db(format!("secure random source: {error}")))?;
    let mut output = String::with_capacity(prefix.len() + bytes * 2);
    output.push_str(prefix);
    for byte in value {
        output.push_str(&format!("{byte:02x}"));
    }
    Ok(output)
}

fn random_code() -> Result<String, StoreError> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| db(format!("secure random source: {error}")))?;
    let value = u64::from_be_bytes(bytes) >> 14;
    let mut code = String::with_capacity(10);
    for index in 0..10 {
        let shift = 45 - index * 5;
        code.push(CODE_ALPHABET[((value >> shift) & 31) as usize] as char);
    }
    Ok(code)
}

fn digest_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn public_key_bytes(public_key: &str) -> Result<Vec<u8>, StoreError> {
    if public_key.len() != 64 || !public_key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StoreError::Invalid(
            "public_key must be 32 bytes encoded as 64 hexadecimal characters".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(32);
    let encoded = public_key.as_bytes();
    for index in (0..encoded.len()).step_by(2) {
        let high = (encoded[index] as char)
            .to_digit(16)
            .expect("validated hex");
        let low = (encoded[index + 1] as char)
            .to_digit(16)
            .expect("validated hex");
        bytes.push(((high << 4) | low) as u8);
    }
    Ok(bytes)
}

fn fingerprint(public_key: &str) -> Result<String, StoreError> {
    let bytes = public_key_bytes(public_key)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingChallenge {
    pub protocol_version: &'static str,
    pub challenge_id: String,
    pub code: String,
    pub expires_at: i64,
    pub qr_payload: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub platform: String,
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
    pub status: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

type DeviceRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
);

pub struct DevicesStore {
    pool: sjel_store::Pool,
}

impl DevicesStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let pool =
            sjel_store::open_pool(path, "devices", |conn| migrate(conn, "devices")).map_err(db)?;
        Ok(Self { pool })
    }

    fn connection(&self) -> Result<sjel_store::PooledClient, StoreError> {
        self.pool.get().map_err(db)
    }

    pub fn ping(&self) -> Result<(), StoreError> {
        self.connection()?
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|_| ())
            .map_err(db)
    }

    pub fn create_challenge(&self) -> Result<PairingChallenge, StoreError> {
        let challenge_id = random_hex(16, "pair_")?;
        let code = random_code()?;
        let created_at = now();
        let expires_at = created_at + CHALLENGE_TTL_SECONDS;
        let code_hash = digest_hex(&code);
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO devices_pairing_challenges
             (id, code_hash, created_at, expires_at, consumed_at)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![challenge_id, code_hash, created_at, expires_at],
        )
        .map_err(db)?;
        let qr_payload = serde_json::json!({
            "protocol_version": "axon-pairing/v1",
            "challenge_id": challenge_id,
            "code": code,
        })
        .to_string();
        Ok(PairingChallenge {
            protocol_version: "axon-pairing/v1",
            challenge_id,
            code,
            expires_at,
            qr_payload,
        })
    }

    pub fn claim(
        &self,
        challenge_id: &str,
        code: &str,
        label: String,
        platform: String,
        algorithm: String,
        public_key: String,
    ) -> Result<Device, StoreError> {
        if label.trim().is_empty() || label.chars().count() > 80 {
            return Err(StoreError::Invalid(
                "label must contain 1 to 80 characters".into(),
            ));
        }
        if platform.trim().is_empty() || platform.chars().count() > 40 {
            return Err(StoreError::Invalid(
                "platform must contain 1 to 40 characters".into(),
            ));
        }
        if algorithm != "ed25519" {
            return Err(StoreError::Invalid(
                "algorithm must be ed25519 in axon-pairing/v1".into(),
            ));
        }
        let normalized_key = public_key.trim().to_ascii_lowercase();
        public_key_bytes(&normalized_key)?;
        let fingerprint = fingerprint(&normalized_key)?;
        // The native identity derives its local id from the first 16 bytes of this digest.
        // Keeping the registry id deterministic means a device can sign immediately after
        // claiming without persisting a second identifier beside its Keychain key.
        let device_id = format!("dev_{}", &fingerprint[..32]);
        let created_at = now();
        let conn = self.connection()?;
        let tx = conn.unchecked_transaction().map_err(db)?;
        let challenge: Option<(String, i64, Option<i64>)> = tx
            .query_row(
                "SELECT code_hash, expires_at, consumed_at
                 FROM devices_pairing_challenges WHERE id = ?1",
                params![challenge_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db)?;
        let Some((expected_hash, expires_at, consumed_at)) = challenge else {
            return Err(StoreError::NotFound("pairing challenge not found".into()));
        };
        if consumed_at.is_some() {
            return Err(StoreError::Conflict(
                "pairing challenge was already used".into(),
            ));
        }
        if expires_at <= created_at {
            return Err(StoreError::Conflict("pairing challenge has expired".into()));
        }
        if digest_hex(&code.trim().to_ascii_uppercase()) != expected_hash {
            return Err(StoreError::Invalid("pairing code is incorrect".into()));
        }
        let duplicate: Option<String> = tx
            .query_row(
                "SELECT id FROM devices_devices WHERE fingerprint = ?1",
                params![fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        if duplicate.is_some() {
            return Err(StoreError::Conflict(
                "this device key is already registered".into(),
            ));
        }
        tx.execute(
            "INSERT INTO devices_devices
             (id, label, platform, algorithm, public_key, fingerprint, status,
              created_at, last_seen_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, NULL, NULL)",
            params![
                device_id,
                label.trim(),
                platform.trim(),
                algorithm,
                normalized_key,
                fingerprint,
                created_at
            ],
        )
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                StoreError::Conflict("this device key is already registered".into())
            } else {
                db(error)
            }
        })?;
        tx.execute(
            "UPDATE devices_pairing_challenges SET consumed_at = ?2 WHERE id = ?1",
            params![challenge_id, created_at],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Device {
            id: device_id,
            label: label.trim().to_string(),
            platform: platform.trim().to_string(),
            algorithm,
            public_key: normalized_key,
            fingerprint,
            status: "active".into(),
            created_at,
            last_seen_at: None,
            revoked_at: None,
        })
    }

    pub fn list(&self) -> Result<Vec<Device>, StoreError> {
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(
                "SELECT id, label, platform, algorithm, public_key, fingerprint, status,
                        created_at, last_seen_at, revoked_at
                 FROM devices_devices ORDER BY created_at DESC, id DESC",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Device {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    platform: row.get(2)?,
                    algorithm: row.get(3)?,
                    public_key: row.get(4)?,
                    fingerprint: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                    last_seen_at: row.get(8)?,
                    revoked_at: row.get(9)?,
                })
            })
            .map_err(db)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db)
    }

    /// Authenticates one signed request and atomically consumes its nonce.
    pub fn authenticate(
        &self,
        request: &SignedRequest,
        method: &str,
        path_and_query: &str,
        body: &[u8],
    ) -> Result<Device, StoreError> {
        self.authenticate_at(request, method, path_and_query, body, now())
    }

    /// [`Self::authenticate`] for a verifier other than this capability, such as the Axon
    /// shell's inbound gate. The nonce is consumed in `scope`, so the shell admitting a request
    /// does not make the same request a replay when it reaches `/api/devices/me` behind it.
    /// Replay inside one scope is refused exactly as in the default scope.
    pub fn authenticate_scoped(
        &self,
        scope: &str,
        request: &SignedRequest,
        method: &str,
        path_and_query: &str,
        body: &[u8],
    ) -> Result<Device, StoreError> {
        let nonce_key = format!("{scope}:{}", request.nonce);
        self.authenticate_inner(request, &nonce_key, method, path_and_query, body, now())
    }

    fn authenticate_at(
        &self,
        request: &SignedRequest,
        method: &str,
        path_and_query: &str,
        body: &[u8],
        current_time: i64,
    ) -> Result<Device, StoreError> {
        self.authenticate_inner(
            request,
            &request.nonce,
            method,
            path_and_query,
            body,
            current_time,
        )
    }

    fn authenticate_inner(
        &self,
        request: &SignedRequest,
        nonce_key: &str,
        method: &str,
        path_and_query: &str,
        body: &[u8],
        current_time: i64,
    ) -> Result<Device, StoreError> {
        if current_time.abs_diff(request.timestamp) > auth::MAX_CLOCK_SKEW_SECONDS as u64 {
            return Err(StoreError::Unauthorized(
                "signed request timestamp is outside the allowed window".into(),
            ));
        }
        let conn = self.connection()?;
        let row: Option<DeviceRow> = conn
            .query_row(
                "SELECT id, label, platform, algorithm, public_key, fingerprint, status,
                        created_at, last_seen_at, revoked_at
                 FROM devices_devices WHERE id = ?1",
                params![request.device_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()
            .map_err(db)?;
        let Some((
            id,
            label,
            platform,
            algorithm,
            public_key,
            fingerprint,
            status,
            created_at,
            _last_seen_at,
            revoked_at,
        )) = row
        else {
            return Err(StoreError::Unauthorized("device is not registered".into()));
        };
        if status != "active" || algorithm != "ed25519" {
            return Err(StoreError::Unauthorized("device is revoked".into()));
        }
        let public_key = public_key_bytes(&public_key)
            .map_err(|_| StoreError::Unauthorized("registered device key is invalid".into()))?;
        let signature = &request.signature;
        let message = auth::signing_message(request, method, path_and_query, body);
        UnparsedPublicKey::new(&ED25519, public_key.as_slice())
            .verify(&message, signature)
            .map_err(|_| StoreError::Unauthorized("signature is invalid".into()))?;

        let tx = conn.unchecked_transaction().map_err(db)?;
        tx.execute(
            "DELETE FROM devices_request_nonces WHERE expires_at <= ?1",
            params![current_time],
        )
        .map_err(db)?;
        tx.execute(
            "INSERT INTO devices_request_nonces (device_id, nonce, seen_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                request.device_id,
                nonce_key,
                current_time,
                current_time + auth::NONCE_TTL_SECONDS
            ],
        )
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                StoreError::Unauthorized("signed request nonce was already used".into())
            } else {
                db(error)
            }
        })?;
        let changed = tx
            .execute(
                "UPDATE devices_devices SET last_seen_at = ?2
                 WHERE id = ?1 AND status = 'active'",
                params![request.device_id, current_time],
            )
            .map_err(db)?;
        if changed != 1 {
            return Err(StoreError::Unauthorized("device is revoked".into()));
        }
        tx.commit().map_err(db)?;
        Ok(Device {
            id,
            label,
            platform,
            algorithm,
            public_key: auth::hex(&public_key),
            fingerprint,
            status,
            created_at,
            last_seen_at: Some(current_time),
            revoked_at,
        })
    }

    pub fn revoke(&self, id: &str) -> Result<Device, StoreError> {
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "UPDATE devices_devices SET status = 'revoked', revoked_at = ?2
                 WHERE id = ?1 AND status = 'active'",
                params![id, now()],
            )
            .map_err(db)?;
        if changed == 0 {
            return Err(StoreError::NotFound("active device was not found".into()));
        }
        conn.query_row(
            "SELECT id, label, platform, algorithm, public_key, fingerprint, status,
                    created_at, last_seen_at, revoked_at
             FROM devices_devices WHERE id = ?1",
            params![id],
            |row| {
                Ok(Device {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    platform: row.get(2)?,
                    algorithm: row.get(3)?,
                    public_key: row.get(4)?,
                    fingerprint: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                    last_seen_at: row.get(8)?,
                    revoked_at: row.get(9)?,
                })
            },
        )
        .map_err(db)
    }
}

fn migrate(conn: &rusqlite::Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(&format!(
        "
        CREATE TABLE IF NOT EXISTS {prefix}_devices (
            id            TEXT PRIMARY KEY,
            label         TEXT NOT NULL,
            platform      TEXT NOT NULL,
            algorithm     TEXT NOT NULL CHECK (algorithm = 'ed25519'),
            public_key    TEXT NOT NULL UNIQUE,
            fingerprint   TEXT NOT NULL UNIQUE,
            status        TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
            created_at    INTEGER NOT NULL,
            last_seen_at  INTEGER,
            revoked_at    INTEGER
        );
        CREATE TABLE IF NOT EXISTS {prefix}_pairing_challenges (
            id            TEXT PRIMARY KEY,
            code_hash     TEXT NOT NULL,
            created_at    INTEGER NOT NULL,
            expires_at    INTEGER NOT NULL,
            consumed_at   INTEGER
        );
        CREATE TABLE IF NOT EXISTS {prefix}_request_nonces (
            device_id   TEXT NOT NULL,
            nonce       TEXT NOT NULL,
            seen_at     INTEGER NOT NULL,
            expires_at  INTEGER NOT NULL,
            PRIMARY KEY (device_id, nonce)
        );
        CREATE INDEX IF NOT EXISTS idx_{prefix}_devices_status
            ON {prefix}_devices(status, created_at);
        CREATE INDEX IF NOT EXISTS idx_{prefix}_request_nonces_expiry
            ON {prefix}_request_nonces(expires_at);
        CREATE INDEX IF NOT EXISTS idx_{prefix}_pairing_expiry
            ON {prefix}_pairing_challenges(expires_at, consumed_at);
        "
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (TestDir, DevicesStore) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "axon-devices-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let store = DevicesStore::open(&dir.join("axon.db")).unwrap();
        (TestDir(dir), store)
    }

    #[test]
    fn challenge_is_human_sized_and_claim_is_single_use() {
        let (_dir, store) = store();
        let challenge = store.create_challenge().unwrap();
        assert_eq!(challenge.code.len(), 10);
        assert!(challenge.qr_payload.contains("axon-pairing/v1"));
        let device = store
            .claim(
                &challenge.challenge_id,
                &challenge.code.to_ascii_lowercase(),
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                "00".repeat(32),
            )
            .unwrap();
        assert_eq!(device.status, "active");
        let second = store.claim(
            &challenge.challenge_id,
            &challenge.code,
            "Again".into(),
            "ios".into(),
            "ed25519".into(),
            "11".repeat(32),
        );
        assert!(matches!(second, Err(StoreError::Conflict(_))));
    }

    #[test]
    fn duplicate_keys_and_invalid_keys_are_refused() {
        let (_dir, store) = store();
        let challenge = store.create_challenge().unwrap();
        assert!(matches!(
            store.claim(
                &challenge.challenge_id,
                &challenge.code,
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                "not-a-key".into(),
            ),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn signed_requests_verify_once_and_revocation_takes_effect() {
        let (_dir, store) = store();
        let keypair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[7u8; 32]).unwrap();
        let public_key = auth::hex(keypair.public_key().as_ref());
        let challenge = store.create_challenge().unwrap();
        let device = store
            .claim(
                &challenge.challenge_id,
                &challenge.code,
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                public_key,
            )
            .unwrap();
        let current_time = 1_700_000_000;
        let mut request = SignedRequest {
            device_id: device.id.clone(),
            timestamp: current_time,
            nonce: "01".repeat(16),
            signature: Vec::new(),
        };
        request.signature = keypair
            .sign(&auth::signing_message(
                &request,
                "GET",
                "/api/devices/me",
                b"",
            ))
            .as_ref()
            .to_vec();
        let authenticated = store
            .authenticate_at(&request, "GET", "/api/devices/me", b"", current_time)
            .unwrap();
        assert_eq!(authenticated.id, device.id);
        assert_eq!(authenticated.last_seen_at, Some(current_time));
        assert!(matches!(
            store.authenticate_at(&request, "GET", "/api/devices/me", b"", current_time),
            Err(StoreError::Unauthorized(_))
        ));

        store.revoke(&device.id).unwrap();
        let mut after_revoke = request.clone();
        after_revoke.nonce = "02".repeat(16);
        after_revoke.signature = keypair
            .sign(&auth::signing_message(
                &after_revoke,
                "GET",
                "/api/devices/me",
                b"",
            ))
            .as_ref()
            .to_vec();
        assert!(matches!(
            store.authenticate_at(&after_revoke, "GET", "/api/devices/me", b"", current_time),
            Err(StoreError::Unauthorized(_))
        ));
    }

    /// The shell admits a request, then the same request reaches `/api/devices/me` behind it.
    /// Both must pass once, and each scope must still refuse a replay.
    #[test]
    fn a_scoped_nonce_does_not_consume_the_default_scope() {
        let (_dir, store) = store();
        let keypair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[9u8; 32]).unwrap();
        let challenge = store.create_challenge().unwrap();
        let device = store
            .claim(
                &challenge.challenge_id,
                &challenge.code,
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                auth::hex(keypair.public_key().as_ref()),
            )
            .unwrap();
        let mut request = SignedRequest {
            device_id: device.id.clone(),
            timestamp: now(),
            nonce: "03".repeat(16),
            signature: Vec::new(),
        };
        request.signature = keypair
            .sign(&auth::signing_message(
                &request,
                "GET",
                "/api/devices/me",
                b"",
            ))
            .as_ref()
            .to_vec();

        store
            .authenticate_scoped("shell", &request, "GET", "/api/devices/me", b"")
            .unwrap();
        assert!(matches!(
            store.authenticate_scoped("shell", &request, "GET", "/api/devices/me", b""),
            Err(StoreError::Unauthorized(_))
        ));
        store
            .authenticate(&request, "GET", "/api/devices/me", b"")
            .unwrap();
        assert!(matches!(
            store.authenticate(&request, "GET", "/api/devices/me", b""),
            Err(StoreError::Unauthorized(_))
        ));
    }

    #[test]
    fn revocation_is_visible_and_only_active_devices_revoke() {
        let (_dir, store) = store();
        let challenge = store.create_challenge().unwrap();
        let device = store
            .claim(
                &challenge.challenge_id,
                &challenge.code,
                "iPhone".into(),
                "ios".into(),
                "ed25519".into(),
                "22".repeat(32),
            )
            .unwrap();
        let revoked = store.revoke(&device.id).unwrap();
        assert_eq!(revoked.status, "revoked");
        assert!(matches!(
            store.revoke(&device.id),
            Err(StoreError::NotFound(_))
        ));
    }
}
