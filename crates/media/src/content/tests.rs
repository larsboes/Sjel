use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=";
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    root: PathBuf,
    ledger: Arc<Ledger>,
    library: Library,
}

impl Scratch {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let root = loop {
            let candidate = parent.join(format!(
                "media-content-{}-{}",
                std::process::id(),
                NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new().mode(0o700).create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        };
        let ledger = Arc::new(Ledger::open(&root.join("ledger.db")).unwrap());
        let library = Library::open(ledger.clone(), root.join("originals")).unwrap();
        Self {
            root,
            ledger,
            library,
        }
    }

    fn upload(&self, name: &str, bytes: &[u8]) -> String {
        let digest = crate::hex_digest(&Sha256::digest(bytes));
        assert!(self.library.upload(name, &digest, bytes).unwrap().1);
        digest
    }

    fn stage(&self, items: Vec<Value>) -> Result<ContentStageResult> {
        let path = self.root.join("synthetic-report.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "formatVersion": 1,
                "engine": ENGINE,
                "os": "Synthetic OS",
                "revision": 1,
                "advisory": "Candidates require explicit review",
                "items": items,
            }))
            .unwrap(),
        )
        .unwrap();
        stage(&self.ledger, &self.library, &path)
    }

    fn all(&self) -> Vec<ContentProposal> {
        let page = list(&self.ledger, "all", 0).unwrap();
        assert!(page.next_after.is_none());
        page.items
    }

    fn assert_original(&self, digest: &str, bytes: &[u8]) {
        assert_eq!(
            fs::read(self.root.join("originals").join(digest)).unwrap(),
            bytes
        );
        assert!(self.library.holding(digest).unwrap().is_some());
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned scratch directory");
    }
}

fn image(digest: &str, score: f64) -> Value {
    json!({
        "path": "untrusted-source-reference.png",
        "sha256": digest,
        "kind": "image",
        "frames": [{
            "requestedSeconds": null,
            "seconds": null,
            "labels": [{ "identifier": "raw_classifier_animal", "score": score }],
            "previewPng": PNG,
        }],
        "error": null,
    })
}

fn decision(
    proposal: &ContentProposal,
    action: ContentAction,
    tags: &[&str],
    collections: &[&str],
) -> ContentDecision {
    ContentDecision {
        fingerprint: proposal.fingerprint.clone(),
        action,
        tags: tags.iter().map(|v| (*v).into()).collect(),
        collections: collections.iter().map(|v| (*v).into()).collect(),
    }
}

#[test]
fn replay_is_idempotent_and_preserves_rejection() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    let item = image(&digest, 0.8);
    assert_eq!(
        scratch.stage(vec![item.clone()]).unwrap(),
        ContentStageResult {
            inserted: 1,
            unchanged: 0,
            failed_items: 0,
        }
    );
    assert_eq!(
        scratch.stage(vec![item.clone()]).unwrap(),
        ContentStageResult {
            inserted: 0,
            unchanged: 1,
            failed_items: 0,
        }
    );
    let proposal = scratch.all().pop().unwrap();
    assert_eq!(proposal.state, ContentState::Pending);
    assert!(proposal.tags.is_empty());
    let reject = decision(&proposal, ContentAction::Reject, &[], &[]);
    let rejected = decide(&scratch.ledger, &scratch.library, proposal.id, &reject).unwrap();
    assert_eq!(rejected.state, ContentState::Rejected);
    assert_eq!(scratch.stage(vec![item]).unwrap().unchanged, 1);
    assert_eq!(scratch.all(), vec![rejected.clone()]);
    assert_eq!(
        decide(&scratch.ledger, &scratch.library, proposal.id, &reject).unwrap(),
        rejected
    );
    assert!(list(&scratch.ledger, "pending", 0)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(list(&scratch.ledger, "rejected", 0).unwrap().items.len(), 1);
    let accept = decision(&proposal, ContentAction::Accept, &["Chosen name"], &[]);
    assert!(decide(&scratch.ledger, &scratch.library, proposal.id, &accept).is_err());
    scratch.assert_original(&digest, b"synthetic original");
}

#[test]
fn acceptance_uses_chosen_names_and_normalized_retries_refuse_conflicts() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    scratch.stage(vec![image(&digest, 0.8)]).unwrap();
    let proposal = scratch.all().pop().unwrap();
    let accept = decision(
        &proposal,
        ContentAction::Accept,
        &["Wildlife", "Family", "Wildlife"],
        &["Trip", "Album"],
    );
    let accepted = decide(&scratch.ledger, &scratch.library, proposal.id, &accept).unwrap();
    assert_eq!(accepted.state, ContentState::Accepted);
    assert_eq!(accepted.tags, ["Family", "Wildlife"]);
    assert_eq!(accepted.collections, ["Album", "Trip"]);
    assert!(!accepted
        .tags
        .iter()
        .chain(&accepted.collections)
        .any(|v| v == "raw_classifier_animal"));
    let retry = decision(
        &proposal,
        ContentAction::Accept,
        &["Family", "Wildlife"],
        &["Album", "Trip", "Album"],
    );
    assert_eq!(
        decide(&scratch.ledger, &scratch.library, proposal.id, &retry).unwrap(),
        accepted
    );
    for conflict in [
        decision(
            &proposal,
            ContentAction::Accept,
            &["Other"],
            &["Album", "Trip"],
        ),
        decision(
            &proposal,
            ContentAction::Accept,
            &["Family", "Wildlife"],
            &["Other"],
        ),
        decision(&proposal, ContentAction::Reject, &[], &[]),
    ] {
        assert!(decide(&scratch.ledger, &scratch.library, proposal.id, &conflict).is_err());
        assert_eq!(scratch.all(), vec![accepted.clone()]);
    }
    let detail = detail(&scratch.ledger, proposal.id).unwrap().unwrap();
    assert_eq!(detail.filenames, ["fixture.png"]);
    assert_eq!(
        detail.frames[0].labels[0].identifier,
        "raw_classifier_animal"
    );
    assert_eq!(
        list(&scratch.ledger, "accepted", 0).unwrap().items,
        vec![accepted]
    );
    scratch.assert_original(&digest, b"synthetic original");
}

#[test]
fn changed_evidence_starts_pending_and_wrong_fingerprints_do_not_decide_it() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    scratch.stage(vec![image(&digest, 0.8)]).unwrap();
    let old = scratch.all().pop().unwrap();
    let accept = decision(
        &old,
        ContentAction::Accept,
        &["Chosen name"],
        &["Chosen collection"],
    );
    decide(&scratch.ledger, &scratch.library, old.id, &accept).unwrap();
    assert_eq!(
        scratch.stage(vec![image(&digest, 0.9)]).unwrap().inserted,
        1
    );
    let pending = list(&scratch.ledger, "pending", 0)
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_ne!(pending.id, old.id);
    assert_ne!(pending.fingerprint, old.fingerprint);
    assert!(pending.tags.is_empty());
    assert!(pending.collections.is_empty());
    for fingerprint in [old.fingerprint, "0".repeat(64), "not-a-digest".into()] {
        let mut stale = decision(&pending, ContentAction::Accept, &["Wrong"], &[]);
        stale.fingerprint = fingerprint;
        assert!(decide(&scratch.ledger, &scratch.library, pending.id, &stale).is_err());
        assert_eq!(
            detail(&scratch.ledger, pending.id)
                .unwrap()
                .unwrap()
                .proposal,
            pending
        );
    }
    assert_eq!(scratch.all().len(), 2);
    scratch.assert_original(&digest, b"synthetic original");
}

#[test]
fn invalid_evidence_rolls_back_valid_first_item() {
    let scratch = Scratch::new();
    let first = scratch.upload("first.png", b"first synthetic original");
    let second = scratch.upload("second.png", b"second synthetic original");
    let valid = image(&first, 0.8);
    let base = image(&second, 0.8);
    let mut invalid = Vec::new();
    for identifier in ["", " padded", "line\nbreak"] {
        let mut item = base.clone();
        item["frames"][0]["labels"][0]["identifier"] = json!(identifier);
        invalid.push(item);
    }
    for score in [-0.1, 1.1] {
        let mut item = base.clone();
        item["frames"][0]["labels"][0]["score"] = json!(score);
        invalid.push(item);
    }
    let mut duplicate = base.clone();
    duplicate["frames"][0]["labels"] = json!([
        {"identifier": "same", "score": 0.8}, {"identifier": "same", "score": 0.7}
    ]);
    invalid.push(duplicate);
    for preview in ["%%%".to_owned(), STANDARD.encode(b"not a PNG")] {
        let mut item = base.clone();
        item["frames"][0]["previewPng"] = json!(preview);
        invalid.push(item);
    }
    for (width, height) in [(0u32, 1u32), (257, 1), (1, 257)] {
        let mut png = STANDARD.decode(PNG).unwrap();
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let mut item = base.clone();
        item["frames"][0]["previewPng"] = json!(STANDARD.encode(png));
        invalid.push(item);
    }
    let mut video = base.clone();
    video["kind"] = json!("video");
    let mut frame = video["frames"][0].clone();
    frame["seconds"] = json!(0.0);
    frame["requestedSeconds"] = json!(0.1);
    video["frames"] = json!([frame.clone(), frame.clone(), frame]);
    invalid.push(video);
    let mut unknown = base;
    unknown["sha256"] = json!("0".repeat(64));
    invalid.push(unknown);
    for (index, item) in invalid.into_iter().enumerate() {
        assert!(
            scratch.stage(vec![valid.clone(), item]).is_err(),
            "invalid case {index}"
        );
        assert!(scratch.all().is_empty(), "partial insert in case {index}");
        scratch.assert_original(&first, b"first synthetic original");
        scratch.assert_original(&second, b"second synthetic original");
    }
}

#[test]
fn corrupt_original_prevents_atomic_staging_and_acceptance_without_repairing_bytes() {
    let scratch = Scratch::new();
    let first = scratch.upload("first.png", b"first synthetic original");
    let second = scratch.upload("second.png", b"second synthetic original");
    let path = scratch.root.join("originals").join(&second);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, b"corrupt synthetic bytes!!").unwrap();
    assert!(scratch
        .stage(vec![image(&first, 0.8), image(&second, 0.8)])
        .is_err());
    assert!(scratch.all().is_empty());
    scratch.stage(vec![image(&first, 0.8)]).unwrap();
    let pending = scratch.all().pop().unwrap();
    let first_path = scratch.root.join("originals").join(&first);
    fs::set_permissions(&first_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&first_path, b"corrupt first original!!").unwrap();
    let accept = decision(&pending, ContentAction::Accept, &["Chosen"], &[]);
    assert!(decide(&scratch.ledger, &scratch.library, pending.id, &accept).is_err());
    assert_eq!(scratch.all(), vec![pending]);
    assert_eq!(fs::read(path).unwrap(), b"corrupt synthetic bytes!!");
    assert_eq!(fs::read(first_path).unwrap(), b"corrupt first original!!");
}

#[test]
fn native_failures_are_counted_but_cannot_smuggle_success_evidence() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    let failure = json!({
        "path": "synthetic-failed.mov", "sha256": null, "kind": null,
        "frames": [], "error": "synthetic decoder failure",
    });
    assert_eq!(
        scratch.stage(vec![failure.clone()]).unwrap(),
        ContentStageResult {
            inserted: 0,
            unchanged: 0,
            failed_items: 1,
        }
    );
    assert!(scratch.all().is_empty());
    let mut mixed = failure.clone();
    mixed["sha256"] = json!(&digest);
    assert!(scratch.stage(vec![image(&digest, 0.8), mixed]).is_err());
    assert!(scratch.all().is_empty());
    assert_eq!(
        scratch.stage(vec![image(&digest, 0.8), failure]).unwrap(),
        ContentStageResult {
            inserted: 1,
            unchanged: 0,
            failed_items: 1,
        }
    );
    assert_eq!(scratch.all().len(), 1);
    scratch.assert_original(&digest, b"synthetic original");
}

#[test]
fn invalid_chosen_names_and_empty_acceptance_leave_proposal_pending() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    scratch.stage(vec![image(&digest, 0.8)]).unwrap();
    let pending = scratch.all().pop().unwrap();
    let mut invalid = vec![
        decision(&pending, ContentAction::Accept, &[], &[]),
        decision(&pending, ContentAction::Reject, &["Unwanted"], &[]),
        decision(&pending, ContentAction::Reject, &[], &["Unwanted"]),
    ];
    for name in [
        "".to_owned(),
        " padded".into(),
        "line\nbreak".into(),
        "x".repeat(129),
    ] {
        invalid.push(decision(&pending, ContentAction::Accept, &[&name], &[]));
        invalid.push(decision(&pending, ContentAction::Accept, &[], &[&name]));
    }
    let mut too_many = decision(&pending, ContentAction::Accept, &[], &[]);
    too_many.tags = (0..33).map(|n| format!("tag-{n}")).collect();
    invalid.push(too_many);
    for request in invalid {
        assert!(decide(&scratch.ledger, &scratch.library, pending.id, &request).is_err());
        assert_eq!(scratch.all(), vec![pending.clone()]);
    }
    scratch.assert_original(&digest, b"synthetic original");
}

#[test]
fn distinct_video_samples_preserve_requested_and_actual_times() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.mov", b"synthetic video original");
    let mut item = image(&digest, 0.8);
    item["kind"] = json!("video");
    let frames: Vec<_> = [1.0, 2.0, 3.0]
        .into_iter()
        .map(|seconds| {
            let mut frame = item["frames"][0].clone();
            frame["requestedSeconds"] = json!(seconds);
            frame["seconds"] = json!(seconds + 0.1);
            frame
        })
        .collect();
    item["frames"] = json!(frames);
    assert_eq!(scratch.stage(vec![item]).unwrap().inserted, 1);
    let proposal = scratch.all().pop().unwrap();
    assert_eq!(proposal.kind, "video");
    let evidence = detail(&scratch.ledger, proposal.id).unwrap().unwrap();
    assert_eq!(evidence.frames.len(), 3);
    for (frame, seconds) in evidence.frames.iter().zip([1.0, 2.0, 3.0]) {
        assert_eq!(frame.requested_seconds, Some(seconds));
        assert_eq!(frame.seconds, Some(seconds + 0.1));
    }
    scratch.assert_original(&digest, b"synthetic video original");
}

#[test]
fn queue_paginates_without_duplicates_and_validates_filters() {
    let scratch = Scratch::new();
    let digest = scratch.upload("fixture.png", b"synthetic original");
    for n in 0..51 {
        assert_eq!(
            scratch
                .stage(vec![image(&digest, f64::from(n) / 100.0)])
                .unwrap()
                .inserted,
            1
        );
    }
    let first = list(&scratch.ledger, "pending", 0).unwrap();
    assert_eq!(first.items.len(), 50);
    assert!(first.items.windows(2).all(|pair| pair[0].id < pair[1].id));
    let cursor = first.next_after.unwrap();
    assert_eq!(cursor, first.items.last().unwrap().id);
    let second = list(&scratch.ledger, "pending", cursor).unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.items[0].id > cursor);
    assert!(second.next_after.is_none());
    assert!(list(&scratch.ledger, "pending", second.items[0].id)
        .unwrap()
        .items
        .is_empty());
    assert!(list(&scratch.ledger, "accepted", 0)
        .unwrap()
        .items
        .is_empty());
    assert!(list(&scratch.ledger, "bogus", 0).is_err());
    assert!(list(&scratch.ledger, "all", -1).is_err());
    assert!(detail(&scratch.ledger, i64::MAX).unwrap().is_none());
    scratch.assert_original(&digest, b"synthetic original");
}
