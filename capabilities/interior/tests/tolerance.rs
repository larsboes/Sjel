//! What a verdict survives when the tape measure was off.
//!
//! The answers are constructed, not recorded: `j-sideboard-knapp` places the one piece with
//! estimated sizes exactly five centimetres before a hard zone.

use interior::model::Model;
use interior::tolerance::{robustness, Holds, HORIZON_CM};
use std::sync::Once;

const FLAT: &str = "muster";

fn model() -> Model {
    fixture_overlay();
    Model::load(FLAT).unwrap_or_else(|e| panic!("{FLAT}: {e}"))
}

/// The same setup as `engine.rs`: one database file per test binary, filled through the real
/// import.
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
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
}

fn robust(layout: &str) -> interior::tolerance::Robustness {
    let m = model();
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{layout}: {e}"));
    robustness(&m, &l).unwrap_or_else(|e| panic!("{layout}: {e}"))
}

/// Five centimetres of air, so five centimetres of error, and at the sixth R7 breaks. Both numbers
/// come from the sideboard's south edge (y 285) and the kitchen approach zone (y 290).
#[test]
fn an_estimated_size_gets_a_number_instead_of_a_verdict() {
    let r = robust("j-sideboard-knapp");
    assert!(r.nominal_pass, "the layout passes with the recorded sizes");
    match r.holds {
        Holds::UpTo { cm } => assert_eq!(cm, 5, "five cm of air, so five cm of tolerance"),
        other => panic!("expected UpTo {{ cm: 5 }}, bekommen {other:?}"),
    }
    assert_eq!(
        r.tips_at,
        vec!["R7".to_string()],
        "the kitchen approach zone is what breaks first"
    );
}

/// Bisection and reserve measure the same thing two ways and must agree: `tightest_reserve_cm`
/// from a rect gap, `holds` from repeated full checks with grown sizes.
#[test]
fn reserve_and_bisection_reach_the_same_number() {
    let r = robust("j-sideboard-knapp");
    let Holds::UpTo { cm } = r.holds else {
        panic!("this layout tips within the horizon")
    };
    assert_eq!(
        r.tightest_reserve_cm,
        Some(cm),
        "zwei Messungen derselben Enge, zwei verschiedene Zahlen"
    );
}

/// Without an estimated size there is nothing to vary, which is a statement, not "holds forever".
#[test]
fn a_layout_without_estimated_sizes_says_so_instead_of_inventing_a_number() {
    let r = robust("a-frei");
    assert!(matches!(r.holds, Holds::NothingGuessed), "{:?}", r.holds);
    assert!(r.tips_at.is_empty());
}

/// What fails already is not disturbed.
#[test]
fn a_failing_layout_is_not_varied() {
    let r = robust("c-terrassentuer");
    assert!(!r.nominal_pass);
    assert!(matches!(r.holds, Holds::Fails), "{:?}", r.holds);
}

/// The bisection's precondition, checked on numbers: sizes only grow, so a layout failing at `t`
/// may pass at no larger `t`.
#[test]
fn passing_falls_monotonically_with_the_measurement_error() {
    let m = model();
    let l = m.load_layout("j-sideboard-knapp").unwrap();
    let mut failed = false;
    for tol in 0..=HORIZON_CM {
        let pass = interior::tolerance::passes_at_for_test(&m, &l, tol).unwrap();
        if failed {
            assert!(!pass, "at {tol} cm it passes again after failing");
        }
        if !pass {
            failed = true;
        }
    }
    assert!(failed, "this layout must tip within the horizon");
}
