//! Die Maschine, gegen eine erfundene Wohnung.
//!
//! Diese Datei ist der Grund, warum die Capability nach PRD Q59 oeffentlich stehen kann und
//! trotzdem geprueft ist. `tests/fixtures/overlay/` ist ein vollstaendiges `data/interior/`
//! mit ausgedachten Massen; die echte Wohnung liegt im privaten Overlay und wird nur von
//! `tests/live_parity.rs` angefasst, das sich ohne Overlay ueberspringt.
//!
//! Was hier NICHT steht: aufgezeichnete Ergebnisse. Jede Behauptung unten ist beim Entwurf der
//! Musterlayouts konstruiert worden — ein Schrank steht in der Anlaufzone, ein Bett ragt durch
//! die Wand — und nicht abgelesen, nachdem der Code sie erzeugt hat. Ein Test, der aufschreibt,
//! was passiert ist, bemerkt eine falsche Regel nie.

use interior::clearance::{check_layout, kind_of, CheckResult, Kind, Severity};
use interior::model::{default_flat, flats, footprint, Model, PlacedItem, Route};
use std::sync::Once;

const FLAT: &str = "muster";
/// Dieselbe Maschine, ein gespiegelter Grundriss: die Kueche steht an der Nordwand und wird
/// von Sueden angelaufen.
const FLAT_NORTH: &str = "muster-nordkueche";

/// Die Capability loest ihre Daten ueber `SJEL_PERSONAL_ROOT` auf. Der Test setzt genau diese
/// Variable, statt `src/` einen Testpfad unterzuschieben: ein Codeweg, den kein Deployment
/// nimmt, beweist nichts ueber das Deployment.
fn model() -> Model {
    model_of(FLAT)
}

fn model_of(flat: &str) -> Model {
    fixture_overlay();
    Model::load(flat).unwrap_or_else(|e| panic!("{flat}: {e}"))
}

/// Genau einmal je Testbinary, weil `set_var` in einem laufenden Prozess sonst gegen die
/// anderen Testfaeden liefe. `SJEL_INTERIOR_FLAT` wird dabei GELOESCHT: dieses Binary prueft
/// auch, dass zwei Wohnungen ohne Wahl ein Fehler sind, und eine geerbte Umgebung waere die
/// eine Bedingung, unter der dieser Test still bestaende, ohne etwas zu pruefen.
///
/// Die Datenbank ist eine frische Datei im Temp-Verzeichnis und wird ueber DENSELBEN Import
/// gefuellt, den `interior import` fuehrt (PRD B25). Kein Test-Seed neben dem echten Weg: eine
/// zweite Art, Zeilen anzulegen, ist eine zweite Art, sie falsch anzulegen.
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

        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
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

// ---------------------------------------------------------------- die Verdikte

#[test]
fn a_free_layout_passes_without_a_single_warning() {
    let r = check("a-frei");
    assert!(r.pass, "harte Verstoesse: {:?}", rules(&r, Severity::Hard));
    assert!(r.hard.is_empty());
    assert!(
        r.soft.is_empty(),
        "weiche Warnungen ohne Anlass: {:?}",
        r.soft.iter().map(|v| &v.message).collect::<Vec<_>>()
    );
}

/// Der Unterschied zwischen "faellt durch" und "hat eine Meinung dazu". Ein Pruefer, der beides
/// gleich behandelt, ist unbrauchbar — deshalb wird hier BEIDES geprueft: dass R3 meldet, und
/// dass das Layout trotzdem besteht.
#[test]
fn a_soft_rule_reports_and_lets_pass() {
    let r = check("b-lichtkorridor");
    assert!(r.pass, "R3 ist weich und darf nicht durchfallen lassen");
    assert_eq!(rules(&r, Severity::Soft), ["R3"]);
    let v = &r.soft[0];
    assert_eq!(v.item.as_deref(), Some("kleiderschrank"));
    // Die Schwelle kommt aus rules.toml, nicht aus dem Code. Steht hier je eine andere Zahl,
    // hat sich jemand einen Wert ausgedacht, statt ihn nachzuschlagen.
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
        rules(&r, Severity::Hard).contains(&"raumgrenze"),
        "{:?}",
        r.hard
    );
}

/// Drehung tauscht Breite und Tiefe — sonst nichts. D und E stellen dasselbe Bett an fast
/// dieselbe Stelle; D ragt heraus, E passt, und `rot = 90` ist der ganze Unterschied.
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
        "quer passt das Bett: {:?}",
        rules(&r, Severity::Hard)
    );
}

// ---------------------------------------------------------------- die Zahlen

/// `Room::area_m2` behauptet, die Flaeche aus dem Polygon zu rechnen statt sie zu lesen.
/// `room.toml` fuehrt deshalb absichtlich 99,0 — wer die Datei liest, faellt hier auf.
#[test]
fn the_area_comes_from_the_polygon_not_the_file() {
    let m = model();
    assert_eq!(
        m.room.main_room.flaeche_m2, 99.0,
        "die Falle muss in der Datei stehen"
    );
    assert!(
        (m.room.area_m2() - 27.0).abs() < 0.001,
        "{}",
        m.room.area_m2()
    );
}

/// Die Routen kommen aus `room.toml` und stehen in der Reihenfolge, in der sie dort stehen.
/// `kuechenfenster` ist eine Oeffnung und trotzdem kein Wegpunkt — sonst misst der Pruefer den
/// Weg zu etwas, durch das niemand geht.
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

/// Ein Moebel, das den Weg verengt, verengt die gemessene Zahl. Ohne diese Kopplung waere die
/// Wegsuche eine Konstante mit Nachkommastellen.
#[test]
fn an_obstacle_narrows_the_route_it_blocks() {
    let frei = check("a-frei");
    let blocked = check("c-terrassentuer");
    let w = |r: &CheckResult| r.metrics.corridors[0].width_cm.unwrap();
    assert!(
        w(&blocked) < w(&frei),
        "Schrank vor der Tuer: {} cm, ohne ihn {} cm",
        w(&blocked),
        w(&frei)
    );
}

/// Eine Schaetzung, die unterwegs zur Messung wird, ist der Fehler, gegen den `unsicher`
/// existiert. Sie muss bis in den Bericht durchschlagen — und nur dort auftauchen, wo das
/// geschaetzte Stueck wirklich steht.
#[test]
fn unmeasured_sizes_go_into_the_report_of_the_layout_using_them() {
    let with = check("e-gedreht");
    assert_eq!(with.uncertainties.len(), 1);
    assert_eq!(with.uncertainties[0].reference, "sideboard");
    assert_eq!(with.uncertainties[0].fields, ["b", "t"]);
    assert!(check("a-frei").uncertainties.is_empty());
}

// ---------------------------------------------------------------- das Inventar

/// Weder `kind` noch `state` steht in einer Datei — beide folgen daraus, unter welchem
/// Schluessel ein Eintrag steht, und der Import trennt sie in ZWEI Tatsachen (PRD B25).
/// Vorher war es eine: eine `group` von `owned` / `slot` / `produkt`, die Besitz und Bauart
/// in ein Wort presste, sodass "ein Produkt, das ich schon habe" nicht sagbar war.
#[test]
fn kind_and_state_are_two_facts_not_one() {
    use interior::store::{Kind, State};
    let m = model();
    // 12 seit 2026-08-31: `trenner_erklaert` und `trenner_stumm` kamen mit B31 dazu, zwei
    // Bretter mit identischen Massen, von denen genau eines `raumtrenner = true` fuehrt. Die
    // Zahl steht hier fest, damit ein Import, der still Zeilen verliert, an ihr auffaellt —
    // sie mitwachsen zu lassen hiesse, die Pruefung abzuschaffen, die sie ist.
    assert_eq!(m.catalogue.len(), 18);
    let kind = |id: &str| m.catalogue.get(id).map(|i| i.kind);
    let state = |id: &str| m.states.get(id).copied();

    // owned.toml -> ein Ding, das da ist
    assert_eq!(
        (kind("bett"), state("bett")),
        (Some(Kind::Piece), Some(State::Owned))
    );
    // [[slot]] -> ein BEDARF, noch ohne Produkt
    assert_eq!(
        (kind("teppich"), state("teppich")),
        (Some(Kind::Slot), Some(State::Wanted))
    );
    // [[produkt]] -> ein konkretes Ding, das noch fehlt. Dieselbe Bauart wie das Bett,
    // anderer Zustand — genau die Unterscheidung, die eine `group` nicht treffen konnte.
    assert_eq!(
        (kind("stehlampe"), state("stehlampe")),
        (Some(Kind::Piece), Some(State::Wanted))
    );

    // Und die Wunschliste ist genau die Menge, die B29 an ein Budget legt.
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

/// Preise wandern in Cent, weil `finance` in Cent rechnet. 49,99 EUR sind 4999 Cent und nicht
/// 4998 — `as i64` allein liefert genau das, und der Fehler ist ein Cent je Position.
#[test]
fn a_price_is_rounded_not_truncated() {
    let m = model();
    let lamp = m.catalogue.get("stehlampe").expect("die Stehleuchte");
    assert_eq!(lamp.price_cents, Some(8900));
}

/// Der Fehler, der die Namensheuristik zum Abschuss freigegeben hat (PRD Q61): `^couch` fing
/// `couchtisch`, und ein Couchtisch wurde gegen die Regeln eines Sofas geprueft. Gefunden wurde
/// er erst, als ein echter Esstisch dazukam. Solange die Heuristik lebt, gehoert sie geprueft.
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
    // Das explizite Feld schlaegt den Namen — der Ausweg, den Q61 zur Regel machen wird.
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

/// Jede Schwelle wird ueber ihren Namen aus `rules.toml` gelesen — die, die der Wohnung
/// gehoeren. Ein unbekannter Name ist ein Fehler und kein Standardwert.
#[test]
fn the_flat_thresholds_come_from_its_file() {
    let m = model();
    assert_eq!(m.rules.abstand("vor_kuechenzeile").unwrap(), 100);
    assert_eq!(m.rules.abstand("bett_zugang_zweite_seite").unwrap(), 40);
    assert!(
        m.rules.abstand("gibt_es_nicht").is_err(),
        "ein unbekannter Abstand ist ein Fehler, kein Standardwert"
    );

    // Und sie schlaegt bis in die Meldung durch.
    let r = check_in(FLAT_NORTH, "b-vor-der-kueche");
    let v = r.hard.iter().find(|v| v.rule == "R7").expect("R7");
    assert_eq!(v.required, Some(100));
}

// ---------------------------------------------------------------- was ein Stueck verlangt

/// **Der Kern von Q61.** Zwei Layouts, identische Grundflaeche, identische Position, ein
/// einziger Unterschied: `rot`. Das eine besteht, das andere faellt durch, weil der Schrank
/// `opens = "sued"` mit `wall_ok = false` fuehrt und seine Tueren gedreht zur Wand zeigen.
///
/// Die Namensfassung konnte das nicht sehen. Sie nahm die BESTE von vier Seiten, also war ein
/// Schrank mit dem Ruecken zum Raum genauso richtig wie einer mit den Tueren zum Raum.
#[test]
fn a_piece_that_declares_its_space_is_no_longer_judged_by_name() {
    assert!(check("a-frei").pass);

    let rotated = check("f-schrank-zur-wand");
    assert!(!rotated.pass);
    let v = rotated
        .hard
        .iter()
        .find(|v| v.rule == "oeffnen")
        .expect("die Tueren zeigen zur Wand");
    assert_eq!(v.item.as_deref(), Some("kleiderschrank"));
    // 65, am Stueck gemessen — NICHT die 90, die rules.toml als generische Schwelle fuehrt.
    // Genau diese Verwechslung ist der Grund, aus dem das Feld ans Moebel gehoert.
    assert_eq!((v.measured, v.required), (Some(0), Some(65)));
    let m = model();
    assert_eq!(m.rules.abstand("schrank_tuer_oeffnen").unwrap(), 90);
}

/// Die alte Fassung laeuft daneben weiter, und das ist Absicht: 42 Zeilen an einem Tag
/// umzustellen waere ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand die Zahlen
/// dahinter geprueft hat. `sideboard` erklaert nichts und wird weiter am Namen gemessen.
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

    // Und das Layout, das beide benutzt, besteht — die Namensfassung stuft ein Sideboard als
    // `Other` ein und verlangt nichts.
    assert!(check("e-gedreht").pass);
}

/// Eine Drehung dreht die erklaerte Side mit. Im Uhrzeigersinn, weil y nach Sueden waechst
/// und das auf dem Plan die Richtung ist, in die ein Zeiger laeuft.
#[test]
fn a_rotation_rotates_the_declared_side() {
    use interior::model::Side;
    assert_eq!(Side::South.rotated(0), Side::South);
    assert_eq!(Side::South.rotated(90), Side::West);
    assert_eq!(Side::South.rotated(180), Side::North);
    assert_eq!(Side::South.rotated(270), Side::East);
    assert_eq!(Side::North.rotated(360), Side::North);
    assert_eq!(Side::East.rotated(-90), Side::North);
    // Keine Vielfachen von 90: `footprint` tauscht die Grundflaeche auch nur nahe 90 Grad, und
    // zwei verschiedene Rundungen fuer dieselbe Drehung waeren ein Fehler, den niemand sucht.
    assert_eq!(Side::South.rotated(45), Side::South);
}

// ---------------------------------------------------------------- die Wege

/// **Der Fund, der B26a ausgeloest hat.** Bis 2026-08-30 rechnete `clearance.rs` die Anlaufzone
/// der Kueche als Rechteck OBERHALB des Einbaus und fand den Einbau ueber den Namen
/// `kuechenzeile`. Beides stimmte fuer genau eine Wohnung. Diese beiden Musterwohnungen
/// unterscheiden sich in nichts als der Side, die ihre `room.toml` nennt.
#[test]
fn an_approach_zone_is_on_the_side_the_flat_names() {
    let north = check_in(FLAT_NORTH, "b-vor-der-kueche");
    let hits: Vec<&str> = north.hard.iter().map(|v| v.rule.as_str()).collect();
    assert_eq!(
        hits,
        ["R7"],
        "erwartet genau R7, bekommen {:?}",
        north.hard
    );
    assert_eq!(north.hard[0].required, Some(100));
    assert!(
        north.hard[0].message.contains("kuechenzeile"),
        "die Meldung nennt den Einbau, nicht eine Kategorie: {}",
        north.hard[0].message
    );

    // Dieselbe Wohnung ohne den Tisch davor: kein Verstoss, also ist es die Position und nicht
    // die blosse Anwesenheit einer Kueche.
    assert!(check_in(FLAT_NORTH, "a-frei").pass);
}

/// Ein Wegpunkt, den es nicht gibt, ist ein Fehler. Die Stelle hat bis 2026-08-30 still
/// weitergemacht: ein Tippfehler kostete eine Route, und der Bericht sah danach aus wie einer
/// ueber eine Wohnung mit weniger Wegen.
#[test]
fn a_route_to_an_unknown_waypoint_is_an_error() {
    let mut m = model();
    m.room.routes.push(Route {
        von: "eingangstuer".into(),
        nach: "kellerabgang".into(),
    });
    let l = m.load_layout("a-frei").unwrap();
    let err = check_layout(&m, &l).expect_err("eine unbekannte Tuer muss auffallen");
    let text = err.to_string();
    assert!(text.contains("kellerabgang"), "{text}");
    // Die Meldung sagt, was es gaebe. Ein Fehler, der nur "unbekannt" sagt, kostet den
    // naechsten Leser genau den Blick in die Datei, den er gerade gemacht hat.
    assert!(text.contains("eingangstuer"), "{text}");
}

/// Wegpunkt und Anlaufzone sind dieselbe Deklaration. Ein fester Einbau ohne sie belegt
/// Flaeche und sonst nichts — er verlangt keinen freien Platz und taugt nicht als Ziel.
#[test]
fn a_fixture_without_an_approach_zone_is_no_target() {
    let mut m = model();
    for f in m.room.fixed_furniture.iter_mut() {
        f.approach_zone = None;
    }
    let l = m.load_layout("a-frei").unwrap();
    let err = check_layout(&m, &l).expect_err("die Route zur Kueche hat kein Ziel mehr");
    assert!(err.to_string().contains("kuechenzeile"), "{err}");
}

/// Leer heisst keiner, und das ist eine Aussage. Ein stiller Standard waere hier der Weg, auf
/// dem eine Wohnung besteht, weil niemand nachgesehen hat, ob jemand hindurchkommt.
#[test]
fn a_flat_without_declared_routes_gets_none_measured() {
    let mut m = model();
    m.room.routes.clear();
    let l = m.load_layout("a-frei").unwrap();
    let r = check_layout(&m, &l).expect("kein Weg ist kein Fehler");
    assert!(r.metrics.corridors.is_empty());
    assert!(
        r.pass,
        "ohne erklaerte Wege gibt es auch nichts zu verletzen"
    );
}

/// Zwei Wohnungen und keine gewaehlt: ein Fehler, keine Vorauswahl. Sobald B28 einen zweiten
/// Raum zum Vergleich stellt, waere ein stiller Standard genau der Weg, auf dem ein Plan der
/// falschen Wohnung als der richtige durchgeht.
#[test]
fn several_flats_without_a_choice_are_an_error() {
    fixture_overlay();
    let alle = flats().expect("flats/ ist lesbar");
    assert_eq!(alle, [FLAT, FLAT_NORTH]);

    let err = default_flat().expect_err("zwei Wohnungen, keine gewaehlt");
    let text = err.to_string();
    assert!(text.contains(FLAT) && text.contains(FLAT_NORTH), "{text}");
    assert!(
        text.contains("SJEL_INTERIOR_FLAT"),
        "nennt den Ausweg: {text}"
    );
}

// ---------------------------------------------------------------- die Reserven

/// Ein Verdikt ist ein Bit, und ein Bit sagt nicht, ob es knapp war.
///
/// Diese Zahl ist der Grund, warum `Reserve` ueberhaupt existiert: zwei Layouts, die beide
/// `pass` melden, koennen 0 cm und 80 cm Luft haben, und bis 2026-08-31 waren sie im Bericht
/// dasselbe Wort. Geprueft wird die Eigenschaft, nicht der Wert — ein aufgeschriebener
/// Zentimeterstand waere ein Protokoll dessen, was der Code tut, und kein Anspruch an ihn.
#[test]
fn a_passing_layout_says_by_how_much_it_passes() {
    let r = check("a-frei");
    assert!(r.pass);
    assert!(
        !r.reserves.is_empty(),
        "keine einzige Messung festgehalten — dann misst der Pruefer nichts, was er nennt"
    );
    let tightest = r
        .tightest_reserve_cm
        .expect("ein bestehendes Layout hat eine knappste harte Messung");
    assert!(
        tightest >= 0,
        "bestanden, aber die knappste harte Reserve ist {tightest} cm"
    );
}

/// Und umgekehrt: wo es reisst, ist die Reserve negativ und sagt, um wie viel.
///
/// `c-terrassentuer` stellt den Schrank in die Anlaufzone der Terrassentuer (R1, hart). Die
/// zugehoerige Reserve muss die Eindringtiefe tragen, sonst ist sie eine Zahl, die nur im
/// guten Fall stimmt.
#[test]
fn a_hard_violation_makes_its_reserve_negative() {
    let r = check("c-terrassentuer");
    assert!(!r.pass);
    let r1 = r
        .reserves
        .iter()
        .find(|x| x.rule == "R1")
        .expect("R1 wird gemessen, ob sie greift oder nicht");
    assert!(
        r1.slack < 0,
        "R1 ist verletzt und die Reserve meldet {} cm",
        r1.slack
    );
    assert!(
        r.tightest_reserve_cm.is_some_and(|c| c < 0),
        "die knappste Reserve eines durchgefallenen Layouts kann nicht positiv sein"
    );
}

/// Reserve und Verdikt duerfen sich nicht widersprechen — ueber ALLE Layouts der Wohnung.
///
/// Das ist die Invariante, die eine zweite Fassung derselben Messung verhindert: sagt eine
/// harte Reserve „verfehlt", muss ein harter Verstoss derselben Regel danebenstehen. Ohne
/// diesen Test koennte die Reserve still ihre eigene Geometrie entwickeln, und genau das ist
/// der Fehler, gegen den diese Capability existiert.
#[test]
fn no_hard_reserve_contradicts_its_verdict() {
    let m = model();
    for name in m.layout_names().unwrap() {
        let r = check(&name);
        for res in r.reserves.iter().filter(|x| x.hart && x.slack < 0) {
            assert!(
                r.hard.iter().any(|v| v.rule == res.rule),
                "{name}: Reserve {} meldet {} cm zu wenig, aber kein harter Verstoss nennt sie",
                res.rule,
                res.slack
            );
        }
    }
}

/// Wo die Messung ihren Horizont erreicht, sagt sie das, statt eine Schranke als Messwert
/// auszugeben.
///
/// In einem 600 x 450 cm grossen Raum mit drei Stuecken hat mindestens eine Side mehr Luft,
/// als `RESERVE_HORIZONT` weit gezaehlt wird. Ohne `gedeckelt` stuende dort eine exakte Zahl,
/// die keine ist.
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
        "in diesem Raum muss mindestens eine Seitentiefe an den Horizont stossen"
    );
    for res in r.reserves.iter().filter(|x| x.capped) {
        assert!(
            res.slack >= interior::geometry::RESERVE_HORIZON,
            "{} ist als gedeckelt gemeldet, liegt aber unter dem Horizont",
            res.rule
        );
    }
}

/// Ein durchgefallenes Layout hat eine negative Reserve — oder eine, die nicht in cm misst.
///
/// Der Anlass steht in der echten Wohnung und nicht in dieser Fixture: `d-schrank-trennt`
/// meldete am 2026-08-31 **DURCHGEFALLEN und +5 cm Reserve zugleich**, weil die einzige
/// verletzte Regel — `raumgrenze` — als einzige harte Regel gar nichts mass. Die knappste Zahl
/// beschrieb dann die Regeln, die bestanden hatten.
///
/// Der Ausweg war nicht, den Test zu lockern, sondern `raumgrenze` messen zu lassen. Was hier
/// steht, ist die Bedingung, unter der das so bleibt: `zugang` zaehlt Seiten und R9 Stunden, und
/// beide koennen reissen, ohne dass ein Zentimeter negativ wird — deshalb ist die Aussage eine
/// Oder-Aussage und keine Aufweichung.
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
            .any(|x| x.hart && x.unit != "cm" && x.slack < 0);
        assert!(
            in_cm || other,
            "{name} faellt durch, aber keine harte Reserve ist negativ: {:?}",
            r.hard.iter().map(|v| &v.rule).collect::<Vec<_>>()
        );
    }
}

/// Ein Stueck, das aus dem Raum ragt, meldet die Tiefe seines Ueberstands.
#[test]
fn the_room_boundary_measures_how_far_a_piece_sticks_out() {
    let r = check("d-ausserhalb");
    assert!(!r.pass);
    let g = r
        .reserves
        .iter()
        .find(|x| x.rule == "raumgrenze")
        .expect("die Raumgrenze wird gemessen");
    assert!(
        g.slack < 0,
        "das Bett ragt durch die Ostwand, die Reserve meldet {} cm",
        g.slack
    );
}

/// Eine Wand im Ruecken ist keine Enge.
///
/// `k-vitrine` stellt ein Stueck buendig an die Nordwand. Die Raumgrenze misst dort einen
/// Zentimeter — den Einzug, mit dem die Regel selbst ihre Ecken prueft —, und wuerde sie
/// mitzaehlen, meldete **jede vernuenftig eingerichtete Wohnung 1 cm Reserve**. Genau das ist
/// am 2026-08-31 an der echten Wohnung passiert, eine Stunde nachdem die Messung dazukam: aus
/// 9 cm wurden 1 cm, und die Zahl sagte nichts mehr.
///
/// Ein NEGATIVER Wert zaehlt weiter — herausragen ist ein Verstoss. Das prueft der Test
/// darueber.
#[test]
fn a_wall_behind_does_not_lower_the_tightest_reserve() {
    let r = check("k-vitrine");
    assert!(r.pass);
    let g = r
        .reserves
        .iter()
        .find(|x| x.rule == "raumgrenze")
        .expect("die Raumgrenze wird gemessen");
    assert_eq!(
        g.slack, 1,
        "buendig an der Wand, also ein Zentimeter Einzug"
    );
    assert!(!g.binding, "eine Wand im Ruecken ist kein knapper Platz");
    assert!(
        r.tightest_reserve_cm.is_some_and(|c| c > 1),
        "die knappste Reserve darf nicht die Wand sein: {:?}",
        r.tightest_reserve_cm
    );
}
