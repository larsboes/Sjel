//! Kommt das Stueck herein — und bis an seinen Platz?
//!
//! Die Faelle hier sind konstruiert und nicht abgelesen. `k-vitrine` ist der Kern: ein Layout,
//! das jede Raeumungsregel erfuellt und trotzdem nie stattfinden kann, weil das Moebel 10 cm
//! breiter ist als die Wohnungstuer. Bis 2026-08-31 hat diese Capability darauf `BESTANDEN`
//! geantwortet — richtig gerechnet, falsche Frage.

use interior::delivery::{through_the_door, delivery, DoorFit};
use interior::model::Model;
use std::sync::Once;

const FLAT: &str = "muster";

fn model() -> Model {
    fixture_overlay();
    Model::load(FLAT).unwrap_or_else(|e| panic!("{FLAT}: {e}"))
}

fn fixture_overlay() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db =
            std::env::temp_dir().join(format!("interior-einbringung-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
}

/// Ein bestandenes Layout und ein Stueck, das nicht hereinkommt — beides zugleich wahr.
///
/// Das ist der ganze Grund fuer dieses Modul. Wuerde `check_layout` hier durchfallen, waere
/// die Frage schon beantwortet und `einbringung.rs` ueberfluessig.
#[test]
fn a_rule_abiding_layout_can_fail_at_the_door() {
    let m = model();
    let l = m.load_layout("k-vitrine").unwrap();
    let verdict = interior::clearance::check_layout(&m, &l).unwrap();
    assert!(
        verdict.pass,
        "die Vitrine steht regelkonform: {:?}",
        verdict.hard
    );

    let e = delivery(&m, &l, "glasvitrine").unwrap();
    assert_eq!(
        e.door,
        DoorFit::DoesNotFit {
            missing_cm: 10,
            door_cm: 100
        },
        "110 cm in beiden Richtungen an einer 100 cm breiten Tuer"
    );
}

/// Die schmale Side entscheidet: ein Schrank geht hochkant durch die Tuer.
///
/// Ohne diese Regel waere jedes Stueck breiter als 100 cm ein Problem, und das ist offenkundig
/// falsch — der 120 x 60 cm Kleiderschrank kommt durch jede Wohnungstuer.
#[test]
fn the_narrow_side_decides_not_the_width() {
    let m = model();
    assert!(
        matches!(through_the_door(&m, 120, 60, false), DoorFit::Fits { .. }),
        "120 x 60 passt hochkant durch 100 cm"
    );
    assert!(
        matches!(
            through_the_door(&m, 120, 110, false),
            DoorFit::DoesNotFit { .. }
        ),
        "120 x 110 passt in keiner Lage"
    );
}

/// Was zerlegt kommt, wird an der Tuer nicht gemessen — und sagt das, statt zu bestehen.
///
/// Das Bett ist 140 cm breit und steht trotzdem in jedem Schlafzimmer. Die Auskunft kommt aus
/// seiner Zeile (`zerlegbar = true`) und nicht aus einer Vermutung ueber Betten.
#[test]
fn a_piece_that_comes_apart_is_not_measured_at_the_door() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    let e = delivery(&m, &l, "bett").unwrap();
    assert_eq!(
        e.door,
        DoorFit::CarriedApart {
            missing_cm: 40,
            door_cm: 100
        }
    );
    assert!(
        !matches!(through_the_door(&m, 140, 200, false), DoorFit::Fits { .. }),
        "ohne die Deklaration bliebe es ein Verstoss"
    );
}

/// Ein Platz, den es gibt, ist ein Platz, zu dem es einen Weg gibt.
#[test]
fn every_piece_of_a_passing_layout_reaches_its_place() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    for it in &l.items {
        let e = delivery(&m, &l, &it.reference).unwrap();
        assert!(
            e.affordable,
            "{} kommt nicht an seinen Platz: {:?}",
            it.reference, e.reason
        );
        assert!(e.steps.is_some_and(|n| n > 0));
    }
}

/// Ohne deklarierten Eingang wird nicht geraten.
///
/// Dieselbe Haltung wie bei R6: `badtuer` und `eingangstuer` tragen denselben `typ`, und sie
/// am Namen zu unterscheiden waere der Fehler, den PRD B26a einmal geschlossen hat.
#[test]
fn without_a_declared_entrance_nothing_is_guessed() {
    let mut m = model();
    for o in m.room.openings.iter_mut() {
        o.entrance = None;
    }
    assert_eq!(through_the_door(&m, 50, 50, false), DoorFit::NoEntrance);
}
