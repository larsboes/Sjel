//! Ein Layout zurueckschreiben, ohne seine Begruendung zu verlieren.
//!
//! ## Warum das eine eigene Datei ist
//!
//! Layouts sind Dateien und keine Zeilen (PRD Q60, bestaetigt 2026-08-31). Die Regel dort ist
//! *hat die Zahl eine Begruendung, die mitwandern muss* — und bei einem Layout hat sie das:
//! `h-esstisch-offen` traegt drei Absaetze darueber, warum die Klappe im Ostschlauch nicht
//! aufgeht, und genau diese Absaetze sind der Grund, aus dem der PINNTORP verworfen wurde. Eine
//! Tabelle hat dafuer keine Spalte, und das Overlay ist ein Git-Repository: als Datei hat jede
//! Umstellung eine Geschichte und einen Diff.
//!
//! Daraus folgt die einzige Anforderung an diesen Schreiber: **die Prosa ueberlebt.** Ein
//! Serialisierer, der die Datei aus der Struktur neu erzeugt, wirft jeden Kommentar weg — und
//! damit das Wertvollste an ihr.
//!
//! ## Warum das ohne `toml_edit` geht
//!
//! Gemessen statt vermutet, 2026-08-31: in **keinem** der Layouts steht ein Kommentar hinter dem
//! ersten `[[item]]`, und die ganze Datei kennt fuenf Schluessel — `name`, und je Eintrag `ref`,
//! `x`, `y`, `rot`, `size`. Damit reicht ein Schnitt am ersten `[[item]]`: der Kopf bleibt Zeichen
//! fuer Zeichen stehen, die Eintraege werden neu geschrieben. Eine Abhaengigkeit fuer
//! Kommentarerhalt waere Aufwand fuer ein Problem, das die Dateien nicht haben.
//!
//! Sollte je ein Kommentar zwischen den Eintraegen stehen, geht er verloren — deshalb prueft
//! `pruefe_kopf_genuegt` das vorher und verweigert die Arbeit, statt still zu kuerzen.

use crate::model::{Layout, Model, ModelError, PlacedItem};
use std::path::Path;

/// Der Kopf einer Layoutdatei: alles vor dem ersten `[[item]]`.
fn header(text: &str) -> &str {
    match text.find("[[item]]") {
        Some(i) => &text[..i],
        None => text,
    }
}

/// Steht hinter dem ersten `[[item]]` noch ein Kommentar, waere er nach dem Schreiben weg.
///
/// Dann bricht der Schreiber ab. Eine Datei stillschweigend zu kuerzen ist genau der
/// Datenverlust, gegen den `deny_unknown_fields` im Import steht.
fn check_header_suffices(text: &str, pfad: &Path) -> Result<(), ModelError> {
    let Some(i) = text.find("[[item]]") else {
        return Ok(());
    };
    for line in text[i..].lines() {
        let z = line.trim();
        if z.starts_with('#') {
            return Err(ModelError::Missing(format!(
                "{}: hinter dem ersten [[item]] steht ein Kommentar ({z:?}). \
                 Dieser Schreiber erhaelt nur den Kopf und wuerde ihn verlieren — \
                 verschiebe ihn nach oben oder schreib die Datei von Hand",
                pfad.display()
            )));
        }
    }
    Ok(())
}

/// Die Eintraege als TOML, in der Reihenfolge, in der sie stehen.
fn entries(items: &[PlacedItem]) -> String {
    let mut s = String::new();
    for it in items {
        s.push_str("[[item]]\n");
        s.push_str(&format!("ref = \"{}\"\n", it.reference));
        s.push_str(&format!("x = {}\ny = {}\nrot = {}\n", it.x, it.y, it.rot));
        if let Some(sz) = it.size {
            s.push_str(&format!("size = [{}, {}]\n", sz[0], sz[1]));
        }
        if let Some(k) = &it.kind {
            s.push_str(&format!("kind = \"{k}\"\n"));
        }
        s.push('\n');
    }
    s
}

/// Ein bestehendes Layout mit neuen Positionen zurueckschreiben. Der Kopf bleibt.
pub fn update(model: &Model, id: &str, items: &[PlacedItem]) -> Result<(), ModelError> {
    let pfad = model.layouts_dir().join(format!("{id}.toml"));
    let text = std::fs::read_to_string(&pfad).map_err(|source| ModelError::Read {
        path: pfad.clone(),
        source,
    })?;
    check_header_suffices(&text, &pfad)?;
    let neu = format!("{}{}", header(&text), entries(items));
    std::fs::write(&pfad, neu).map_err(|source| ModelError::Read { path: pfad, source })
}

/// Ein Layoutname, der eine Datei sein darf.
///
/// Der Name wird zu einem Dateinamen, also entscheidet diese Pruefung, was ein Aufrufer ueber
/// HTTP in das Overlay schreiben kann. Ein Punkt oder ein Trennzeichen ist deshalb kein
/// Schoenheitsfehler, sondern der Weg aus dem Verzeichnis heraus.
pub fn check_id(id: &str) -> Result<(), ModelError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ModelError::Missing(format!(
            "`{id}` ist kein Layoutname — erlaubt sind Buchstaben, Ziffern, - und _"
        )));
    }
    Ok(())
}

/// Die Notiz als Kommentarblock, Zeile fuer Zeile.
///
/// Mehrzeilig, weil die Herkunft eines Layouts ein Absatz ist und keine Zeile davon: `interior
/// compose` hat drei Saetze zu sagen, die API einen.
fn as_comment(notiz: &str) -> String {
    notiz
        .lines()
        .map(|z| {
            let z = z.trim_end();
            if z.is_empty() {
                "#\n".to_string()
            } else {
                format!("# {z}\n")
            }
        })
        .collect()
}

/// Ein neues Layout anlegen. Der Kopf sagt, woher es kommt.
///
/// **Was hier NICHT steht, ist der Punkt.** Bis 2026-08-31 schrieb dieser Kopf in jede neue
/// Datei „Von einer Maschine gesetzt … jede Position hat die volle Raeumungspruefung
/// durchlaufen". Fuer `interior compose` stimmt das; fuer einen leeren Plan, den jemand ueber
/// `POST /api/layouts` anlegt und danach von Hand zieht, ist es schlicht falsch — und eine
/// Datei, die ihre eigene Herkunft falsch behauptet, ist schlimmer als eine ohne Kopf. Wer
/// etwas ueber die Herkunft zu sagen hat, sagt es in `notiz`.
pub fn create(model: &Model, id: &str, layout: &Layout, notiz: &str) -> Result<(), ModelError> {
    check_id(id)?;
    let pfad = model.layouts_dir().join(format!("{id}.toml"));
    if pfad.exists() {
        return Err(ModelError::Missing(format!(
            "`{id}` gibt es schon; ein bestehendes Layout wird nicht ueberschrieben"
        )));
    }
    let header = format!(
        "{}#\n\
         # Dieser Kopf ist der Platz fuer das WARUM: wofuer diese Aufstellung steht und wogegen\n\
         # sie entschieden wurde. Er ueberlebt jedes spaetere Verschieben — die Eintraege\n\
         # darunter werden neu geschrieben, dieser Absatz nicht.\n\n\
         name = \"{}\"\n\n",
        as_comment(notiz),
        layout.name.replace('"', "'")
    );
    if let Some(dir) = pfad.parent() {
        std::fs::create_dir_all(dir).map_err(|source| ModelError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&pfad, format!("{header}{}", entries(&layout.items)))
        .map_err(|source| ModelError::Read { path: pfad, source })
}

/// Wohin ein zurueckgelegtes Layout wandert. Ein Unterverzeichnis, kein Muelleimer.
pub const ARCHIVE: &str = "archiv";

/// Ein Layout aus der Liste nehmen, ohne es zu verlieren.
///
/// **Geloescht wird nicht.** Der Kopf einer Layoutdatei traegt die Begruendung, aus der ein
/// Moebel verworfen wurde (PRD Q60) — `h-esstisch-offen` traegt drei Absaetze darueber, warum
/// eine Klappe nicht aufgeht, und das ist der einzige Ort, an dem dieses Argument steht. Ein
/// `DELETE`, das die Datei entfernt, entfernt die Begruendung mit; wer sie spaeter braucht,
/// braucht sie genau dann, wenn dasselbe Moebel wieder zur Debatte steht.
///
/// `layouts/archiv/` faellt aus `Model::layout_names` heraus, weil ein Verzeichnis keine
/// `.toml`-Endung hat. Das Layout verschwindet also aus jeder Liste und bleibt auf der Platte,
/// mit seiner Geschichte im Git des Overlays.
pub fn archive_layout(model: &Model, id: &str) -> Result<String, ModelError> {
    check_id(id)?;
    let von = model.layouts_dir().join(format!("{id}.toml"));
    if !von.exists() {
        return Err(ModelError::Missing(format!("kein Layout `{id}`")));
    }
    let archive = model.layouts_dir().join(ARCHIVE);
    std::fs::create_dir_all(&archive).map_err(|source| ModelError::Read {
        path: archive.clone(),
        source,
    })?;
    let nach = archive.join(format!("{id}.toml"));
    if nach.exists() {
        return Err(ModelError::Missing(format!(
            "im Archiv liegt schon ein `{id}` — zwei Fassungen unter einem Namen waeren eine \
             verlorene Begruendung"
        )));
    }
    std::fs::rename(&von, &nach).map_err(|source| ModelError::Read { path: von, source })?;
    // Relativ zum Layoutverzeichnis: der absolute Pfad zeigt in das private Overlay und hat in
    // einer HTTP-Antwort nichts verloren.
    Ok(format!("{ARCHIVE}/{id}.toml"))
}

/// Das heutige Datum als `JJJJ-MM-TT`, aus der Uhr statt aus dem Quelltext.
///
/// Ein Kopf, der sein Datum nennt, ist der Unterschied zwischen „ueber die API angelegt" und
/// einer Aussage, die jemand nachschlagen kann.
pub fn heute() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (j, m, t) = date_from_days(seconds.div_euclid(86_400));
    format!("{j:04}-{m:02}-{t:02}")
}

/// Tage seit 1970-01-01 als Kalenderdatum, nach Howard Hinnants `civil_from_days`.
///
/// Von Hand gerechnet und nicht aus einer Kalenderbibliothek: fuer ein Datum in einem Kommentar
/// waere das eine Abhaengigkeit zu viel. `sonne.rs` traegt dieselbe Rechnung fuer die Jahreszahl
/// aus demselben Grund.
fn date_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let tag = doy - (153 * mp + 2) / 5 + 1;
    // Hinnants Jahr beginnt im Maerz, damit der Schalttag ans Ende faellt.
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, tag)
}

#[cfg(test)]
mod tests {
    use super::{as_comment, date_from_days, entries, header, check_id, check_header_suffices};
    use crate::model::PlacedItem;
    use std::path::Path;

    const FILE: &str = "# Variante K — Bett quer an der Nordwand.\n\
                         #\n\
                         # Drei Absaetze Begruendung, die kein Serialisierer kennt.\n\n\
                         name = \"K\"\n\n\
                         [[item]]\nref = \"bett\"\nx = 0\ny = 0\nrot = 0\n\n";

    #[test]
    fn the_header_survives_character_for_character() {
        let k = header(FILE);
        assert!(k.contains("Drei Absaetze Begruendung"));
        assert!(k.contains("name = \"K\""));
        assert!(!k.contains("[[item]]"));
    }

    #[test]
    fn the_entries_are_rewritten() {
        let items = vec![PlacedItem {
            reference: "bett".into(),
            x: 120,
            y: 40,
            rot: 90,
            size: None,
            kind: None,
        }];
        let s = entries(&items);
        assert!(s.contains("ref = \"bett\""));
        assert!(s.contains("x = 120"));
        assert!(s.contains("rot = 90"));
        assert!(!s.contains("size"), "kein size, wenn keins gesetzt ist");
    }

    /// Ein Kommentar zwischen den Eintraegen bricht ab, statt still verloren zu gehen.
    #[test]
    fn a_comment_after_the_entries_prevents_writing() {
        let with =
            format!("{FILE}# ein spaeter Gedanke\n[[item]]\nref = \"x\"\nx = 1\ny = 1\nrot = 0\n");
        let e = check_header_suffices(&with, Path::new("k.toml"))
            .expect_err("der Kommentar wuerde verloren gehen");
        assert!(e.to_string().contains("spaeter Gedanke"), "{e}");
    }

    #[test]
    fn without_trailing_comments_writing_is_allowed() {
        assert!(check_header_suffices(FILE, Path::new("k.toml")).is_ok());
    }

    /// Ein Absatz bleibt ein Absatz, und die Leerzeile bleibt eine Zeile.
    #[test]
    fn a_multi_line_note_becomes_a_comment_block() {
        let k = as_comment("Erste Zeile.\n\nDritte Zeile.");
        assert_eq!(k, "# Erste Zeile.\n#\n# Dritte Zeile.\n");
    }

    /// Der Name wird zu einem Dateinamen. Was hier durchkommt, darf ins Overlay schreiben.
    #[test]
    fn a_layout_name_cannot_point_out_of_the_dir() {
        assert!(check_id("p-gerechnet_2").is_ok());
        for bad in ["", "../heim", "a/b", "a.toml", "a b"] {
            assert!(check_id(bad).is_err(), "`{bad}` darf keine Datei sein");
        }
    }

    /// Gegen zwei Tage, die von Hand nachschlagbar sind, und gegen den Schalttag.
    #[test]
    fn days_since_the_epoch_become_the_calendar_date() {
        assert_eq!(date_from_days(0), (1970, 1, 1));
        assert_eq!(date_from_days(-1), (1969, 12, 31));
        assert_eq!(date_from_days(11_016), (2000, 2, 29), "Schalttag");
        assert_eq!(date_from_days(20_696), (2026, 8, 31));
    }
}
