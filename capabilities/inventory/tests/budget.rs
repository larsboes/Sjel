//! Die Naht zwischen `inventory` und `finance` (PRD B29), gegen erfundene Buchungen.
//!
//! Die echten Zahlen stehen im Overlay und werden hier nicht angefasst. Was geprueft wird, ist
//! die Rechnung: welcher Monat zaehlt, welche Buchung zaehlt, und was passiert, wenn am Ende
//! nichts uebrig bleibt.

use sjel_inventory::budget::{monate_bis_bezahlt, monatssaldo};
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
