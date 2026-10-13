//! The search, and what its ranking holds right. A ranking, not a verdict: `room_divider` cannot
//! make a layout pass or fail; it decides which of several allowed proposals ranks first.

use interior::model::Model;
use interior::search::{search, Spec};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Once;

const FLAT: &str = "muster";

fn model() -> Model {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-search-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
    Model::load(FLAT).expect("Musterwohnung")
}

/// Searches the layout holding just this one board. Both layouts place their board at the same
/// coordinates, so the two runs differ in one thing: whether the item has `room_divider = true`.
fn search_state(m: &Model, variant: &str) -> interior::search::SearchReport {
    let base = m
        .load_layout(&format!("g-trenner-{variant}"))
        .unwrap_or_else(|e| panic!("g-trenner-{variant}: {e}"));
    search(
        m,
        &base,
        &Spec {
            move_refs: vec![format!("trenner_{variant}")],
            step: 50,
            bands: BTreeMap::new(),
            limit: 0,
        },
    )
    .expect("search")
}

/// The evidence, in a comparison with one variable: `trenner_erklaert` and `trenner_stumm` have
/// the same size in the same flat. Moving the quiet board finds wall places rated higher; the
/// declared one's wall contact does not count at all. Without the comparison, a wall sum of 0 could
/// also mean no wall place was found.
#[test]
fn a_declared_room_divider_is_not_penalised_for_standing_free() {
    let m = model();

    let quiet = search_state(&m, "stumm");
    assert!(
        !quiet.hits.is_empty(),
        "the search finds places for the quiet board"
    );
    let best_wall = quiet
        .hits
        .iter()
        .map(|h| h.wall_contact_cm)
        .max()
        .unwrap_or(0);
    assert!(
        best_wall > 0,
        "a board that declares nothing finds a wall and is credited for it; \
         otherwise the comparison below checks nothing"
    );

    let declared = search_state(&m, "erklaert");
    assert!(
        !declared.hits.is_empty(),
        "and places for the declared board"
    );
    assert!(
        declared.hits.iter().all(|h| h.wall_contact_cm == 0),
        "a declared room divider does not count in the wall sum at all; it should neither raise \
         nor lower the rating of the other pieces"
    );
}

/// The ranking is stable and in the documented order: front, then no warnings, then wall behind,
/// then the widest bottleneck, so a reordered `then` chain shows.
#[test]
fn the_ranking_sorts_front_then_warnings_then_wall_then_bottleneck() {
    let m = model();
    let r = search_state(&m, "stumm");
    assert!(
        r.hits.len() > 1,
        "a ranking needs more than one hit"
    );

    for paar in r.hits.windows(2) {
        let (a, b) = (&paar[0], &paar[1]);
        let a_key = (
            !a.pareto,
            a.soft,
            -a.wall_contact_cm,
            -a.bottleneck_cm,
            std::cmp::Reverse(a.tightest_reserve_cm),
        );
        let b_key = (
            !b.pareto,
            b.soft,
            -b.wall_contact_cm,
            -b.bottleneck_cm,
            std::cmp::Reverse(b.tightest_reserve_cm),
        );
        assert!(a_key <= b_key, "ranking broken: {a:?} before {b:?}");
    }
}

/// The front is a selection, not a relabelling of every hit: a flag that is `true` everywhere
/// sorts nothing.
#[test]
fn the_front_is_strictly_smaller_than_the_hits() {
    let m = model();
    let r = search_state(&m, "stumm");
    let front = r.hits.iter().filter(|h| h.pareto).count();
    assert!(front > 0, "without a front there is no best answer");
    assert!(
        front < r.hits.len(),
        "{front} of {} hits are front; then the flag separates nothing",
        r.hits.len()
    );
}

/// No dominated hit ranks before one that dominates it, checked on the numbers rather than the
/// flag.
#[test]
fn no_hit_ranks_before_one_that_beats_it_everywhere() {
    let m = model();
    let r = search_state(&m, "stumm");
    let target = |h: &interior::search::Hit| {
        [
            h.tightest_reserve_cm.unwrap_or(i32::MIN),
            h.wall_contact_cm,
            h.bottleneck_cm,
            -(h.soft as i32),
        ]
    };
    for (i, a) in r.hits.iter().enumerate() {
        for b in r.hits.iter().skip(i + 1) {
            let (za, zb) = (target(a), target(b));
            let dominates = zb.iter().zip(za.iter()).all(|(x, y)| x >= y)
                && zb.iter().zip(za.iter()).any(|(x, y)| x > y);
            assert!(
                !dominates,
                "{b:?} beats {a:?} in everything and ranks behind it"
            );
        }
    }
}

/// A hard violation never comes back as a hit.
#[test]
fn no_hit_breaks_a_hard_rule() {
    let m = model();
    let r = search_state(&m, "stumm");
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");

    for h in &r.hits {
        let mut items = base.items.clone();
        for (reference, pos) in &h.places {
            let it = items
                .iter_mut()
                .find(|i| &i.reference == reference)
                .expect("the moved ref is in the layout");
            it.x = pos[0];
            it.y = pos[1];
        }
        let l = interior::model::Layout {
            name: "nachgerechnet".into(),
            id: String::new(),
            items,
        };
        let c = interior::clearance::check_layout(&m, &l).expect("Nachrechnung");
        assert!(
            c.hard.is_empty(),
            "hit at {:?} breaks {:?}",
            h.places,
            c.hard.iter().map(|v| &v.rule).collect::<Vec<_>>()
        );
    }
}

/// Hard edges: what the list allows really lies in the room and not in a fixture. And the list is
/// not simply the bounding rect, the obvious wrong version in a browser.
#[test]
fn allowed_positions_lie_in_the_room_not_in_a_fixture() {
    use interior::model::{Layout, PlacedItem};
    let m = model();
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");
    let a = interior::search::allowed_positions(&m, &base, "trenner_stumm", 0)
        .expect("erlaubte Positionen");

    assert!(!a.rows.is_empty(), "there is room for the board");
    assert_eq!(a.step, 5);

    // Every allowed position survives the room boundary.
    let mut checked = 0;
    for row in &a.rows {
        for [lo, hi] in &row.x {
            for x in [*lo, *hi] {
                let l = Layout {
                    name: "Kantenprobe".into(),
                    id: String::new(),
                    items: vec![PlacedItem {
                        reference: "trenner_stumm".into(),
                        x,
                        y: row.y,
                        rot: 0,
                        size: None,
                        kind: None,
                    }],
                };
                let r = interior::clearance::check_layout(&m, &l).expect("check");
                assert!(
                    !r.hard.iter().any(|v| v.rule == "room_boundary"),
                    "erlaubte Position {x},{} liegt ausserhalb des Raums",
                    row.y
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 50, "only {checked} edges checked");

    // And the list is narrower than the bounding rect: the kitchen run lies at x 0..250,
    // y 390..450, so a row touching it must not start at x = 0, while a free row may.
    let start = |y: i32| -> Option<i32> { a.rows.iter().find(|r| r.y == y).map(|r| r.x[0][0]) };
    assert_eq!(start(0), Some(0), "at the top the wall is the only limit");
    let down = start(380).expect("there is a row at y = 380");
    assert!(
        down > 0,
        "at y = 380 the board reaches into the kitchen run, so x = 0 must not be allowed \
         (got: {down})"
    );
}

/// A position inside a fixture is not on the list.
#[test]
fn a_position_in_the_kitchen_run_is_not_allowed() {
    let m = model();
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");
    let a = interior::search::allowed_positions(&m, &base, "trenner_stumm", 0)
        .expect("erlaubte Positionen");

    // The sample flat's kitchen run lies at x 0..250, y 390..450.
    let drin = |x: i32, y: i32| {
        a.rows
            .iter()
            .find(|r| r.y == y)
            .is_some_and(|r| r.x.iter().any(|[lo, hi]| x >= *lo && x <= *hi))
    };
    assert!(
        !drin(0, 390),
        "0,390 lies in the kitchen run and must not be allowed"
    );
}
