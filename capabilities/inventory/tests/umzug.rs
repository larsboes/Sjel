//! Der Umzug aus `interior_item` (ISA F13, 2026-10-05).
//!
//! Was hier geprueft wird, ist nicht "gibt es die Spalten", sondern: **eine Datei, die noch
//! `interior_item` heisst, kommt mit ihren Zeilen, ihrer Zustandsgeschichte und ihren
//! Platzierungen herueber — und die alte Tabelle ist danach weg.**
//!
//! Die Platzierungen sind der eigentliche Grund fuer diesen Test. `interior_placement` zeigt mit
//! `ON DELETE CASCADE` auf `interior_item`; ein `DROP` der Elterntabelle nimmt sie mit, und
//! `libs/sjel-store` schaltet Fremdschluessel auf jeder Verbindung ein. Der Umzug baut die
//! Kindtabelle deshalb vorher ohne Fremdschluessel neu. Ohne diesen Test waere der Verlust
//! still: die Zeilen sind weg, und niemand hat eine Zahl, gegen die er es merkt.

use inventory::store::{Item, Kind, State, Store};
use rusqlite::Connection;
use std::path::Path;

/// Die Form, in der die Tabelle bis 2026-10-05 auf der Platte lag: unter `interior_item`, mit
/// dem Fremdschluessel in `interior_placement` und mit einem Zustand und einer Platzierung.
fn datei_vor_dem_umzug(pfad: &Path) {
    let conn = Connection::open(pfad).unwrap();
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

fn tempdatei(name: &str) -> std::path::PathBuf {
    let pfad =
        std::env::temp_dir().join(format!("inventory-umzug-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&pfad);
    pfad
}

#[test]
fn der_umzug_haelt_zeilen_zustand_und_platzierung() {
    let pfad = tempdatei("voll");
    datei_vor_dem_umzug(&pfad);

    let store = Store::open(&pfad).unwrap();

    // Die Zeilen sind da, mit allem, was sie trugen.
    let (hemd, zustand) = store.item("hemd").unwrap().expect("Hemd ueberlebt");
    assert_eq!(hemd.label, "Hemd");
    assert_eq!(hemd.category.as_deref(), Some("kleidung"));
    assert_eq!(hemd.groesse.as_deref(), Some("M"));
    assert_eq!(hemd.saison, vec!["ganzjahr".to_string()]);
    assert_eq!(hemd.preis_cent, Some(7990));
    // Die Revision gehoert dem Server und wandert mit: ein Telefon mit einem gelesenen Stand
    // soll nicht einmal grundlos einen Konflikt sehen (PRD §10 A5).
    assert_eq!(hemd.revision, 7);
    assert_eq!(zustand, Some(State::Owned));

    let (regal, _) = store.item("regal").unwrap().expect("Regal ueberlebt");
    assert_eq!((regal.b, regal.t, regal.h), (Some(80), Some(30), Some(200)));

    // Die Zustandsgeschichte ist eine Geschichte und keine Zeile: zwei Eintraege, in der
    // Reihenfolge, in der sie geschehen sind.
    let verlauf = store.state_history("hemd").unwrap();
    assert_eq!(verlauf.len(), 2);
    assert_eq!(verlauf[0].0, State::Wanted);
    assert_eq!(verlauf[1].0, State::Owned);
    assert_eq!(verlauf[1].1, "2026-03-04");

    // Und die Platzierung — der Grund, aus dem dieser Test existiert. Sie wird direkt aus der
    // Tabelle gelesen und nicht ueber diesen Store: `interior_placement` gehoert
    // `capabilities/interior`, und diese Capability fasst sie nur im Umzug an.
    let conn = Connection::open(&pfad).unwrap();
    let platz: Vec<(String, String, i32, i32)> = {
        let mut stmt = conn
            .prepare("SELECT item_id, flat, x, y FROM interior_placement ORDER BY item_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    };
    assert_eq!(platz.len(), 1, "die Platzierung hat den DROP ueberlebt");
    assert_eq!(platz[0].0, "regal");
    assert_eq!(platz[0].1, "wohnung");

    let _ = std::fs::remove_file(&pfad);
}

#[test]
fn die_alte_tabelle_ist_danach_weg() {
    // Zwei Tabellen mit denselben Zeilen waeren zwei Wahrheiten, und gelesen wuerde die, die
    // niemand mehr schreibt. Der Umzug laesst die alte fallen.
    let pfad = tempdatei("weg");
    datei_vor_dem_umzug(&pfad);
    let _ = Store::open(&pfad).unwrap();

    let conn = Connection::open(&pfad).unwrap();
    let alt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('interior_item','interior_item_state')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        alt, 0,
        "interior_item und interior_item_state sind gefallen"
    );
    let neu: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='inventory_item'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(neu, 1);

    let _ = std::fs::remove_file(&pfad);
}

/// Der Fremdschluessel in `interior_placement` zeigt danach auf **keine** Tabelle. Das ist
/// Absicht und keine Nachlaessigkeit: die Zeilen gehoeren einer anderen Capability, und ein FK
/// ueber zwei Praefixe liesse spaeter jeden Loeschvorgang der einen an einer Zeile der anderen
/// scheitern. `trips/src/pack.rs` begruendet dieselbe Entscheidung fuer seinen `item_ref`.
#[test]
fn die_platzierung_traegt_keinen_fremdschluessel_mehr() {
    let pfad = tempdatei("fk");
    datei_vor_dem_umzug(&pfad);
    let _ = Store::open(&pfad).unwrap();

    let conn = Connection::open(&pfad).unwrap();
    let ddl: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='interior_placement'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!ddl.contains("REFERENCES"), "kein FK mehr: {ddl}");

    let _ = std::fs::remove_file(&pfad);
}

#[test]
fn ein_zweiter_start_holt_nichts_noch_einmal() {
    // Der Umzug haengt am Vorhandensein der alten Tabelle. Ohne diese Bedingung kopierte jeder
    // Start die Zeilen erneut — und beim zweiten Mal waere die alte Tabelle schon weg, also
    // faellt es hier auf, wenn die Bedingung fehlt.
    let pfad = tempdatei("idempotent");
    datei_vor_dem_umzug(&pfad);
    let store = Store::open(&pfad).unwrap();
    store
        .upsert_item(&Item {
            id: "mantel".into(),
            kind: Kind::Piece,
            label: "Mantel".into(),
            ..Item::default()
        })
        .unwrap();
    drop(store);

    let store = Store::open(&pfad).unwrap();
    assert_eq!(store.item_count().unwrap(), 3);
    assert!(store.item("mantel").unwrap().is_some());
    assert_eq!(store.state_history("hemd").unwrap().len(), 2);

    let _ = std::fs::remove_file(&pfad);
}

/// Eine Datei, die **auch** die Ausruestungs- und Kleidungsspalten noch nicht hat. Der Umzug
/// kopiert dann nur die Spalten, die beide Tabellen fuehren, und die neuen bekommen ihren
/// Vorgabewert — ein `INSERT` ueber eine fehlende Spalte faellt um statt zu heilen.
#[test]
fn eine_aeltere_datei_verliert_durch_den_umzug_nichts() {
    let pfad = tempdatei("alt");
    let conn = Connection::open(&pfad).unwrap();
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

    let store = Store::open(&pfad).unwrap();
    let (regal, zustand) = store.item("regal").unwrap().expect("Regal ueberlebt");
    assert_eq!((regal.b, regal.t, regal.h), (Some(80), Some(30), Some(200)));
    assert_eq!(zustand, Some(State::Owned));
    // Was die alte Datei nicht hatte, kommt als leerer Vorgabewert an — nicht als Fehler.
    assert_eq!(regal.category, None);
    assert_eq!(regal.saison, Vec::<String>::new());
    assert_eq!(regal.revision, 1);

    let _ = std::fs::remove_file(&pfad);
}
