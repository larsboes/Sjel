//! The feed library reaching the vault, over a real store and a real folder.
//!
//! `src/projection.rs`'s unit tests build `FeedItem`s by hand, so they prove the
//! shape and the guards but not the query that finds the library. This one goes
//! through SQLite: `upsert_feed` → `set_feed_status` → `feed_library` →
//! `export_all`, which is the path a save actually takes, and it is the only place
//! `feed_library`'s SQL meets the schema.
//!
//! One `#[test]`, like `rung0_registry.rs` beside it, because the sequence is the
//! subject: save, save again, unsave. Splitting it would either share one database
//! between tests in one process or repeat the setup three times.

use comms::store::{FeedItem, Store};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "axon-comms-library-{}-{name}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("a writable temp directory");
    dir
}

fn saved_item(store: &Store, url: &str, kind: &str, title: &str, summary: &str) -> String {
    let mut item = FeedItem::new(url, "media", kind);
    item.title = Some(title.to_string());
    item.summary = Some(summary.to_string());
    store.upsert_feed(&item).expect("upsert");
    assert!(
        store
            .set_feed_status(&item.id, "keeper", "api")
            .expect("keep"),
        "the item must exist to be saved"
    );
    item.id
}

#[test]
fn saving_writes_a_note_unsaving_removes_it_and_a_human_note_survives_both() {
    let db = temp_dir("db").join("axon.db");
    let vault = temp_dir("vault");
    let store = Store::open(&db).expect("a store");
    let root = markdown_root::MarkdownRoot::declare(&vault).expect("a vault root");

    // An item nobody saved is not in the library, whatever else is true of it.
    let unsaved = FeedItem::new("https://example.test/unread", "media", "article");
    store.upsert_feed(&unsaved).expect("upsert");

    let sqlite = saved_item(
        &store,
        "https://example.test/sqlite",
        "youtube",
        "Reliability Lessons From SQLite",
        "Why SQLite tests the way it does.",
    );
    let ladybird = saved_item(
        &store,
        "https://example.test/ladybird",
        "github",
        "ladybird",
        "Truly independent web browser.",
    );

    let library = store.feed_library().expect("the library");
    assert_eq!(
        library.len(),
        2,
        "the library is the saved rows and nothing else"
    );

    let first = comms::projection::export_all(&root, &library).expect("export");
    assert_eq!((first.created, first.updated, first.unchanged), (2, 0, 0));
    let note = vault.join("Resources/Sources/ladybird.md");
    let body = std::fs::read_to_string(&note).expect("a projected note");
    assert!(body.contains("Truly independent web browser."), "{body}");
    assert!(
        body.contains("url: \"https://example.test/ladybird\""),
        "the note carries the link it was saved from: {body}"
    );
    assert!(
        body.contains(&format!("axon_feed_id: \"{ladybird}\"")),
        "the note names the row, which is also the argument `comms dismiss` takes"
    );
    assert_eq!(
        std::fs::read_dir(vault.join("Resources/Sources"))
            .expect("the folder")
            .count(),
        2,
        "two saved items, two notes — the unsaved item gets none"
    );

    // A second run over unchanged rows must open nothing for writing, or every
    // save produces a vault commit for files nobody changed.
    let second = comms::projection::export_all(&root, &library).expect("export");
    assert_eq!(
        (second.created, second.updated, second.unchanged),
        (0, 0, 2)
    );

    // A human writes their own note in the folder — the case Resources/Sjel never
    // had. It must survive the sweep untouched.
    let human = vault.join("Resources/Sources/Why SQLite is the one I trust.md");
    let human_bytes = "---\ntype: source\n---\n\nBecause of the test suite.\n";
    std::fs::write(&human, human_bytes).expect("a human note");

    // Unsaving: the row leaves the library, so the note goes with it.
    assert!(store
        .set_feed_status(&ladybird, "dismissed", "api")
        .expect("dismiss"));
    let library = store.feed_library().expect("the library");
    assert_eq!(library.len(), 1);
    let third = comms::projection::export_all(&root, &library).expect("export");
    assert_eq!(third.removed, vec!["Resources/Sources/ladybird.md"]);
    assert!(!note.exists(), "an unsaved item keeps no note");
    assert_eq!(
        std::fs::read_to_string(&human).expect("the human note"),
        human_bytes,
        "the sweep removes machine files only"
    );
    assert!(
        vault
            .join("Resources/Sources/Reliability Lessons From SQLite.md")
            .exists(),
        "the item still saved keeps its note"
    );

    // And a human note holding a saved item's name is refused rather than
    // overwritten: promotion, not a collision to work around.
    let taken = vault.join("Resources/Sources/Reliability Lessons From SQLite.md");
    std::fs::write(&taken, "# In my own words\n").expect("a human takeover");
    let fourth = comms::projection::export_all(&root, &library).expect("export");
    assert_eq!(
        fourth.refused,
        vec!["Resources/Sources/Reliability Lessons From SQLite.md"]
    );
    assert_eq!(fourth.created + fourth.updated, 0);
    assert_eq!(
        std::fs::read_to_string(&taken).expect("the human takeover"),
        "# In my own words\n"
    );

    // Unsaving that item does not delete the human's note either.
    assert!(store
        .set_feed_status(&sqlite, "new", "api")
        .expect("unsave"));
    let library = store.feed_library().expect("the library");
    assert!(library.is_empty());
    let fifth = comms::projection::export_all(&root, &library).expect("export");
    assert!(fifth.removed.is_empty(), "nothing left of ours to remove");
    assert!(taken.exists(), "a refused path is never deleted either");

    std::fs::remove_dir_all(&vault).ok();
    std::fs::remove_dir_all(db.parent().unwrap()).ok();
}
