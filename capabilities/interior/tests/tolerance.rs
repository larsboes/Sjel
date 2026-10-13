//! Was ein Verdikt aushaelt, wenn das Bandmass sich geirrt hat.
//!
//! Die Antworten hier sind konstruiert und nicht aufgezeichnet: `j-sideboard-knapp` stellt das
//! einzige Stueck mit geschaetzten Massen genau fuenf Zentimeter vor eine harte Zone. Die
//! Zahlen unten folgen aus der Geometrie der Musterwohnung, nicht daraus, was der Code beim
//! ersten Lauf ausgegeben hat.

use interior::model::Model;
use interior::tolerance::{robustness, Holds, HORIZON_CM};
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

fn robust(layout: &str) -> interior::tolerance::Robustness {
    let m = model();
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{layout}: {e}"));
    robustness(&m, &l).unwrap_or_else(|e| panic!("{layout}: {e}"))
}

/// Fuenf Zentimeter Luft, also fuenf Zentimeter Messfehler — und beim sechsten reisst R7.
///
/// Beide Zahlen stehen im Kopf der Layoutdatei, bevor dieser Test lief. Sie kommen aus der
/// Suedkante des Sideboards (y 285) und dem Beginn der Kuechen-Anlaufzone (y 290).
#[test]
fn an_estimated_size_gets_a_number_instead_of_a_verdict() {
    let r = robust("j-sideboard-knapp");
    assert!(
        r.nominal_pass,
        "das Layout besteht mit den eingetragenen Massen"
    );
    match r.holds {
        Holds::UpTo { cm } => assert_eq!(cm, 5, "fuenf cm Luft, also fuenf cm Toleranz"),
        other => panic!("expected UpTo {{ cm: 5 }}, bekommen {other:?}"),
    }
    assert_eq!(
        r.tips_at,
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
fn reserve_and_bisection_reach_the_same_number() {
    let r = robust("j-sideboard-knapp");
    let Holds::UpTo { cm } = r.holds else {
        panic!("dieses Layout kippt innerhalb des Horizonts")
    };
    assert_eq!(
        r.tightest_reserve_cm,
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
fn a_layout_without_estimated_sizes_says_so_instead_of_inventing_a_number() {
    let r = robust("a-frei");
    assert!(
        matches!(r.holds, Holds::NothingGuessed),
        "{:?}",
        r.holds
    );
    assert!(r.tips_at.is_empty());
}

/// Was schon durchfaellt, wird nicht gestoert. Ein Messfehler ist dort nicht die Frage.
#[test]
fn a_failing_layout_is_not_varied() {
    let r = robust("c-terrassentuer");
    assert!(!r.nominal_pass);
    assert!(matches!(r.holds, Holds::Fails), "{:?}", r.holds);
}

/// Die Voraussetzung der Halbierung, an den Zahlen geprueft statt im Kommentar behauptet.
///
/// Die Stoerung vergroessert Masse nur, also darf ein Layout, das bei `t` durchfaellt, bei
/// keinem groesseren `t` wieder bestehen. Waere das falsch, waere die Bisektion in
/// `toleranz.rs` ein Verfahren, das eine beliebige Kippstelle findet und die kleinste meldet.
#[test]
fn passing_falls_monotonically_with_the_measurement_error() {
    let m = model();
    let l = m.load_layout("j-sideboard-knapp").unwrap();
    let mut failed = false;
    for tol in 0..=HORIZON_CM {
        let pass = interior::tolerance::passes_at_for_test(&m, &l, tol).unwrap();
        if failed {
            assert!(
                !pass,
                "bei {tol} cm besteht es wieder, nachdem es gefallen war"
            );
        }
        if !pass {
            failed = true;
        }
    }
    assert!(
        failed,
        "innerhalb des Horizonts muss dieses Layout kippen"
    );
}
