//! Die Suche, und was ihre Rangfolge fuer richtig haelt.
//!
//! `search.rs` hatte bis 2026-08-31 keinen einzigen Test — 257 Zeilen, der teuerste Codepfad
//! der Capability (3,5 Mio. Kandidaten ueber rayon), und die einzige Stelle, an der etwas
//! anderes als „erlaubt/verboten" entschieden wird.
//!
//! Was hier geprueft wird, ist eine RANGFOLGE und kein Verdikt. `raumtrenner` kann kein Layout
//! bestehen oder durchfallen lassen; es entscheidet nur, welcher von mehreren zulaessigen
//! Vorschlaegen oben steht.

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
        std::env::remove_var("AXON_INTERIOR_FLAT");
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    Model::load(FLAT).expect("Musterwohnung")
}

/// Sucht in dem Layout, das genau dieses eine Brett enthaelt.
///
/// Beide Layouts stellen ihr Brett auf dieselben Koordinaten, also unterscheiden sich die zwei
/// Laeufe in einer einzigen Sache: ob das Inventar `raumtrenner = true` fuehrt.
fn suche(m: &Model, variante: &str) -> interior::search::SearchReport {
    let base = m
        .load_layout(&format!("g-trenner-{variante}"))
        .unwrap_or_else(|e| panic!("g-trenner-{variante}: {e}"));
    search(
        m,
        &base,
        &Spec {
            move_refs: vec![format!("trenner_{variante}")],
            step: 50,
            bands: BTreeMap::new(),
            limit: 0,
        },
    )
    .expect("Suche")
}

/// Der Beleg, in einem Vergleich mit genau einer Variablen.
///
/// `trenner_erklaert` und `trenner_stumm` haben dieselben Masse und stehen in derselben
/// Wohnung. Das eine sagt `raumtrenner = true`, das andere sagt nichts. Bewegt die Suche das
/// stumme Brett, findet sie Plaetze an der Wand und bewertet sie hoeher; bewegt sie das
/// erklaerte, zaehlt seine Wandberuehrung ueberhaupt nicht mit.
///
/// Ohne den Vergleich waere der Test wertlos: eine Wandsumme von 0 kann auch heissen, dass die
/// Suche keinen Platz an einer Wand gefunden hat.
#[test]
fn ein_erklaerter_raumtrenner_wird_nicht_fuer_freies_stehen_bestraft() {
    let m = model();

    let stumm = suche(&m, "stumm");
    assert!(
        !stumm.hits.is_empty(),
        "die Suche findet Plaetze fuer das stumme Brett"
    );
    let beste_wand = stumm
        .hits
        .iter()
        .map(|h| h.wandkontakt_cm)
        .max()
        .unwrap_or(0);
    assert!(
        beste_wand > 0,
        "ein Brett, das nichts erklaert, findet eine Wand und bekommt sie angerechnet — \
         sonst prueft der Vergleich unten nichts"
    );

    let erklaert = suche(&m, "erklaert");
    assert!(
        !erklaert.hits.is_empty(),
        "und Plaetze fuer das erklaerte Brett"
    );
    assert!(
        erklaert.hits.iter().all(|h| h.wandkontakt_cm == 0),
        "ein erklaerter Raumtrenner zaehlt gar nicht in die Wandsumme — er soll die Bewertung \
         der uebrigen Stuecke weder heben noch senken"
    );
}

/// Die Rangfolge ist stabil und in der dokumentierten Reihenfolge.
///
/// Erst die Front, dann keine Warnungen, dann Wand im Ruecken, dann der breiteste Engpass. Der
/// Kommentar in `search.rs` behauptet das; hier steht es als Pruefung, damit eine vertauschte
/// `then`-Kette auffaellt.
///
/// Die erste Stufe kam am 2026-08-31 dazu und ist die einzige, die keine Meinung ist: ein
/// dominierter Treffer ist unter JEDER Gewichtung der vier Ziele schlechter als ein bestimmter
/// anderer. Innerhalb einer Klasse gilt die Rangfolge vom 2026-08-30 unveraendert weiter.
#[test]
fn die_rangfolge_sortiert_front_vor_warnungen_vor_wand_vor_engpass() {
    let m = model();
    let r = suche(&m, "stumm");
    assert!(
        r.hits.len() > 1,
        "fuer eine Rangfolge braucht es mehr als einen Treffer"
    );

    for paar in r.hits.windows(2) {
        let (a, b) = (&paar[0], &paar[1]);
        let a_key = (
            !a.pareto,
            a.soft,
            -a.wandkontakt_cm,
            -a.bottleneck_cm,
            std::cmp::Reverse(a.engste_reserve_cm),
        );
        let b_key = (
            !b.pareto,
            b.soft,
            -b.wandkontakt_cm,
            -b.bottleneck_cm,
            std::cmp::Reverse(b.engste_reserve_cm),
        );
        assert!(a_key <= b_key, "Rangfolge verletzt: {a:?} steht vor {b:?}");
    }
}

/// Die Front ist eine Auswahl und keine Umbenennung aller Treffer.
///
/// Ein Flag, das ueberall `true` steht, sortiert nichts und meldet trotzdem Erfolg — genau die
/// Art von stiller Nulloperation, die diese Datei bei `raumtrenner` schon einmal gefunden hat.
#[test]
fn die_front_ist_echt_kleiner_als_die_treffermenge() {
    let m = model();
    let r = suche(&m, "stumm");
    let front = r.hits.iter().filter(|h| h.pareto).count();
    assert!(front > 0, "ohne Front gibt es keine beste Antwort");
    assert!(
        front < r.hits.len(),
        "{front} von {} Treffern sind Front — dann trennt das Flag nichts",
        r.hits.len()
    );
}

/// Kein dominierter Treffer steht vor einem, der ihn dominiert.
///
/// Das ist die Eigenschaft, wegen der die Front ueberhaupt vorne sortiert wird, und sie wird
/// hier an den Zahlen geprueft statt am Flag: ein falsch gesetztes Flag bestuende den Test
/// oben und diesen nicht.
#[test]
fn kein_treffer_steht_vor_einem_der_ihn_in_allem_schlaegt() {
    let m = model();
    let r = suche(&m, "stumm");
    let ziel = |h: &interior::search::Hit| {
        [
            h.engste_reserve_cm.unwrap_or(i32::MIN),
            h.wandkontakt_cm,
            h.bottleneck_cm,
            -(h.soft as i32),
        ]
    };
    for (i, a) in r.hits.iter().enumerate() {
        for b in r.hits.iter().skip(i + 1) {
            let (za, zb) = (ziel(a), ziel(b));
            let dominiert = zb.iter().zip(za.iter()).all(|(x, y)| x >= y)
                && zb.iter().zip(za.iter()).any(|(x, y)| x > y);
            assert!(
                !dominiert,
                "{b:?} schlaegt {a:?} in allem und steht dahinter"
            );
        }
    }
}

/// Ein harter Verstoss kommt nie als Treffer zurueck.
///
/// Die Suche schlaegt Plaetze vor, und ein Vorschlag, der eine harte Regel verletzt, ist kein
/// Vorschlag. `clearance.rs` sagt dasselbe von der anderen Seite: es gibt keinen Weg, ein
/// Bestehen zu melden, solange ein harter Verstoss offen ist.
#[test]
fn kein_treffer_verletzt_eine_harte_regel() {
    let m = model();
    let r = suche(&m, "stumm");
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");

    for h in &r.hits {
        let mut items = base.items.clone();
        for (reference, pos) in &h.places {
            let it = items
                .iter_mut()
                .find(|i| &i.reference == reference)
                .expect("der bewegte Ref steht im Layout");
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
            "Treffer bei {:?} verletzt {:?}",
            h.places,
            c.hard.iter().map(|v| &v.rule).collect::<Vec<_>>()
        );
    }
}

/// Harte Kanten: was die Liste erlaubt, steht wirklich im Raum und nicht in einem Einbau.
///
/// Zwei Richtungen, und die zweite ist die wichtigere. Dass jede erlaubte Position zulaessig
/// ist, faellt bei einem Fehler sofort auf. Dass die Liste **nicht einfach das umschliessende
/// Rechteck** ist, faellt sonst nie auf — und genau das waere die naheliegende falsche Fassung
/// im Browser. Die Musterwohnung hat dafuer eine Kuechenzeile und einen Tuerschwenkbereich; die
/// echte Wohnung ist obendrein ein Sechseck.
#[test]
fn erlaubte_positionen_liegen_im_raum_und_nicht_in_einem_einbau() {
    use interior::model::{Layout, PlacedItem};
    let m = model();
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");
    let a = interior::search::allowed_positions(&m, &base, "trenner_stumm", 0)
        .expect("erlaubte Positionen");

    assert!(!a.rows.is_empty(), "es gibt Platz fuer das Brett");
    assert_eq!(a.step, 5);

    // Jede erlaubte Position ueberlebt die Raumgrenze.
    let mut geprueft = 0;
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
                let r = interior::clearance::check_layout(&m, &l).expect("Pruefung");
                assert!(
                    !r.hard.iter().any(|v| v.rule == "raumgrenze"),
                    "erlaubte Position {x},{} liegt ausserhalb des Raums",
                    row.y
                );
                geprueft += 1;
            }
        }
    }
    assert!(geprueft > 50, "nur {geprueft} Kanten geprueft");

    // Und die Liste ist enger als das umschliessende Rechteck. Die Kuechenzeile liegt bei
    // x 0..250, y 390..450: eine Zeile, die sie beruehrt, darf nicht bei x = 0 anfangen, eine
    // freie Zeile schon. Genau diese Verengung waere in einer Rechteck-Fassung verschwunden —
    // und die Rechteck-Fassung ist die, die im Browser naheliegt.
    let anfang = |y: i32| -> Option<i32> { a.rows.iter().find(|r| r.y == y).map(|r| r.x[0][0]) };
    assert_eq!(anfang(0), Some(0), "oben ist die Wand die einzige Grenze");
    let unten = anfang(380).expect("es gibt eine Zeile bei y = 380");
    assert!(
        unten > 0,
        "bei y = 380 reicht das Brett in die Kuechenzeile, also darf x = 0 nicht erlaubt sein \
         (bekommen: {unten})"
    );
}

/// Eine Position IM Einbau steht nicht auf der Liste.
#[test]
fn eine_position_in_der_kuechenzeile_ist_nicht_erlaubt() {
    let m = model();
    let base = m.load_layout("g-trenner-stumm").expect("g-trenner-stumm");
    let a = interior::search::allowed_positions(&m, &base, "trenner_stumm", 0)
        .expect("erlaubte Positionen");

    // Die Kuechenzeile der Musterwohnung liegt bei x 0..250, y 390..450.
    let drin = |x: i32, y: i32| {
        a.rows
            .iter()
            .find(|r| r.y == y)
            .is_some_and(|r| r.x.iter().any(|[lo, hi]| x >= *lo && x <= *hi))
    };
    assert!(
        !drin(0, 390),
        "0,390 liegt in der Kuechenzeile und darf nicht erlaubt sein"
    );
}
