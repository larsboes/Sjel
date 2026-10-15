//! The engine against an invented flat.
//!
//! `tests/fixtures/overlay/` is a complete `data/interior/` with made-up sizes, which is why the
//! capability can be public and still tested; the real flat is only touched by
//! `tests/live_parity.rs`. The fixtures keep the older German file format on purpose: they prove
//! the aliases that read existing overlays.
//!
//! No recorded results here: every claim below was designed into the sample layouts (a wardrobe
//! in the approach zone, a bed through the wall), not read off after the code produced it.

use interior::clearance::{check_layout, kind_of, CheckResult, Kind, Severity};
use interior::model::{default_flat, flats, footprint, Model, PlacedItem, Route};
use std::sync::Once;

const FLAT: &str = "muster";
/// The same engine on a mirrored plan: the kitchen is on the north wall and approached from the
/// south.
const FLAT_NORTH: &str = "muster-nordkueche";

/// The capability finds its data through `SJEL_PERSONAL_ROOT`; the test sets exactly that
/// variable rather than slipping a test path into `src/`.
fn model() -> Model {
    model_of(FLAT)
}

fn model_of(flat: &str) -> Model {
    fixture_overlay();
    Model::load(flat).unwrap_or_else(|e| panic!("{flat}: {e}"))
}

/// Once per test binary, since `set_var` would race the other test threads. `SJEL_INTERIOR_FLAT`
/// is removed, because this binary also checks that two flats without a choice are an error.
///
/// The database is a fresh temp file filled through the same import `interior import` uses: a
/// second way to create rows is a second way to create them wrong.
fn fixture_overlay() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-engine-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");

        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
}

fn check(layout: &str) -> CheckResult {
    check_in(FLAT, layout)
}

fn check_in(flat: &str, layout: &str) -> CheckResult {
    let m = model_of(flat);
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{flat}/{layout}: {e}"));
    check_layout(&m, &l).unwrap_or_else(|e| panic!("{flat}/{layout}: {e}"))
}

fn rules(r: &CheckResult, severity: Severity) -> Vec<&str> {
    let list = match severity {
        Severity::Hard => &r.hard,
        Severity::Soft => &r.soft,
    };
    list.iter().map(|v| v.rule.as_str()).collect()
}

// ---------------------------------------------------------------- verdicts

#[test]
fn a_free_layout_passes_without_a_single_warning() {
    let r = check("a-frei");
    assert!(r.pass, "hard violations: {:?}", rules(&r, Severity::Hard));
    assert!(r.hard.is_empty());
    assert!(
        r.soft.is_empty(),
        "soft warnings without cause: {:?}",
        r.soft.iter().map(|v| &v.message).collect::<Vec<_>>()
    );
}

/// Failing and having an opinion differ: R3 reports, and the layout still passes.
#[test]
fn a_soft_rule_reports_and_lets_pass() {
    let r = check("b-lichtkorridor");
    assert!(r.pass, "R3 is soft and must not fail the layout");
    assert_eq!(rules(&r, Severity::Soft), ["R3"]);
    let v = &r.soft[0];
    assert_eq!(v.item.as_deref(), Some("kleiderschrank"));
    // The threshold comes from rules.toml, not from the code.
    assert_eq!((v.measured, v.required), (Some(200), Some(140)));
}

#[test]
fn a_hard_rule_fails_the_layout() {
    let r = check("c-terrassentuer");
    assert!(!r.pass);
    assert!(rules(&r, Severity::Hard).contains(&"R1"), "{:?}", r.hard);
    assert_eq!(r.hard[0].item.as_deref(), Some("kleiderschrank"));
}

#[test]
fn a_piece_outside_the_room_is_no_close_call() {
    let r = check("d-ausserhalb");
    assert!(!r.pass);
    assert!(
        rules(&r, Severity::Hard).contains(&"room_boundary"),
        "{:?}",
        r.hard
    );
}

/// Rotation swaps width and depth, nothing else: D and E place the same bed almost in the same
/// spot, D sticks out, E fits, and `rot = 90` is the whole difference.
#[test]
fn a_rotation_by_90_degrees_swaps_width_and_depth() {
    let m = model();
    let mut it = PlacedItem {
        reference: "bett".into(),
        x: 0,
        y: 0,
        rot: 0,
        size: None,
        kind: None,
    };
    let (b, t, _) = footprint(&it, &m.catalogue).unwrap();
    it.rot = 90;
    let (b90, t90, _) = footprint(&it, &m.catalogue).unwrap();
    assert_eq!((b, t), (140, 200));
    assert_eq!((b90, t90), (t, b));

    let r = check("e-gedreht");
    assert!(
        r.pass,
        "across, the bed fits: {:?}",
        rules(&r, Severity::Hard)
    );
}

// ---------------------------------------------------------------- numbers

/// `Room::area_m2` computes the area from the polygon; `room.toml` states 99.0 on purpose, so
/// reading the file would show here.
#[test]
fn the_area_comes_from_the_polygon_not_the_file() {
    let m = model();
    assert_eq!(
        m.room.main_room.area_m2, 99.0,
        "the trap must be in the file"
    );
    assert!(
        (m.room.area_m2() - 27.0).abs() < 0.001,
        "{}",
        m.room.area_m2()
    );
}

/// Routes come from `room.toml` in its order. A window is an opening but no waypoint.
#[test]
fn routes_run_to_doors_not_windows() {
    let r = check("a-frei");
    let pairs: Vec<(&str, &str)> = r
        .metrics
        .corridors
        .iter()
        .map(|c| (c.from.as_str(), c.to.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("eingangstuer", "terrassentuer"),
            ("eingangstuer", "kuechenzeile"),
            ("eingangstuer", "badtuer"),
        ]
    );
    for c in &r.metrics.corridors {
        let w = c
            .width_cm
            .unwrap_or_else(|| panic!("{} → {} hat keinen Weg", c.from, c.to));
        assert!(w >= 90, "{} → {} misst {w} cm", c.from, c.to);
    }
}

/// A piece that narrows the path narrows the measured number.
#[test]
fn an_obstacle_narrows_the_route_it_blocks() {
    let frei = check("a-frei");
    let blocked = check("c-terrassentuer");
    let w = |r: &CheckResult| r.metrics.corridors[0].width_cm.unwrap();
    assert!(
        w(&blocked) < w(&frei),
        "wardrobe in front of the door: {} cm, without it {} cm",
        w(&blocked),
        w(&frei)
    );
}

/// An estimate must reach the report, and only for the layout where the estimated piece stands.
#[test]
fn unmeasured_sizes_go_into_the_report_of_the_layout_using_them() {
    let with = check("e-gedreht");
    assert_eq!(with.uncertainties.len(), 1);
    assert_eq!(with.uncertainties[0].reference, "sideboard");
    assert_eq!(with.uncertainties[0].fields, ["width", "depth"]);
    assert!(check("a-frei").uncertainties.is_empty());
}

// ---------------------------------------------------------------- inventory

/// Neither `kind` nor `state` is in a file; both follow from the key an entry sits under, and the
/// import keeps them as two facts.
#[test]
fn kind_and_state_are_two_facts_not_one() {
    use interior::store::{Kind, State};
    let m = model();
    // The count is fixed so an import that silently loses rows shows up here.
    assert_eq!(m.catalogue.len(), 18);
    let kind = |id: &str| m.catalogue.get(id).map(|i| i.kind);
    let state = |id: &str| m.states.get(id).copied();

    // owned.toml -> a thing that is there
    assert_eq!(
        (kind("bett"), state("bett")),
        (Some(Kind::Piece), Some(State::Owned))
    );
    // [[slot]] -> a need, no product yet
    assert_eq!(
        (kind("teppich"), state("teppich")),
        (Some(Kind::Slot), Some(State::Wanted))
    );
    // [[produkt]] -> a concrete thing still missing: same kind as the bed, different state.
    assert_eq!(
        (kind("stehlampe"), state("stehlampe")),
        (Some(Kind::Piece), Some(State::Wanted))
    );

    // And the wishlist is exactly that set.
    let mut offen: Vec<&str> = m.wishlist().iter().map(|i| i.id.as_str()).collect();
    offen.sort_unstable();
    assert_eq!(
        offen,
        [
            "bilderleiste",
            "deckenlampe",
            "garderobenhaken",
            "nachttisch",
            "stehlampe",
            "teppich",
            "wandregal"
        ]
    );
}

/// Prices go in cents: 49.99 EUR is 4999 cents, not the 4998 `as i64` alone gives.
#[test]
fn a_price_is_rounded_not_truncated() {
    let m = model();
    let lamp = m.catalogue.get("stehlampe").expect("the floor lamp");
    assert_eq!(lamp.price_cents, Some(8900));
}

/// The bug behind declarations: `^couch` caught `couchtisch`, so a coffee table was checked
/// against a sofa's rules. While the name heuristic lives, it is tested.
#[test]
fn a_coffee_table_is_no_sofa() {
    let k = |r: &str| {
        kind_of(&PlacedItem {
            reference: r.into(),
            x: 0,
            y: 0,
            rot: 0,
            size: None,
            kind: None,
        })
    };
    assert_eq!(k("couchtisch"), Kind::CoffeeTable);
    assert_eq!(k("couch"), Kind::Couch);
    assert_eq!(k("bett"), Kind::Bed);
    assert_eq!(k("kleiderschrank"), Kind::Wardrobe);
    assert_eq!(k("esstisch"), Kind::Table);
    assert_eq!(k("kallax_regal"), Kind::Shelf);
    // The explicit field beats the name.
    assert_eq!(
        kind_of(&PlacedItem {
            reference: "couchtisch".into(),
            x: 0,
            y: 0,
            rot: 0,
            size: None,
            kind: Some("couch".into()),
        }),
        Kind::Couch
    );
}

/// Every threshold is read by name from `rules.toml`; an unknown name is an error, not a default.
#[test]
fn the_flat_thresholds_come_from_its_file() {
    let m = model();
    assert_eq!(m.rules.threshold("vor_kuechenzeile").unwrap(), 100);
    assert_eq!(m.rules.threshold("bett_zugang_zweite_seite").unwrap(), 40);
    assert!(
        m.rules.threshold("gibt_es_nicht").is_err(),
        "an unknown threshold is an error, not a default"
    );

    // And it reaches the message.
    let r = check_in(FLAT_NORTH, "b-vor-der-kueche");
    let v = r.hard.iter().find(|v| v.rule == "R7").expect("R7");
    assert_eq!(v.required, Some(100));
}

// ---------------------------------------------------------------- what a piece demands

/// Two layouts, same footprint and position, one difference: `rot`. One passes, one fails,
/// because the wardrobe has `opens = "sued"` with `wall_ok = false` and its rotated doors face the
/// wall. The name version took the best of four sides and could not see this.
#[test]
fn a_piece_that_declares_its_space_is_no_longer_judged_by_name() {
    assert!(check("a-frei").pass);

    let rotated = check("f-schrank-zur-wand");
    assert!(!rotated.pass);
    let v = rotated
        .hard
        .iter()
        .find(|v| v.rule == "open")
        .expect("the doors face the wall");
    assert_eq!(v.item.as_deref(), Some("kleiderschrank"));
    // 65, measured on the piece, not the generic 90 from rules.toml.
    assert_eq!((v.measured, v.required), (Some(0), Some(65)));
    let m = model();
    assert_eq!(m.rules.threshold("schrank_tuer_oeffnen").unwrap(), 90);
}

/// The old version keeps running beside it: `sideboard` declares nothing and is still judged by
/// its name.
#[test]
fn who_declares_nothing_is_still_judged_by_name() {
    let m = model();
    let declared = m.catalogue.get("kleiderschrank").unwrap();
    let quiet = m.catalogue.get("sideboard").unwrap();
    assert_eq!(declared.open_clear, Some(65));
    assert_eq!(declared.opens, Some(interior::model::Side::South));
    assert_eq!(
        (quiet.open_clear, quiet.access_sides, quiet.expands_to),
        (None, None, None)
    );

    // And the layout using both passes; the name version files a sideboard as `Other`.
    assert!(check("e-gedreht").pass);
}

/// A rotation turns the declared side, clockwise, since y grows south.
#[test]
fn a_rotation_rotates_the_declared_side() {
    use interior::model::Side;
    assert_eq!(Side::South.rotated(0), Side::South);
    assert_eq!(Side::South.rotated(90), Side::West);
    assert_eq!(Side::South.rotated(180), Side::North);
    assert_eq!(Side::South.rotated(270), Side::East);
    assert_eq!(Side::North.rotated(360), Side::North);
    assert_eq!(Side::East.rotated(-90), Side::North);
    // No multiples of 90: `footprint` only swaps near 90° too.
    assert_eq!(Side::South.rotated(45), Side::South);
}

// ---------------------------------------------------------------- routes

/// The two sample flats differ only in the side their `room.toml` names for the kitchen's
/// approach zone; the zone follows it.
#[test]
fn an_approach_zone_is_on_the_side_the_flat_names() {
    let north = check_in(FLAT_NORTH, "b-vor-der-kueche");
    let hits: Vec<&str> = north.hard.iter().map(|v| v.rule.as_str()).collect();
    assert_eq!(hits, ["R7"], "expected exactly R7, got {:?}", north.hard);
    assert_eq!(north.hard[0].required, Some(100));
    assert!(
        north.hard[0].message.contains("kuechenzeile"),
        "the message names the fixture, not a category: {}",
        north.hard[0].message
    );

    // The same flat without the table in front: no violation, so it is the position.
    assert!(check_in(FLAT_NORTH, "a-frei").pass);
}

/// A waypoint that does not exist is an error, not a lost route.
#[test]
fn a_route_to_an_unknown_waypoint_is_an_error() {
    let mut m = model();
    m.room.routes.push(Route {
        from: "eingangstuer".into(),
        to: "kellerabgang".into(),
    });
    let l = m.load_layout("a-frei").unwrap();
    let err = check_layout(&m, &l).expect_err("an unknown door must show up");
    let text = err.to_string();
    assert!(text.contains("kellerabgang"), "{text}");
    // The message says what exists.
    assert!(text.contains("eingangstuer"), "{text}");
}

/// Waypoint and approach zone are one declaration: a fixture without it occupies space and is no
/// target.
#[test]
fn a_fixture_without_an_approach_zone_is_no_target() {
    let mut m = model();
    for f in m.room.fixed_furniture.iter_mut() {
        f.approach_zone = None;
    }
    let l = m.load_layout("a-frei").unwrap();
    let err = check_layout(&m, &l).expect_err("the route to the kitchen has no target anymore");
    assert!(err.to_string().contains("kuechenzeile"), "{err}");
}

/// Empty means none, stated rather than defaulted.
#[test]
fn a_flat_without_declared_routes_gets_none_measured() {
    let mut m = model();
    m.room.routes.clear();
    let l = m.load_layout("a-frei").unwrap();
    let r = check_layout(&m, &l).expect("no path is not an error");
    assert!(r.metrics.corridors.is_empty());
    assert!(
        r.pass,
        "without declared routes there is nothing to violate"
    );
}

/// Two flats and none chosen: an error, not a default.
#[test]
fn several_flats_without_a_choice_are_an_error() {
    fixture_overlay();
    let alle = flats().expect("flats/ is readable");
    assert_eq!(alle, [FLAT, FLAT_NORTH]);

    let err = default_flat().expect_err("two flats, none chosen");
    let text = err.to_string();
    assert!(text.contains(FLAT) && text.contains(FLAT_NORTH), "{text}");
    assert!(
        text.contains("SJEL_INTERIOR_FLAT"),
        "nennt den Ausweg: {text}"
    );
}

// ---------------------------------------------------------------- reserves

/// A verdict is one bit and does not say whether it was close. The property is checked, not a
/// recorded value.
#[test]
fn a_passing_layout_says_by_how_much_it_passes() {
    let r = check("a-frei");
    assert!(r.pass);
    assert!(
        !r.reserves.is_empty(),
        "no measurement recorded at all; then the checker measures nothing it names"
    );
    let tightest = r
        .tightest_reserve_cm
        .expect("an existing layout has a tightest hard measurement");
    assert!(
        tightest >= 0,
        "passes, but the tightest hard reserve is {tightest} cm"
    );
}

/// Where it breaks, the reserve is negative by the penetration depth: `c-terrassentuer` puts the
/// wardrobe into the terrace door's approach zone (R1, hard).
#[test]
fn a_hard_violation_makes_its_reserve_negative() {
    let r = check("c-terrassentuer");
    assert!(!r.pass);
    let r1 = r
        .reserves
        .iter()
        .find(|x| x.rule == "R1")
        .expect("R1 is measured whether it fires or not");
    assert!(
        r1.slack < 0,
        "R1 is broken and the reserve reports {} cm",
        r1.slack
    );
    assert!(
        r.tightest_reserve_cm.is_some_and(|c| c < 0),
        "the tightest reserve of a failing layout cannot be positive"
    );
}

/// Reserve and verdict must not contradict each other, across all layouts: a hard reserve that
/// misses has a hard violation of the same rule beside it.
#[test]
fn no_hard_reserve_contradicts_its_verdict() {
    let m = model();
    for name in m.layout_names().unwrap() {
        let r = check(&name);
        for res in r.reserves.iter().filter(|x| x.hard && x.slack < 0) {
            assert!(
                r.hard.iter().any(|v| v.rule == res.rule),
                "{name}: reserve {} reports {} cm too little, but no hard violation names it",
                res.rule,
                res.slack
            );
        }
    }
}

/// Where a measurement reaches its horizon it says so, instead of reporting a bound as a value.
#[test]
fn a_capped_measurement_shows_itself_as_a_lower_bound() {
    let r = check("a-frei");
    let capped: Vec<&str> = r
        .reserves
        .iter()
        .filter(|x| x.capped)
        .map(|x| x.rule.as_str())
        .collect();
    assert!(
        !capped.is_empty(),
        "in this room at least one side depth must reach the horizon"
    );
    for res in r.reserves.iter().filter(|x| x.capped) {
        assert!(
            res.slack >= interior::geometry::RESERVE_HORIZON,
            "{} is reported capped, but lies below the horizon",
            res.rule
        );
    }
}

/// A failing layout has a negative reserve, or one that does not measure cm. `access` counts
/// sides and R9 hours; both can fail without a negative centimetre.
#[test]
fn a_failing_layout_has_no_positive_tightest_reserve() {
    let m = model();
    for name in m.layout_names().unwrap() {
        let r = check(&name);
        if r.pass {
            continue;
        }
        let in_cm = r.tightest_reserve_cm.is_some_and(|c| c < 0);
        let other = r
            .reserves
            .iter()
            .any(|x| x.hard && x.unit != "cm" && x.slack < 0);
        assert!(
            in_cm || other,
            "{name} fails, but no hard reserve is negative: {:?}",
            r.hard.iter().map(|v| &v.rule).collect::<Vec<_>>()
        );
    }
}

/// A piece sticking out of the room reports the depth it sticks out.
#[test]
fn the_room_boundary_measures_how_far_a_piece_sticks_out() {
    let r = check("d-ausserhalb");
    assert!(!r.pass);
    let g = r
        .reserves
        .iter()
        .find(|x| x.rule == "room_boundary")
        .expect("the room boundary is measured");
    assert!(
        g.slack < 0,
        "the bed sticks through the east wall, the reserve reports {} cm",
        g.slack
    );
}

/// A wall behind is no tight spot. `k-vitrine` stands flush at the north wall, where the room
/// boundary measures the 1 cm inset; counted, every sensibly furnished flat would report 1 cm. A
/// negative value still counts (the test above).
#[test]
fn a_wall_behind_does_not_lower_the_tightest_reserve() {
    let r = check("k-vitrine");
    assert!(r.pass);
    let g = r
        .reserves
        .iter()
        .find(|x| x.rule == "room_boundary")
        .expect("the room boundary is measured");
    assert_eq!(
        g.slack, 1,
        "flush against the wall, so the one centimetre inset"
    );
    assert!(!g.binding, "a wall behind is no tight spot");
    assert!(
        r.tightest_reserve_cm.is_some_and(|c| c > 1),
        "the tightest reserve must not be the wall: {:?}",
        r.tightest_reserve_cm
    );
}
