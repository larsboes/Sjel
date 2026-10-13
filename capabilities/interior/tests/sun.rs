//! The sun position, checked against properties rather than a table: a copied table only checks
//! the copying. The claims below follow from celestial mechanics and can be worked on paper: noon
//! altitude is 90 degrees minus the distance between latitude and declination, and the declination
//! reaches the obliquity of the ecliptic at the solstices. A wrong equation of time, a swapped axis
//! or a sign error in the declination fails one of these.

use interior::sun::sun_position;

/// The obliquity of the ecliptic: not a parameter of this code, but the value it must reproduce.
const ECLIPTIC: f64 = 23.44;

fn peak(day: u32, latitude: f64, longitude: f64) -> (f64, f64) {
    let mut best = (-90.0f64, 0.0);
    for q in 0..(24 * 60) {
        let s = sun_position(2026, day, q as f64 / 60.0, latitude, longitude);
        if s.altitude_deg > best.0 {
            best = (s.altitude_deg, s.azimuth_deg);
        }
    }
    best
}

/// At the summer solstice the sun stands exactly the obliquity higher than at the equinox, and at
/// the winter solstice as much lower.
#[test]
fn the_solstices_hit_the_obliquity_of_the_ecliptic() {
    let latitude = 50.0;
    let (june, _) = peak(172, latitude, 0.0);
    let (december, _) = peak(355, latitude, 0.0);
    assert!(
        (june - (90.0 - latitude + ECLIPTIC)).abs() < 0.3,
        "21 June at {latitude} degrees north: {june:.2} instead of {:.2}",
        90.0 - latitude + ECLIPTIC
    );
    assert!(
        (december - (90.0 - latitude - ECLIPTIC)).abs() < 0.3,
        "21 December at {latitude} degrees north: {december:.2} instead of {:.2}",
        90.0 - latitude - ECLIPTIC
    );
}

/// At the equator on the summer solstice the sun stays exactly the obliquity below the zenith; a
/// sign error in the declination gives 90 degrees here.
#[test]
fn at_the_equator_the_sun_stays_within_the_ecliptic_of_the_zenith() {
    let (june, _) = peak(172, 0.0, 0.0);
    assert!(
        (june - (90.0 - ECLIPTIC)).abs() < 0.3,
        "Aequator am 21. Juni: {june:.2} statt {:.2}",
        90.0 - ECLIPTIC
    );
}

/// At the equinox the noon altitude is the complement of the latitude, at every latitude.
#[test]
fn at_the_equinox_the_noon_altitude_is_the_latitude_complement() {
    for latitude in [0.0, 20.0, 50.0, 60.0] {
        let (h, _) = peak(266, latitude, 0.0);
        assert!(
            (h - (90.0 - latitude)).abs() < 1.0,
            "at {latitude} degrees: {h:.2} instead of {:.2}",
            90.0 - latitude
        );
    }
}

/// In the northern hemisphere the sun culminates in the south, and azimuth counts from north.
#[test]
fn the_peak_is_in_the_south() {
    let (_, azimuth) = peak(172, 50.0, 0.0);
    assert!(
        (azimuth - 180.0).abs() < 3.0,
        "culmination at azimuth {azimuth:.1} instead of 180"
    );
}

/// East in the morning, west in the evening; a swapped hour angle mirrors exactly that.
#[test]
fn the_sun_moves_from_east_to_west() {
    let morgens = sun_position(2026, 172, 5.0, 50.0, 0.0);
    let evening = sun_position(2026, 172, 17.0, 50.0, 0.0);
    assert!(
        morgens.azimuth_deg < 130.0,
        "at 05:00 UTC it is in the east, not at {:.1}",
        morgens.azimuth_deg
    );
    assert!(
        evening.azimuth_deg > 230.0,
        "at 17:00 UTC it is in the west, not at {:.1}",
        evening.azimuth_deg
    );
}

/// At night it is below the horizon, without needing an exception.
#[test]
fn at_midnight_it_is_below_the_horizon() {
    let s = sun_position(2026, 355, 0.0, 50.0, 0.0);
    assert!(
        s.altitude_deg < 0.0,
        "{:.2} Grad um Mitternacht",
        s.altitude_deg
    );
}

// ---------------------------------------------------------------- the light patch

use interior::model::{Model, Rect};
use interior::sun::{report, touch, light_patch, SunPosition};
use std::sync::Once;

fn model() -> Model {
    fixture_overlay();
    Model::load("muster").unwrap_or_else(|e| panic!("muster: {e}"))
}

fn fixture_overlay() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-sonne-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
}

/// At 45 degrees altitude the offset equals the height, since tan 45 is one. The sample flat's
/// terrace door is in the west wall, glazed to 210 cm, so with the sun due west at 45 degrees the
/// patch must reach exactly 210 cm east into the room.
#[test]
fn at_forty_five_degrees_the_offset_is_the_glass_height() {
    let m = model();
    let door = m
        .room
        .openings
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let stand = SunPosition {
        altitude_deg: 45.0,
        azimuth_deg: 270.0,
    };
    let f = light_patch(&m, door, stand, 0.0).expect("the west wall gets western sun");
    let furthest = f.corners.iter().fold(0.0f64, |a, e| a.max(e[0]));
    assert!(
        (furthest - 210.0).abs() < 1.0,
        "the patch reaches {furthest:.1} cm instead of 210"
    );
}

/// A lower sun means a longer patch: half the tangent, twice the reach.
#[test]
fn a_lower_sun_throws_the_patch_further() {
    let m = model();
    let door = m
        .room
        .openings
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let reach = |h: f64| {
        light_patch(
            &m,
            door,
            SunPosition {
                altitude_deg: h,
                azimuth_deg: 270.0,
            },
            0.0,
        )
        .map(|f| f.corners.iter().fold(0.0f64, |a, e| a.max(e[0])))
    };
    let up = reach(60.0).unwrap();
    let flat_angle = reach(30.0).unwrap();
    assert!(
        flat_angle > up * 2.0,
        "30 degrees throws {flat_angle:.0} cm, 60 degrees {up:.0} cm: that is not the tangent"
    );
}

/// The sun in the east does not shine through a door in the west wall.
#[test]
fn the_sun_does_not_shine_through_the_back_of_a_wall() {
    let m = model();
    let door = m
        .room
        .openings
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    assert!(
        light_patch(
            &m,
            door,
            SunPosition {
                altitude_deg: 45.0,
                azimuth_deg: 90.0
            },
            0.0
        )
        .is_none(),
        "eastern sun does not come through the west wall"
    );
}

/// Below the horizon there is no light, and no division running to infinity.
#[test]
fn below_the_horizon_no_light_falls() {
    let m = model();
    let door = m
        .room
        .openings
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    assert!(light_patch(
        &m,
        door,
        SunPosition {
            altitude_deg: -5.0,
            azimuth_deg: 270.0
        },
        0.0
    )
    .is_none());
}

/// A piece inside the patch is hit, the same piece beside it is not; a test of hits alone would
/// pass a function that always says `true`.
#[test]
fn the_separating_axis_theorem_really_separates() {
    let m = model();
    let door = m
        .room
        .openings
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let f = light_patch(
        &m,
        door,
        SunPosition {
            altitude_deg: 45.0,
            azimuth_deg: 270.0,
        },
        0.0,
    )
    .unwrap();
    // The door spans y 120..300; the patch reaches x 0..210.
    let drin = Rect {
        x: 100,
        y: 200,
        w: 50,
        d: 50,
    };
    let beside = Rect {
        x: 100,
        y: 20,
        w: 50,
        d: 50,
    };
    let behind = Rect {
        x: 300,
        y: 200,
        w: 50,
        d: 50,
    };
    assert!(touch(&drin, &f), "mitten im Fleck");
    assert!(!touch(&beside, &f), "north of the door span");
    assert!(
        !touch(&behind, &f),
        "further into the room than the patch reaches"
    );
}

/// The year report runs and counts.
#[test]
fn the_year_report_names_hours_and_gaps() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    let b = report(&m, &l).unwrap();
    assert_eq!(
        b.hours.len(),
        interior::sun::DAYS.len() * interior::sun::HOURS.count(),
        "four days times eleven hours"
    );
    assert!(
        b.without_glass_heights.is_empty(),
        "the sample flat declares both glazings: {:?}",
        b.without_glass_heights
    );
    assert!(
        b.hours.iter().any(|s| !s.hit.is_empty()),
        "a west-facing flat without a single sun hit would be a computing error"
    );
    assert!(
        b.hours.iter().any(|s| s.altitude_deg < 0.0),
        "on 21 December at 08:00 the sun is not up here yet"
    );
}

/// R9 tells apart instead of firing everywhere: the same desk gets 11 of 44 hours in `a-frei` and
/// stays quiet, 15 in `i-blick-aufs-bett` and reports; the threshold in `rules.toml` is 12.
#[test]
fn r9_tells_the_sunny_spot_from_the_desk() {
    let m = model();
    let hell = m.load_layout("i-blick-aufs-bett").unwrap();
    let r = interior::clearance::check_layout(&m, &hell).unwrap();
    assert_eq!(
        r.soft.iter().filter(|v| v.rule == "R9").count(),
        1,
        "15 of 44 hours above a threshold of 12"
    );

    let frei = m.load_layout("a-frei").unwrap();
    let r = interior::clearance::check_layout(&m, &frei).unwrap();
    assert!(
        !r.soft.iter().any(|v| v.rule == "R9"),
        "11 of 44 lies below it"
    );
    // And the reserve carries hours, not centimetres, so it stays out of `tightest_reserve_cm`.
    let res = r
        .reserves
        .iter()
        .find(|x| x.rule == "R9")
        .expect("it is measured even when nothing reports");
    assert_eq!(res.unit, "stunden");
    assert_eq!(res.slack, 1, "12 allowed, 11 measured");
}

/// A flat with R9 but no location is a gap, not a pass.
#[test]
fn r9_without_a_location_reports_unchecked() {
    let mut m = model();
    m.room.location = None;
    let l = m.load_layout("a-frei").unwrap();
    let r = interior::clearance::check_layout(&m, &l).unwrap();
    let offen = r
        .unchecked
        .iter()
        .find(|u| u.rule == "R9")
        .expect("R9 reports itself unchecked");
    assert!(
        offen.reason.contains("[lage]"),
        "and names what is missing: {}",
        offen.reason
    );
}
