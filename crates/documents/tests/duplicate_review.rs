use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use documents::{
    review::{CandidateKind, DecisionAction, DuplicateDecision},
    Library, Stored,
};

fn scratch() -> (Library, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "sjel-documents-duplicate-review-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    (
        Library::open(&root.join("test.db"), root.join("library")).unwrap(),
        root,
    )
}

#[test]
fn decision_persists_without_changing_document_state_or_original_bytes() {
    let (library, root) = scratch();
    let Stored::New(document) = library.intake("source.txt", b"synthetic original").unwrap() else {
        panic!("expected a new record")
    };
    let decision = DuplicateDecision {
        candidate_key: format!("exact_copy:{}", document.sha256),
        kind: CandidateKind::ExactCopy,
        member_ids: vec![document.id.clone()],
        action: DecisionAction::ConfirmGroup {
            survivor_id: document.id.clone(),
        },
    };

    library.save_duplicate_decision(&decision).unwrap();
    library.save_duplicate_decision(&decision).unwrap();

    assert_eq!(library.duplicate_decisions().unwrap(), vec![decision]);
    assert_eq!(library.get(&document.id).unwrap().unwrap(), document);
    assert_eq!(library.bytes(&document.id).unwrap(), b"synthetic original");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn decision_for_a_missing_record_is_rejected_atomically() {
    let (library, root) = scratch();
    let missing = "a".repeat(64);
    let decision = DuplicateDecision {
        candidate_key: "rescan:stale".into(),
        kind: CandidateKind::Rescan,
        member_ids: vec![missing.clone()],
        action: DecisionAction::ConfirmGroup {
            survivor_id: missing,
        },
    };

    assert!(library.save_duplicate_decision(&decision).is_err());
    assert!(library.duplicate_decisions().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}
