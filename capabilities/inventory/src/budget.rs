//! Was der offene Bedarf kostet, gemessen an dem, was am Monatsende uebrig bleibt (PRD B29).
//!
//! ## Warum das eine SQL-Abfrage ist und kein HTTP-Aufruf
//!
//! `finance` und `interior` schreiben in **dieselbe Datei** — das ist der Grund, aus dem die
//! geteilte Instanz ueberhaupt existiert (`capabilities/store/README.md`: "Cross-schema joins
//! were the reason the shared instance existed"). `capabilities/places/src/backfill.rs` liest
//! `finance_*` schon genauso. Der HTTP-Weg waere schlechter und nicht nur umstaendlicher:
//! `finance` verlangt einen Inbound-Token, also haette `interior` ein Geheimnis zu halten, um
//! eine Zahl zu lesen, die einen Meter weiter in derselben Datei steht.
//!
//! ## Was hier NICHT behauptet wird
//!
//! Es gibt kein Budget. Niemand hat eine Obergrenze gesetzt, und diese Datei erfindet keine.
//! Sie meldet **eine Messung** — den Median des Monatssaldos ueber die letzten
//! abgeschlossenen Monate — und ueberlaesst das Urteil dem Menschen. Median und nicht
//! Mittelwert: ein einzelner Monat mit einer Jahreszahlung darin verschiebt einen Mittelwert
//! um Hunderte Euro, und der Median haelt still.
//!
//! Der **laufende Kalendermonat faellt heraus.** Er ist unvollstaendig, und ein halber Monat
//! mit ganzen Fixkosten und halbem Einkommen ist kein Datenpunkt, sondern eine Verzerrung mit
//! Nachkommastellen.

use rusqlite::Connection;

type Fehler = Box<dyn std::error::Error>;

/// Wie viele abgeschlossene Monate das Fenster umfasst. Zwoelf, damit ein Jahreszyklus einmal
/// vollstaendig darin liegt: Versicherungen, Weihnachten und die Nebenkostenabrechnung sind
/// keine Ausreisser, sie sind der Jahresrhythmus.
pub const FENSTER_MONATE: usize = 12;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Saldo {
    /// Median des Monatssaldos in Cent. Negativ heisst: es bleibt nichts uebrig.
    pub median_cent: i64,
    /// Wie viele abgeschlossene Monate tatsaechlich gemessen wurden.
    pub monate: usize,
    /// Der erste und der letzte Monat im Fenster, als `YYYY-MM`.
    pub von: String,
    pub bis: String,
}

/// Der Monatssaldo aus `finance_transaction_projection`, oder `None`, wenn finance auf dieser
/// Maschine nichts geschrieben hat.
///
/// `transfer` zaehlt nicht mit. Eine Umbuchung zwischen eigenen Konten ist kein Einkommen und
/// keine Ausgabe; sie beidseitig mitzuzaehlen faelscht beide Seiten und hebt sich nur dann auf,
/// wenn beide Haelften im selben Monat liegen — was bei einer Kreditkartenabrechnung selten ist.
pub fn monatssaldo(conn: &Connection) -> Result<Option<Saldo>, Fehler> {
    let vorhanden: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master
          WHERE type='table' AND name='finance_transaction_projection'",
        [],
        |row| row.get(0),
    )?;
    if vorhanden == 0 {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT substr(booked_at, 1, 7) AS monat,
                sum(CASE WHEN kind = 'income'  THEN amount_cents ELSE 0 END)
              - sum(CASE WHEN kind = 'expense' THEN amount_cents ELSE 0 END) AS saldo
           FROM finance_transaction_projection
          WHERE kind IN ('income', 'expense')
            AND substr(booked_at, 1, 7) < strftime('%Y-%m', 'now')
          GROUP BY monat
          ORDER BY monat DESC
          LIMIT ?1",
    )?;
    let rows = stmt.query_map([FENSTER_MONATE as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut monate: Vec<(String, i64)> = rows.collect::<Result<_, _>>()?;
    if monate.is_empty() {
        return Ok(None);
    }
    monate.sort_by(|a, b| a.0.cmp(&b.0));
    let von = monate.first().unwrap().0.clone();
    let bis = monate.last().unwrap().0.clone();

    let mut salden: Vec<i64> = monate.iter().map(|(_, s)| *s).collect();
    salden.sort_unstable();
    // Bei gerader Anzahl das Mittel der beiden mittleren, damit zwoelf Monate nicht still auf
    // den sechsten oder siebten Wert kippen, je nachdem wie gerundet wird.
    let n = salden.len();
    let median_cent = if n % 2 == 1 {
        salden[n / 2]
    } else {
        (salden[n / 2 - 1] + salden[n / 2]) / 2
    };

    Ok(Some(Saldo {
        median_cent,
        monate: n,
        von,
        bis,
    }))
}

/// Wie viele Monatssalden der offene Bedarf kostet.
///
/// `None`, wenn der Median null oder negativ ist. Das ist kein fehlender Wert, sondern die
/// Antwort: aus einem Saldo, der nicht positiv ist, laesst sich nichts ansparen, und eine Zahl
/// wie "-3,2 Monate" hinzuschreiben waere eine Rechnung, die so tut, als beantworte sie etwas.
pub fn monate_bis_bezahlt(offen_cent: i64, saldo: &Saldo) -> Option<f64> {
    if saldo.median_cent <= 0 {
        return None;
    }
    Some(offen_cent as f64 / saldo.median_cent as f64)
}
