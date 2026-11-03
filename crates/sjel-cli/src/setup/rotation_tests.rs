use super::*;
use sjel_credentials::{CredentialError, ProviderKind};
use std::collections::HashMap;
use std::sync::Mutex;

// ---------------------------------------------------------------------------------------------
// Fixtures: an isolated overlay, a fake Keychain, a proof that records what it was asked
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct FakeProvider {
    values: Mutex<HashMap<String, Vec<u8>>>,
    deletes: Mutex<usize>,
    fail_create: Mutex<bool>,
    fail_delete: Mutex<bool>,
}

impl CredentialProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Keychain
    }

    fn status(
        &self,
        id: &CredentialId,
        _: &ProviderReference,
    ) -> Result<Presence, CredentialError> {
        Ok(if self.values.lock().unwrap().contains_key(id.as_str()) {
            Presence::Present
        } else {
            Presence::Missing
        })
    }

    fn create(
        &self,
        id: &CredentialId,
        _: &str,
        value: &SecretValue,
    ) -> Result<ProviderReference, CredentialError> {
        if *self.fail_create.lock().unwrap() {
            return Err(CredentialError::ProviderOperation {
                operation: "fixture-create",
            });
        }
        self.values
            .lock()
            .unwrap()
            .insert(id.as_str().to_owned(), value.expose_bytes().to_vec());
        Ok(ProviderReference::Keychain {
            account: id.as_str().to_owned(),
        })
    }

    fn update(
        &self,
        id: &CredentialId,
        _: &ProviderReference,
        value: &SecretValue,
    ) -> Result<(), CredentialError> {
        let mut values = self.values.lock().unwrap();
        let item = values
            .get_mut(id.as_str())
            .ok_or(CredentialError::NotFound)?;
        *item = value.expose_bytes().to_vec();
        Ok(())
    }

    fn get(
        &self,
        id: &CredentialId,
        _: &ProviderReference,
    ) -> Result<SecretValue, CredentialError> {
        let values = self.values.lock().unwrap();
        SecretValue::new(
            values
                .get(id.as_str())
                .ok_or(CredentialError::NotFound)?
                .clone(),
        )
    }

    fn delete(&self, id: &CredentialId, _: &ProviderReference) -> Result<(), CredentialError> {
        *self.deletes.lock().unwrap() += 1;
        if *self.fail_delete.lock().unwrap() {
            return Err(CredentialError::ProviderOperation {
                operation: "fixture-delete",
            });
        }
        self.values.lock().unwrap().remove(id.as_str());
        Ok(())
    }
}

#[derive(Default)]
struct Recorded {
    public_key: String,
    old_token: String,
    new_token: String,
}

#[derive(Default)]
struct RecordingProof {
    fail_with: Mutex<Option<&'static str>>,
    claim: Mutex<Option<Recorded>>,
    marker: Mutex<Option<PathBuf>>,
    journal_at_proof: Mutex<Option<Vec<u8>>>,
}

impl Proof for RecordingProof {
    fn prove(&self, claim: &Claim<'_>) -> Result<(), String> {
        *self.claim.lock().unwrap() = Some(Recorded {
            public_key: claim.public_key.to_owned(),
            old_token: claim.old_token.to_owned(),
            new_token: claim.new_token.to_owned(),
        });
        if let Some(marker) = self.marker.lock().unwrap().as_ref() {
            *self.journal_at_proof.lock().unwrap() = std::fs::read(marker).ok();
        }
        match *self.fail_with.lock().unwrap() {
            Some(message) => Err(message.to_owned()),
            None => Ok(()),
        }
    }
}

struct Fixture {
    root: PathBuf,
    paths: Paths,
    provider: FakeProvider,
    previous_public_key: String,
    previous_pkcs8: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("sjel-rotation-{}", random_hex().unwrap()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        let paths = Paths {
            root: root.clone(),
            caps_dir: root.join("capabilities"),
            overlay_root: Some(root.clone()),
            machine_toml: None,
            overlay_caps_dir: None,
        };
        let key = PrimaryKey::generate().unwrap();
        let previous_public_key = key.public_key_base64url();
        std::fs::write(mint::public_key_path(&paths).unwrap(), &previous_public_key).unwrap();
        let provider = FakeProvider::default();
        let (id, _) = mint::key_reference().unwrap();
        provider
            .values
            .lock()
            .unwrap()
            .insert(id.as_str().to_owned(), key.pkcs8().to_vec());
        Self {
            root,
            paths,
            provider,
            previous_public_key,
            previous_pkcs8: key.pkcs8().to_vec(),
        }
    }

    fn preflight(&self) -> Preflight {
        Preflight {
            public_key: self.previous_public_key.clone(),
        }
    }

    fn marker(&self) -> PathBuf {
        rotation_guard::marker_path(&self.paths).unwrap()
    }

    fn public_key_file(&self) -> String {
        std::fs::read_to_string(mint::public_key_path(&self.paths).unwrap()).unwrap()
    }

    /// The key `mint::signing_key()` would answer, read through the same reference it uses.
    fn active(&self) -> PrimaryKey {
        let (id, reference) = mint::key_reference().unwrap();
        stored_key(&self.provider, &id, &reference, "the signing key").unwrap()
    }

    fn journal(&self) -> Option<Journal> {
        let body = std::fs::read(self.marker()).ok()?;
        serde_json::from_slice(&body).ok()
    }

    fn rotate(&self, proof: &RecordingProof) -> Result<Report, String> {
        Rotation {
            paths: &self.paths,
            provider: &self.provider,
            proof,
        }
        .execute(&self.preflight())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------------------------

/// Each precondition is its own refusal, so a person reading one sentence knows the one thing that
/// is missing rather than a list of everything the command wants.
#[test]
fn each_gate_refuses_with_its_own_reason() {
    assert_eq!(refusal(true, false, true), None);
    let agent = refusal(true, true, true).unwrap();
    assert!(agent.contains("agent session"), "{agent}");
    let terminal = refusal(false, false, true).unwrap();
    assert!(terminal.contains("terminal"), "{terminal}");
    let confirm = refusal(true, false, false).unwrap();
    assert!(confirm.contains("--confirm"), "{confirm}");
}

// ---------------------------------------------------------------------------------------------
// The probe token, and what makes a rejection evidence
// ---------------------------------------------------------------------------------------------

#[test]
fn a_probe_token_is_evidence_only_while_it_is_genuine_and_unexpired() {
    let old = PrimaryKey::generate().unwrap();
    let other = PrimaryKey::generate().unwrap();
    let token = old.mint(None, Access::Write, 200).unwrap();
    let public = old.public_key_base64url();
    assert!(verify_old_probe(&token, &public, 199).is_ok());
    assert!(verify_old_probe(&token, &public, 200).is_err());
    assert!(verify_old_probe(&token, &other.public_key_base64url(), 199).is_err());
    for malformed in ["", "fixture-secret-token", "a.b.c.d", "a.b.c"] {
        let error = verify_old_probe(malformed, &public, 199).unwrap_err();
        assert!(!error.contains("fixture-secret-token"), "{error}");
    }
    assert!(verify_old_probe(&format!("{token}x"), &public, 199).is_err());
}

// ---------------------------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------------------------

/// The one supervisor answer that matters: can something start the primary behind this command?
#[test]
fn persistence_that_could_start_the_primary_is_refused() {
    assert!(persistence_state("sqld\tmissing\t/unit\n").is_ok());
    assert!(persistence_state("sqld\tn/a\tno autostart\n").is_ok());
    assert!(persistence_state("sqld\tinstalled-not-loaded\tthe unit exists\n").is_ok());
    for (line, needle) in [
        (
            "sqld\tinstalled\t/Library/LaunchAgents/x.plist\n",
            "installed",
        ),
        ("sqld\tstale\tno longer matches\n", "stale"),
        ("sqld\tunsupported\tnothing to render\n", "unsupported"),
        ("sqld\n", "cannot tell"),
        ("", "cannot tell"),
    ] {
        let error = persistence_state(line).unwrap_err();
        assert!(error.contains(needle), "{line:?}: {error}");
    }
}

// ---------------------------------------------------------------------------------------------
// A rotation that works
// ---------------------------------------------------------------------------------------------

#[test]
fn the_key_is_replaced_the_old_one_is_proven_dead_and_the_inhibit_is_cleared() {
    let fixture = Fixture::new();
    let proof = RecordingProof::default();
    *proof.marker.lock().unwrap() = Some(fixture.marker());

    let report = fixture.rotate(&proof).unwrap();

    // The marker held only public metadata, and it existed while the proof ran.
    let journal = proof.journal_at_proof.lock().unwrap().clone().unwrap();
    assert!(!journal
        .windows(fixture.previous_pkcs8.len())
        .any(|window| window == fixture.previous_pkcs8));
    let recorded: Journal = serde_json::from_slice(&journal).unwrap();
    assert_eq!(recorded.previous_public_key, fixture.previous_public_key);
    assert_eq!(
        recorded.public_key.as_deref(),
        Some(report.public_key.as_str())
    );

    // The three copies agree: the report, the file sqld reads, and the Keychain slot `signing_key`
    // mints from.
    assert_eq!(report.previous_public_key, fixture.previous_public_key);
    assert_ne!(report.public_key, fixture.previous_public_key);
    assert_eq!(
        report.public_key_path,
        mint::public_key_path(&fixture.paths).unwrap()
    );
    assert_eq!(fixture.public_key_file(), report.public_key);
    assert_eq!(fixture.active().public_key_base64url(), report.public_key);

    // The proof saw a token the previous key really signed, and one the new key signed.
    let claim = proof.claim.lock().unwrap();
    let claim = claim.as_ref().unwrap();
    assert_eq!(claim.public_key, report.public_key);
    assert_ne!(claim.old_token, claim.new_token);
    assert!(verify_old_probe(&claim.old_token, &fixture.previous_public_key, now()).is_ok());
    assert!(verify_old_probe(&claim.new_token, &report.public_key, now()).is_ok());
    assert!(!claim.old_token.contains(&fixture.previous_public_key));

    // The retained copy did its work and is gone; the marker with it.
    assert_eq!(report.previous_key_left, None);
    let (previous_id, _) = previous_reference().unwrap();
    assert_eq!(
        fixture
            .provider
            .status(&previous_id, &previous_reference().unwrap().1),
        Ok(Presence::Missing)
    );
    assert_eq!(*fixture.provider.deletes.lock().unwrap(), 1);
    assert!(!fixture.marker().exists());
    assert_eq!(fixture.journal(), None);
}

/// A surviving retained key is reported, not hidden. The rotation itself succeeded, so the inhibit
/// goes; the copy a person has to delete is named.
#[test]
fn a_previous_key_that_survives_the_delete_is_reported() {
    let fixture = Fixture::new();
    *fixture.provider.fail_delete.lock().unwrap() = true;
    let report = fixture.rotate(&RecordingProof::default()).unwrap();

    let reason = report.previous_key_left.as_deref().unwrap_or_default();
    assert!(reason.contains("still there"), "{reason}");
    assert!(!fixture.marker().exists());
    let (previous_id, previous_reference) = previous_reference().unwrap();
    assert_eq!(
        fixture.provider.status(&previous_id, &previous_reference),
        Ok(Presence::Present)
    );
}

// ---------------------------------------------------------------------------------------------
// Refusals, and what a failure leaves behind
// ---------------------------------------------------------------------------------------------

/// A failed proof keeps the inhibit, keeps the new key, and keeps the retained previous key: the
/// only state that can be finished deliberately.
#[test]
fn a_failed_proof_keeps_the_inhibit_and_never_restores_the_previous_key() {
    let fixture = Fixture::new();
    let proof = RecordingProof::default();
    *proof.fail_with.lock().unwrap() =
        Some("sqld rotation: the disposable sqld exited before it answered");

    let error = fixture.rotate(&proof).unwrap_err();
    assert!(error.contains("exited"), "{error}");

    assert!(fixture.marker().is_file());
    let journal = fixture.journal().unwrap();
    let new_public = fixture.active().public_key_base64url();
    assert_ne!(new_public, fixture.previous_public_key);
    assert_eq!(journal.public_key.as_deref(), Some(new_public.as_str()));
    assert_eq!(fixture.public_key_file(), new_public);
    // Retained, because the rejection is not proven yet; and the delete was never attempted.
    let (previous_id, previous_reference) = previous_reference().unwrap();
    assert_eq!(
        fixture.provider.status(&previous_id, &previous_reference),
        Ok(Presence::Present)
    );
    assert_eq!(*fixture.provider.deletes.lock().unwrap(), 0);
}

/// The file and the Keychain already disagreeing is a state a person has to look at, so nothing is
/// written at all.
#[test]
fn a_key_file_that_already_disagrees_with_the_keychain_is_refused_before_any_write() {
    let fixture = Fixture::new();
    let other = PrimaryKey::generate().unwrap();
    let preflight = Preflight {
        public_key: other.public_key_base64url(),
    };
    let proof = RecordingProof::default();
    let error = Rotation {
        paths: &fixture.paths,
        provider: &fixture.provider,
        proof: &proof,
    }
    .execute(&preflight)
    .unwrap_err();

    assert!(error.contains("already disagree"), "{error}");
    assert!(!fixture.marker().exists());
    assert_eq!(
        fixture.active().public_key_base64url(),
        fixture.previous_public_key
    );
    assert!(proof.claim.lock().unwrap().is_none());
}

/// A retained key that is already there may be an unfinished rotation. Refusing costs a run;
/// overwriting it loses the only copy of a key whose rejection was never proven.
#[test]
fn an_occupied_previous_reference_refuses_the_rotation() {
    let fixture = Fixture::new();
    let (previous_id, previous_reference) = previous_reference().unwrap();
    let stale = PrimaryKey::generate().unwrap();
    retain(
        &fixture.provider,
        &previous_id,
        previous_reference.clone(),
        &stale,
    )
    .unwrap();

    let error = fixture.rotate(&RecordingProof::default()).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    // The inhibit was written first, so this is a state `recover` can report.
    assert!(fixture.marker().is_file());
    assert_eq!(fixture.journal().unwrap().public_key, None);
    // The active key was not touched, and the occupied reference still holds what it held.
    assert_eq!(
        fixture.active().public_key_base64url(),
        fixture.previous_public_key
    );
    assert_eq!(
        fixture.provider.status(&previous_id, &previous_reference),
        Ok(Presence::Present)
    );
}

#[test]
fn a_recorded_rotation_refuses_another_one() {
    let fixture = Fixture::new();
    {
        let guard = rotation_guard::acquire(&fixture.paths).unwrap();
        let journal = Journal::start(&guard, &fixture.previous_public_key).unwrap();
        let other = PrimaryKey::generate().unwrap();
        journal
            .record_replacement(&guard, &other.public_key_base64url())
            .unwrap();
    }

    let error = fixture.rotate(&RecordingProof::default()).unwrap_err();
    assert!(error.contains("recover"), "{error}");
    assert_eq!(
        fixture.active().public_key_base64url(),
        fixture.previous_public_key
    );
    assert!(fixture.marker().is_file());
}

#[test]
fn the_journal_refuses_a_marker_it_did_not_write() {
    let fixture = Fixture::new();
    let guard = rotation_guard::acquire(&fixture.paths).unwrap();
    let journal = Journal::start(&guard, &fixture.previous_public_key).unwrap();
    assert!(journal.public_key.is_none());
    // A second start cannot overwrite the first, which is what keeps a crash from erasing the key
    // that was being replaced.
    assert!(Journal::start(&guard, &fixture.previous_public_key).is_err());
    assert_eq!(Journal::read(&guard).unwrap(), journal);

    std::fs::write(guard.marker(), "not a journal").unwrap();
    assert!(Journal::read(&guard).is_err());
}

// ---------------------------------------------------------------------------------------------
// The proof the command actually runs
// ---------------------------------------------------------------------------------------------

/// A sqld of its own, on a port it reserves, over a temporary database — the machinery behind the
/// `✓ proof` line. Skipped when the pinned sqld is not installed.
#[test]
fn the_disposable_proof_accepts_a_new_key_token_and_refuses_an_old_one() {
    if pinned_sqld().is_err() {
        eprintln!("skipped: sqld {PINNED_SQLD} is not installed");
        return;
    }
    let old = PrimaryKey::generate().unwrap();
    let new = PrimaryKey::generate().unwrap();
    let old_token = old.mint_for_days(None, Access::Write, PROBE_DAYS).unwrap();
    let new_token = new.mint_for_days(None, Access::Write, PROBE_DAYS).unwrap();
    let public_key = new.public_key_base64url();

    assert_eq!(
        DisposableSqld.prove(&Claim {
            public_key: &public_key,
            old_token: &old_token,
            new_token: &new_token,
        }),
        Ok(())
    );
}

// ---------------------------------------------------------------------------------------------
// `recover`
// ---------------------------------------------------------------------------------------------

/// Recover reports both keys and the file in between, and touches nothing: the inhibit is the one
/// thing it must not clear on a guess.
#[test]
fn recover_reports_the_journal_and_leaves_the_inhibit_in_place() {
    let fixture = Fixture::new();
    let new_public;
    {
        let guard = rotation_guard::acquire(&fixture.paths).unwrap();
        let journal = Journal::start(&guard, &fixture.previous_public_key).unwrap();
        new_public = PrimaryKey::generate().unwrap().public_key_base64url();
        journal.record_replacement(&guard, &new_public).unwrap();
        // The file still holds the previous key: exactly the ambiguity a person must resolve.
    }

    let lines = inspect(&fixture.paths).unwrap().unwrap().join("\n");
    assert!(lines.contains(&fixture.previous_public_key), "{lines}");
    assert!(lines.contains(&new_public), "{lines}");
    assert!(lines.contains("the key that was active"), "{lines}");
    assert!(lines.contains(PREVIOUS_ID), "{lines}");
    assert!(lines.contains("stays refused"), "{lines}");
    assert!(fixture.marker().is_file());
}

#[test]
fn recover_says_nothing_is_inhibited_when_no_journal_is_recorded() {
    let fixture = Fixture::new();
    assert_eq!(inspect(&fixture.paths), Ok(None));
}

#[test]
fn recover_refuses_a_marker_it_cannot_read() {
    let fixture = Fixture::new();
    std::fs::write(fixture.marker(), "not a journal").unwrap();
    let error = inspect(&fixture.paths).unwrap_err();
    assert!(error.contains("inhibit stands"), "{error}");
    assert!(fixture.marker().is_file());
}
