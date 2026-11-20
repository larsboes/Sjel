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

#[test]
fn confirmed_group_archives_every_member_except_the_survivor() {
    let (library, root) = scratch();
    let Stored::New(survivor) = library
        .intake("scan-a.txt", b"synthetic first scan")
        .unwrap()
    else {
        panic!("expected a new record")
    };
    let Stored::New(copy) = library
        .intake("scan-b.txt", b"synthetic second scan")
        .unwrap()
    else {
        panic!("expected a new record")
    };
    let mut member_ids = vec![survivor.id.clone(), copy.id.clone()];
    member_ids.sort();
    let decision = DuplicateDecision {
        candidate_key: format!("rescan:{}:{}", member_ids[0], member_ids[1]),
        kind: CandidateKind::Rescan,
        member_ids,
        action: DecisionAction::ConfirmGroup {
            survivor_id: survivor.id.clone(),
        },
    };

    assert_eq!(
        library.save_duplicate_decision(&decision).unwrap(),
        vec![copy.id.clone()]
    );

    assert_eq!(library.get(&survivor.id).unwrap().unwrap(), survivor);
    assert_eq!(library.get(&copy.id).unwrap().unwrap().status, "archived");
    assert_eq!(library.bytes(&copy.id).unwrap(), b"synthetic second scan");
    assert_eq!(
        library.absorbed_ids().unwrap(),
        std::iter::once(copy.id.clone()).collect()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_intake_of_known_bytes_reuses_the_existing_record() {
    let (library, root) = scratch();
    let Stored::New(first) = library.intake("invoice.pdf", b"synthetic invoice").unwrap() else {
        panic!("expected a new record")
    };
    let Stored::Unchanged(again) = library
        .intake_known("invoice-copy.pdf", b"synthetic invoice")
        .unwrap()
    else {
        panic!("known bytes must not create a record")
    };
    assert_eq!(again.id, first.id);
    assert_eq!(library.list(None).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn merge_copies_previews_then_keeps_one_record_per_exact_copy_group() {
    let (library, root) = scratch();
    for name in ["a.pdf", "b.pdf", "c.pdf"] {
        library.intake(name, b"synthetic shared bytes").unwrap();
    }
    library
        .intake("other.pdf", b"synthetic other bytes")
        .unwrap();
    let run = |args: &[&str]| {
        let mut out = Vec::new();
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        documents::commands::run(&library, &args, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };

    assert!(run(&["merge-copies"]).contains("1 exact-copy group(s); would archive 2 record(s)"));
    assert_eq!(library.absorbed_ids().unwrap().len(), 0);
    assert!(run(&["merge-copies", "--apply"]).contains("archived 2 record(s)"));
    let active = library
        .list(None)
        .unwrap()
        .into_iter()
        .filter(|d| d.status != "archived")
        .count();
    assert_eq!(active, 2);
    assert!(run(&["merge-copies"]).contains("0 exact-copy group(s)"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cached_candidates_follow_every_write() {
    let (library, root) = scratch();
    library.intake("a.pdf", b"synthetic same").unwrap();
    library.intake("b.pdf", b"synthetic same").unwrap();
    let first = library.duplicate_candidates().unwrap();
    assert_eq!(first.len(), 1);
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &library.duplicate_candidates().unwrap()
    ));
    library.intake("c.pdf", b"synthetic same").unwrap();
    let grown = library.duplicate_candidates().unwrap();
    assert_eq!(grown[0].member_ids.len(), 3);
    let survivor = grown[0].member_ids[0].clone();
    let decision = DuplicateDecision {
        candidate_key: grown[0].candidate_key.clone(),
        kind: CandidateKind::ExactCopy,
        member_ids: grown[0].member_ids.clone(),
        action: DecisionAction::ConfirmGroup {
            survivor_id: survivor,
        },
    };
    library.save_duplicate_decision(&decision).unwrap();
    assert!(library.duplicate_candidates().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}
