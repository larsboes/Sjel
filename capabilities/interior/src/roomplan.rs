//! Was in einer RoomPlan-Aufnahme steckt, und was daran gegen das Bandmass spricht.
//!
//! Bis 2026-09-26 stand in der Sitzungsnotiz, Wand-, Tuer- und Fenstermasse seien „noch
//! ungelesen", weil der semantische `CapturedRoom`-Entwurf (`roomplan-capture/v1`) den Mac nie
//! erreicht hat. Das war falsch. In der rohen USDZ steht alles, was dafuer gebraucht wird: jede
//! `WallN.usda` traegt einen lokalen Rahmen in Metern plus eine `matrix4d xformOp:transform`,
//! und die `Door*`- und `Window*`-Dateien tragen denselben. Was der Entwurf vom Telefon
//! zusaetzlich liefert, ist Apples eigenes Vertrauen je Objekt — das steht in der USDZ nicht.
//!
//! # Meter, und warum sie genau hier stehen duerfen
//!
//! `model.rs` sagt: „Meter kommen in diesem Programm nirgends vor." Das gilt weiter, und dieses
//! Modul ist die eine Ausnahme mit Begruendung: das Zielformat ist `roomplan-capture/v1`, dessen
//! Felder `dimensions_m` heissen und dessen `coordinate_system.units` `"meters"` ist. Eine
//! Aufnahme in Zentimeter umzuschreiben hiesse, das Schema zu brechen und die Zahl zu verwerfen,
//! die Apple geschrieben hat. Umgerechnet wird an genau einer Grenze, in [`Diff`] — dort, wo der
//! Scan gegen `room.toml` gehalten wird, und nirgends sonst.
//!
//! # Kein Entpacker
//!
//! Eine USDZ ist ein ZIP, und Apple schreibt ihre Eintraege mit Verfahren 0 (`Stored`, am
//! 2026-09-26 an allen 28 Eintraegen nachgesehen). Die Datei liegt damit ohne Dekomprimierer vor,
//! und dieses Modul nimmt keinen auf. Ein Eintrag mit einem anderen Verfahren ist ein **Fehler
//! und keine stille Fehlantwort** — dieselbe Regel, die `room.toml` fuer unbenannte Routen
//! fuehrt: was aussieht wie ein Ergebnis und keines ist, ist schlimmer als eine Fehlermeldung.
//!
//! # Was die Aufnahme nicht hergibt
//!
//! - **Vertrauen.** Die USDZ traegt kein `confidence` je Element. Jedes Element geht deshalb als
//!   `"low"` in den Entwurf. Das ist keine Vorsicht und keine Verzierung: die Beobachtung daneben
//!   sagt `"source_contract": "raw RoomPlan USDZ preserved; USDA is inspection evidence only"`,
//!   und ein Scan ist nach der Regel dieses Projekts ein Beleg und keine Messung.
//! - **Raumzugehoerigkeit.** `CapturedRoom.usda` fuehrt unter `Section_grp` die Raumgruppen mit
//!   ihren Mittelpunkten und **keine Mitgliedschaft**: die Maschen haengen unter `Mesh_grp` und
//!   nicht unter den Gruppen. Ein Element laesst sich daher keinem Raum zuordnen — und genau
//!   deshalb laesst sich auch die im Scan fehlende Badtuer keinem Raum zuschreiben.
//! - **Wandstaerke als Messung.** Alle Waende tragen dieselbe Staerke. Ein Wert, der sich nicht
//!   unterscheidet, ist eine Konstante des Modells und keine Messung; die Staerke geht deshalb
//!   nicht in den Vergleich ein, und [`Scan::wandstaerke_m`] gibt sie nur zur Ansicht heraus.
//!
//! # Es wird nichts zurueckgeschrieben
//!
//! [`Diff`] ist ein Bericht. Weder `room.toml` noch die Tabellen noch eine Platzierung werden von
//! diesem Modul angefasst. Der Entwurf geht auf Wunsch als `draft.json` neben die Aufnahme, wo
//! `/api/roomplan/revisions` ihn schon erwartet, und die Entscheidung darueber faellt im Review.

use crate::model::{Model, Room};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Verfahren 0 im ZIP. Alles andere kann dieses Modul nicht und sagt das.
const VERFAHREN_GESPEICHERT: u16 = 0;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} ist kein lesbares ZIP: {detail}")]
    KeinZip { path: PathBuf, detail: String },
    #[error(
        "{path}: `{name}` liegt mit Verfahren {verfahren} vor — dieses Modul entpackt nicht, \
         und ein halb gelesener Scan waere schlimmer als kein Scan"
    )]
    Komprimiert {
        path: PathBuf,
        name: String,
        verfahren: u16,
    },
    #[error("{path}: `{name}` fehlt im Archiv")]
    Fehlt { path: PathBuf, name: String },
    #[error("{path}: `{name}` ist kein lesbares USDA: {detail}")]
    Form {
        path: PathBuf,
        name: String,
        detail: String,
    },
    #[error("kein Aufnahmeverzeichnis: {0}")]
    KeineAufnahme(String),
    #[error("{0}")]
    Assets(String),
}

/// Ein Element aus der Aufnahme, in der Form, die `roomplan-capture/v1` verlangt.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Der Dateiname ohne Endung, also `Wall0`, `Door1`, `Bed0`.
    pub id: String,
    /// Apples Kategorie, woertlich: `Wall`, `Door(Isopen: False)`, `Window`, `Bed`, `Storage` …
    pub category: String,
    /// Ausdehnung im lokalen Rahmen, in Metern. Fuer Waende und Objekte ist `y` die Hoehe; die
    /// Bodenplatte ist die Ausnahme, dort ist `z` die Staerke.
    pub dimensions_m: [f64; 3],
    /// 16 Zahlen in der Reihenfolge, in der `dashboard/src/lib/roomplan.ts` sie liest: die
    /// Verschiebung steht in `[12..15]`.
    ///
    /// USD schreibt zeilenweise mit der Verschiebung in der vierten Zeile, und das ist dieselbe
    /// Reihenfolge. [`transform`] begruendet, warum hier **nicht** transponiert wird.
    pub transform: [f64; 16],
    /// Immer `"low"`: die USDZ traegt kein Vertrauen, siehe Modulkopf.
    pub confidence: &'static str,
    /// Die native UUID aus `customData`. Die Aufnahme meldet `native_identifiers_present: true`,
    /// und das ist das Feld, das damit gemeint ist.
    pub source_id: String,
}

impl Element {
    /// Wohin das Element in der Aufnahme steht, in Metern. Aus derselben Spalte, die der Client
    /// liest.
    pub fn position_m(&self) -> [f64; 3] {
        [self.transform[12], self.transform[13], self.transform[14]]
    }

    pub fn ist_flaeche(&self) -> bool {
        matches!(self.category.as_str(), "Wall" | "Floor")
    }

    /// Apples Kategorie traegt bei Tueren den Zustand mit (`Door(Isopen: False)`), deshalb wird
    /// auf den Anfang geprueft und nicht auf Gleichheit.
    pub fn ist_oeffnung(&self) -> bool {
        self.category.starts_with("Door")
            || self.category.starts_with("Window")
            || self.category.starts_with("Opening")
    }

    pub fn ist_tuer(&self) -> bool {
        self.category.starts_with("Door") || self.category.starts_with("Opening")
    }

    /// Alles, was weder Flaeche noch Oeffnung ist: Moebelstuecke und Geraete.
    pub fn ist_objekt(&self) -> bool {
        !self.ist_flaeche() && !self.ist_oeffnung()
    }
}

/// Eine gelesene Aufnahme.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scan {
    pub elements: Vec<Element>,
    /// Wann das Archiv geschrieben wurde, aus dem ZIP-Zeitstempel — nicht „jetzt". Der Scan
    /// datiert sich selbst, und ein Entwurf, der das Datum des Lesens traegt, verliert genau die
    /// Angabe, die ueber seine Gueltigkeit entscheidet.
    pub capture_finished_at: Option<String>,
    /// Anzahl der Eintraege unter `assets/Mesh/`.
    pub mesh_assets: usize,
    /// Wie viele Raumgruppen `CapturedRoom.usda` fuehrt, ohne Mitgliedschaft.
    pub room_groups: usize,
    pub sha256: String,
    pub byte_length: usize,
    /// Der USDA-Text der Bodenplatte. Die Zusammenfassung traegt nur Spannen, und die Flaeche
    /// einer L-Form steht nicht in ihrer Spanne.
    pub boden_usda: Option<String>,
}

impl Scan {
    pub fn flaechen(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.ist_flaeche())
    }
    pub fn oeffnungen(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.ist_oeffnung())
    }
    pub fn objekte(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.ist_objekt())
    }

    pub fn kategorien(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for e in &self.elements {
            *out.entry(e.category.clone()).or_insert(0) += 1;
        }
        out
    }

    fn einheitlich(&self, achse: usize, kategorie: &str) -> Option<f64> {
        let werte: Vec<f64> = self
            .flaechen()
            .filter(|e| e.category == kategorie)
            .map(|e| e.dimensions_m[achse])
            .collect();
        let erste = *werte.first()?;
        // Ein Mikrometer und nicht Gleichheit: die Maschenausgabe traegt Werte wie -0,15999998
        // neben 0,16, und ein Gleichheitsvergleich erklaert elf einheitliche Waende plus eine
        // gerundete fuer uneinig. Ein Mikrometer liegt weit unter allem, was dieses Programm in
        // Zentimetern ausdruecken kann.
        if werte.iter().all(|w| (w - erste).abs() < 1e-6) {
            Some(erste)
        } else {
            None
        }
    }

    /// Die Hoehe, die **jede** Wand traegt, in Metern.
    ///
    /// `None`, wenn sich die Waende widersprechen. Ein einzelner Wert aus einem widerspruechigen
    /// Scan waere die Zahl, die am Ende niemand mehr nachpruefen kann.
    pub fn wandhoehe_m(&self) -> Option<f64> {
        self.einheitlich(1, "Wall")
    }

    /// Die Wandstaerke in Metern, wenn alle Waende dieselbe tragen, sonst `None`.
    pub fn wandstaerke_m(&self) -> Option<f64> {
        self.einheitlich(2, "Wall")
    }

    /// Flaeche der oberen Bodenseite in Quadratmetern.
    pub fn bodenflaeche_m2(&self) -> Option<f64> {
        obere_flaeche_m2(self.boden_usda.as_deref()?)
    }
}

// ------------------------------------------------------------------ ZIP

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Der Zeitstempel eines ZIP-Eintrags als ISO-8601 ohne Zonenangabe.
///
/// ZIP fuehrt MS-DOS-Zeit: zwei Sekunden Aufloesung, Jahre ab 1980, kein Zeitzonenfeld. Was hier
/// herauskommt, ist die Uhrzeit des aufnehmenden Geraets und **nicht** als UTC ausgewiesen — das
/// Feld existiert im Archiv nicht, und eine Zone zu erfinden waere eine Genauigkeit, die die
/// Quelle nicht hat.
fn dos_zeit(datum: u16, zeit: u16) -> Option<String> {
    let jahr = 1980 + ((datum >> 9) & 0x7f) as i64;
    let monat = ((datum >> 5) & 0x0f) as u32;
    let tag = (datum & 0x1f) as u32;
    let stunde = (zeit >> 11) as u32;
    let minute = ((zeit >> 5) & 0x3f) as u32;
    let sekunde = ((zeit & 0x1f) * 2) as u32;
    if !(1..=12).contains(&monat) || !(1..=31).contains(&tag) {
        return None;
    }
    Some(format!(
        "{jahr:04}-{monat:02}-{tag:02}T{stunde:02}:{minute:02}:{sekunde:02}"
    ))
}

struct Eintrag<'a> {
    name: String,
    daten: &'a [u8],
    zeit: Option<String>,
}

/// Liest die Eintraege eines ZIP, sofern sie alle gespeichert sind.
///
/// Gelesen wird das zentrale Verzeichnis und nicht der erste lokale Kopf: nur dort stehen die
/// Groessen verlaesslich. Ein lokaler Kopf darf sie als null fuehren und die Zahlen in einem
/// Datenbeschreiber hinter den Nutzdaten nachreichen, und ein Leser, der das annimmt, liest
/// stillschweigend zu kurz.
fn zip_eintraege<'a>(bytes: &'a [u8], path: &Path) -> Result<Vec<Eintrag<'a>>, ScanError> {
    let kein_zip = |detail: &str| ScanError::KeinZip {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    };
    // Das EOCD steht am Ende, hinter einem Kommentar von hoechstens 65535 Bytes.
    let suche_ab = bytes.len().saturating_sub(65_557);
    let eocd = (suche_ab..bytes.len().saturating_sub(3))
        .rev()
        .find(|&i| u32_at(bytes, i) == 0x0605_4b50)
        .ok_or_else(|| kein_zip("kein End-of-Central-Directory gefunden"))?;
    let anzahl = u16_at(bytes, eocd + 10) as usize;
    let mut pos = u32_at(bytes, eocd + 16) as usize;

    let mut out = Vec::with_capacity(anzahl);
    for _ in 0..anzahl {
        if pos + 46 > bytes.len() || u32_at(bytes, pos) != 0x0201_4b50 {
            return Err(kein_zip("zentrales Verzeichnis endet vorzeitig"));
        }
        let verfahren = u16_at(bytes, pos + 10);
        let zeit_roh = u16_at(bytes, pos + 12);
        let datum_roh = u16_at(bytes, pos + 14);
        let groesse = u32_at(bytes, pos + 24) as usize;
        let name_len = u16_at(bytes, pos + 28) as usize;
        let extra_len = u16_at(bytes, pos + 30) as usize;
        let kommentar_len = u16_at(bytes, pos + 32) as usize;
        let lokal = u32_at(bytes, pos + 42) as usize;
        let name = String::from_utf8_lossy(&bytes[pos + 46..pos + 46 + name_len]).into_owned();
        pos += 46 + name_len + extra_len + kommentar_len;

        if name.ends_with('/') {
            continue;
        }
        if verfahren != VERFAHREN_GESPEICHERT {
            return Err(ScanError::Komprimiert {
                path: path.to_path_buf(),
                name,
                verfahren,
            });
        }
        // Der lokale Kopf traegt eigene Namens- und Extra-Laengen, und sie duerfen von den
        // Werten im zentralen Verzeichnis abweichen.
        if lokal + 30 > bytes.len() || u32_at(bytes, lokal) != 0x0403_4b50 {
            return Err(kein_zip("lokaler Kopf fehlt"));
        }
        let l_name = u16_at(bytes, lokal + 26) as usize;
        let l_extra = u16_at(bytes, lokal + 28) as usize;
        let start = lokal + 30 + l_name + l_extra;
        if start + groesse > bytes.len() {
            return Err(kein_zip("Eintrag reicht ueber das Archiv hinaus"));
        }
        out.push(Eintrag {
            name,
            daten: &bytes[start..start + groesse],
            zeit: dos_zeit(datum_roh, zeit_roh),
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------ USDA

/// Alle `(a, b, c)`-Tripel aus einem Feld wie `point3f[] points = [ … ]`.
///
/// Gesucht wird der Feldname und danach die **erste** oeffnende Klammer. Beim Feldnamen
/// `point3f[] points` liegt das `[]` noch davor, das erste `[` nach ihm ist also die echte Liste.
fn tripel(text: &str, feld: &str) -> Option<Vec<[f64; 3]>> {
    let start = text.find(feld)? + feld.len();
    let rest = &text[start..];
    let auf = rest.find('[')?;
    let zu = rest[auf..].find(']')? + auf;
    let mut out = Vec::new();
    for gruppe in rest[auf + 1..zu].split('(').skip(1) {
        let ende = gruppe.find(')')?;
        let zahlen: Vec<f64> = gruppe[..ende]
            .split(',')
            .filter_map(|z| z.trim().parse::<f64>().ok())
            .collect();
        if zahlen.len() == 3 {
            out.push([zahlen[0], zahlen[1], zahlen[2]]);
        }
    }
    Some(out)
}

/// Eine Liste ganzer Zahlen aus `int[] NAME = [ … ]`.
///
/// Der Feldname darf das `[]` nicht enthalten, sonst findet `find('[')` die Klammer des Typs und
/// liefert eine leere Liste — still, und damit sieht jede Flaeche danach wie null aus.
fn zahlen(text: &str, feld: &str) -> Option<Vec<usize>> {
    let start = text.find(feld)? + feld.len();
    let rest = &text[start..];
    let auf = rest.find('[')?;
    let zu = rest[auf..].find(']')? + auf;
    Some(
        rest[auf + 1..zu]
            .split(',')
            .filter_map(|z| z.trim().parse::<usize>().ok())
            .collect(),
    )
}

/// Ein `string NAME = "…"` aus dem Kopf einer USDA.
fn zeichenkette(text: &str, feld: &str) -> Option<String> {
    let start = text.find(feld)? + feld.len();
    let rest = &text[start..];
    let auf = rest.find('"')? + 1;
    let zu = rest[auf..].find('"')? + auf;
    Some(rest[auf..zu].to_string())
}

/// Die `matrix4d xformOp:transform`, **ohne** Umsetzung.
///
/// Das sieht nach einem vergessenen Rechenschritt aus und ist genau richtig. Der Client liest die
/// Verschiebung aus `transform[12..15]`; USD schreibt sie in die vierte **Zeile** eines
/// zeilenweise abgelegten Feldes, und die liegt bei 12 bis 15. Beides faellt zusammen, weil USD
/// den Punkt als Zeilenvektor fuehrt (`p' = p · M`) und das Client-Format als Spaltenvektor
/// (`p' = M · p`): die beiden Fassungen sind Transponierte voneinander, und die Transponierung
/// schiebt die Verschiebung in der einen in die vierte Zeile und in der anderen in die vierte
/// Spalte — dieselben vier Indizes.
///
/// Wer hier transponiert, schiebt jedes Element nach (0, 0, 0). Das ist nicht nur falsch, es
/// sieht auch plausibel aus: die Aufnahme hat dann eben einen Schwerpunkt im Ursprung, und
/// nichts meldet es. Ein erster Anlauf dieses Moduls tat genau das, und
/// `die_verschiebung_landet_in_der_vierten_spalte` hat es gefangen.
fn transform(text: &str) -> Option<[f64; 16]> {
    let start = text.find("matrix4d xformOp:transform")?;
    let rest = &text[start..];
    let auf = rest.find('(')? + 1;
    // Die Matrix steht in vier Klammergruppen; die fuenfte Klammer schliesst das Feld. Gesucht
    // werden die Gruppen, nicht das erste `)`.
    let mut zeilen = Vec::new();
    for zeile in rest[auf..].split('(').skip(1) {
        let ende = zeile.find(')')?;
        let zahlen: Vec<f64> = zeile[..ende]
            .split(',')
            .filter_map(|z| z.trim().parse::<f64>().ok())
            .collect();
        if zahlen.len() == 4 {
            zeilen.push(zahlen);
        }
        if zeilen.len() == 4 {
            break;
        }
    }
    if zeilen.len() != 4 {
        return None;
    }
    let mut out = [0.0f64; 16];
    for (z, zeile) in zeilen.iter().enumerate() {
        for (s, wert) in zeile.iter().enumerate() {
            out[z * 4 + s] = *wert;
        }
    }
    Some(out)
}

/// Ein einzelnes `…/Wall0.usda` in ein Element.
///
/// Die Ausdehnung kommt aus der Spanne der Punkte im **lokalen** Rahmen: das ist die Groesse, die
/// das Element vor seiner Verschiebung hat, und damit die einzige, die unveraendert bleibt, wenn
/// dieselbe Aufnahme spaeter anders ausgerichtet wird.
pub fn parse_usda(name: &str, text: &str) -> Option<Element> {
    let punkte = tripel(text, "point3f[] points")?;
    if punkte.is_empty() {
        return None;
    }
    let transform = transform(text)?;
    let mut spanne = [0.0f64; 3];
    for achse in 0..3 {
        let min = punkte
            .iter()
            .map(|p| p[achse])
            .fold(f64::INFINITY, f64::min);
        let max = punkte
            .iter()
            .map(|p| p[achse])
            .fold(f64::NEG_INFINITY, f64::max);
        spanne[achse] = max - min;
    }
    Some(Element {
        id: name
            .rsplit('/')
            .next()
            .unwrap_or(name)
            .trim_end_matches(".usda")
            .to_string(),
        category: zeichenkette(text, "string Category").unwrap_or_else(|| "Unknown".to_string()),
        dimensions_m: spanne,
        transform,
        confidence: "low",
        source_id: zeichenkette(text, "string UUID").unwrap_or_default(),
    })
}

/// Die Flaeche der **oberen** Seite eines flachen Koerpers, aus seinen Dreiecken.
///
/// Nicht `x * y`: die Bodenplatte ist eine L-Form, und ihr Rahmen misst die Huelle, nicht die
/// Flaeche. Ebenso wenig aus einem Ring ihrer Randpunkte — der Boden traegt 28 Facetten, davon 6
/// nach oben, 6 nach unten und 16 als Kante (nachgesehen am 2026-09-26 an der echten Aufnahme;
/// ein Ring aus acht Randpunkten ergab 32,99 statt 31,57 m², weil die L-Form dabei zu ihrer
/// Huelle wird). Gerechnet wird deshalb ueber genau die Dreiecke, deren Normale im lokalen Rahmen
/// nach oben zeigt: bei der Bodenplatte ist das die lokale z-Achse, weil sie flach liegt.
pub fn obere_flaeche_m2(text: &str) -> Option<f64> {
    let punkte = tripel(text, "point3f[] points")?;
    let normalen = tripel(text, "normal3f[] normals")?;
    let ecken_je_flaeche = zahlen(text, "faceVertexCounts")?;
    let indizes = zahlen(text, "faceVertexIndices")?;

    let mut summe = 0.0;
    let mut o = 0usize;
    for n in ecken_je_flaeche {
        if o + n > indizes.len() {
            return None;
        }
        let ecken = &indizes[o..o + n];
        o += n;
        let hoch = ecken
            .iter()
            .filter_map(|&i| normalen.get(i))
            .map(|n| n[2])
            .sum::<f64>()
            / n as f64;
        if hoch <= 0.9 {
            continue;
        }
        for k in 1..n.saturating_sub(1) {
            let (a, b, c) = (punkte[ecken[0]], punkte[ecken[k]], punkte[ecken[k + 1]]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let kreuz = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            summe += (kreuz[0] * kreuz[0] + kreuz[1] * kreuz[1] + kreuz[2] * kreuz[2]).sqrt() / 2.0;
        }
    }
    Some(summe)
}

/// Eine USDZ lesen. Bekommt den Pfad, nicht den Inhalt: die Aufnahme ist gross und bleibt liegen,
/// wo sie liegt.
pub fn read_usdz(path: &Path) -> Result<Scan, ScanError> {
    let bytes = std::fs::read(path).map_err(|source| ScanError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let sha256 = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&bytes))
    };
    let eintraege = zip_eintraege(&bytes, path)?;

    let mut elements = Vec::new();
    let mut mesh_assets = 0usize;
    let mut boden_usda = None;
    for e in &eintraege {
        if e.name.starts_with("assets/Mesh/") {
            mesh_assets += 1;
        }
        if !e.name.ends_with(".usda") || !e.name.starts_with("assets/Mesh/") {
            continue;
        }
        let text = std::str::from_utf8(e.daten).map_err(|err| ScanError::Form {
            path: path.to_path_buf(),
            name: e.name.clone(),
            detail: format!("kein UTF-8: {err}"),
        })?;
        let element = parse_usda(&e.name, text).ok_or_else(|| ScanError::Form {
            path: path.to_path_buf(),
            name: e.name.clone(),
            detail: "Punkte, Normale oder Transform fehlen".to_string(),
        })?;
        if element.category == "Floor" {
            boden_usda = Some(text.to_string());
        }
        elements.push(element);
    }

    let kopf = eintraege
        .iter()
        .find(|e| e.name == "CapturedRoom.usda")
        .ok_or_else(|| ScanError::Fehlt {
            path: path.to_path_buf(),
            name: "CapturedRoom.usda".to_string(),
        })?;
    let kopf_text = std::str::from_utf8(kopf.daten).map_err(|err| ScanError::Form {
        path: path.to_path_buf(),
        name: kopf.name.clone(),
        detail: format!("kein UTF-8: {err}"),
    })?;
    // Jede Raumgruppe fuehrt neben sich einen Mittelpunkt `…_centerTop`. Deren Anzahl ist die
    // Anzahl der Gruppen, ohne die Klammerstruktur der Datei nachzubauen.
    let room_groups = kopf_text.matches("_centerTop").count();

    elements.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Scan {
        elements,
        capture_finished_at: kopf.zeit.clone(),
        mesh_assets,
        room_groups,
        sha256,
        byte_length: bytes.len(),
        boden_usda,
    })
}

// ------------------------------------------------------------------ Aufnahmen auf der Platte

/// `<assets_root>/captures/<flat>`, oder ein Fehler mit dem Pfad, der gefehlt hat.
pub fn captures_root(flat: &str) -> Result<PathBuf, ScanError> {
    let root = crate::model::assets_dir()
        .map_err(|e| ScanError::Assets(e.to_string()))?
        .join("captures")
        .join(flat);
    if root.is_dir() {
        Ok(root)
    } else {
        Err(ScanError::KeineAufnahme(root.display().to_string()))
    }
}

/// Die Aufnahmedaten unter einem Wurzelverzeichnis, aufsteigend sortiert. Der Name ist das
/// Datum, und ein Datum sortiert sich als Zeichenkette richtig.
pub fn capture_dates(flat: &str) -> Result<Vec<PathBuf>, ScanError> {
    let root = captures_root(flat)?;
    let mut dates: Vec<PathBuf> = std::fs::read_dir(&root)
        .map_err(|source| ScanError::Read {
            path: root.clone(),
            source,
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dates.sort();
    Ok(dates)
}

/// Die juengste Aufnahme, deren USDZ **und** Beobachtung beide da sind.
///
/// Dieselbe Bedingung, die `/api/roomplan/reference` fuehrt: eine Aufnahme ohne ihre Beobachtung
/// ist eine halbe, und ein halber Scan sieht aus wie ein ganzer.
pub fn latest_capture(flat: &str) -> Result<PathBuf, ScanError> {
    for dir in capture_dates(flat)?.into_iter().rev() {
        if dir.join("captured-room.usdz").is_file() && dir.join("observation.json").is_file() {
            return Ok(dir);
        }
    }
    Err(ScanError::KeineAufnahme(format!(
        "unter {} liegt keine vollstaendige Aufnahme",
        captures_root(flat)?.display()
    )))
}

/// Der Entwurf als Datei neben die Aufnahme — dort, wo `/api/roomplan/revisions` ihn liest.
///
/// Geschrieben wird `draft.json`. Ein bereits vorhandener Entwurf wird ersetzt: er ist aus der
/// USDZ abgeleitet und traegt keine Information, die dabei verloren gehen koennte. Das **Review**
/// daneben (`review.json`) wird nicht angefasst, und die USDZ selbst schon gar nicht.
pub fn write_draft(
    scan: &Scan,
    flat: &str,
    verzeichnis: &Path,
    heute: &str,
) -> std::io::Result<PathBuf> {
    let pfad = verzeichnis.join("draft.json");
    let inhalt = draft(scan, flat, heute);
    std::fs::write(&pfad, serde_json::to_vec_pretty(&inhalt)?)?;
    Ok(pfad)
}

// ------------------------------------------------------------------ Entwurf

/// Der Entwurf als `roomplan-capture/v1` — die Form, die `/api/roomplan/revisions` erwartet und
/// `dashboard/src/lib/roomplan.ts` liest.
///
/// `created_at` ist der Tag, an dem dieser Entwurf entsteht; `provenance.capture_finished_at`
/// traegt den Tag der Aufnahme. Zwei Angaben, weil sie zwei Dinge sind.
pub fn draft(scan: &Scan, flat: &str, created_at: &str) -> serde_json::Value {
    let als_json = |e: &Element| {
        serde_json::json!({
            "id": e.id,
            "category": e.category,
            "dimensions_m": e.dimensions_m,
            "transform": e.transform,
            "confidence": e.confidence,
            "source_id": e.source_id,
        })
    };
    let mut flaechen = Vec::new();
    let mut oeffnungen = Vec::new();
    let mut objekte = Vec::new();
    for e in &scan.elements {
        if e.ist_flaeche() {
            flaechen.push(als_json(e));
        } else if e.ist_oeffnung() {
            oeffnungen.push(als_json(e));
        } else {
            objekte.push(als_json(e));
        }
    }
    let kurz = &scan.sha256[..16.min(scan.sha256.len())];
    serde_json::json!({
        "schema_version": "roomplan-capture/v1",
        "draft_id": format!("raw-{kurz}"),
        "created_at": created_at,
        "source": {
            "platform": "macos",
            "roomplan_version": "raw-usdz",
            "app_version": env!("CARGO_PKG_VERSION"),
        },
        "coordinate_system": { "units": "meters", "up_axis": "Y" },
        "room": {
            "id": flat,
            "surfaces": flaechen,
            "openings": oeffnungen,
            "objects": objekte,
        },
        "assets": [{
            "asset_id": format!("usdz-{kurz}"),
            "role": "mesh",
            "format": "usdz",
            "byte_length": scan.byte_length,
            "sha256": scan.sha256,
            "storage_token": "captures",
        }],
        "provenance": {
            "capture_mode": "new_room",
            "capture_finished_at": scan.capture_finished_at,
            "notes": "Read from the raw USDZ. RoomPlan confidence is not carried by the USDZ, \
                      so every element is low. Room membership is not carried either.",
        },
    })
}

// ------------------------------------------------------------------ Vergleich

/// Ein Befund: was der Scan sagt, was `room.toml` sagt, und wie weit sie auseinanderliegen.
///
/// Der Vergleich behauptet **keine Identitaet** zwischen einem Scan-Element und einem Eintrag aus
/// `room.toml`. Eine Oeffnung im Scan hat keinen Namen und einen Zustand (`Isopen`), ein Eintrag
/// in `room.toml` hat eine Kennung und eine Schwelle; beides zusammenzubringen ist eine
/// Entscheidung und keine Rechnung. Hier stehen deshalb beide Seiten nebeneinander, und die
/// Zuordnung passiert im Review.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    /// Die Wandhoehe in Zentimetern, wenn alle Waende einig sind.
    pub scan_wandhoehe_cm: Option<f64>,
    /// Was `room.toml` als Hoehe fuehrt. `0` heisst dort „nicht gemessen" und nicht „null".
    pub modell_hoehe_cm: i32,
    /// Tueren und Fenster aus dem Scan: Kennung, Kategorie, Breite und Hoehe in Zentimetern.
    pub scan_oeffnungen: Vec<(String, String, f64, f64)>,
    /// Tueren und Fenster aus `room.toml`: Kennung, Breite in Zentimetern.
    pub modell_oeffnungen: Vec<(String, i32)>,
    /// Der Boden, den die Aufnahme zeigt, als Flaeche der oberen Bodenseite.
    pub scan_grundflaeche_m2: Option<f64>,
    /// Die innere Grundflaeche, die `room.toml` kennt.
    pub modell_innenflaeche_m2: f64,
    /// Gesamtlaenge aller Waende aus dem Scan, in Zentimetern. Nicht die Summe aus `room.toml`:
    /// der Scan trennt Waende an ihren Oeffnungen, `room.toml` fuehrt eine Wand mit einer
    /// Oeffnung darin, und die beiden Summen sind deshalb nicht dieselbe Groesse. Der Wert steht
    /// zum Nachsehen hier und geht in kein Urteil ein.
    pub scan_wandlaenge_cm: f64,
    /// Moebelstuecke und Geraete aus dem Scan: `Element (Kategorie)` und die Aussenmasse in
    /// Zentimetern, in der Reihenfolge des lokalen Rahmens.
    pub scan_objekte: Vec<(String, [f64; 3])>,
}

impl Diff {
    /// Wo der Scan eine Zahl hat, die `room.toml` als offen fuehrt.
    pub fn offene_frage_beantwortet(&self) -> bool {
        self.scan_wandhoehe_cm.is_some() && self.modell_hoehe_cm == 0
    }
}

/// Die innere Grundflaeche, die das Modell kennt: das Polygon des Hauptraums (die Kuechennische
/// ist Teil davon) plus das Bad.
///
/// Gerechnet und nicht aus `[flat.flaeche]` gelesen — dieselbe Begruendung, die `Room::area_m2`
/// schon fuehrt: eine abgeschriebene Zahl kann von der Geometrie abdriften, aus der jede Pruefung
/// sie ableitet. Die Aufnahme zeigt die ganze Wohnung und nicht nur einen Raum, und
/// `innen_gemessen_m2` in `room.toml` ist genau diese Summe; gelesen wird sie hier trotzdem
/// nicht.
fn modell_innenflaeche_m2(room: &Room) -> f64 {
    room.area_m2() + room.bad.as_ref().map_or(0.0, |bad| bad.flaeche_m2)
}

/// Der Scan gegen das gemessene Modell. Rechnet, schreibt nichts.
pub fn diff(scan: &Scan, room: &Room) -> Diff {
    let mut oeffnungen: Vec<(String, String, f64, f64)> = scan
        .oeffnungen()
        .map(|e| {
            (
                e.id.clone(),
                e.category.clone(),
                e.dimensions_m[0] * 100.0,
                e.dimensions_m[1] * 100.0,
            )
        })
        .collect();
    oeffnungen.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut objekte: Vec<(String, [f64; 3])> = scan
        .objekte()
        .map(|e| {
            (
                format!("{} ({})", e.id, e.category),
                [
                    e.dimensions_m[0] * 100.0,
                    e.dimensions_m[1] * 100.0,
                    e.dimensions_m[2] * 100.0,
                ],
            )
        })
        .collect();
    objekte.sort_by(|a, b| a.0.cmp(&b.0));
    Diff {
        scan_wandhoehe_cm: scan.wandhoehe_m().map(|m| m * 100.0),
        modell_hoehe_cm: room.hauptraum.hoehe,
        scan_oeffnungen: oeffnungen,
        modell_oeffnungen: room
            .oeffnungen
            .iter()
            .map(|o| (o.id.clone(), o.breite))
            .collect(),
        scan_grundflaeche_m2: scan.bodenflaeche_m2(),
        modell_innenflaeche_m2: modell_innenflaeche_m2(room),
        scan_wandlaenge_cm: scan
            .flaechen()
            .filter(|e| e.category == "Wall")
            .map(|e| e.dimensions_m[0] * 100.0)
            .sum(),
        scan_objekte: objekte,
    }
}

/// Der Scan gegen ein geladenes Modell.
pub fn vergleiche(scan: &Scan, model: &Model) -> Diff {
    diff(scan, &model.room)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein ZIP mit gespeicherten Eintraegen, wie `zip -0` es schreibt. Nur so viel Format, wie
    /// der Leser oben behauptet zu verstehen.
    fn zip(eintraege: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut zentral = Vec::new();
        // Ein fester Zeitstempel: 1980-01-01 00:00, damit der Test nicht von der Uhr abhaengt.
        for (name, inhalt) in eintraege {
            let offset = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // Version
            out.extend_from_slice(&0u16.to_le_bytes()); // Flags
            out.extend_from_slice(&VERFAHREN_GESPEICHERT.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // Zeit
            out.extend_from_slice(&0x0021u16.to_le_bytes()); // Datum: 1980-01-01
            out.extend_from_slice(&0u32.to_le_bytes()); // CRC, hier nicht geprueft
            out.extend_from_slice(&(inhalt.len() as u32).to_le_bytes());
            out.extend_from_slice(&(inhalt.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // Extra
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(inhalt.as_bytes());

            zentral.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            zentral.extend_from_slice(&20u16.to_le_bytes()); // Version, die es schrieb
            zentral.extend_from_slice(&20u16.to_le_bytes()); // Version, die es braucht
            zentral.extend_from_slice(&0u16.to_le_bytes()); // Flags
            zentral.extend_from_slice(&VERFAHREN_GESPEICHERT.to_le_bytes());
            zentral.extend_from_slice(&0u16.to_le_bytes());
            zentral.extend_from_slice(&0x0021u16.to_le_bytes());
            zentral.extend_from_slice(&0u32.to_le_bytes());
            zentral.extend_from_slice(&(inhalt.len() as u32).to_le_bytes());
            zentral.extend_from_slice(&(inhalt.len() as u32).to_le_bytes());
            zentral.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zentral.extend_from_slice(&0u16.to_le_bytes()); // Extra
            zentral.extend_from_slice(&0u16.to_le_bytes()); // Kommentar
            zentral.extend_from_slice(&0u16.to_le_bytes()); // Platte
            zentral.extend_from_slice(&0u16.to_le_bytes()); // Attribute
            zentral.extend_from_slice(&0u32.to_le_bytes()); // extern
            zentral.extend_from_slice(&offset.to_le_bytes());
            zentral.extend_from_slice(name.as_bytes());
        }
        let zentral_start = out.len() as u32;
        out.extend_from_slice(&zentral);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(eintraege.len() as u16).to_le_bytes());
        out.extend_from_slice(&(eintraege.len() as u16).to_le_bytes());
        out.extend_from_slice(&(zentral.len() as u32).to_le_bytes());
        out.extend_from_slice(&zentral_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// Eine Wand von 3,00 m Laenge, 2,50 m Hoehe und 0,16 m Staerke, verschoben um (1, 2, 3).
    /// Die Zahlen sind erfunden und haben mit keiner Wohnung zu tun.
    fn wand_usda() -> String {
        r#"#usda 1.0
(
    defaultPrim = "Wall0"
    metersPerUnit = 1
)
def Xform "Wall0" (
    customData = {
        string Category = "Wall"
        string UUID = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"
    }
)
{
    def Mesh "Wall0"
    {
        point3f[] points = [(0, 0, 0), (300, 0, 0), (300, 250, 0), (0, 250, 0), (0, 0, 16), (300, 0, 16), (300, 250, 16), (0, 250, 16)]
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (1, 2, 3, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }
}
"#
        .to_string()
    }

    #[test]
    fn ein_gespeicherter_eintrag_wird_gelesen_und_sein_zeitstempel_erkannt() {
        let bytes = zip(&[("CapturedRoom.usda", "def Xform \"Section_grp\"\n")]);
        let path = std::env::temp_dir().join(format!("interior-zip-{}.usdz", std::process::id()));
        std::fs::write(&path, &bytes).expect("Testdatei");
        let eintraege = zip_eintraege(&bytes, &path).expect("gelesen");
        assert_eq!(eintraege.len(), 1);
        assert_eq!(eintraege[0].name, "CapturedRoom.usda");
        assert_eq!(
            eintraege[0].zeit.as_deref(),
            Some("1980-01-01T00:00:00"),
            "das Datum 0x0021 ist der 1.1.1980"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Ein komprimierter Eintrag ist ein Fehler und keine leere Antwort. Ohne diesen Test waere
    /// die Regel „kein Entpacker" eine Behauptung im Modulkopf.
    #[test]
    fn ein_komprimierter_eintrag_ist_ein_fehler() {
        let mut bytes = zip(&[("CapturedRoom.usda", "leer")]);
        // Verfahren im lokalen UND im zentralen Kopf auf 8 (deflate) setzen.
        let lokal = bytes.iter().position(|_| true).unwrap();
        bytes[lokal + 8] = 8;
        let zentral_start = u32::from_le_bytes([
            bytes[bytes.len() - 6],
            bytes[bytes.len() - 5],
            bytes[bytes.len() - 4],
            bytes[bytes.len() - 3],
        ]) as usize;
        bytes[zentral_start + 10] = 8;
        let path =
            std::env::temp_dir().join(format!("interior-deflate-{}.usdz", std::process::id()));
        std::fs::write(&path, &bytes).expect("Testdatei");
        match zip_eintraege(&bytes, &path) {
            Err(ScanError::Komprimiert { verfahren, .. }) => assert_eq!(verfahren, 8),
            Err(other) => panic!("erwartet Komprimiert, bekommen: {other:?}"),
            // Kein `{:?}` auf dem Erfolgsfall: der traegt die Nutzdaten, und ein Test, der bei
            // einem Fehlschlag ein Archiv ausdruckt, ist beim Lesen schlimmer als nutzlos.
            Ok(gelesen) => panic!(
                "erwartet einen Fehler, gelesen wurden {} Eintraege",
                gelesen.len()
            ),
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Die Verschiebung muss in `[12..15]` landen. USD schreibt sie in die vierte Zeile; steht sie
    /// nach der Umsetzung dort, liest der Client sie falsch, ohne dass irgendetwas fehlschlaegt.
    #[test]
    fn die_verschiebung_landet_in_der_vierten_spalte() {
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wand_usda()).expect("Element");
        assert_eq!(e.id, "Wall0");
        assert_eq!(e.category, "Wall");
        assert_eq!(
            e.source_id, "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
            "die native UUID ist die Identitaet, nicht der Dateiname"
        );
        assert_eq!(e.position_m(), [1.0, 2.0, 3.0]);
        assert_eq!(e.confidence, "low", "die USDZ traegt kein Vertrauen");
    }

    #[test]
    fn die_ausdehnung_kommt_aus_dem_lokalen_rahmen_in_metern() {
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wand_usda()).expect("Element");
        // Punkte in Zentimetern geschrieben und durch metersPerUnit = 1 als Meter gemeint: die
        // Spanne ist die Differenz der Punkte, unveraendert.
        assert_eq!(e.dimensions_m, [300.0, 250.0, 16.0]);
    }

    #[test]
    fn kategorien_werden_getrennt() {
        let fenster = r#"def Xform "Window0" ( customData = { string Category = "Window" } )
{ def Mesh "Window0" {
    point3f[] points = [(0, 0, 0), (90, 0, 0), (90, 120, 0)]
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
} }"#;
        let w = parse_usda("Wall0.usda", &wand_usda()).expect("Wand");
        let f = parse_usda("Window0.usda", fenster).expect("Fenster");
        assert!(w.ist_flaeche() && !w.ist_oeffnung() && !w.ist_objekt());
        assert!(f.ist_oeffnung() && !f.ist_tuer());
        assert!(f.dimensions_m[0] > 0.0);
    }

    #[test]
    fn eine_geschlossene_tuer_ist_immer_noch_eine_tuer() {
        let tuer = r#"def Xform "Door0" ( customData = { string Category = "Door(Isopen: False)" } )
{ def Mesh "Door0" {
    point3f[] points = [(0, 0, 0), (90, 0, 0), (90, 200, 0)]
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
} }"#;
        let t = parse_usda("Door0.usda", tuer).expect("Tuer");
        assert!(
            t.ist_tuer(),
            "der Zustand in der Kategorie macht sie nicht zu etwas anderem"
        );
    }

    /// Die Koerper sind flach, und ihre Flaeche steht nicht in ihrer Spanne. Der Test haelt den
    /// Unterschied fest: die Dreieckssumme zweier gleich grosser Platten ist ihre Summe, waehrend
    /// die Spanne desselben Koerpers doppelt so gross waere.
    #[test]
    fn die_bodenflaeche_kommt_aus_den_dreiecken_und_nicht_aus_der_spanne() {
        // Zwei Platten von je 2 x 3, nebeneinander: die Gesamtspanne ist 4 x 3, die Flaeche 12.
        let text = r#"def Xform "Floor0" {
    def Mesh "Floor0"
    {
        int[] faceVertexCounts = [3, 3, 3, 3]
        int[] faceVertexIndices = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]
        normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1)]
        point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 3, 0), (0, 3, 0), (2, 0, 0), (4, 0, 0), (4, 3, 0), (2, 3, 0)]
    }
}"#;
        let flaeche = obere_flaeche_m2(text).expect("Flaeche");
        assert!(
            (flaeche - 12.0).abs() < 1e-9,
            "erwartet 12, bekommen {flaeche}"
        );
    }

    /// Nur nach oben zeigende Dreiecke zaehlen. Sonst waere die Flaeche die doppelte.
    ///
    /// Das zweite Dreieck benutzt die Punkte 3 bis 5 und nicht noch einmal 0 bis 2: eine
    /// Maschenausgabe, die zwei Flaechen mit verschiedener Richtung teilt, muss die Punkte doppelt
    /// fuehren, sonst traegt ein Punkt zwei Normalen zugleich. Die echte Bodenplatte tut das auch —
    /// sie traegt 28 Facetten auf 84 Punkte.
    #[test]
    fn die_unterseite_zaehlt_nicht_mit() {
        let text = r#"def Mesh "Floor0"
{
    int[] faceVertexCounts = [3, 3]
    int[] faceVertexIndices = [0, 1, 2, 3, 4, 5]
    normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, -1), (0, 0, -1), (0, 0, -1)]
    point3f[] points = [(0, 0, 0), (2, 0, 0), (0, 3, 0), (0, 0, 0), (2, 0, 0), (0, 3, 0)]
}"#;
        let flaeche = obere_flaeche_m2(text).expect("Flaeche");
        assert!(
            (flaeche - 3.0).abs() < 1e-9,
            "erwartet 3, bekommen {flaeche}"
        );
    }

    /// Ein widerspruechiger Scan gibt keine Zahl heraus. Eine Wandhoehe aus zwoelf Waenden, von
    /// denen zwei anderer Meinung sind, ist die Zahl, die niemand mehr nachpruefen kann.
    #[test]
    fn widersprechende_waende_ergeben_keine_hoehe() {
        let mut a = parse_usda("Wall0.usda", &wand_usda()).expect("Wand");
        let mut b = a.clone();
        b.dimensions_m[1] = 111.0;
        a.dimensions_m[1] = 222.0;
        let scan = Scan {
            elements: vec![a, b],
            ..Default::default()
        };
        assert_eq!(scan.wandhoehe_m(), None);
        let eines = Scan {
            elements: vec![parse_usda("Wall0.usda", &wand_usda()).expect("Wand")],
            ..Default::default()
        };
        assert_eq!(eines.wandhoehe_m(), Some(250.0));
    }

    /// Der Entwurf muss die Form tragen, die `roomplan.ts` liest. Ein falsch geschriebenes
    /// `schema_version` faellt dort nirgends auf, es kommt nur kein Entwurf an.
    #[test]
    fn der_entwurf_traegt_die_form_des_clients() {
        let scan = Scan {
            elements: vec![
                parse_usda("Wall0.usda", &wand_usda()).expect("Wand"),
                parse_usda("Bed0.usda", &wand_usda().replace("Wall", "Bed")).expect("Bett"),
            ],
            sha256: "a".repeat(64),
            byte_length: 123,
            ..Default::default()
        };
        let d = draft(&scan, "muster", "2026-09-26");
        assert_eq!(d["schema_version"], "roomplan-capture/v1");
        assert_eq!(d["coordinate_system"]["units"], "meters");
        assert_eq!(d["room"]["id"], "muster");
        assert_eq!(d["room"]["surfaces"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(d["room"]["openings"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(d["room"]["objects"].as_array().map(|a| a.len()), Some(1));
        let e = &d["room"]["surfaces"][0];
        assert_eq!(
            e["transform"][12], 1.0,
            "der Client liest die Verschiebung hier"
        );
        assert_eq!(e["confidence"], "low");
    }
}
