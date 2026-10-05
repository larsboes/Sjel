//! Der einmalige Weg von `inventory/*.toml` in die Tabellen (PRD B25).
//!
//! Die TOML-Dateien sind die **Migrationsquelle**, nicht eine zweite Wahrheit. Nach dem Import
//! liest die Pruefung den Katalog aus der Datenbank; die Dateien bleiben als Herkunftsprotokoll
//! liegen, so wie `ts-baseline.json` neben der Wohnung liegt, die sie beschreibt.
//!
//! ## `deny_unknown_fields`, und warum es der wichtigste Teil dieser Datei ist
//!
//! Die alte `model::Item`-Struktur las 22 Felder und die Dateien fuehrten 31. Neun Felder —
//! `zustaende`, `quelle_masse`, `kosten_schaetzung_eur`, `ersetzt`, `varianten_rueckwand`,
//! `laenge`, `entscheidung_offen`, `basiert_auf`, `artikelnummer` — wurden von serde still
//! verworfen, jahrelang, ohne dass irgendetwas es gemeldet haette. Ein Import, der dasselbe
//! tut, verliert sie endgueltig. Hier faellt deshalb jedes unbekannte Feld als Fehler auf.
//!
//! ## Zwei Namen fuer dieselbe Sache
//!
//! `owned.toml` sagt `quelle`, `wishlist.toml` sagt `quelle_masse`. Beides ist "woher das Mass
//! stammt". Der Import fuehrt sie in einer Spalte zusammen — ein zweites Vokabular fuer
//! dieselbe Sache ist die teure Version dieses Fehlers, und sie stirbt hier.

use crate::store::{Item, Kind, State, Store};
use serde::Deserialize;
use std::path::Path;

type Fehler = Box<dyn std::error::Error>;

/// Eine Zeile, wie sie in den Dateien steht. Jedes Feld beider Dateien, keins mehr.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Roh {
    id: String,
    label: String,
    #[serde(default)]
    b: Option<i32>,
    #[serde(default)]
    t: Option<i32>,
    #[serde(default)]
    h: Option<i32>,
    #[serde(default)]
    h_min: Option<i32>,
    #[serde(default)]
    b_aufgeklappt: Option<i32>,
    #[serde(default)]
    t_ausgeklappt: Option<i32>,
    #[serde(default)]
    laenge: Option<i32>,
    #[serde(default)]
    anzahl: Option<i32>,
    #[serde(default)]
    zustaende: Vec<String>,
    #[serde(default)]
    unsicher: Vec<String>,
    #[serde(default)]
    platzbedarf_zone: Option<i32>,
    #[serde(default)]
    platzbedarf_block: Option<i32>,
    #[serde(default)]
    preis_eur: Option<f64>,
    /// `[min, max]` in Euro. Eine Spanne, weil eine Schaetzung eine Spanne IST — eine Zahl
    /// daraus zu machen waere die Praezision, die sie nicht hat.
    #[serde(default)]
    kosten_schaetzung_eur: Option<[f64; 2]>,
    #[serde(default)]
    link: Option<String>,
    /// Steht in der Datei als Zahl. Als Text gehalten: eine Artikelnummer wird nie gerechnet,
    /// und eine fuehrende Null waere als Zahl weg.
    #[serde(default)]
    artikelnummer: Option<toml::Value>,
    #[serde(default)]
    quelle: Option<String>,
    #[serde(default)]
    quelle_masse: Option<String>,
    #[serde(default)]
    gemessen_am: Option<toml::value::Datetime>,
    #[serde(default)]
    mitnahme: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    basiert_auf: Option<String>,
    #[serde(default)]
    ersetzt: Vec<String>,
    #[serde(default)]
    varianten_rueckwand: Vec<String>,
    #[serde(default)]
    ziel: Option<String>,
    #[serde(default)]
    hinweis: Option<String>,
    #[serde(default)]
    begruendung: Option<String>,
    #[serde(default)]
    entscheidung_offen: Option<String>,

    // Was das Stueck an Platz verlangt (PRD Q61). Optional: ohne diese Felder entscheidet
    // weiterhin der Name, und der Name ist die Fassung, die abgeloest wird.
    #[serde(default)]
    opens: Option<crate::store::Seite>,
    #[serde(default)]
    open_clear: Option<i32>,
    #[serde(default)]
    wall_ok: Option<bool>,
    #[serde(default)]
    expands: Option<Expands>,
    #[serde(default)]
    access_sides: Option<i32>,
    #[serde(default)]
    access_clear: Option<i32>,
    /// `raumtrenner = true` — dieses Stueck soll frei stehen und teilt den Raum.
    #[serde(default)]
    raumtrenner: Option<bool>,
    /// `zerlegbar = true` — kommt in Teilen durch die Tuer (siehe `einbringung.rs`).
    zerlegbar: Option<bool>,
    /// Pfad unterhalb von `media/`.
    #[serde(default)]
    bild: Option<String>,
}

/// `expands = { dir = "sued", to = 145 }` — die Gesamttiefe ausgeklappt, nicht der Zuwachs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expands {
    dir: crate::store::Seite,
    to: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedFile {
    #[serde(default)]
    item: Vec<Roh>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WishlistFile {
    #[serde(default)]
    slot: Vec<Roh>,
    #[serde(default)]
    produkt: Vec<Roh>,
}

/// Euro als Fliesskomma in Cent als Ganzzahl. Gerundet, nicht abgeschnitten: 49,99 sind
/// 4999 Cent und nicht 4998, und `as i64` allein liefert genau das falsche Ergebnis.
fn cent(eur: f64) -> i64 {
    (eur * 100.0).round() as i64
}

impl Roh {
    fn into_item(self, kind: Kind) -> Item {
        Item {
            id: self.id,
            kind,
            label: self.label,
            b: self.b,
            t: self.t,
            h: self.h,
            h_min: self.h_min,
            b_aufgeklappt: self.b_aufgeklappt,
            t_ausgeklappt: self.t_ausgeklappt,
            laenge: self.laenge,
            anzahl: self.anzahl,
            zustaende: self.zustaende,
            unsicher: self.unsicher,
            platzbedarf_zone: self.platzbedarf_zone,
            platzbedarf_block: self.platzbedarf_block,
            preis_cent: self.preis_eur.map(cent),
            kosten_min_cent: self.kosten_schaetzung_eur.map(|k| cent(k[0])),
            kosten_max_cent: self.kosten_schaetzung_eur.map(|k| cent(k[1])),
            link: self.link,
            artikelnummer: self.artikelnummer.map(|v| match v {
                toml::Value::String(s) => s,
                other => other.to_string(),
            }),
            quelle: self.quelle.or(self.quelle_masse),
            gemessen_am: self.gemessen_am.map(|d| d.to_string()),
            mitnahme: self.mitnahme,
            prioritaet: self.status,
            basiert_auf: self.basiert_auf,
            ersetzt: self.ersetzt,
            varianten: self.varianten_rueckwand,
            ziel: self.ziel,
            hinweis: self.hinweis,
            begruendung: self.begruendung,
            entscheidung_offen: self.entscheidung_offen,
            opens: self.opens,
            open_clear: self.open_clear,
            wall_ok: self.wall_ok,
            expands_dir: self.expands.as_ref().map(|e| e.dir),
            expands_to: self.expands.as_ref().map(|e| e.to),
            access_sides: self.access_sides,
            access_clear: self.access_clear,
            raumtrenner: self.raumtrenner,
            zerlegbar: self.zerlegbar,
            bild: self.bild,
            // Die sieben Ausruestungsfelder bleiben leer, weil diese Quelle keine haelt:
            // `inventory/*.toml` beschreibt Moebel. Gepaeck kommt ueber `trips gear import`
            // herein, gegen dieselbe Tabelle (PRD Q92 / B51).
            weight_g: None,
            category: None,
            packable: None,
            waterproof: None,
            quick_dry: None,
            pack_location: None,
            trip_types: Vec::new(),
            // Kleidung kommt ueber `interior wunsch` und das Formular herein, nicht aus diesen
            // Dateien: `inventory/*.toml` beschreibt Moebel. Ein `groesse` in einer dieser
            // Dateien ist deshalb ein FEHLER und keine stille Verwerfung — die Richtung, die
            // der Kopf dieser Datei begruendet.
            groesse: None,
            farbe: None,
            saison: Vec::new(),
            // Der Server vergibt sie beim Schreiben; ein Wert hier wuerde nie geschrieben.
            revision: 0,
        }
    }
}

/// Was ein Lauf getan hat. Zahlen, damit ein Import nicht "ok" meldet und nichts geschrieben hat.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Bericht {
    pub gear: usize,
    pub pieces: usize,
    pub slots: usize,
    pub zustandswechsel: usize,
}

/// Ein Bedarf, den ein anderer Eintrag abgeloest hat, ist kein Wunsch mehr. Das ist die einzige
/// Stelle, an der aus `status` ein Zustand wird, und sie steht hier statt in der Datei, weil
/// `status` Dringlichkeit meint und `state` Besitz — zwei Fragen, ein Wort, getrennt beim Import.
fn state_for(prioritaet: Option<&str>, vorgabe: State) -> State {
    match prioritaet {
        Some("ersetzt") => State::Gone,
        _ => vorgabe,
    }
}

/// Liest `<dir>/owned.toml` und `<dir>/wishlist.toml` und schreibt sie in die Tabellen.
///
/// Wiederholbar: dieselben Dateien zweimal importiert ergeben dieselben Zeilen und **keinen
/// zweiten Zustandswechsel** — sonst waere die Zustandsgeschichte ein Protokoll der Importe
/// statt eines Protokolls der Wirklichkeit.
pub fn inventory(store: &Store, dir: &Path) -> Result<Bericht, Fehler> {
    let owned: OwnedFile = read(&dir.join("owned.toml"))?;
    let wish: WishlistFile = read(&dir.join("wishlist.toml"))?;
    let mut b = Bericht::default();

    for (roh, kind, vorgabe) in owned
        .item
        .into_iter()
        .map(|r| (r, Kind::Piece, State::Owned))
        .chain(
            wish.slot
                .into_iter()
                .map(|r| (r, Kind::Slot, State::Wanted)),
        )
        .chain(
            wish.produkt
                .into_iter()
                .map(|r| (r, Kind::Piece, State::Wanted)),
        )
    {
        let item = roh.into_item(kind);
        let state = state_for(item.prioritaet.as_deref(), vorgabe);
        store.upsert_item(&item)?;
        if store.record_state(&item.id, state, Some("aus inventory/*.toml importiert"))? {
            b.zustandswechsel += 1;
        }
        match kind {
            Kind::Piece => b.pieces += 1,
            Kind::Slot => b.slots += 1,
            // Unerreichbar: die Schleife oben kennt nur `Kind::Slot` und `Kind::Piece`. Der
            // Zweig steht hier, damit ein spaeterer Importweg fuer Ausruestung eine Zahl
            // bekommt, statt still in `pieces` zu landen.
            Kind::Gear => b.gear += 1,
        }
    }
    Ok(b)
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Fehler> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}
