//! `sjel setup rotate turso` — replace the signing key of the primary this machine hosts.
//!
//! Rotation here is one act rather than a promotion. This primary holds no data and serves no
//! client that a person cannot provision again, so there is no second key to stage behind a live
//! one, no cutover to schedule and nothing to roll back to. What has to be durable instead is the
//! **refusal**: every Turso setup write consults the inhibit marker, and the marker is flushed to
//! disk before the Keychain is touched and removed only after a disposable sqld has accepted a
//! token signed by the new key and refused one signed by the old.
//!
//! The order is the whole design, and each step is there because of what its absence would leave:
//!
//! 1. **The inhibit marker first**, written and fsynced together with its directory. A crash after
//!    this point leaves an inhibited machine that says so, which is the recoverable failure.
//! 2. **The old-key probe token before the old key leaves its slot.** A JWT signed by a key that
//!    is gone proves nothing; the rejection is only evidence if the token could have been accepted.
//! 3. **A second copy of the previous private half**, under `turso-rotation-previous`, for exactly
//!    as long as the rejection is being proven. It is deleted afterwards, and the report says
//!    plainly whether it still exists.
//! 4. **The Keychain before the public key file.** `mint::signing_key()` reads the Keychain and
//!    sqld reads the file; replacing the file first would leave a window in which this machine
//!    mints credentials the primary rejects.
//! 5. **The proof runs a disposable sqld** on a loopback port it reserves for itself, with a
//!    temporary database and the new public key. It proves key mechanics: the new public key
//!    accepts a new-key token and refuses the old one. It proves nothing about the live primary,
//!    which this command refuses to have running, and it says so.
//! 6. **The marker goes only on success.** A failure keeps it, exits nonzero, and never restores
//!    the previous key — an automatic rollback would be a second silent key change on a machine
//!    whose state nobody has looked at yet.
//!
//! `sjel setup rotate recover` is the other half: it reports what the journal and the public key
//! file say, and it stays inhibited rather than guessing.

#[cfg(test)]
#[path = "rotation_tests.rs"]
mod tests;

use std::fs::{File, OpenOptions};
use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use libsql_auth::{Access, PrimaryKey};
use serde::{Deserialize, Serialize};
use sjel_credentials::{
    CredentialId, CredentialProvider, KeychainProvider, Presence, ProviderReference, SecretValue,
};

use crate::capability;
use crate::paths::Paths;
use crate::rotation_guard::{self, Guard};
use crate::rotation_probe::probe_rotation;

use super::mint;

/// The phrase a person types. Not a flag: a flag is something a wrapper passes.
const PHRASE: &str = "rotate turso signing key";

/// The sqld version the proof runs, and the one `tools/install-sqld.sh` installs.
const PINNED_SQLD: &str = "0.24.32";

/// How long the disposable primary is given to accept the new key. Startup on this Mac is under a
/// second; the deadline is for a machine under load, not for a slow primary.
const PROOF_DEADLINE: Duration = Duration::from_secs(30);

/// A probe token lives for a day. It is minted, sent over loopback and dropped in this process;
/// nothing stores it, and it is never an argument on a command line.
const PROBE_DAYS: u64 = 1;

/// How long a loopback connect is given before the port counts as free.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

const JOURNAL_VERSION: u8 = 1;

/// The Keychain account holding the previous private half while its rejection is proven.
const PREVIOUS_ID: &str = "turso-rotation-previous";

const CONSEQUENCES: &str = "\
This replaces the signing key of the primary this machine hosts. What follows:

  1. Every token signed by the key being replaced stops working. There is no overlap and no grace
     period: sqld checks a signature against the public key file it was started with, and that file
     is replaced.
  2. Every client of this primary needs reprovisioning — each replica's SJEL_TURSO_AUTH_TOKEN,
     every other device's credential, and this machine's own recorded one.
  3. The old private half is deleted from the Keychain on success. There is no rollback: a failure
     leaves the inhibit marker in place and a person finishes or investigates it.

Nothing is started or installed to do this; the primary must not be running.";

// ---------------------------------------------------------------------------------------------
// The journal
// ---------------------------------------------------------------------------------------------

/// What the inhibit marker records: the two public halves and nothing secret.
///
/// Two states, not a phase machine: either the replacement is recorded or it is not. A marker that
/// does not say which key replaced which is a marker a person has to investigate, and that is what
/// `recover` reports instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    /// The public half of the key that was active when the rotation began.
    previous_public_key: String,
    /// The public half that replaced it, recorded once the Keychain and the file agree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
}

impl Journal {
    /// Create the marker: written, fsynced, and its directory fsynced, before anything else moves.
    fn start(guard: &Guard, previous_public_key: &str) -> Result<Self, String> {
        let journal = Self {
            version: JOURNAL_VERSION,
            previous_public_key: previous_public_key.to_owned(),
            public_key: None,
        };
        let body = journal.encode()?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(guard.marker())
            .map_err(|e| {
                format!(
                    "sqld rotation: cannot create the inhibit marker {}: {e}",
                    guard.marker().display()
                )
            })?;
        file.write_all(&body)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("sqld rotation: cannot persist the inhibit marker: {e}"))?;
        sync_directory(guard.marker())?;
        Ok(journal)
    }

    fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| "sqld rotation: cannot encode the journal".to_owned())
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != JOURNAL_VERSION {
            return Err("sqld rotation: unsupported journal version".to_owned());
        }
        validate_public_key(&self.previous_public_key)?;
        if let Some(public_key) = &self.public_key {
            validate_public_key(public_key)?;
            if public_key == &self.previous_public_key {
                return Err("sqld rotation: the journal records one key twice".to_owned());
            }
        }
        Ok(())
    }

    fn read(guard: &Guard) -> Result<Self, String> {
        let body = std::fs::read(guard.marker()).map_err(|e| {
            format!(
                "sqld rotation: cannot read the inhibit marker {}: {e}",
                guard.marker().display()
            )
        })?;
        if body.len() > 1024 {
            return Err("sqld rotation: the inhibit marker is not a journal".to_owned());
        }
        let journal: Journal = serde_json::from_slice(&body)
            .map_err(|_| "sqld rotation: the inhibit marker is not a journal".to_owned())?;
        journal.validate()?;
        Ok(journal)
    }

    /// Record the replacement, by write-fsync-rename so the marker is never half a document.
    fn record_replacement(&self, guard: &Guard, public_key: &str) -> Result<Self, String> {
        if Self::read(guard)? != *self {
            return Err(
                "sqld rotation: the journal changed under this run; the inhibit stands".to_owned(),
            );
        }
        let next = Self {
            version: self.version,
            previous_public_key: self.previous_public_key.clone(),
            public_key: Some(public_key.to_owned()),
        };
        let body = next.encode()?;
        let temp = parent(guard.marker())?.join(format!(".sqld-rotation-{}.tmp", random_hex()?));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| format!("sqld rotation: cannot create the journal temporary file: {e}"))?;
        let written = file
            .write_all(&body)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("sqld rotation: cannot persist the journal: {e}"));
        drop(file);
        if let Err(message) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(message);
        }
        if let Err(e) = std::fs::rename(&temp, guard.marker()) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("sqld rotation: cannot replace the journal: {e}"));
        }
        sync_directory(guard.marker())?;
        Ok(next)
    }

    /// Remove the marker, which is the one thing that authorizes Turso setup to write again.
    fn clear(guard: &Guard) -> Result<(), String> {
        std::fs::remove_file(guard.marker()).map_err(|e| {
            format!(
                "sqld rotation: cannot remove the inhibit marker {}: {e}",
                guard.marker().display()
            )
        })?;
        sync_directory(guard.marker())
    }
}

fn parent(path: &Path) -> Result<&Path, String> {
    path.parent()
        .ok_or_else(|| "sqld rotation: invalid journal path".to_owned())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(parent(path)?)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("sqld rotation: cannot sync the journal directory: {e}"))
}

fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| "sqld rotation: random source unavailable".to_owned())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A public half is 32 bytes in base64url without padding, and nothing else.
fn validate_public_key(public_key: &str) -> Result<(), String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(public_key)
        .map_err(|_| "sqld rotation: invalid public key".to_owned())?;
    if bytes.len() != 32
        || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes) != public_key
    {
        return Err("sqld rotation: invalid public key".to_owned());
    }
    Ok(())
}

fn read_public(path: &Path) -> Result<String, String> {
    let body = std::fs::read(path)
        .map_err(|_| format!("the public key file {} is unavailable", path.display()))?;
    let public_key =
        std::str::from_utf8(&body).map_err(|_| "the public key file is not text".to_owned())?;
    validate_public_key(public_key)?;
    Ok(public_key.to_owned())
}

/// Read the public key file back off disk, so the report names what sqld will actually read.
fn sync_file_and_parent(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("sqld rotation: cannot sync {}: {e}", path.display()))?;
    sync_directory(path)
}

/// A token is only evidence of rejection if it is a genuine, unexpired token for that key.
fn verify_old_probe(token: &str, public_key: &str, at: u64) -> Result<(), String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let invalid = "sqld rotation: the old-key probe token is not a valid unexpired token";
    let mut parts = token.split('.');
    let header = parts.next().ok_or(invalid)?;
    let claims = parts.next().ok_or(invalid)?;
    let signature = parts.next().ok_or(invalid)?;
    if parts.next().is_some() {
        return Err(invalid.to_owned());
    }
    let header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).map_err(|_| invalid)?)
            .map_err(|_| invalid)?;
    let payload: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).map_err(|_| invalid)?)
            .map_err(|_| invalid)?;
    if header.get("alg").and_then(|value| value.as_str()) != Some("EdDSA")
        || payload
            .get("exp")
            .and_then(|value| value.as_u64())
            .is_none_or(|exp| exp <= at)
    {
        return Err(invalid.to_owned());
    }
    let key = URL_SAFE_NO_PAD.decode(public_key).map_err(|_| invalid)?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid)?;
    let (signed, _) = token.rsplit_once('.').ok_or(invalid)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| invalid.to_owned())
}

// ---------------------------------------------------------------------------------------------
// The Keychain items
// ---------------------------------------------------------------------------------------------

fn previous_reference() -> Result<(CredentialId, ProviderReference), String> {
    let id = CredentialId::parse(PREVIOUS_ID)
        .map_err(|_| "sqld rotation: invalid previous key reference".to_owned())?;
    Ok((
        id,
        ProviderReference::Keychain {
            account: PREVIOUS_ID.to_owned(),
        },
    ))
}

fn stored_key(
    provider: &dyn CredentialProvider,
    id: &CredentialId,
    reference: &ProviderReference,
    what: &str,
) -> Result<PrimaryKey, String> {
    let value = provider
        .get(id, reference)
        .map_err(|e| format!("sqld rotation: {what} is unavailable: {e}"))?;
    PrimaryKey::from_pkcs8(value.expose_bytes())
        .map_err(|_| format!("sqld rotation: {what} is not a usable key"))
}

/// Keep the previous private half in a second slot, because a token it signed has to be usable
/// after it stops being the active key.
fn retain_previous(provider: &dyn CredentialProvider, previous: &PrimaryKey) -> Result<(), String> {
    let (id, reference) = previous_reference()?;
    if provider
        .status(&id, &reference)
        .map_err(|e| format!("sqld rotation: cannot inspect the previous key reference: {e}"))?
        != Presence::Missing
    {
        return Err(format!(
            "the Keychain item com.sjel.credential.{PREVIOUS_ID} already exists, so a previous \
             rotation may not have finished; inspect it before rotating"
        ));
    }
    retain(provider, &id, reference, previous)
}

fn retain(
    provider: &dyn CredentialProvider,
    id: &CredentialId,
    reference: ProviderReference,
    previous: &PrimaryKey,
) -> Result<(), String> {
    let value = SecretValue::new(previous.pkcs8().to_vec())
        .map_err(|_| "sqld rotation: cannot wrap the previous key".to_owned())?;
    let written = provider
        .create(id, "Turso rotation previous key", &value)
        .map_err(|e| format!("sqld rotation: cannot retain the previous key: {e}"))?;
    if written != reference {
        return Err("sqld rotation: the previous key landed at another reference".to_owned());
    }
    Ok(())
}

/// Delete the retained copy and answer why it is still there, when it is.
fn drop_previous(provider: &dyn CredentialProvider) -> Result<Option<String>, String> {
    let (id, reference) = previous_reference()?;
    let failure = provider.delete(&id, &reference).err();
    match provider.status(&id, &reference) {
        Ok(Presence::Missing) => Ok(None),
        Ok(Presence::Present) => Ok(Some(match failure {
            Some(error) => format!("the delete failed ({error}) and the item is still there"),
            None => "the delete reported success and the item is still there".to_owned(),
        })),
        Err(error) => Ok(Some(format!(
            "the Keychain could not be asked after the delete ({error})"
        ))),
    }
}

// ---------------------------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------------------------

/// The facts the rotation was planned against, which stay true only because nothing is running.
pub(crate) struct Preflight {
    /// The public half deployed for sqld, which must be the key the Keychain mints from.
    pub(crate) public_key: String,
}

/// Refuse on anything uncertain, before a single byte is written.
pub(crate) fn preflight(paths: &Paths) -> Result<Preflight, String> {
    let port = super::turso::sqld_port(paths)?;
    let address: SocketAddr = format!("127.0.0.1:{port}").parse().map_err(|_| {
        format!("sqld rotation: the sqld manifest declares an unusable port '{port}'")
    })?;
    if TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok() {
        return Err(format!(
            "sqld answers on {address}, so the primary is running; stop it and leave it stopped \
             (this command never starts or stops a service)"
        ));
    }
    persistence_absent(paths)?;
    // The proof needs the pinned binary; its absence is a refusal, not a mid-rotation surprise.
    pinned_sqld()?;
    Ok(Preflight {
        public_key: read_public(&mint::public_key_path(paths)?)?,
    })
}

/// Boot persistence must be absent or not loaded: a supervisor that starts sqld mid-rotation is
/// the one thing the inhibit marker cannot stop.
fn persistence_absent(paths: &Paths) -> Result<(), String> {
    let runner = paths.root.join("tools/service-runner.sh");
    let output = Command::new(&runner)
        .args(["persistence-status", "sqld"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            format!(
                "cannot ask {} for sqld's persistence state: {e}",
                runner.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "{} persistence-status sqld failed: {}",
            runner.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    persistence_state(&String::from_utf8_lossy(&output.stdout))
}

/// One `capability<TAB>state<TAB>detail` line: only `missing` and `n/a` mean nothing will start
/// sqld behind this command.
fn persistence_state(status: &str) -> Result<(), String> {
    let state = status
        .lines()
        .next()
        .and_then(|line| line.split('\t').nth(1))
        .unwrap_or_default();
    match state {
        "missing" | "n/a" | "installed-not-loaded" => Ok(()),
        "installed" => Err(
            "boot persistence for sqld is installed and loaded, so a supervisor could start the \
             primary mid-rotation; remove it first (tools/service-runner.sh remove-persistence \
             sqld)"
                .to_owned(),
        ),
        "" => Err(
            "cannot tell whether sqld has boot persistence: persistence-status printed no usable \
             state"
                .to_owned(),
        ),
        other => Err(format!(
            "sqld boot persistence is '{other}'; rotation needs it absent or not loaded"
        )),
    }
}

fn pinned_sqld() -> Result<PathBuf, String> {
    let binary = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("sqld"))
                .find(|candidate| candidate.is_file())
        })
        .and_then(|candidate| candidate.canonicalize().ok())
        .ok_or_else(|| {
            "sqld is not on PATH, and the proof runs against it; install the pinned sqld first \
             (tools/install-sqld.sh got executed when the primary was hosted)"
                .to_owned()
        })?;
    let output = Command::new(&binary)
        .env_clear()
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !version.split_whitespace().any(|part| part == PINNED_SQLD) {
        return Err(format!(
            "{} is not sqld {PINNED_SQLD}, which the proof is pinned to",
            binary.display()
        ));
    }
    Ok(binary)
}

// ---------------------------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------------------------

/// What the disposable primary is asked to show: this public key accepts a token signed by the new
/// key and refuses one signed by the old.
pub(crate) struct Claim<'a> {
    pub(crate) public_key: &'a str,
    pub(crate) old_token: &'a str,
    pub(crate) new_token: &'a str,
}

/// The verification, injectable so a test never starts a process.
pub(crate) trait Proof {
    fn prove(&self, claim: &Claim<'_>) -> Result<(), String>;
}

/// A sqld of its own, on a port it reserves, over a temporary database.
struct DisposableSqld;

impl Proof for DisposableSqld {
    fn prove(&self, claim: &Claim<'_>) -> Result<(), String> {
        let binary = pinned_sqld()?;
        let dir = std::env::temp_dir().join(format!("sjel-rotation-proof-{}", random_hex()?));
        std::fs::create_dir(&dir)
            .map_err(|e| format!("cannot create the disposable data directory: {e}"))?;
        let mut disposable = Disposable { dir, child: None };
        let key_file = disposable.dir.join("sqld-auth.pub");
        std::fs::write(&key_file, claim.public_key)
            .map_err(|e| format!("cannot write the disposable public key: {e}"))?;

        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("cannot reserve an isolated loopback port: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("cannot read the reserved loopback port: {e}"))?;
        drop(listener);

        disposable.child = Some(
            Command::new(&binary)
                .env_clear()
                .current_dir(&disposable.dir)
                .args(["--db-path", "database", "--http-listen-addr"])
                .arg(address.to_string())
                .arg("--auth-jwt-key-file")
                .arg(&key_file)
                .args(["--no-welcome", "--disable-metrics"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("cannot start the disposable sqld: {e}"))?,
        );

        let endpoint = format!("http://{address}");
        let deadline = Instant::now() + PROOF_DEADLINE;
        loop {
            if let Some(child) = disposable.child.as_mut() {
                if child
                    .try_wait()
                    .map_err(|e| format!("cannot check the disposable sqld: {e}"))?
                    .is_some()
                {
                    return Err(
                        "the disposable sqld exited before it answered, so nothing is proven and \
                         the inhibit stands"
                            .to_owned(),
                    );
                }
            }
            let reason = match probe_rotation(&endpoint, claim.new_token, claim.old_token) {
                Ok(()) => return Ok(()),
                Err(reason) => reason,
            };
            if Instant::now() >= deadline {
                return Err(format!(
                    "the disposable sqld on {address} did not settle within {}s: {reason}",
                    PROOF_DEADLINE.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Kills the disposable primary and removes its data, however `prove` leaves.
struct Disposable {
    dir: PathBuf,
    child: Option<Child>,
}

impl Drop for Disposable {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------------------------
// The rotation
// ---------------------------------------------------------------------------------------------

/// What a completed rotation leaves behind, for the report.
#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) previous_public_key: String,
    pub(crate) public_key: String,
    pub(crate) public_key_path: PathBuf,
    /// Why the previous private half still exists, when it does.
    pub(crate) previous_key_left: Option<String>,
}

pub(crate) struct Rotation<'a> {
    pub(crate) paths: &'a Paths,
    pub(crate) provider: &'a dyn CredentialProvider,
    pub(crate) proof: &'a dyn Proof,
}

impl Rotation<'_> {
    /// Replace the key. Every failure keeps the inhibit marker and leaves the new key in place: the
    /// one thing this never does is put the previous key back.
    pub(crate) fn execute(&self, preflight: &Preflight) -> Result<Report, String> {
        let guard = rotation_guard::acquire(self.paths)?;
        if guard.marker_is_present()? {
            return Err(inhibited());
        }

        let (active_id, active_reference) = mint::key_reference()?;
        if self
            .provider
            .status(&active_id, &active_reference)
            .map_err(|e| format!("sqld rotation: cannot read the Keychain: {e}"))?
            == Presence::Missing
        {
            return Err(
                "no signing key is stored, so there is nothing to rotate; `sjel setup apply turso` \
                 creates one"
                    .to_owned(),
            );
        }
        let previous = stored_key(
            self.provider,
            &active_id,
            &active_reference,
            "the signing key",
        )?;
        let previous_public_key = previous.public_key_base64url();
        if preflight.public_key != previous_public_key {
            return Err(format!(
                "{} holds {}, which is not the key in the Keychain ({}), so the primary this file \
                 describes and the key this machine mints with already disagree; nothing was \
                 changed",
                mint::public_key_path(self.paths)?.display(),
                preflight.public_key,
                previous_public_key
            ));
        }

        // The inhibit stands from here on, and every failure below leaves it standing.
        let journal = Journal::start(&guard, &previous_public_key)?;

        // The old-key token has to exist while the old key is still the active one.
        let old_token = previous
            .mint_for_days(None, Access::Write, PROBE_DAYS)
            .map_err(|e| format!("sqld rotation: cannot mint the old-key probe token: {e}"))?;
        verify_old_probe(&old_token, &previous_public_key, now())?;
        retain_previous(self.provider, &previous)?;

        // The Keychain first: `mint::signing_key()` must never sign with a key sqld rejects.
        let next = PrimaryKey::generate()
            .map_err(|e| format!("sqld rotation: cannot generate a key: {e}"))?;
        let value = SecretValue::new(next.pkcs8().to_vec())
            .map_err(|_| "sqld rotation: cannot wrap the new key".to_owned())?;
        self.provider
            .update(&active_id, &active_reference, &value)
            .map_err(|e| format!("sqld rotation: cannot store the new key: {e}"))?;
        let public_key_path = mint::write_public_key(self.paths, &next)?;
        sync_file_and_parent(&public_key_path)?;
        let public_key = next.public_key_base64url();

        // Read the active slot back through the address `signing_key()` uses, not from `next`.
        let readback = stored_key(
            self.provider,
            &active_id,
            &active_reference,
            "the new signing key",
        )?;
        if readback.public_key_base64url() != public_key {
            return Err(
                "sqld rotation: the stored key does not read back as the generated one; the \
                 inhibit stands"
                    .to_owned(),
            );
        }
        journal.record_replacement(&guard, &public_key)?;

        let new_token = next
            .mint_for_days(None, Access::Write, PROBE_DAYS)
            .map_err(|e| format!("sqld rotation: cannot mint the new-key probe token: {e}"))?;
        self.proof.prove(&Claim {
            public_key: &public_key,
            old_token: &old_token,
            new_token: &new_token,
        })?;

        // Proven: the retained copy has done its work, and the report says whether it is gone.
        let previous_key_left = drop_previous(self.provider)?;
        Journal::clear(&guard)?;

        Ok(Report {
            previous_public_key,
            public_key,
            public_key_path,
            previous_key_left,
        })
    }
}

fn inhibited() -> String {
    "an unfinished rotation is recorded, so Turso setup is inhibited; run `sjel setup rotate \
     recover` and read what it says before touching anything"
        .to_owned()
}

// ---------------------------------------------------------------------------------------------
// The verb
// ---------------------------------------------------------------------------------------------

pub(crate) fn run(positional: &[String], confirm: bool) -> std::process::ExitCode {
    match positional {
        [] => usage("one target is required — turso or recover"),
        [target] if target == "turso" => rotate(confirm),
        [target] if target == "recover" => recover(),
        [target] => usage(&format!(
            "unknown target '{target}' — expected turso or recover"
        )),
        _ => usage("one target at a time — turso or recover"),
    }
}

fn usage(message: &str) -> std::process::ExitCode {
    eprintln!("sjel setup rotate: {message}");
    eprintln!("Usage: sjel setup rotate turso --confirm   (a person, at a terminal)");
    eprintln!("       sjel setup rotate recover           (reports an unfinished rotation)");
    std::process::ExitCode::from(2)
}

/// Why the command refuses to run, if it does. Each gate has its own sentence, because a rotation
/// a script, an agent or an inattentive shell can start is not one a person authorized.
fn refusal(terminal: bool, agent_session: bool, confirm: bool) -> Option<&'static str> {
    if agent_session {
        return Some(
            "refusing to rotate a signing key in an agent session — this command is invoked by a \
             person, at a terminal",
        );
    }
    if !terminal {
        return Some(
            "refusing to rotate without a terminal on stdin and stderr — there is no confirmation \
             phrase to type, and the consequences are printed here",
        );
    }
    if !confirm {
        return Some("--confirm is required: nothing was changed");
    }
    None
}

fn rotate(confirm: bool) -> std::process::ExitCode {
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if let Some(reason) = refusal(terminal, capability::in_agent_session(), confirm) {
        eprintln!("sjel setup rotate turso: {reason}");
        return std::process::ExitCode::from(2);
    }
    let paths = match Paths::from_env() {
        Ok(paths) => paths,
        Err(message) => return usage(&message),
    };
    match rotation_guard::marker_path(&paths) {
        Ok(marker) if marker.exists() => {
            eprintln!("sjel setup rotate turso: {}", inhibited());
            return std::process::ExitCode::FAILURE;
        }
        Ok(_) => {}
        Err(message) => {
            eprintln!("sjel setup rotate turso: {message}");
            return std::process::ExitCode::from(2);
        }
    }
    let preflight = match preflight(&paths) {
        Ok(preflight) => preflight,
        Err(message) => {
            eprintln!("sjel setup rotate turso: {message}");
            eprintln!("Nothing was changed.");
            return std::process::ExitCode::FAILURE;
        }
    };

    println!("sjel setup rotate turso\n\n{CONSEQUENCES}\n");
    print!("Type `{PHRASE}` to continue: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() || answer.trim() != PHRASE {
        eprintln!(
            "\nsjel setup rotate turso: aborted — the phrase was not typed. Nothing was changed."
        );
        return std::process::ExitCode::FAILURE;
    }

    let provider = KeychainProvider::system();
    let proof = DisposableSqld;
    let rotation = Rotation {
        paths: &paths,
        provider: &provider,
        proof: &proof,
    };
    match rotation.execute(&preflight) {
        Ok(report) => {
            print_report(&report);
            std::process::ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("sjel setup rotate turso: {message}");
            eprintln!(
                "The inhibit marker stays in place, so Turso setup refuses to write. The previous \
                 key was not restored: run `sjel setup rotate recover` and read what it says."
            );
            std::process::ExitCode::FAILURE
        }
    }
}

fn print_report(report: &Report) {
    println!("\nsjel setup rotate turso — the signing key was replaced\n");
    println!("  ✓ previous key   {}", report.previous_public_key);
    println!("  ✓ new key        {}", report.public_key);
    println!(
        "  ✓ public key     written and flushed to {}",
        report.public_key_path.display()
    );
    println!(
        "  ✓ proof          a disposable sqld accepted a token signed by the new key and refused \
         one signed by the previous key"
    );
    match &report.previous_key_left {
        None => println!("  ✓ previous key   deleted from the Keychain"),
        Some(reason) => println!(
            "  ! previous key   still in the Keychain at com.sjel.credential.{PREVIOUS_ID} ({reason})"
        ),
    }
    println!("  ✓ inhibit        removed, so Turso setup may write again");
    println!(
        "\nThis proves key mechanics, not the live primary: nothing here contacted {}. The proof \
         ran against a disposable sqld on loopback, with its own temporary database and the new \
         public key. The primary will read that file when it next starts, and it was not started \
         by this command.",
        report.public_key_path.display()
    );
    if report.previous_key_left.is_some() {
        println!(
            "\nA previous private half survives in the Keychain. It can no longer authenticate \
             anything, because the public key file no longer matches it. Delete the Keychain item \
             named com.sjel.credential.{PREVIOUS_ID} in Keychain Access by hand."
        );
    }
    println!(
        "\nStill to do before this deployment works again, both by a person:\n  - mint a credential \
         for this machine: `sjel setup apply turso --remint --confirm` (the recorded token was \
         signed by the old key and no longer verifies)\n  - reprovision every other client of this \
         primary: each replica's SJEL_TURSO_AUTH_TOKEN, and each other device's credential"
    );
}

// ---------------------------------------------------------------------------------------------
// `recover`
// ---------------------------------------------------------------------------------------------

fn recover() -> std::process::ExitCode {
    let paths = match Paths::from_env() {
        Ok(paths) => paths,
        Err(message) => return usage(&message),
    };
    match inspect(&paths) {
        Ok(None) => {
            println!(
                "sjel setup rotate recover: no rotation is recorded, so nothing is inhibited."
            );
            std::process::ExitCode::SUCCESS
        }
        Ok(Some(lines)) => {
            println!("sjel setup rotate recover\n");
            for line in lines {
                println!("  {line}");
            }
            std::process::ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("sjel setup rotate recover: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// What the marker and the public key file say, as lines for a person to read.
///
/// Reads files only: it does not ask the Keychain and it does not clear anything, so an inhibited
/// machine is reported rather than guessed at. `None` means no rotation is recorded.
fn inspect(paths: &Paths) -> Result<Option<Vec<String>>, String> {
    let marker = rotation_guard::marker_path(paths)?;
    let body = match std::fs::read(&marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read the inhibit marker {}: {error}",
                marker.display()
            ))
        }
        Ok(body) => body,
    };
    let journal: Journal = match serde_json::from_slice(&body) {
        Ok(journal) => journal,
        Err(_) => {
            return Err(format!(
                "{} is present but is not a rotation journal, so the inhibit stands and a person \
                 must look at it",
                marker.display()
            ))
        }
    };
    journal.validate()?;

    let mut lines = vec![format!("marker: {} (still in place)", marker.display())];
    match &journal.public_key {
        None => lines.push(format!(
            "journal: no replacement is recorded, so the Keychain may hold the key whose public \
             half is {} or one generated after this journal was written",
            journal.previous_public_key
        )),
        Some(public_key) => lines.push(format!(
            "journal: the replacement key is recorded — {} (was {})",
            public_key, journal.previous_public_key
        )),
    }
    match mint::public_key_path(paths).and_then(|path| read_public(&path)) {
        Ok(public_key) => {
            lines.push(format!("file: sqld-auth.pub holds {public_key}"));
            if journal.public_key.as_deref() == Some(public_key.as_str()) {
                lines.push("file: that is the replacement key".to_owned());
            } else if public_key == journal.previous_public_key {
                lines.push(
                    "file: that is the key that was active when the rotation began".to_owned(),
                );
            } else {
                lines.push(
                    "file: that is neither recorded key, so this machine's signing state has \
                     drifted and a person must decide what it should be"
                        .to_owned(),
                );
            }
        }
        Err(message) => lines.push(format!("file: {message}")),
    }
    lines.push(format!(
        "keychain: the previous private half, if it was retained, is com.sjel.credential.{PREVIOUS_ID} \
         — this report does not read the Keychain, so its presence is unverified"
    ));
    lines.push(
        "this command clears nothing and restores nothing: Turso setup stays refused until a \
         person removes the marker"
            .to_owned(),
    );
    Ok(Some(lines))
}
