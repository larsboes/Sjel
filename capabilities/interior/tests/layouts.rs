//! Ein Plan entsteht, und ein Plan verschwindet aus der Liste, ohne dass etwas verloren geht.
//!
//! Layouts sind Dateien und keine Zeilen (PRD Q60), und der Kopf einer solchen Datei traegt die
//! Begruendung — bei einem verworfenen Moebel ist er der einzige Ort, an dem sie steht. Daraus
//! folgen die drei Faelle hier: ein Kopf, der eine Herkunft behauptet, die es nicht gibt; ein
//! Name, der aus dem Verzeichnis zeigt; und ein Wegraeumen, das die Begruendung mitnimmt.
//!
//! Gearbeitet wird auf einer KOPIE der Musterwohnung, weil hier geschrieben wird. Dieselbe
//! Trennung wie in `tests/rules.rs`, aus demselben Grund: eine Datei zu veraendern, die andere
//! Testbinaries lesen, ist ein Test, der seine Nachbarn kaputt macht.

use interior::layout_io;
use interior::model::{Layout, Model, PlacedItem};
use std::path::{Path, PathBuf};
use std::sync::Once;

const MUSTER: &str = "muster";
/// Eins der Layouts der Musterwohnung, als Vorlage zum Kopieren.
const TEMPLATE: &str = "a-frei";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Setzt die Umgebung genau einmal je Binary und fuellt das Inventar ueber denselben Import,
/// den `interior import` fuehrt — kein zweiter Weg, Zeilen anzulegen.
fn unique_root() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-layouts-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&wurzel);
        std::fs::create_dir_all(wurzel.join("data/interior/flats")).expect("Testwurzel");
        let db = wurzel.join("test.db");
        std::env::set_var("SJEL_PERSONAL_ROOT", &wurzel);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture().join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    wurzel
}

/// Eine eigene Kopie der Musterwohnung je Test.
///
/// Jeder Test hier schreibt Dateien, und `SJEL_PERSONAL_ROOT` ist prozessweit: zwei Tests, die
/// sich eine Wohnung teilen, zaehlen die Layouts des jeweils anderen mit.
fn flat(name: &str) -> Model {
    let wurzel = unique_root();
    let target = wurzel.join("data/interior/flats").join(name);
    if !target.exists() {
        copy_layout(&fixture().join("data/interior/flats").join(MUSTER), &target);
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn copy_layout(von: &Path, nach: &Path) {
    std::fs::create_dir_all(nach).expect("Zielverzeichnis");
    for e in std::fs::read_dir(von).expect("Quellverzeichnis") {
        let e = e.expect("Eintrag");
        let target = nach.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            copy_layout(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).expect("Datei kopieren");
        }
    }
}

fn file(m: &Model, id: &str) -> String {
    std::fs::read_to_string(m.layouts_dir().join(format!("{id}.toml")))
        .unwrap_or_else(|e| panic!("{id}.toml: {e}"))
}

fn plan(name: &str, items: Vec<PlacedItem>) -> Layout {
    Layout {
        name: name.to_string(),
        items,
        id: String::new(),
    }
}

/// Ein leerer Plan ist der Anfang jeder Planung von Hand und kein Sonderfall.
#[test]
fn a_new_plan_may_be_empty() {
    let m = flat("leer");
    layout_io::create(
        &m,
        "noch-nichts",
        &plan("Noch nichts", vec![]),
        "Ein Anfang.",
    )
    .expect("ein leerer Plan wird geschrieben");

    let l = m
        .load_layout("noch-nichts")
        .expect("und wird wieder gelesen");
    assert!(l.items.is_empty(), "kein Stueck, weil keins gesetzt wurde");
    assert_eq!(l.name, "Noch nichts");
    assert!(
        m.layout_names()
            .expect("Liste")
            .contains(&"noch-nichts".to_string()),
        "und er steht in der Liste"
    );
}

/// Der Kern: der Kopf sagt, was der Aufrufer gesagt hat, und nichts darueber hinaus.
///
/// Bis 2026-08-31 schrieb `create` in JEDE neue Datei „Von einer Maschine gesetzt … jede
/// Position hat die volle Raeumungspruefung durchlaufen". Fuer `interior compose` stimmt das.
/// Fuer einen leeren Plan, den jemand ueber die API anlegt und danach von Hand zieht, ist es
/// falsch — und eine Datei, die ihre eigene Herkunft falsch behauptet, ist schlimmer als eine
/// ohne Kopf, weil sie geglaubt wird.
#[test]
fn the_header_claims_no_check_that_did_not_happen() {
    let m = flat("herkunft");
    let notiz = "Erstellt 2026-08-31 ueber die API.\n\nZweiter Absatz.";
    layout_io::create(&m, "von-hand", &plan("Von Hand", vec![]), notiz).expect("geschrieben");

    let text = file(&m, "von-hand");
    assert!(
        text.contains("# Erstellt 2026-08-31 ueber die API."),
        "die Notiz steht als Kommentar im Kopf:\n{text}"
    );
    assert!(
        text.contains("#\n# Zweiter Absatz."),
        "und ein Absatz bleibt ein Absatz:\n{text}"
    );
    assert!(
        !text.contains("Raeumungspruefung"),
        "niemand hat hier etwas geprueft, also behauptet der Kopf es auch nicht:\n{text}"
    );
}

/// Eine Vorlage wird gelesen, nicht angefasst.
#[test]
fn a_template_is_copied_and_stays_as_it_was() {
    let m = flat("kopie");
    let before = file(&m, TEMPLATE);
    let template = m.load_layout(TEMPLATE).expect("die Vorlage");

    layout_io::create(
        &m,
        "abgeschrieben",
        &plan("Abgeschrieben", template.items.clone()),
        "Kopie.",
    )
    .expect("geschrieben");

    let neu = m.load_layout("abgeschrieben").expect("die Kopie");
    assert_eq!(neu.items.len(), template.items.len());
    for (a, b) in neu.items.iter().zip(&template.items) {
        assert_eq!(
            (&a.reference, a.x, a.y, a.rot),
            (&b.reference, b.x, b.y, b.rot)
        );
    }
    assert_eq!(
        file(&m, TEMPLATE),
        before,
        "die Vorlage hat Zeichen fuer Zeichen ueberlebt"
    );
}

/// Ein bestehender Plan wird nicht ueberschrieben, und zwar auch dann nicht, wenn der neue
/// besser ist: der Kopf des alten ist das Argument, das ihn ersetzt hat.
#[test]
fn an_existing_layout_is_not_overwritten() {
    let m = flat("kein-ueberschreiben");
    let before = file(&m, TEMPLATE);
    let e = layout_io::create(&m, TEMPLATE, &plan("Neu", vec![]), "Zweiter Versuch.")
        .expect_err("den Namen gibt es schon");
    assert!(e.to_string().contains("exists already"), "{e}");
    assert_eq!(file(&m, TEMPLATE), before, "und die Datei ist unberuehrt");
}

/// Der Name wird zu einem Dateinamen. Ueber die API kommt er von aussen.
#[test]
fn a_layout_name_cannot_point_out_of_the_dir() {
    let m = flat("namen");
    let starts = m.layout_names().expect("Liste").len();
    for bad in ["../room", "unter/plan", "plan.toml", ""] {
        assert!(
            layout_io::create(&m, bad, &plan("X", vec![]), "n").is_err(),
            "`{bad}` darf keine Datei werden"
        );
    }
    assert_eq!(
        m.layout_names().expect("Liste").len(),
        starts,
        "und keiner davon hat sich an der Pruefung vorbei eine Datei geschrieben"
    );
}

/// Archivieren heisst: aus der Liste, nicht von der Platte.
#[test]
fn an_archived_layout_leaves_the_list_not_the_disk() {
    let m = flat("archiv");
    layout_io::create(
        &m,
        "verworfen",
        &plan("Verworfen", vec![]),
        "Drei Absaetze Begruendung.",
    )
    .expect("geschrieben");
    let before = file(&m, "verworfen");

    let whither = layout_io::archive_layout(&m, "verworfen").expect("archiviert");
    assert_eq!(
        whither, "archiv/verworfen.toml",
        "und die Antwort nennt keinen Pfad des Overlays"
    );

    let names = m.layout_names().expect("Liste");
    assert!(
        !names.contains(&"verworfen".to_string()),
        "aus der Liste: {names:?}"
    );
    assert!(
        !names.contains(&"archiv".to_string()),
        "und das Verzeichnis selbst ist kein Layout: {names:?}"
    );
    let stored = m.layouts_dir().join(&whither);
    assert_eq!(
        std::fs::read_to_string(&stored).expect("die Datei liegt im Archiv"),
        before,
        "mit ihrer Begruendung, Zeichen fuer Zeichen"
    );
}

/// Zwei Fassungen unter einem Namen waeren eine verlorene Begruendung.
#[test]
fn archiving_twice_under_one_name_is_refused() {
    let m = flat("archiv-zweimal");
    layout_io::create(&m, "doppelt", &plan("Erst", vec![]), "Erste Fassung.").expect("erste");
    layout_io::archive_layout(&m, "doppelt").expect("erste archiviert");
    layout_io::create(&m, "doppelt", &plan("Dann", vec![]), "Zweite Fassung.").expect("zweite");

    let e = layout_io::archive_layout(&m, "doppelt").expect_err("das Archiv ist belegt");
    assert!(e.to_string().contains("archive"), "{e}");
    assert!(
        file(&m, "doppelt").contains("Zweite Fassung"),
        "und die zweite steht noch da, statt still ueber die erste zu wandern"
    );
}

#[test]
fn a_layout_that_does_not_exist_is_not_archived() {
    let m = flat("archiv-leer");
    let e = layout_io::archive_layout(&m, "gab-es-nie").expect_err("es gibt nichts wegzuraeumen");
    assert!(e.to_string().contains("gab-es-nie"), "{e}");
}
