//! Was auf dem Blatt steht, muss lesbar sein.
//!
//! Am 2026-08-31 war es das nicht. Der Plan trug die Katalogtexte, nur an Komma und Klammer
//! gekuerzt: zwei Stuehle mit demselben Wort quer uebereinander, ein Pflanzenname am Bildrand
//! abgeschnitten, der Name eines Geraets quer ueber drei Nachbarn — und ein Buerostuhl, der
//! "Schreibtisch" hiess, weil er die Zonen eines Arbeitsplatzes erbt. Dazu spannte eine
//! geschaetzte Aussenflaeche den Bildausschnitt mit auf und liess den gemessenen Raum kleiner
//! aussehen, als er ist.
//!
//! Geprueft wird gegen die Musterwohnung. Die echte liegt im Overlay und geht dieses Binary
//! nichts an.

use interior::model::Model;
use interior::plan;
use std::sync::Once;

const FLAT: &str = "muster";

/// Dieselbe Umgebung, die `tests/engine.rs` setzt, und aus demselben Grund: die Capability loest
/// ihre Daten ueber `SJEL_PERSONAL_ROOT` auf, und ein Codeweg, den kein Deployment nimmt,
/// beweist nichts ueber das Deployment. Gelesen wird hier nur.
fn model() -> Model {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-plan-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    Model::load(FLAT).unwrap_or_else(|e| panic!("{FLAT}: {e}"))
}

fn zeichne(layout: &str) -> String {
    let m = model();
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{layout}: {e}"));
    plan::svg(&m, &l).unwrap_or_else(|e| panic!("{layout}: {e}"))
}

/// Die viewBox als vier Zahlen.
fn ausschnitt(svg: &str) -> [i32; 4] {
    let von = svg.find("viewBox=\"").expect("das SVG traegt eine viewBox") + 9;
    let bis = von + svg[von..].find('"').expect("die viewBox endet");
    let mut it = svg[von..bis].split_whitespace().map(|n| {
        n.parse::<i32>()
            .unwrap_or_else(|_| panic!("die viewBox traegt Zahlen, nicht {n:?}"))
    });
    [
        it.next().expect("min_x"),
        it.next().expect("min_y"),
        it.next().expect("Breite"),
        it.next().expect("Hoehe"),
    ]
}

/// Jeder Text zwischen `>` und `</text>`.
fn texte(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(i) = rest.find("<text") {
        rest = &rest[i..];
        let Some(auf) = rest.find('>') else { break };
        let Some(zu) = rest.find("</text>") else {
            break;
        };
        out.push(rest[auf + 1..zu].to_string());
        rest = &rest[zu + 7..];
    }
    out
}

/// Der Katalogtext gehoert an die Oberflaeche und nicht ins Rechteck. Geprueft an dem Layout,
/// dessen drei Stuecke im Musterinventar bewusst lange Bezeichnungen tragen.
#[test]
fn im_moebel_steht_der_kurze_name() {
    let svg = zeichne("a-frei");
    let m = model();
    for ref_ in ["kleiderschrank", "schreibtisch", "bett"] {
        let lang = &m.catalogue.get(ref_).expect("im Katalog").label;
        assert!(
            !svg.contains(lang.as_str()),
            "der Plan traegt den Katalogtext {lang:?} von {ref_}"
        );
    }
    for kurz in ["Schrank", "Schreibtisch", "Bett"] {
        assert!(
            texte(&svg).iter().any(|t| t == kurz),
            "der Plan traegt {kurz:?} nicht: {:?}",
            texte(&svg)
        );
    }
}

/// Ein Stueck, eine Beschriftung. Der Fehler vom 2026-08-31 war nicht, dass ein Name falsch war,
/// sondern dass zwei Kaesten denselben trugen und uebereinander lagen.
#[test]
fn jedes_stueck_traegt_genau_einen_text() {
    let svg = zeichne("a-frei");
    let m = model();
    let l = m.load_layout("a-frei").expect("das Musterlayout");
    for gruppe in svg.split("<g data-ref=\"").skip(1) {
        let ende = gruppe.find("</g>").expect("jede Gruppe schliesst");
        assert_eq!(
            gruppe[..ende].matches("<text").count(),
            1,
            "eine Gruppe traegt nicht genau einen Text: {}",
            &gruppe[..ende.min(120)]
        );
    }
    assert_eq!(svg.matches("<g data-ref=\"").count(), l.items.len());
}

/// Eine Schaetzung bestimmt den Massstab des Gemessenen nicht.
///
/// Die Musterwohnung fuehrt eine geschaetzte Terrasse westlich der Verglasung. Der
/// Bildausschnitt darf links nur den Zeichenrand zeigen und nicht ihre geschaetzte Tiefe —
/// gezeichnet wird sie trotzdem, sonst waere aus dem Plan nicht zu erkennen, dass es sie gibt.
#[test]
fn die_geschaetzte_terrasse_spannt_das_blatt_nicht_auf() {
    let m = model();
    let t = m
        .room
        .terrasse
        .as_ref()
        .expect("die Fixture fuehrt eine Terrasse");
    assert!(t.geschaetzt, "diese Pruefung gilt der geschaetzten Flaeche");
    let svg = zeichne("a-frei");
    let [min_x, _, breite, _] = ausschnitt(&svg);

    let links = m
        .room
        .hauptraum
        .polygon
        .iter()
        .map(|p| p[0])
        .min()
        .expect("das Polygon hat Ecken");
    assert!(
        min_x > t.x[0],
        "der Bildausschnitt beginnt bei der geschaetzten Terrassentiefe ({min_x})"
    );
    assert!(
        min_x < links,
        "vom Band der Terrasse ist nichts zu sehen ({min_x} statt links von {links})"
    );

    let gemessen = m.room.bad.as_ref().map_or(links, |b| b.x[1]).max(
        m.room
            .hauptraum
            .polygon
            .iter()
            .map(|p| p[0])
            .max()
            .unwrap_or(links),
    );
    assert!(
        (gemessen - links) * 2 > breite,
        "das Gemessene nimmt weniger als die halbe Blattbreite ein ({} von {breite})",
        gemessen - links
    );
    assert!(
        texte(&svg).iter().any(|t| t.starts_with("TERRASSE")),
        "die Terrasse ist nicht mehr beschriftet"
    );
}
