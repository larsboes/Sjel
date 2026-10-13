//! An entry's revision and writing against it. Phone and Mac edit the same entries; with a
//! revision the second writer is refused and gets the current row. Checked against a real SQLite
//! file, because compare-and-write in one statement does not exist in a mock.

use sjel_inventory::store::{Item, Kind, Store, WriteOutcome};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn temp_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "inventory-revision-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn wardrobe(label: &str) -> Item {
    Item {
        id: "wardrobe".into(),
        kind: Kind::Piece,
        label: label.into(),
        width: Some(100),
        ..Item::default()
    }
}

#[test]
fn every_write_raises_the_revision() {
    let path = temp_file("counts");
    let store = Store::open(&path).unwrap();

    assert_eq!(store.upsert_item(&wardrobe("Wardrobe")).unwrap(), 1);
    assert_eq!(store.upsert_item(&wardrobe("Wardrobe")).unwrap(), 2);
    let (read, _) = store.item("wardrobe").unwrap().unwrap();
    assert_eq!(read.revision, 2);

    // A revision in the body is never written: it belongs to the server.
    let mut wrong = wardrobe("Wardrobe");
    wrong.revision = 99;
    assert_eq!(store.upsert_item(&wrong).unwrap(), 3);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_matching_revision_writes_and_a_stale_one_does_not() {
    let path = temp_file("cas");
    let store = Store::open(&path).unwrap();
    store.upsert_item(&wardrobe("old")).unwrap();

    match store.update_item_if_revision(&wardrobe("new"), 1).unwrap() {
        WriteOutcome::Written(r) => assert_eq!(r, 2),
        other => panic!("expected written, got {other:?}"),
    }
    match store.update_item_if_revision(&wardrobe("too late"), 1).unwrap() {
        WriteOutcome::Stale(current, _) => {
            assert_eq!(current.revision, 2);
            assert_eq!(current.label, "new", "the stale writer changed nothing");
        }
        other => panic!("expected stale, got {other:?}"),
    }
    let mut missing = wardrobe("x");
    missing.id = "does-not-exist".into();
    assert!(matches!(
        store.update_item_if_revision(&missing, 1).unwrap(),
        WriteOutcome::Missing
    ));
    assert!(
        store.item("does-not-exist").unwrap().is_none(),
        "a conditional write creates nothing"
    );

    let _ = std::fs::remove_file(&path);
}

/// Two devices read revision 1 and write at once. Each writer has its own `Store` and pool, like
/// two processes; the barrier releases all together, over several rounds so a lucky sequential
/// run does not pass as proof.
#[test]
fn of_concurrent_writers_with_one_revision_exactly_one_wins() {
    const WRITERS: usize = 8;
    const ROUNDS: usize = 5;
    let path = temp_file("race");
    Store::open(&path)
        .unwrap()
        .upsert_item(&wardrobe("start"))
        .unwrap();

    for round in 0..ROUNDS {
        let read = Store::open(&path)
            .unwrap()
            .item("wardrobe")
            .unwrap()
            .unwrap()
            .0
            .revision;
        let barrier = Arc::new(Barrier::new(WRITERS));
        let threads: Vec<_> = (0..WRITERS)
            .map(|n| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = Store::open(&path).unwrap();
                    let label = format!("round {round} writer {n}");
                    barrier.wait();
                    let outcome = store
                        .update_item_if_revision(&wardrobe(&label), read)
                        .unwrap();
                    (label, outcome)
                })
            })
            .collect();
        let outcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();

        let winners: Vec<_> = outcomes
            .iter()
            .filter_map(|(label, o)| match o {
                WriteOutcome::Written(r) => Some((label.clone(), *r)),
                _ => None,
            })
            .collect();
        assert_eq!(winners.len(), 1, "round {round}: {outcomes:?}");
        let (winner, revision) = &winners[0];
        assert_eq!(*revision, read + 1);
        for (_, o) in &outcomes {
            match o {
                WriteOutcome::Written(_) => {}
                WriteOutcome::Stale(current, _) => {
                    assert_eq!(current.revision, read + 1);
                    assert_eq!(&current.label, winner);
                }
                WriteOutcome::Missing => panic!("round {round}: the entry is missing"),
            }
        }
        let (after, _) = Store::open(&path)
            .unwrap()
            .item("wardrobe")
            .unwrap()
            .unwrap();
        assert_eq!(&after.label, winner);
        assert_eq!(after.revision, read + 1);
    }

    let _ = std::fs::remove_file(&path);
}
