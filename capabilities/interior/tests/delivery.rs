//! Does the piece get in, and as far as its place?
//!
//! The cases are constructed. `k-vitrine` is the core: a layout meeting every clearance rule that
//! can never happen, because the piece is 10 cm wider than the entrance door.

use interior::delivery::{delivery, through_the_door, DoorFit};
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
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
}

/// A passing layout and a piece that does not get in, both true at once: the reason this module
/// exists.
#[test]
fn a_rule_abiding_layout_can_fail_at_the_door() {
    let m = model();
    let l = m.load_layout("k-vitrine").unwrap();
    let verdict = interior::clearance::check_layout(&m, &l).unwrap();
    assert!(
        verdict.pass,
        "the display cabinet stands by the rules: {:?}",
        verdict.hard
    );

    let e = delivery(&m, &l, "glasvitrine").unwrap();
    assert_eq!(
        e.door,
        DoorFit::DoesNotFit {
            missing_cm: 10,
            door_cm: 100
        },
        "110 cm both ways at a 100 cm door"
    );
}

/// The narrow side decides: a wardrobe goes through a door upright.
#[test]
fn the_narrow_side_decides_not_the_width() {
    let m = model();
    assert!(
        matches!(through_the_door(&m, 120, 60, false), DoorFit::Fits { .. }),
        "120 x 60 fits upright through 100 cm"
    );
    assert!(
        matches!(
            through_the_door(&m, 120, 110, false),
            DoorFit::DoesNotFit { .. }
        ),
        "120 x 110 fits in no position"
    );
}

/// What comes apart is not measured at the door, and says so: the answer comes from the row
/// (`disassembles = true`), not from a guess about beds.
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
        "without the declaration it would stay a violation"
    );
}

/// A place that exists is a place there is a path to.
#[test]
fn every_piece_of_a_passing_layout_reaches_its_place() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    for it in &l.items {
        let e = delivery(&m, &l, &it.reference).unwrap();
        assert!(
            e.affordable,
            "{} does not reach its place: {:?}",
            it.reference, e.reason
        );
        assert!(e.steps.is_some_and(|n| n > 0));
    }
}

/// Without a declared entrance nothing is guessed: the bathroom and entrance doors share a kind.
#[test]
fn without_a_declared_entrance_nothing_is_guessed() {
    let mut m = model();
    for o in m.room.openings.iter_mut() {
        o.entrance = None;
    }
    assert_eq!(through_the_door(&m, 50, 50, false), DoorFit::NoEntrance);
}
