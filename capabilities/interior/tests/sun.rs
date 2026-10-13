//! Der Sonnenstand, gegen Eigenschaften geprueft statt gegen eine Tabelle.
//!
//! Es gibt hier keine abgeschriebenen Referenzwerte, und das ist Absicht: eine Tabelle aus
//! einer fremden Quelle prueft, ob jemand richtig abgeschrieben hat. Die Aussagen unten folgen
//! dagegen aus der Himmelsmechanik selbst und lassen sich am Papier nachrechnen —
//! **Mittagshoehe = 90 Grad minus dem Abstand zwischen Breite und Deklination**, und die
//! Deklination erreicht an den Sonnenwenden die Schiefe der Ekliptik.
//!
//! Eine falsch gerechnete Zeitgleichung, eine vertauschte Achse oder ein Vorzeichenfehler in
//! der Deklination faellt an genau diesen Saetzen um.

use interior::sun::sun_position;

/// Die Schiefe der Ekliptik. Kein Parameter dieses Codes, sondern die Groesse, die er
/// reproduzieren muss.
const ECLIPTIC: f64 = 23.44;

fn peak(tag: u32, width: f64, length: f64) -> (f64, f64) {
    let mut best = (-90.0f64, 0.0);
    for q in 0..(24 * 60) {
        let s = sun_position(2026, tag, q as f64 / 60.0, width, length);
        if s.altitude_deg > best.0 {
            best = (s.altitude_deg, s.azimuth_deg);
        }
    }
    best
}

/// Zur Sommersonnenwende steht die Sonne genau um die Schiefe der Ekliptik hoeher als zur
/// Tagundnachtgleiche, und zur Wintersonnenwende ebenso viel tiefer.
#[test]
fn the_solstices_hit_the_obliquity_of_the_ecliptic() {
    let width = 50.0;
    let (june, _) = peak(172, width, 0.0);
    let (dezember, _) = peak(355, width, 0.0);
    assert!(
        (june - (90.0 - width + ECLIPTIC)).abs() < 0.3,
        "21. Juni auf {width} Grad Nord: {june:.2} statt {:.2}",
        90.0 - width + ECLIPTIC
    );
    assert!(
        (dezember - (90.0 - width - ECLIPTIC)).abs() < 0.3,
        "21. Dezember auf {width} Grad Nord: {dezember:.2} statt {:.2}",
        90.0 - width - ECLIPTIC
    );
}

/// Am Aequator zur Sommersonnenwende: die Sonne bleibt genau um die Ekliptikschiefe unter dem
/// Zenit. Ein Vorzeichenfehler in der Deklination gibt hier 90 Grad und faellt sofort auf.
#[test]
fn at_the_equator_the_sun_stays_within_the_ecliptic_of_the_zenith() {
    let (june, _) = peak(172, 0.0, 0.0);
    assert!(
        (june - (90.0 - ECLIPTIC)).abs() < 0.3,
        "Aequator am 21. Juni: {june:.2} statt {:.2}",
        90.0 - ECLIPTIC
    );
}

/// Zur Tagundnachtgleiche ist die Mittagshoehe das Komplement der Breite — auf jeder Breite.
#[test]
fn at_the_equinox_the_noon_altitude_is_the_latitude_complement() {
    for width in [0.0, 20.0, 50.0, 60.0] {
        let (h, _) = peak(266, width, 0.0);
        assert!(
            (h - (90.0 - width)).abs() < 1.0,
            "auf {width} Grad: {h:.2} statt {:.2}",
            90.0 - width
        );
    }
}

/// Auf der Nordhalbkugel kulminiert die Sonne im Sueden, und der Azimut zaehlt von Norden.
#[test]
fn the_peak_is_in_the_south() {
    let (_, azimuth) = peak(172, 50.0, 0.0);
    assert!(
        (azimuth - 180.0).abs() < 3.0,
        "Kulmination bei Azimut {azimuth:.1} statt bei 180"
    );
}

/// Morgens im Osten, abends im Westen. Ein vertauschter Stundenwinkel spiegelt genau das.
#[test]
fn the_sun_moves_from_east_to_west() {
    let morgens = sun_position(2026, 172, 5.0, 50.0, 0.0);
    let evening = sun_position(2026, 172, 17.0, 50.0, 0.0);
    assert!(
        morgens.azimuth_deg < 130.0,
        "um 05:00 UTC steht sie im Osten, nicht bei {:.1}",
        morgens.azimuth_deg
    );
    assert!(
        evening.azimuth_deg > 230.0,
        "um 17:00 UTC steht sie im Westen, nicht bei {:.1}",
        evening.azimuth_deg
    );
}

/// Nachts steht sie unter dem Horizont, und das darf keine Ausnahme brauchen.
#[test]
fn at_midnight_it_is_below_the_horizon() {
    let s = sun_position(2026, 355, 0.0, 50.0, 0.0);
    assert!(
        s.altitude_deg < 0.0,
        "{:.2} Grad um Mitternacht",
        s.altitude_deg
    );
}

// ---------------------------------------------------------------- der Lichtfleck

use interior::model::{Model, Rect};
use interior::sun::{bericht, touch, light_patch, SunPosition};
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
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
}

/// Bei 45 Grad Sonnenhoehe ist der Versatz gleich der Hoehe — Tangens von 45 ist eins.
///
/// Die Musterwohnung hat ihre Terrassentuer in der Westwand, bodentief bis 210 cm. Steht die
/// Sonne genau im Westen und 45 Grad hoch, muss der Lichtfleck exakt 210 cm weit nach Osten in
/// den Raum reichen. Diese Zahl folgt aus der Geometrie und nicht aus einem Probelauf.
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
    let f = light_patch(&m, door, stand, 0.0).expect("die Westwand bekommt Westsonne");
    let furthest = f.corners.iter().fold(0.0f64, |a, e| a.max(e[0]));
    assert!(
        (furthest - 210.0).abs() < 1.0,
        "der Fleck reicht {furthest:.1} cm weit statt 210"
    );
}

/// Flacher Stand heisst laengerer Fleck. Halber Tangens, doppelte Weite.
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
        "30 Grad wirft {flat_angle:.0} cm, 60 Grad {up:.0} cm — das ist nicht der Tangens"
    );
}

/// Die Sonne im Osten scheint nicht durch eine Tuer in der Westwand.
///
/// Ohne diese Pruefung waere der Lichtfleck ein Rechteck, das aus jeder Wand in jede Richtung
/// faellt — die Sorte Fehler, die im Bericht wie ein sehr sonniger Raum aussieht.
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
        "Ostsonne kommt nicht durch die Westwand"
    );
}

/// Unter dem Horizont gibt es kein Licht — und keine Division, die gegen unendlich laeuft.
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

/// Ein Stueck IM Fleck wird getroffen, dasselbe Stueck daneben nicht.
///
/// Der Trennachsensatz ist eine Aussage in beide Richtungen, und ein Test, der nur den Treffer
/// prueft, bestuende auch bei einer Funktion, die immer `true` sagt.
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
    // Die Tuer spannt y 120..300; der Fleck reicht x 0..210.
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
    assert!(!touch(&beside, &f), "noerdlich der Tuerspanne");
    assert!(
        !touch(&behind, &f),
        "weiter im Raum als der Fleck reicht"
    );
}

/// Der Jahresbericht laeuft und zaehlt, statt eine Meinung zu haben.
#[test]
fn the_year_report_names_hours_and_gaps() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    let b = bericht(&m, &l).unwrap();
    assert_eq!(
        b.hours.len(),
        interior::sun::DAYS.len() * interior::sun::HOURS.count(),
        "vier Tage mal elf Stunden"
    );
    assert!(
        b.without_glass_heights.is_empty(),
        "die Musterwohnung erklaert beide Verglasungen: {:?}",
        b.without_glass_heights
    );
    assert!(
        b.hours.iter().any(|s| !s.hit.is_empty()),
        "eine Westwohnung ohne einen einzigen Sonnentreffer waere ein Rechenfehler"
    );
    assert!(
        b.hours.iter().any(|s| s.altitude_deg < 0.0),
        "am 21. Dezember um 08:00 steht die Sonne hier noch nicht"
    );
}

/// R9 trennt, statt ueberall zu feuern.
///
/// Derselbe Schreibtisch in derselben Wohnung: in `a-frei` bekommt er 11 der 44 geprueften
/// Stunden Sonne und schweigt, in `i-blick-aufs-bett` 15 und meldet. Die Schwelle steht in
/// `rules.toml` auf 12. Eine Regel, an deren Schwelle kein Layout auf der einen und keines auf
/// der anderen Side liegt, prueft nichts — dieselbe Lehre, die R5 einmal gekostet hat.
#[test]
fn r9_tells_the_sunny_spot_from_the_desk() {
    let m = model();
    let hell = m.load_layout("i-blick-aufs-bett").unwrap();
    let r = interior::clearance::check_layout(&m, &hell).unwrap();
    assert_eq!(
        r.soft.iter().filter(|v| v.rule == "R9").count(),
        1,
        "15 von 44 Stunden ueber einer Schwelle von 12"
    );

    let frei = m.load_layout("a-frei").unwrap();
    let r = interior::clearance::check_layout(&m, &frei).unwrap();
    assert!(
        !r.soft.iter().any(|v| v.rule == "R9"),
        "11 von 44 liegt darunter"
    );
    // Und die Reserve traegt die Stunden, nicht Zentimeter — sonst liefe sie in
    // `engste_reserve_cm` ein und machte daraus eine Zahl ohne Einheit.
    let res = r
        .reserves
        .iter()
        .find(|x| x.rule == "R9")
        .expect("gemessen wird auch, wenn nichts meldet");
    assert_eq!(res.unit, "stunden");
    assert_eq!(res.slack, 1, "12 erlaubt, 11 gemessen");
}

/// Fuehrt eine Wohnung R9 ohne Standort, ist das eine Luecke und kein Bestehen.
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
        .expect("R9 meldet sich als ungeprueft");
    assert!(
        offen.reason.contains("[lage]"),
        "und nennt, was fehlt: {}",
        offen.reason
    );
}
