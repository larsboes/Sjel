//! Kommt das Stueck herein — und bis an seinen Platz?
//!
//! Die Faelle hier sind konstruiert und nicht abgelesen. `k-vitrine` ist der Kern: ein Layout,
//! das jede Raeumungsregel erfuellt und trotzdem nie stattfinden kann, weil das Moebel 10 cm
//! breiter ist als die Wohnungstuer. Bis 2026-08-31 hat diese Capability darauf `BESTANDEN`
//! geantwortet — richtig gerechnet, falsche Frage.

use interior::einbringung::{durch_die_tuer, einbringung, Tuerpass};
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
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
}

/// Ein bestandenes Layout und ein Stueck, das nicht hereinkommt — beides zugleich wahr.
///
/// Das ist der ganze Grund fuer dieses Modul. Wuerde `check_layout` hier durchfallen, waere
/// die Frage schon beantwortet und `einbringung.rs` ueberfluessig.
#[test]
fn ein_regelkonformes_layout_kann_an_der_tuer_scheitern() {
    let m = model();
    let l = m.load_layout("k-vitrine").unwrap();
    let verdikt = interior::clearance::check_layout(&m, &l).unwrap();
    assert!(
        verdikt.pass,
        "die Vitrine steht regelkonform: {:?}",
        verdikt.hard
    );

    let e = einbringung(&m, &l, "glasvitrine").unwrap();
    assert_eq!(
        e.tuer,
        Tuerpass::PasstNicht {
            fehlen_cm: 10,
            tuer_cm: 100
        },
        "110 cm in beiden Richtungen an einer 100 cm breiten Tuer"
    );
}

/// Die schmale Seite entscheidet: ein Schrank geht hochkant durch die Tuer.
///
/// Ohne diese Regel waere jedes Stueck breiter als 100 cm ein Problem, und das ist offenkundig
/// falsch — der 120 x 60 cm Kleiderschrank kommt durch jede Wohnungstuer.
#[test]
fn die_schmale_seite_entscheidet_und_nicht_die_breite() {
    let m = model();
    assert!(
        matches!(durch_die_tuer(&m, 120, 60, false), Tuerpass::Passt { .. }),
        "120 x 60 passt hochkant durch 100 cm"
    );
    assert!(
        matches!(
            durch_die_tuer(&m, 120, 110, false),
            Tuerpass::PasstNicht { .. }
        ),
        "120 x 110 passt in keiner Lage"
    );
}

/// Was zerlegt kommt, wird an der Tuer nicht gemessen — und sagt das, statt zu bestehen.
///
/// Das Bett ist 140 cm breit und steht trotzdem in jedem Schlafzimmer. Die Auskunft kommt aus
/// seiner Zeile (`zerlegbar = true`) und nicht aus einer Vermutung ueber Betten.
#[test]
fn ein_zerlegbares_stueck_wird_an_der_tuer_nicht_gemessen() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    let e = einbringung(&m, &l, "bett").unwrap();
    assert_eq!(
        e.tuer,
        Tuerpass::ZerlegtGetragen {
            fehlen_cm: 40,
            tuer_cm: 100
        }
    );
    assert!(
        !matches!(durch_die_tuer(&m, 140, 200, false), Tuerpass::Passt { .. }),
        "ohne die Deklaration bliebe es ein Verstoss"
    );
}

/// Ein Platz, den es gibt, ist ein Platz, zu dem es einen Weg gibt.
#[test]
fn jedes_stueck_eines_bestandenen_layouts_erreicht_seinen_platz() {
    let m = model();
    let l = m.load_layout("a-frei").unwrap();
    for it in &l.items {
        let e = einbringung(&m, &l, &it.reference).unwrap();
        assert!(
            e.erreichbar,
            "{} kommt nicht an seinen Platz: {:?}",
            it.reference, e.grund
        );
        assert!(e.schritte.is_some_and(|n| n > 0));
    }
}

/// Ohne deklarierten Eingang wird nicht geraten.
///
/// Dieselbe Haltung wie bei R6: `badtuer` und `eingangstuer` tragen denselben `typ`, und sie
/// am Namen zu unterscheiden waere der Fehler, den PRD B26a einmal geschlossen hat.
#[test]
fn ohne_deklarierten_eingang_wird_nicht_geraten() {
    let mut m = model();
    for o in m.room.oeffnungen.iter_mut() {
        o.eingang = None;
    }
    assert_eq!(durch_die_tuer(&m, 50, 50, false), Tuerpass::KeinEingang);
}
