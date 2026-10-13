//! The one-time move out of `interior_item`, from a file in the old German schema: rows, state
//! history and placements arrive, the column names become English, and the old table is gone.
//!
//! Placements are why this exists: `interior_placement` cascades on `interior_item`, so dropping
//! the parent without rebuilding the child first would lose them silently.

use rusqlite::Connection;
use sjel_inventory::store::{Item, Kind, State, Store};
use std::path::Path;

/// The table as it lay on disk until 2026-10-05: German columns, under `interior_item`, with the
/// foreign key in `interior_placement`.
fn file_before_the_move(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE interior_item (
            id                 TEXT PRIMARY KEY,
            kind               TEXT NOT NULL CHECK (kind IN ('piece','slot','gear')),
            label              TEXT NOT NULL,
            b                  INTEGER,
            t                  INTEGER,
            h                  INTEGER,
            h_min              INTEGER,
            b_aufgeklappt      INTEGER,
            t_ausgeklappt      INTEGER,
            laenge             INTEGER,
            anzahl             INTEGER,
            zustaende          TEXT NOT NULL DEFAULT '[]',
            unsicher           TEXT NOT NULL DEFAULT '[]',
            platzbedarf_zone   INTEGER,
            platzbedarf_block  INTEGER,
            preis_cent         INTEGER,
            kosten_min_cent    INTEGER,
            kosten_max_cent    INTEGER,
            link               TEXT,
            artikelnummer      TEXT,
            quelle             TEXT,
            gemessen_am        TEXT,
            mitnahme           TEXT,
            prioritaet         TEXT,
            basiert_auf        TEXT,
            ersetzt            TEXT NOT NULL DEFAULT '[]',
            varianten          TEXT NOT NULL DEFAULT '[]',
            ziel               TEXT,
            hinweis            TEXT,
            begruendung        TEXT,
            entscheidung_offen TEXT,
            opens              TEXT,
            open_clear         INTEGER,
            wall_ok            INTEGER,
            expands_dir        TEXT,
            expands_to         INTEGER,
            access_sides       INTEGER,
            access_clear       INTEGER,
            raumtrenner        INTEGER,
            zerlegbar          INTEGER,
            bild               TEXT,
            weight_g           INTEGER,
            category           TEXT,
            packable           INTEGER,
            waterproof         INTEGER,
            quick_dry          INTEGER,
            pack_location      TEXT,
            trip_types         TEXT NOT NULL DEFAULT '[]',
            groesse            TEXT,
            farbe              TEXT,
            saison             TEXT NOT NULL DEFAULT '[]',
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            revision           INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE interior_item_state (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL REFERENCES interior_item(id) ON DELETE CASCADE,
            state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
            since   TEXT NOT NULL,
            note    TEXT
         );
         CREATE TABLE interior_placement (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL REFERENCES interior_item(id) ON DELETE CASCADE,
            flat    TEXT NOT NULL,
            x       INTEGER NOT NULL,
            y       INTEGER NOT NULL,
            rot     INTEGER NOT NULL DEFAULT 0,
            since   TEXT NOT NULL,
            UNIQUE (item_id, flat)
         );
         INSERT INTO interior_item
            (id, kind, label, b, t, h, category, groesse, saison, preis_cent, revision,
             created_at, updated_at)
            VALUES ('hemd', 'piece', 'Hemd', NULL, NULL, NULL, 'kleidung', 'M', '[\"ganzjahr\"]',
                    7990, 7, '2026-01-01', '2026-02-02');
         INSERT INTO interior_item
            (id, kind, label, b, t, h, created_at, updated_at)
            VALUES ('regal', 'piece', 'Regal', 80, 30, 200, '2026-01-01', '2026-01-01');
         INSERT INTO interior_item_state (item_id, state, since, note)
            VALUES ('hemd', 'wanted', '2026-01-02', 'aus wishlist.toml importiert');
         INSERT INTO interior_item_state (item_id, state, since, note)
            VALUES ('hemd', 'owned', '2026-03-04', 'gekauft');
         INSERT INTO interior_item_state (item_id, state, since, note)
            VALUES ('regal', 'owned', '2026-01-02', 'gekauft');
         INSERT INTO interior_placement (item_id, flat, x, y, since)
            VALUES ('regal', 'wohnung', 10, 20, '2026-01-03');",
    )
    .unwrap();
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("inventory-move-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn the_move_keeps_rows_state_and_placement() {
    let path = temp_file("full");
    file_before_the_move(&path);

    let store = Store::open(&path).unwrap();

    let (shirt, state) = store.item("hemd").unwrap().expect("the shirt survives");
    assert_eq!(shirt.label, "Hemd");
    assert_eq!(shirt.category.as_deref(), Some("kleidung"));
    assert_eq!(shirt.size.as_deref(), Some("M"));
    assert_eq!(shirt.seasons, vec!["ganzjahr".to_string()]);
    assert_eq!(shirt.price_cents, Some(7990));
    // The revision moves too, so a phone holding a read row sees no false conflict.
    assert_eq!(shirt.revision, 7);
    assert_eq!(state, Some(State::Owned));

    let (shelf, _) = store.item("regal").unwrap().expect("the shelf survives");
    assert_eq!(
        (shelf.width, shelf.depth, shelf.height),
        (Some(80), Some(30), Some(200))
    );

    let history = store.state_history("hemd").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].0, State::Wanted);
    assert_eq!(history[1].0, State::Owned);
    assert_eq!(history[1].1, "2026-03-04");

    // Read from the table directly: `interior_placement` belongs to interior.
    let conn = Connection::open(&path).unwrap();
    let placements: Vec<(String, String, i32, i32)> = {
        let mut stmt = conn
            .prepare("SELECT item_id, flat, x, y FROM interior_placement ORDER BY item_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(placements.len(), 1, "the placement survived the DROP");
    assert_eq!(placements[0].0, "regal");
    assert_eq!(placements[0].1, "wohnung");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_old_table_is_gone_afterwards() {
    let path = temp_file("gone");
    file_before_the_move(&path);
    let _ = Store::open(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let old: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('interior_item','interior_item_state')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 0, "interior_item and interior_item_state are dropped");
    let new: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='inventory_item'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new, 1);

    let _ = std::fs::remove_file(&path);
}

/// On purpose: a foreign key across two prefixes would make one capability's deletes fail on the
/// other's rows.
#[test]
fn the_placement_has_no_foreign_key_anymore() {
    let path = temp_file("fk");
    file_before_the_move(&path);
    let _ = Store::open(&path).unwrap();

    let conn = Connection::open(&path).unwrap();
    let ddl: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='interior_placement'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!ddl.contains("REFERENCES"), "no FK anymore: {ddl}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_second_start_moves_nothing_again() {
    let path = temp_file("idempotent");
    file_before_the_move(&path);
    let store = Store::open(&path).unwrap();
    store
        .upsert_item(&Item {
            id: "mantel".into(),
            kind: Kind::Piece,
            label: "Mantel".into(),
            ..Item::default()
        })
        .unwrap();
    drop(store);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.item_count().unwrap(), 3);
    assert!(store.item("mantel").unwrap().is_some());
    assert_eq!(store.state_history("hemd").unwrap().len(), 2);

    let _ = std::fs::remove_file(&path);
}

/// A file without the gear and clothing columns: only shared columns are copied, the rest get
/// their default.
#[test]
fn an_older_file_loses_nothing_in_the_move() {
    let path = temp_file("old");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE interior_item (
            id                 TEXT PRIMARY KEY,
            kind               TEXT NOT NULL CHECK (kind IN ('piece','slot')),
            label              TEXT NOT NULL,
            b                  INTEGER,
            t                  INTEGER,
            h                  INTEGER,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL
         );
         CREATE TABLE interior_item_state (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL REFERENCES interior_item(id) ON DELETE CASCADE,
            state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
            since   TEXT NOT NULL,
            note    TEXT
         );
         INSERT INTO interior_item (id, kind, label, b, t, h, created_at, updated_at)
             VALUES ('regal', 'piece', 'Regal', 80, 30, 200, '2026-01-01', '2026-01-01');
         INSERT INTO interior_item_state (item_id, state, since, note)
             VALUES ('regal', 'owned', '2026-01-02', 'gekauft');",
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).unwrap();
    let (shelf, state) = store.item("regal").unwrap().expect("the shelf survives");
    assert_eq!(
        (shelf.width, shelf.depth, shelf.height),
        (Some(80), Some(30), Some(200))
    );
    assert_eq!(state, Some(State::Owned));
    assert_eq!(shelf.category, None);
    assert_eq!(shelf.seasons, Vec::<String>::new());
    assert_eq!(shelf.revision, 1);

    let _ = std::fs::remove_file(&path);
}
