use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use documents::{Library, Stored};

#[test]
fn collections_group_documents_and_survive_restore_index() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "sjel-documents-collections-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let library = Library::open(&root.join("first.db"), root.join("library")).unwrap();
    let Stored::New(lease) = library.intake("lease.pdf", b"synthetic lease").unwrap() else {
        panic!("expected a new record")
    };
    let Stored::New(deposit) = library.intake("deposit.pdf", b"synthetic deposit").unwrap() else {
        panic!("expected a new record")
    };

    let flat = library.create_collection("Flat", Some("Housing")).unwrap();
    assert!(library.create_collection(" Flat ", None).is_err());
    library
        .update_collection("Flat", &[lease.id.clone(), deposit.id.clone()], &[])
        .unwrap();
    let flat = library
        .update_collection(&flat.id, &[], std::slice::from_ref(&deposit.id))
        .unwrap();
    assert_eq!(flat.member_ids, vec![lease.id.clone()]);
    assert!(library
        .update_collection("Flat", &["0".repeat(64)], &[])
        .is_err());
    assert_eq!(library.get(&lease.id).unwrap().unwrap().status, "inbox");

    let rebuilt = Library::open(&root.join("second.db"), root.join("library")).unwrap();
    rebuilt.restore_index().unwrap();
    assert_eq!(rebuilt.collections().unwrap(), vec![flat]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_list_reads_cached_values_that_match_the_full_record() {
    let root = std::env::temp_dir().join(format!("sjel-documents-listing-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let library = Library::open(&root.join("list.db"), root.join("library")).unwrap();
    for (name, body) in [("a.txt", "erste Rechnung"), ("b.txt", "zweite Notiz")] {
        library.intake(name, body.as_bytes()).unwrap();
    }
    assert_eq!(library.backfill_derived(100).unwrap(), 2);
    assert_eq!(library.backfill_derived(100).unwrap(), 0);
    let (rows, total) = library.list_rows(None, 0, 10).unwrap();
    assert_eq!(total, 2);
    for row in rows {
        assert!(row.document.extracted_text.is_none());
        let full = library.get(&row.document.id).unwrap().unwrap();
        assert_eq!(row.derived, documents::Derived::of(&full));
    }
    fs::remove_dir_all(root).unwrap();
}
