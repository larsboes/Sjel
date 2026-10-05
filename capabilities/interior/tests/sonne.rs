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

use interior::sonne::sonnenstand;

/// Die Schiefe der Ekliptik. Kein Parameter dieses Codes, sondern die Groesse, die er
/// reproduzieren muss.
const EKLIPTIK: f64 = 23.44;

fn hoechststand(tag: u32, breite: f64, laenge: f64) -> (f64, f64) {
    let mut best = (-90.0f64, 0.0);
    for q in 0..(24 * 60) {
        let s = sonnenstand(2026, tag, q as f64 / 60.0, breite, laenge);
        if s.hoehe_grad > best.0 {
            best = (s.hoehe_grad, s.azimut_grad);
        }
    }
    best
}

/// Zur Sommersonnenwende steht die Sonne genau um die Schiefe der Ekliptik hoeher als zur
/// Tagundnachtgleiche, und zur Wintersonnenwende ebenso viel tiefer.
#[test]
fn die_sonnenwenden_treffen_die_schiefe_der_ekliptik() {
    let breite = 50.0;
    let (juni, _) = hoechststand(172, breite, 0.0);
    let (dezember, _) = hoechststand(355, breite, 0.0);
    assert!(
        (juni - (90.0 - breite + EKLIPTIK)).abs() < 0.3,
        "21. Juni auf {breite} Grad Nord: {juni:.2} statt {:.2}",
        90.0 - breite + EKLIPTIK
    );
    assert!(
        (dezember - (90.0 - breite - EKLIPTIK)).abs() < 0.3,
        "21. Dezember auf {breite} Grad Nord: {dezember:.2} statt {:.2}",
        90.0 - breite - EKLIPTIK
    );
}

/// Am Aequator zur Sommersonnenwende: die Sonne bleibt genau um die Ekliptikschiefe unter dem
/// Zenit. Ein Vorzeichenfehler in der Deklination gibt hier 90 Grad und faellt sofort auf.
#[test]
fn am_aequator_bleibt_die_sonne_um_die_ekliptik_unter_dem_zenit() {
    let (juni, _) = hoechststand(172, 0.0, 0.0);
    assert!(
        (juni - (90.0 - EKLIPTIK)).abs() < 0.3,
        "Aequator am 21. Juni: {juni:.2} statt {:.2}",
        90.0 - EKLIPTIK
    );
}

/// Zur Tagundnachtgleiche ist die Mittagshoehe das Komplement der Breite — auf jeder Breite.
#[test]
fn zur_tagundnachtgleiche_ist_die_mittagshoehe_das_komplement_der_breite() {
    for breite in [0.0, 20.0, 50.0, 60.0] {
        let (h, _) = hoechststand(266, breite, 0.0);
        assert!(
            (h - (90.0 - breite)).abs() < 1.0,
            "auf {breite} Grad: {h:.2} statt {:.2}",
            90.0 - breite
        );
    }
}

/// Auf der Nordhalbkugel kulminiert die Sonne im Sueden, und der Azimut zaehlt von Norden.
#[test]
fn der_hoechststand_liegt_im_sueden() {
    let (_, azimut) = hoechststand(172, 50.0, 0.0);
    assert!(
        (azimut - 180.0).abs() < 3.0,
        "Kulmination bei Azimut {azimut:.1} statt bei 180"
    );
}

/// Morgens im Osten, abends im Westen. Ein vertauschter Stundenwinkel spiegelt genau das.
#[test]
fn die_sonne_wandert_von_ost_nach_west() {
    let morgens = sonnenstand(2026, 172, 5.0, 50.0, 0.0);
    let abends = sonnenstand(2026, 172, 17.0, 50.0, 0.0);
    assert!(
        morgens.azimut_grad < 130.0,
        "um 05:00 UTC steht sie im Osten, nicht bei {:.1}",
        morgens.azimut_grad
    );
    assert!(
        abends.azimut_grad > 230.0,
        "um 17:00 UTC steht sie im Westen, nicht bei {:.1}",
        abends.azimut_grad
    );
}

/// Nachts steht sie unter dem Horizont, und das darf keine Ausnahme brauchen.
#[test]
fn um_mitternacht_steht_sie_unter_dem_horizont() {
    let s = sonnenstand(2026, 355, 0.0, 50.0, 0.0);
    assert!(
        s.hoehe_grad < 0.0,
        "{:.2} Grad um Mitternacht",
        s.hoehe_grad
    );
}

// ---------------------------------------------------------------- der Lichtfleck

use interior::model::{Model, Rect};
use interior::sonne::{bericht, beruehren, lichtfleck, Sonnenstand};
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
        let store = inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
}

/// Bei 45 Grad Sonnenhoehe ist der Versatz gleich der Hoehe — Tangens von 45 ist eins.
///
/// Die Musterwohnung hat ihre Terrassentuer in der Westwand, bodentief bis 210 cm. Steht die
/// Sonne genau im Westen und 45 Grad hoch, muss der Lichtfleck exakt 210 cm weit nach Osten in
/// den Raum reichen. Diese Zahl folgt aus der Geometrie und nicht aus einem Probelauf.
#[test]
fn bei_fuenfundvierzig_grad_ist_der_versatz_die_glashoehe() {
    let m = model();
    let tuer = m
        .room
        .oeffnungen
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let stand = Sonnenstand {
        hoehe_grad: 45.0,
        azimut_grad: 270.0,
    };
    let f = lichtfleck(&m, tuer, stand, 0.0).expect("die Westwand bekommt Westsonne");
    let weiteste = f.ecken.iter().fold(0.0f64, |a, e| a.max(e[0]));
    assert!(
        (weiteste - 210.0).abs() < 1.0,
        "der Fleck reicht {weiteste:.1} cm weit statt 210"
    );
}

/// Flacher Stand heisst laengerer Fleck. Halber Tangens, doppelte Weite.
#[test]
fn ein_flacherer_stand_wirft_den_fleck_weiter() {
    let m = model();
    let tuer = m
        .room
        .oeffnungen
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let weite = |h: f64| {
        lichtfleck(
            &m,
            tuer,
            Sonnenstand {
                hoehe_grad: h,
                azimut_grad: 270.0,
            },
            0.0,
        )
        .map(|f| f.ecken.iter().fold(0.0f64, |a, e| a.max(e[0])))
    };
    let hoch = weite(60.0).unwrap();
    let flach = weite(30.0).unwrap();
    assert!(
        flach > hoch * 2.0,
        "30 Grad wirft {flach:.0} cm, 60 Grad {hoch:.0} cm — das ist nicht der Tangens"
    );
}

/// Die Sonne im Osten scheint nicht durch eine Tuer in der Westwand.
///
/// Ohne diese Pruefung waere der Lichtfleck ein Rechteck, das aus jeder Wand in jede Richtung
/// faellt — die Sorte Fehler, die im Bericht wie ein sehr sonniger Raum aussieht.
#[test]
fn die_sonne_scheint_nicht_durch_die_rueckseite_einer_wand() {
    let m = model();
    let tuer = m
        .room
        .oeffnungen
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    assert!(
        lichtfleck(
            &m,
            tuer,
            Sonnenstand {
                hoehe_grad: 45.0,
                azimut_grad: 90.0
            },
            0.0
        )
        .is_none(),
        "Ostsonne kommt nicht durch die Westwand"
    );
}

/// Unter dem Horizont gibt es kein Licht — und keine Division, die gegen unendlich laeuft.
#[test]
fn unter_dem_horizont_faellt_kein_licht() {
    let m = model();
    let tuer = m
        .room
        .oeffnungen
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    assert!(lichtfleck(
        &m,
        tuer,
        Sonnenstand {
            hoehe_grad: -5.0,
            azimut_grad: 270.0
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
fn der_trennachsensatz_trennt_wirklich() {
    let m = model();
    let tuer = m
        .room
        .oeffnungen
        .iter()
        .find(|o| o.id == "terrassentuer")
        .unwrap();
    let f = lichtfleck(
        &m,
        tuer,
        Sonnenstand {
            hoehe_grad: 45.0,
            azimut_grad: 270.0,
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
    let daneben = Rect {
        x: 100,
        y: 20,
        w: 50,
        d: 50,
    };
    let dahinter = Rect {
        x: 300,
        y: 200,
        w: 50,
        d: 50,
    };
    assert!(beruehren(&drin, &f), "mitten im Fleck");
    assert!(!beruehren(&daneben, &f), "noerdlich der Tuerspanne");
    assert!(
        !beruehren(&dahinter, &f),
        "weiter im Raum als der Fleck reicht"
    );
}

/// Der Jahresbericht laeuft und zaehlt, statt eine Meinung zu haben.
#[test]
fn der_jahresbericht_nennt_stunden_und_luecken() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    let b = bericht(&m, &l).unwrap();
    assert_eq!(
        b.stunden.len(),
        interior::sonne::TAGE.len() * interior::sonne::STUNDEN.count(),
        "vier Tage mal elf Stunden"
    );
    assert!(
        b.ohne_glashoehen.is_empty(),
        "die Musterwohnung erklaert beide Verglasungen: {:?}",
        b.ohne_glashoehen
    );
    assert!(
        b.stunden.iter().any(|s| !s.getroffen.is_empty()),
        "eine Westwohnung ohne einen einzigen Sonnentreffer waere ein Rechenfehler"
    );
    assert!(
        b.stunden.iter().any(|s| s.hoehe_grad < 0.0),
        "am 21. Dezember um 08:00 steht die Sonne hier noch nicht"
    );
}

/// R9 trennt, statt ueberall zu feuern.
///
/// Derselbe Schreibtisch in derselben Wohnung: in `a-frei` bekommt er 11 der 44 geprueften
/// Stunden Sonne und schweigt, in `i-blick-aufs-bett` 15 und meldet. Die Schwelle steht in
/// `rules.toml` auf 12. Eine Regel, an deren Schwelle kein Layout auf der einen und keines auf
/// der anderen Seite liegt, prueft nichts — dieselbe Lehre, die R5 einmal gekostet hat.
#[test]
fn r9_trennt_den_sonnenplatz_vom_arbeitsplatz() {
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
        .reserven
        .iter()
        .find(|x| x.rule == "R9")
        .expect("gemessen wird auch, wenn nichts meldet");
    assert_eq!(res.einheit, "stunden");
    assert_eq!(res.slack, 1, "12 erlaubt, 11 gemessen");
}

/// Fuehrt eine Wohnung R9 ohne Standort, ist das eine Luecke und kein Bestehen.
#[test]
fn r9_ohne_lage_meldet_sich_als_ungeprueft() {
    let mut m = model();
    m.room.lage = None;
    let l = m.load_layout("a-frei").unwrap();
    let r = interior::clearance::check_layout(&m, &l).unwrap();
    let offen = r
        .nicht_geprueft
        .iter()
        .find(|u| u.rule == "R9")
        .expect("R9 meldet sich als ungeprueft");
    assert!(
        offen.grund.contains("[lage]"),
        "und nennt, was fehlt: {}",
        offen.grund
    );
}
