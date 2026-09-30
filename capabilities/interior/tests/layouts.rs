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
const VORLAGE: &str = "a-frei";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Setzt die Umgebung genau einmal je Binary und fuellt das Inventar ueber denselben Import,
/// den `interior import` fuehrt — kein zweiter Weg, Zeilen anzulegen.
fn einmalige_wurzel() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-layouts-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&wurzel);
        std::fs::create_dir_all(wurzel.join("data/interior/flats")).expect("Testwurzel");
        let db = wurzel.join("test.db");
        std::env::set_var("SJEL_PERSONAL_ROOT", &wurzel);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture().join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    wurzel
}

/// Eine eigene Kopie der Musterwohnung je Test.
///
/// Jeder Test hier schreibt Dateien, und `SJEL_PERSONAL_ROOT` ist prozessweit: zwei Tests, die
/// sich eine Wohnung teilen, zaehlen die Layouts des jeweils anderen mit.
fn wohnung(name: &str) -> Model {
    let wurzel = einmalige_wurzel();
    let ziel = wurzel.join("data/interior/flats").join(name);
    if !ziel.exists() {
        kopiere(&fixture().join("data/interior/flats").join(MUSTER), &ziel);
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn kopiere(von: &Path, nach: &Path) {
    std::fs::create_dir_all(nach).expect("Zielverzeichnis");
    for e in std::fs::read_dir(von).expect("Quellverzeichnis") {
        let e = e.expect("Eintrag");
        let ziel = nach.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            kopiere(&e.path(), &ziel);
        } else {
            std::fs::copy(e.path(), &ziel).expect("Datei kopieren");
        }
    }
}

fn datei(m: &Model, id: &str) -> String {
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
fn ein_neuer_plan_darf_leer_sein() {
    let m = wohnung("leer");
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
fn der_kopf_behauptet_keine_pruefung_die_nicht_stattgefunden_hat() {
    let m = wohnung("herkunft");
    let notiz = "Erstellt 2026-08-31 ueber die API.\n\nZweiter Absatz.";
    layout_io::create(&m, "von-hand", &plan("Von Hand", vec![]), notiz).expect("geschrieben");

    let text = datei(&m, "von-hand");
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
fn eine_vorlage_wird_kopiert_und_bleibt_wie_sie_war() {
    let m = wohnung("kopie");
    let vorher = datei(&m, VORLAGE);
    let vorlage = m.load_layout(VORLAGE).expect("die Vorlage");

    layout_io::create(
        &m,
        "abgeschrieben",
        &plan("Abgeschrieben", vorlage.items.clone()),
        "Kopie.",
    )
    .expect("geschrieben");

    let neu = m.load_layout("abgeschrieben").expect("die Kopie");
    assert_eq!(neu.items.len(), vorlage.items.len());
    for (a, b) in neu.items.iter().zip(&vorlage.items) {
        assert_eq!(
            (&a.reference, a.x, a.y, a.rot),
            (&b.reference, b.x, b.y, b.rot)
        );
    }
    assert_eq!(
        datei(&m, VORLAGE),
        vorher,
        "die Vorlage hat Zeichen fuer Zeichen ueberlebt"
    );
}

/// Ein bestehender Plan wird nicht ueberschrieben, und zwar auch dann nicht, wenn der neue
/// besser ist: der Kopf des alten ist das Argument, das ihn ersetzt hat.
#[test]
fn ein_bestehendes_layout_wird_nicht_ueberschrieben() {
    let m = wohnung("kein-ueberschreiben");
    let vorher = datei(&m, VORLAGE);
    let e = layout_io::create(&m, VORLAGE, &plan("Neu", vec![]), "Zweiter Versuch.")
        .expect_err("den Namen gibt es schon");
    assert!(e.to_string().contains("gibt es schon"), "{e}");
    assert_eq!(datei(&m, VORLAGE), vorher, "und die Datei ist unberuehrt");
}

/// Der Name wird zu einem Dateinamen. Ueber die API kommt er von aussen.
#[test]
fn ein_layoutname_kann_nicht_aus_dem_verzeichnis_zeigen() {
    let m = wohnung("namen");
    let anfangs = m.layout_names().expect("Liste").len();
    for boese in ["../room", "unter/plan", "plan.toml", ""] {
        assert!(
            layout_io::create(&m, boese, &plan("X", vec![]), "n").is_err(),
            "`{boese}` darf keine Datei werden"
        );
    }
    assert_eq!(
        m.layout_names().expect("Liste").len(),
        anfangs,
        "und keiner davon hat sich an der Pruefung vorbei eine Datei geschrieben"
    );
}

/// Archivieren heisst: aus der Liste, nicht von der Platte.
#[test]
fn ein_archiviertes_layout_faellt_aus_der_liste_und_nicht_von_der_platte() {
    let m = wohnung("archiv");
    layout_io::create(
        &m,
        "verworfen",
        &plan("Verworfen", vec![]),
        "Drei Absaetze Begruendung.",
    )
    .expect("geschrieben");
    let vorher = datei(&m, "verworfen");

    let wohin = layout_io::archiviere(&m, "verworfen").expect("archiviert");
    assert_eq!(
        wohin, "archiv/verworfen.toml",
        "und die Antwort nennt keinen Pfad des Overlays"
    );

    let namen = m.layout_names().expect("Liste");
    assert!(
        !namen.contains(&"verworfen".to_string()),
        "aus der Liste: {namen:?}"
    );
    assert!(
        !namen.contains(&"archiv".to_string()),
        "und das Verzeichnis selbst ist kein Layout: {namen:?}"
    );
    let abgelegt = m.layouts_dir().join(&wohin);
    assert_eq!(
        std::fs::read_to_string(&abgelegt).expect("die Datei liegt im Archiv"),
        vorher,
        "mit ihrer Begruendung, Zeichen fuer Zeichen"
    );
}

/// Zwei Fassungen unter einem Namen waeren eine verlorene Begruendung.
#[test]
fn zweimal_archivieren_unter_einem_namen_wird_abgelehnt() {
    let m = wohnung("archiv-zweimal");
    layout_io::create(&m, "doppelt", &plan("Erst", vec![]), "Erste Fassung.").expect("erste");
    layout_io::archiviere(&m, "doppelt").expect("erste archiviert");
    layout_io::create(&m, "doppelt", &plan("Dann", vec![]), "Zweite Fassung.").expect("zweite");

    let e = layout_io::archiviere(&m, "doppelt").expect_err("das Archiv ist belegt");
    assert!(e.to_string().contains("Archiv"), "{e}");
    assert!(
        datei(&m, "doppelt").contains("Zweite Fassung"),
        "und die zweite steht noch da, statt still ueber die erste zu wandern"
    );
}

#[test]
fn ein_layout_das_es_nicht_gibt_wird_nicht_archiviert() {
    let m = wohnung("archiv-leer");
    let e = layout_io::archiviere(&m, "gab-es-nie").expect_err("es gibt nichts wegzuraeumen");
    assert!(e.to_string().contains("gab-es-nie"), "{e}");
}
