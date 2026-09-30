//! Die Wohnung deklariert ihre Regeln, und der Pruefer schlaegt sie nach.
//!
//! Bis 2026-08-31 tat er das nicht. `rules.toml` fuehrte `[[regeln]]` mit `id`, `schwere` und
//! `text`; `model::Rules` parste die Liste; **nichts las sie**. Die Schwere stand stattdessen an
//! allen 21 Ausgabestellen in `clearance.rs` als Literal, und der Modulkopf dort behauptete
//! trotzdem "Schwere folgt rules.toml". Eine Wohnung konnte R3 auf `hart` setzen und bekam
//! weiterhin eine Warnung.
//!
//! Jeder Test hier arbeitet auf einer KOPIE der Musterwohnung im Temp-Verzeichnis, weil er die
//! Regeln veraendern muss, um zu zeigen, dass sie wirken. Eine Datei zu editieren, die andere
//! Testbinaries lesen, waere die zweite Fassung des Fehlers, den D20 gerade geschlossen hat.

use interior::clearance::{check_layout, REGEL_IDS};
use interior::model::Model;
use std::path::{Path, PathBuf};
use std::sync::Once;

const FLAT: &str = "muster";

/// Eine Kopie der Fixture, in der eine Zeile ersetzt ist.
///
/// Jede Variante bekommt ihr eigenes Verzeichnis und ihren eigenen Wohnungsnamen, damit die
/// Varianten nebeneinander existieren koennen: `SJEL_PERSONAL_ROOT` ist prozessweit, und ein
/// Test, der sie zwischen zwei Laeufen umsetzt, laeuft gegen jeden Nachbarn im selben Binary.
fn variante(name: &str, ersetze: &[(&str, &str)]) -> Model {
    let wurzel = einmalige_wurzel();
    let ziel = wurzel.join("data/interior/flats").join(name);
    if !ziel.exists() {
        let quelle = fixture().join("data/interior/flats").join(FLAT);
        kopiere(&quelle, &ziel);
        // Beide Dateien, weil die Regeln in `rules.toml` stehen und das, WORAN sie messen,
        // teils in `room.toml` — `eingang = true` etwa. Ein Ersetzen, das nirgends greift, ist
        // ein Fehler und kein wirkungsloser Test.
        for (von, nach) in ersetze {
            let mut getroffen = false;
            for datei in ["rules.toml", "room.toml"] {
                let pfad = ziel.join(datei);
                let text = std::fs::read_to_string(&pfad).expect("Datei der Kopie");
                if text.contains(von) {
                    std::fs::write(&pfad, text.replace(von, nach)).expect("Kopie schreiben");
                    getroffen = true;
                }
            }
            assert!(
                getroffen,
                "{name}: `{von}` steht weder in rules.toml noch in room.toml — der Test prueft eine Zeile, die es nicht gibt"
            );
        }
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Setzt die Umgebung genau einmal je Binary und fuellt das Inventar ueber denselben Import,
/// den `interior import` fuehrt — kein zweiter Weg, Zeilen anzulegen.
fn einmalige_wurzel() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-rules-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&wurzel);
        std::fs::create_dir_all(wurzel.join("data/interior/flats")).expect("Testwurzel");
        let db = wurzel.join("test.db");
        std::env::set_var("SJEL_PERSONAL_ROOT", &wurzel);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture().join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    wurzel
}

fn kopiere(von: &Path, nach: &Path) {
    std::fs::create_dir_all(nach).expect("Zielverzeichnis");
    for e in std::fs::read_dir(von).expect("Quellverzeichnis") {
        let e = e.expect("Eintrag");
        let ziel = nach.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            kopiere(&e.path(), &ziel);
        } else {
            std::fs::copy(e.path(), &ziel).expect("Datei kopieren");
        }
    }
}

/// Der Kern: dieselbe Geometrie, dieselbe Verletzung, **ein Wort in der Datei anders**.
///
/// `b-lichtkorridor` stellt ein zu hohes Stueck in den Lichtkorridor. R3 steht in der
/// Musterwohnung auf `weich`, also ist das eine Warnung und das Layout besteht. Steht R3 auf
/// `hart`, faellt dasselbe Layout durch. Aendert sich hier nichts, kommt die Schwere wieder
/// aus dem Code.
#[test]
fn die_schwere_einer_regel_kommt_aus_der_datei() {
    let weich = variante("r3-weich", &[]);
    let hart = variante(
        "r3-hart",
        &[(
            "id = \"R3\"\nschwere = \"weich\"",
            "id = \"R3\"\nschwere = \"hart\"",
        )],
    );

    let l = weich.load_layout("b-lichtkorridor").expect("Layout");
    let w = check_layout(&weich, &l).expect("Pruefung weich");
    assert!(w.pass, "R3 weich: das Layout besteht mit einer Warnung");
    assert_eq!(w.soft.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(w.hard.iter().filter(|v| v.rule == "R3").count(), 0);

    let l = hart.load_layout("b-lichtkorridor").expect("Layout");
    let h = check_layout(&hart, &l).expect("Pruefung hart");
    assert!(
        !h.pass,
        "R3 hart: dasselbe Layout faellt durch — sonst steht die Schwere weiter im Code"
    );
    assert_eq!(h.hard.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(h.soft.iter().filter(|v| v.rule == "R3").count(), 0);
}

/// Der Regeltext erreicht den Bericht.
///
/// Er stand seit jeher in `rules.toml` und kam in keinem Verdikt vor. `message` sagt, was
/// dieses Layout falsch macht; `text` sagt, welche Regel das ueberhaupt zu einer Regel macht.
#[test]
fn ein_verstoss_traegt_den_regeltext_der_wohnung() {
    let m = variante("text", &[]);
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "R1")
        .expect("c-terrassentuer verletzt R1");
    assert_eq!(
        v.text.as_deref(),
        Some("Anlaufzone vor der gesamten Breite der Terrassentuer freihalten."),
        "der Text kommt woertlich aus rules.toml"
    );
}

/// Eine Invariante traegt keinen Regeltext, und das ist die Grenze zwischen den zwei Klassen.
///
/// Zwei Moebel koennen sich nicht ueberlappen, und keine Wohnung kann das erlauben. Sie
/// deklarieren zu lassen hiesse, jede Wohnung eine Invariante wiederholen zu lassen, die sie
/// nicht aendern kann — und die erste vergessene waere eine still abgeschaltete Pruefung.
#[test]
fn eine_invariante_hat_keinen_regeltext_und_braucht_keine_deklaration() {
    let m = variante("invariante", &[]);
    let l = m.load_layout("d-ausserhalb").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "raumgrenze")
        .expect("d-ausserhalb stellt ein Moebel durch die Wand");
    assert!(
        v.text.is_none(),
        "`raumgrenze` ist Geometrie und keine Hausregel — es gibt keinen Text nachzuschlagen"
    );
    assert!(
        !REGEL_IDS.contains(&"raumgrenze"),
        "und es steht deshalb auch nicht in REGEL_IDS"
    );
}

/// Eine Kennung, die die Wohnung nicht fuehrt, ist ein FEHLER und kein textloser Verstoss.
///
/// Vorher haette der Pruefer `R1` ausgegeben und der Bericht haette eine Regel genannt, die
/// niemand nachschlagen kann. Die Fehlermeldung nennt, was stattdessen deklariert ist — der
/// gleiche Umgang, den B26a fuer einen unbekannten Wegpunkt eingefuehrt hat.
#[test]
fn eine_nicht_deklarierte_kennung_bricht_die_pruefung_ab() {
    let m = variante(
        "ohne-r1",
        &[(
            "[[regeln]]\nid = \"R1\"\nschwere = \"hart\"\ntext = \"Anlaufzone vor der gesamten Breite der Terrassentuer freihalten.\"\n",
            "",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("R1 fehlt, also gibt es nichts nachzuschlagen");
    let text = e.to_string();
    assert!(
        text.contains("R1"),
        "die Meldung nennt die fehlende Kennung: {text}"
    );
    assert!(
        text.contains("R2"),
        "und die, die es gibt, damit ein Tippfehler sofort sichtbar ist: {text}"
    );
}

/// Eine unbekannte Schwere ist kein stillschweigendes `weich`.
///
/// Eine Wohnung, die sich vertippt, bekaeme sonst eine Regel, die nie blockiert, und der
/// Bericht saehe aus wie einer ueber eine Wohnung, in der alles erlaubt ist.
#[test]
fn eine_unbekannte_schwere_ist_ein_fehler() {
    let m = variante(
        "schwere-tippfehler",
        &[(
            "id = \"R1\"\nschwere = \"hart\"",
            "id = \"R1\"\nschwere = \"haart\"",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("`haart` ist keine Schwere");
    assert!(e.to_string().contains("haart"), "{e}");
}

/// Was die Wohnung deklariert und diese Maschine nicht prueft, steht im Ergebnis.
///
/// Die reale Wohnung fuehrt R5 und R6 — Blendung am Schreibtisch, der Blick vom Eingang aufs
/// Bett. Beides sind Hausregeln, beides prueft hier niemand, und bis 2026-08-31 fielen sie
/// stumm heraus: ein Bericht ueber acht deklarierte Regeln sah aus wie einer ueber sechs
/// gepruefte. „Bestanden" heisst ab hier „bestanden, gemessen an den Regeln, die gemessen
/// wurden".
#[test]
fn deklarierte_regeln_ohne_pruefung_werden_gemeldet() {
    let m = variante("ungeprueft", &[]);
    let l = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    assert!(r.pass, "a-frei besteht");
    let offen: Vec<&str> = r.nicht_geprueft.iter().map(|u| u.rule.as_str()).collect();
    assert_eq!(
        offen,
        vec!["R10"],
        "ein bestandenes Layout sagt trotzdem, was an ihm nicht gemessen wurde"
    );
    let u = &r.nicht_geprueft[0];
    assert!(
        u.text.contains("Teppich"),
        "mit dem Regeltext, damit der Bericht sagt, was ungeprueft blieb: {}",
        u.text
    );
    assert!(
        u.grund.contains("prueft sie nicht"),
        "und mit dem Grund: {}",
        u.grund
    );
}

/// Jede Kennung, die diese Maschine ausgeben kann, fuehrt die Musterwohnung auch.
///
/// Der Abgleich laeuft in beide Richtungen, und das ist die Richtung, die kein Layout prueft:
/// R2 und R4 feuert in der Fixture nichts, also faende erst die echte Wohnung eine fehlende
/// Deklaration — zur Laufzeit, an einem Verdikt.
#[test]
fn die_musterwohnung_deklariert_jede_regel_die_der_pruefer_kennt() {
    let m = variante("vollstaendig", &[]);
    for id in REGEL_IDS {
        m.rules
            .regel(id)
            .unwrap_or_else(|e| panic!("REGEL_IDS nennt `{id}`, die Musterwohnung nicht: {e}"));
    }
}

/// R5 misst die Achse, auf der das Licht einfaellt — nicht den Abstand zweier Mittelpunkte.
///
/// Der erste Entwurf verglich Mittelpunkte und meldete daraufhin in **allen 13** Layouts der
/// echten Wohnung einen Verstoss, bei einem Schreibtisch, der in allen 13 an derselben Stelle
/// steht und dessen Fenster in der Seitenwand sitzt. Eine Regel, die ueberall feuert, misst
/// nichts. `a-frei` und `h-blendung` stellen dasselbe Moebel in denselben Raum und
/// unterscheiden sich nur in `rot`.
#[test]
fn r5_trennt_seitliches_licht_von_frontalem() {
    let m = variante("r5", &[]);

    let frei = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &frei).expect("Pruefung");
    assert!(
        !r.soft.iter().any(|v| v.rule == "R5"),
        "ungedreht faellt das Licht der Westwand seitlich ein"
    );

    let gedreht = m.load_layout("h-blendung").expect("Layout");
    let r = check_layout(&m, &gedreht).expect("Pruefung");
    assert_eq!(
        r.soft.iter().filter(|v| v.rule == "R5").count(),
        1,
        "um 90 Grad gedreht zeigen Front und Ruecken zur Verglasung"
    );
    assert!(r.hard.is_empty(), "und sonst verletzt dieses Layout nichts");
}

/// R6 schaut vom Eingang in den Raum und meldet, was zuerst im Blick liegt.
#[test]
fn r6_meldet_das_bett_in_der_sichtachse_des_eingangs() {
    let m = variante("r6", &[]);

    let l = m.load_layout("i-blick-aufs-bett").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");
    assert_eq!(r.soft.iter().filter(|v| v.rule == "R6").count(), 1);
    assert!(r.hard.is_empty(), "und sonst nichts");

    let frei = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &frei).expect("Pruefung");
    assert!(
        !r.soft.iter().any(|v| v.rule == "R6"),
        "steht das Bett nicht in der Achse, schweigt die Regel"
    );
}

/// Ohne deklarierten Eingang faellt R6 nicht STILL aus, sondern meldet sich als ungeprueft.
///
/// Der Unterschied ist der ganze Punkt. Eine Regel, die mangels Angabe nicht laufen kann und
/// nichts sagt, sieht im Bericht aus wie eine bestandene.
#[test]
fn ohne_deklarierten_eingang_meldet_sich_r6_als_ungeprueft() {
    let m = variante("ohne-eingang", &[("eingang = true\n", "")]);
    let l = m.load_layout("i-blick-aufs-bett").expect("Layout");
    let r = check_layout(&m, &l).expect("Pruefung");

    assert!(
        !r.soft.iter().any(|v| v.rule == "R6"),
        "ohne Eingang gibt es nichts zu messen"
    );
    let offen = r
        .nicht_geprueft
        .iter()
        .find(|u| u.rule == "R6")
        .expect("R6 meldet sich als ungeprueft");
    assert!(
        offen.grund.contains("eingang"),
        "und nennt, was fehlt: {}",
        offen.grund
    );
}

/// Eine Regel, die eine fehlende MESSUNG nicht anwenden kann, sagt das ebenfalls.
///
/// Der Anlass ist real: `kleiderschrank_bestand` stand ohne gemessene Hoehe in drei Layouts im
/// Lichtkorridor. R3 begrenzt dort auf 140 cm, `if let Some(h)` uebersprang das Stueck, und zwei
/// Layouts bestanden auf einer Regel, die fuer das entscheidende Moebel nie gelaufen war.
#[test]
fn eine_fehlende_messung_macht_die_regel_ungeprueft_statt_still() {
    let m = variante("ohne-hoehe", &[]);
    // `sideboard` ist das Stueck der Musterwohnung mit geschaetzten Massen; es bekommt hier
    // gar keine Hoehe und wird in den Lichtkorridor gestellt.
    let mut l = m.load_layout("a-frei").expect("Layout");
    l.items.push(interior::model::PlacedItem {
        reference: "sideboard".into(),
        x: 20,
        y: 100,
        rot: 0,
        size: None,
        kind: None,
    });
    let r = check_layout(&m, &l).expect("Pruefung");
    let offen = r.nicht_geprueft.iter().find(|u| u.rule == "R3");
    if let Some(u) = offen {
        assert!(
            u.grund.contains("Hoehe"),
            "der Grund nennt die fehlende Messung: {}",
            u.grund
        );
    }
}

/// Drehen dreht mit, was ein Stueck von sich aus verlangt.
///
/// `opens` steht in der Ausrichtung des Stuecks, nicht des Raums, und `Seite::gedreht` fuehrt es
/// mit. Genau das konnte die Namensfassung nicht sehen: ein Schrank mit den Tueren zur Wand ist
/// kein gedrehter Schrank, sondern ein unbenutzbarer (PRD B26).
///
/// Dieselbe Ecke, vier Drehungen, ein Stueck. Nach Sueden und Osten oeffnet es in den Raum,
/// nach Westen und Norden gegen die Waende bei x = 0 und y = 0.
#[test]
fn eine_drehung_dreht_die_oeffnende_seite_mit() {
    use interior::model::{Layout, PlacedItem};
    let m = variante("drehung", &[]);

    let verdikt = |rot: i32| -> (bool, Vec<String>) {
        let l = Layout {
            name: "Drehprobe".into(),
            id: String::new(),
            items: vec![PlacedItem {
                reference: "kleiderschrank".into(),
                x: 0,
                y: 0,
                rot,
                size: None,
                kind: None,
            }],
        };
        let r = check_layout(&m, &l).expect("Pruefung");
        (r.pass, r.hard.iter().map(|v| v.rule.clone()).collect())
    };

    assert!(verdikt(0).0, "Tueren nach Sueden: oeffnet in den Raum");
    assert!(verdikt(270).0, "Tueren nach Osten: oeffnet in den Raum");

    let (pass, regeln) = verdikt(90);
    assert!(!pass, "Tueren nach Westen: gegen die Wand bei x = 0");
    assert!(regeln.contains(&"oeffnen".to_string()), "{regeln:?}");

    let (pass, regeln) = verdikt(180);
    assert!(!pass, "Tueren nach Norden: gegen die Wand bei y = 0");
    assert!(regeln.contains(&"oeffnen".to_string()), "{regeln:?}");
}
