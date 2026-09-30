//! Die Naht zwischen `interior` und `finance` (PRD B29), gegen erfundene Buchungen.
//!
//! Die echten Zahlen stehen im Overlay und werden hier nicht angefasst. Was geprueft wird, ist
//! die Rechnung: welcher Monat zaehlt, welche Buchung zaehlt, und was passiert, wenn am Ende
//! nichts uebrig bleibt.

use interior::budget::{monate_bis_bezahlt, monatssaldo};
use rusqlite::Connection;

/// Eine leere Datenbank mit finance' Projektionstabelle, so wie finance sie anlegt — die
/// Spalten, die diese Abfrage liest, und keine mehr.
fn db_mit_finance() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory");
    conn.execute_batch(
        "CREATE TABLE finance_transaction_projection (
            id TEXT PRIMARY KEY, booked_at TEXT, kind TEXT, amount_cents INTEGER
         );",
    )
    .unwrap();
    conn
}

/// Euro rein, Cent raus. Die Schreibweise mit dem Unterstrich vor den letzten zwei Stellen
/// waere lesbarer und ist genau die, die clippys `inconsistent_digit_grouping` verbietet —
/// also einmal umrechnen, statt an jeder Stelle Nullen zu zaehlen.
fn eur(betrag: i64) -> i64 {
    betrag * 100
}

fn buchen(conn: &Connection, id: &str, tag: &str, kind: &str, cents: i64) {
    conn.execute(
        "INSERT INTO finance_transaction_projection (id, booked_at, kind, amount_cents)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, tag, kind, cents],
    )
    .unwrap();
}

/// Ohne finance gibt es keine Zahl — und das ist `None` und keine Null. Eine Null waere die
/// Behauptung, es bleibe genau nichts uebrig, gemessen.
#[test]
fn ohne_finance_gibt_es_keinen_saldo() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(monatssaldo(&conn).unwrap().is_none());
}

#[test]
fn eine_leere_projektion_ergibt_auch_keinen_saldo() {
    let conn = db_mit_finance();
    assert!(monatssaldo(&conn).unwrap().is_none());
}

/// Der Median, nicht der Mittelwert. Ein Monat mit einer Jahreszahlung darin verschiebt einen
/// Mittelwert um Hunderte Euro; der Median haelt still. Hier: elf Monate mit +100, einer mit
/// -5000 — Mittelwert waere negativ, Median ist 100.
#[test]
fn ein_einzelner_ausreisser_kippt_den_median_nicht() {
    let conn = db_mit_finance();
    for m in 1..=11 {
        buchen(
            &conn,
            &format!("e{m}"),
            &format!("2025-{m:02}-15"),
            "income",
            eur(100),
        );
    }
    buchen(&conn, "krach", "2025-12-15", "expense", eur(5000));

    let s = monatssaldo(&conn).unwrap().expect("zwoelf Monate");
    assert_eq!(s.monate, 12);
    // Zwoelf Werte: elfmal +10000, einmal -500000. Sortiert liegt die Mitte zwischen zwei
    // Plusmonaten, also bleibt der Median positiv.
    assert_eq!(s.median_cent, eur(100));
}

/// Eine Umbuchung zwischen eigenen Konten ist kein Einkommen und keine Ausgabe. Zaehlte sie
/// mit, waere ein Monat mit einer Kreditkartenabrechnung darin doppelt falsch.
#[test]
fn eine_umbuchung_zaehlt_nicht() {
    let conn = db_mit_finance();
    buchen(&conn, "a", "2025-05-01", "income", eur(200));
    buchen(&conn, "b", "2025-05-02", "expense", eur(50));
    buchen(&conn, "c", "2025-05-03", "transfer", eur(900));

    let s = monatssaldo(&conn).unwrap().unwrap();
    assert_eq!(s.monate, 1);
    assert_eq!(s.median_cent, eur(150));
}

/// Der laufende Monat ist unvollstaendig. Ein halber Monat mit ganzen Fixkosten und halbem
/// Einkommen ist keine Messung, sondern eine Verzerrung mit Nachkommastellen.
#[test]
fn der_laufende_monat_zaehlt_nicht_mit() {
    let conn = db_mit_finance();
    let heute: String = conn
        .query_row("SELECT strftime('%Y-%m-15','now')", [], |r| r.get(0))
        .unwrap();
    buchen(&conn, "jetzt", &heute, "expense", eur(9999));
    assert!(
        monatssaldo(&conn).unwrap().is_none(),
        "nur der laufende Monat hat Buchungen, also gibt es nichts Abgeschlossenes zu messen"
    );

    buchen(&conn, "vorher", "2025-03-10", "income", eur(300));
    let s = monatssaldo(&conn).unwrap().unwrap();
    assert_eq!((s.monate, s.median_cent), (1, eur(300)));
    assert_eq!((s.von.as_str(), s.bis.as_str()), ("2025-03", "2025-03"));
}

/// Aus einem Saldo, der nicht positiv ist, laesst sich nichts ansparen. `None` ist hier die
/// Antwort und kein fehlender Wert — "-3,2 Monate" waere eine Rechnung, die so tut, als
/// beantworte sie etwas.
#[test]
fn ein_negativer_saldo_ergibt_keine_monatszahl() {
    let conn = db_mit_finance();
    buchen(&conn, "a", "2025-05-01", "income", eur(100));
    buchen(&conn, "b", "2025-05-02", "expense", eur(400));
    let s = monatssaldo(&conn).unwrap().unwrap();
    assert_eq!(s.median_cent, -eur(300));
    assert_eq!(monate_bis_bezahlt(eur(1000), &s), None);
}

#[test]
fn ein_positiver_saldo_ergibt_die_zahl_der_monate() {
    let conn = db_mit_finance();
    buchen(&conn, "a", "2025-05-01", "income", eur(500));
    buchen(&conn, "b", "2025-05-02", "expense", eur(250));
    let s = monatssaldo(&conn).unwrap().unwrap();
    assert_eq!(s.median_cent, eur(250));
    assert_eq!(monate_bis_bezahlt(eur(1000), &s), Some(4.0));
}

// ---------------------------------------------------------------- die Reihenfolge

/// Fuer die Reihenfolge braucht es das Musterinventar, nicht nur erfundene Buchungen: sie
/// sortiert Zeilen und rechnet nicht mit ihnen.
fn model() -> interior::model::Model {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-budget-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = interior::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        interior::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    interior::model::Model::load("muster").expect("muster")
}

/// Dringlichkeit schlaegt Preis, und ein Entwurf schlaegt eine Idee.
///
/// Die Fixture ist so gebaut, dass sich die drei Stufen widersprechen: `nachttisch` ist
/// Pflicht und teuer, `wandregal` nur Empfehlung und billig, `bilderleiste` Konzept und am
/// billigsten. Eine Sortierung nach Preis kaeme genau umgekehrt heraus — deshalb ist diese
/// Reihenfolge eine Pruefung und keine Aufzeichnung.
#[test]
fn dringlichkeit_steht_vor_preis() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert!(
        pos("nachttisch") < pos("wandregal"),
        "Pflicht vor Empfehlung, auch wenn sie teurer ist: {ids:?}"
    );
    assert!(
        pos("wandregal") < pos("bilderleiste"),
        "Empfehlung vor Konzept: {ids:?}"
    );
}

/// Die ENTSCHEIDUNGSachse zaehlt auch, und sie war bis 2026-08-31 unsichtbar.
///
/// `gesetzt` heisst "entschieden, das wird gekauft" und `zurueckgestellt` heisst "bewusst
/// verschoben". Beide Woerter stehen im Inventar der echten Wohnung, beide fehlten in `rang`,
/// und beide fielen deshalb auf denselben Rang wie alles Unbekannte — was zwischen ihnen
/// entschied, war der Preis. Hier ist der teure Posten der gesetzte und der billige der
/// zurueckgestellte, damit eine Preissortierung genau umgekehrt herauskaeme.
#[test]
fn eine_gefallene_entscheidung_steht_vor_einer_verschobenen() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert_eq!(ids[0], "deckenlampe", "das Gesetzte zuerst: {ids:?}");
    assert_eq!(
        *ids.last().unwrap(),
        "garderobenhaken",
        "das Zurueckgestellte zuletzt: {ids:?}"
    );
    assert!(pos("deckenlampe") < pos("nachttisch"), "{ids:?}");
}

/// Ein Wort, das diese Rangfolge nicht kennt, wird gemeldet statt einsortiert.
///
/// Das ist die Lehre aus dem Fehler selbst: die erste Fassung kannte drei der acht Woerter und
/// sah trotzdem sortiert aus. Ein neues Wort in den Daten muss auffallen.
#[test]
fn ein_unbekanntes_prioritaetswort_wird_gemeldet() {
    let mut m = model();
    if let Some(i) = m.catalogue.get_mut("wandregal") {
        i.prioritaet = Some("vielleicht-irgendwann".into());
    }
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert_eq!(
        r.unbekannte_prioritaeten,
        vec!["vielleicht-irgendwann".to_string()]
    );
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        *ids.last().unwrap(),
        "wandregal",
        "was niemand einordnen kann, steht hinten: {ids:?}"
    );
}

/// Was abgeloest wurde, steht in keiner Kaufreihenfolge.
#[test]
fn ein_ersetzter_bedarf_faellt_heraus() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(r
        .posten
        .iter()
        .chain(r.ohne_preis.iter())
        .all(|p| p.prioritaet.as_deref() != Some("ersetzt")));
}

/// Die kumulierte Summe waechst monoton und endet auf der Gesamtsumme.
#[test]
fn die_kumulierte_summe_zaehlt_wirklich_zusammen() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let mut letzte = 0;
    for p in &r.posten {
        assert!(p.kumuliert_cent >= letzte, "die Summe faellt: {:?}", p.id);
        letzte = p.kumuliert_cent;
    }
    let summe: i64 = r.posten.iter().map(|p| p.preis_cent.unwrap_or(0)).sum();
    assert_eq!(letzte, summe);
}

/// Ein Bedarf ohne Preis steht nicht in der Reihenfolge — und schon gar nicht vorn.
///
/// Mit null angesetzt waere er der billigste Posten und stuende zuerst; das waere eine
/// Kaufempfehlung, die aus einer fehlenden Zahl folgt. PRD B29 zaehlt sie in der Wunschliste
/// schon als Warnung, und hier ist die Handlung dazu ein Preis und kein Kauf.
#[test]
fn ein_bedarf_ohne_preis_steht_in_keiner_reihenfolge() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(
        r.posten.iter().all(|p| p.preis_cent.is_some()),
        "in der Reihenfolge steht nur, was einen Preis hat"
    );
    assert!(
        r.ohne_preis.iter().any(|p| p.id == "teppich"),
        "und der Teppich steht daneben, mit Namen"
    );
}

/// Ohne positiven Saldo gibt es keine Monatszahl — dieselbe Haltung wie `monate_bis_bezahlt`.
#[test]
fn ohne_saldo_gibt_es_keine_zeitachse() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(
        r.posten.iter().all(|p| p.erreichbar_nach_monaten.is_none()),
        "eine Zeitachse ohne Messung waere eine erfundene Zahl"
    );
}
