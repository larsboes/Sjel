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
