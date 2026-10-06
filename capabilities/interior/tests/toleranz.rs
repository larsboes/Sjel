//! Was ein Verdikt aushaelt, wenn das Bandmass sich geirrt hat.
//!
//! Die Antworten hier sind konstruiert und nicht aufgezeichnet: `j-sideboard-knapp` stellt das
//! einzige Stueck mit geschaetzten Massen genau fuenf Zentimeter vor eine harte Zone. Die
//! Zahlen unten folgen aus der Geometrie der Musterwohnung, nicht daraus, was der Code beim
//! ersten Lauf ausgegeben hat.

use interior::model::Model;
use interior::toleranz::{robustheit, Haltbarkeit, HORIZONT_CM};
use std::sync::Once;

const FLAT: &str = "muster";

fn model() -> Model {
    fixture_overlay();
    Model::load(FLAT).unwrap_or_else(|e| panic!("{FLAT}: {e}"))
}

/// Dieselbe Vorbereitung wie in `engine.rs`: eigene Datenbankdatei je Testbinary, gefuellt
/// ueber den echten Import.
fn fixture_overlay() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-toleranz-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
}

fn robust(layout: &str) -> interior::toleranz::Robustheit {
    let m = model();
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{layout}: {e}"));
    robustheit(&m, &l).unwrap_or_else(|e| panic!("{layout}: {e}"))
}

/// Fuenf Zentimeter Luft, also fuenf Zentimeter Messfehler — und beim sechsten reisst R7.
///
/// Beide Zahlen stehen im Kopf der Layoutdatei, bevor dieser Test lief. Sie kommen aus der
/// Suedkante des Sideboards (y 285) und dem Beginn der Kuechen-Anlaufzone (y 290).
#[test]
fn ein_geschaetztes_mass_bekommt_eine_zahl_statt_eines_verdikts() {
    let r = robust("j-sideboard-knapp");
    assert!(
        r.nominal_pass,
        "das Layout besteht mit den eingetragenen Massen"
    );
    match r.haelt {
        Haltbarkeit::Bis { cm } => assert_eq!(cm, 5, "fuenf cm Luft, also fuenf cm Toleranz"),
        andere => panic!("erwartet Bis {{ cm: 5 }}, bekommen {andere:?}"),
    }
    assert_eq!(
        r.kippt_an,
        vec!["R7".to_string()],
        "die Kuechen-Anlaufzone ist das, was zuerst reisst"
    );
}

/// Die Bisektion und die Reserve messen dasselbe auf zwei Wegen und muessen sich einigen.
///
/// `engste_reserve_cm` faellt aus einem Rechteckabstand, `haelt` aus wiederholten vollen
/// Pruefungen mit vergroesserten Massen. Dass beide 5 sagen, ist der Grund, einer von beiden
/// zu glauben — und wenn sie je auseinanderlaufen, ist eine der beiden Fassungen falsch.
#[test]
fn reserve_und_bisektion_kommen_auf_dieselbe_zahl() {
    let r = robust("j-sideboard-knapp");
    let Haltbarkeit::Bis { cm } = r.haelt else {
        panic!("dieses Layout kippt innerhalb des Horizonts")
    };
    assert_eq!(
        r.engste_reserve_cm,
        Some(cm),
        "zwei Messungen derselben Enge, zwei verschiedene Zahlen"
    );
}

/// Ohne geschaetztes Mass gibt es nichts zu variieren, und das ist eine Aussage.
///
/// Nicht „haelt unendlich": es heisst, dass das Inventar nichts als unsicher fuehrt. Der
/// Unterschied ist genau der, den `Haltbarkeit` als eigene Variante fuehrt, statt `None`
/// zweimal mit verschiedener Bedeutung auszugeben.
#[test]
fn ein_layout_ohne_geschaetzte_masse_sagt_das_statt_eine_zahl_zu_erfinden() {
    let r = robust("a-frei");
    assert!(
        matches!(r.haelt, Haltbarkeit::NichtsGeraten),
        "{:?}",
        r.haelt
    );
    assert!(r.kippt_an.is_empty());
}

/// Was schon durchfaellt, wird nicht gestoert. Ein Messfehler ist dort nicht die Frage.
#[test]
fn ein_durchgefallenes_layout_wird_nicht_variiert() {
    let r = robust("c-terrassentuer");
    assert!(!r.nominal_pass);
    assert!(matches!(r.haelt, Haltbarkeit::FaelltDurch), "{:?}", r.haelt);
}

/// Die Voraussetzung der Halbierung, an den Zahlen geprueft statt im Kommentar behauptet.
///
/// Die Stoerung vergroessert Masse nur, also darf ein Layout, das bei `t` durchfaellt, bei
/// keinem groesseren `t` wieder bestehen. Waere das falsch, waere die Bisektion in
/// `toleranz.rs` ein Verfahren, das eine beliebige Kippstelle findet und die kleinste meldet.
#[test]
fn bestehen_faellt_monoton_mit_dem_messfehler() {
    let m = model();
    let l = m.load_layout("j-sideboard-knapp").unwrap();
    let mut gefallen = false;
    for tol in 0..=HORIZONT_CM {
        let pass = interior::toleranz::besteht_bei_fuer_test(&m, &l, tol).unwrap();
        if gefallen {
            assert!(
                !pass,
                "bei {tol} cm besteht es wieder, nachdem es gefallen war"
            );
        }
        if !pass {
            gefallen = true;
        }
    }
    assert!(
        gefallen,
        "innerhalb des Horizonts muss dieses Layout kippen"
    );
}
