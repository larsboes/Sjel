//! Das Inventar als Zeilen, in der einen geteilten SQLite-Datei.
//!
//! PRD Q58 (2026-08-30): **eine Item-Tabelle, zwei Konsumenten.** Ein Zelt und ein
//! Kleiderschrank sind dieselbe Zeilenform — ein Ding mit Massen, einem Preis, einer Herkunft
//! und einem Zustand. Was sich unterscheidet, ist die Platzierung, und die ist eine zweite
//! Tabelle, keine zweite Kopie. Der Tabellenname `interior_item` ist deshalb heute schon
//! falsch und die Tabelle richtig: wenn Ausruestung dazukommt, wird umbenannt, nicht geforkt.
//!
//! ## Was hier liegt und was nicht
//!
//! Q60 zieht die Grenze bei *hat die Zahl eine Begruendung, die mitwandern muss*. Ein Moebel
//! ist eine Zeile: Masse, Preis, welche Seite sich oeffnet. Ein Raum ist keine —
//! `room.toml` traegt datierte Korrekturkommentare, und drei davon sind das Protokoll eines
//! Bugs, der einen falschen Plan erzeugt hat. Dafuer hat eine Tabelle keine Spalte.
//!
//! Geometrie bleibt also Datei. Das Inventar wird Zeile.
//!
//! ## Drei Tabellen, und warum der Zustand eine eigene ist
//!
//! Ein Wunsch, der gekauft wird, und ein Moebel, das weggegeben wird, sind **zwei Zeilen und
//! nicht eine ueberschriebene**. Ein `state`-Feld auf dem Item wuerde beim ersten Kauf
//! vergessen, wann etwas ein Wunsch war — und genau diese Spanne ist das, was die Wunschliste
//! spaeter mit `finance` verbindet.

use crate::model::Seite;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Jede Spalte von `{prefix}_item`, in der Reihenfolge, in der `widen_kind_check` sie neu
/// anlegt. Ausgeschrieben, damit ein spaeteres Feld beim Umbau auffaellt: `SELECT *` haette
/// die Zeilen still in die falschen Spalten kopiert, sobald sich eine Reihenfolge aendert.
const ITEM_COLUMNS: [&str; 50] = [
    "id",
    "kind",
    "label",
    "b",
    "t",
    "h",
    "h_min",
    "b_aufgeklappt",
    "t_ausgeklappt",
    "laenge",
    "anzahl",
    "zustaende",
    "unsicher",
    "platzbedarf_zone",
    "platzbedarf_block",
    "preis_cent",
    "kosten_min_cent",
    "kosten_max_cent",
    "link",
    "artikelnummer",
    "quelle",
    "gemessen_am",
    "mitnahme",
    "prioritaet",
    "basiert_auf",
    "ersetzt",
    "varianten",
    "ziel",
    "hinweis",
    "begruendung",
    "entscheidung_offen",
    "opens",
    "open_clear",
    "wall_ok",
    "expands_dir",
    "expands_to",
    "access_sides",
    "access_clear",
    "raumtrenner",
    "zerlegbar",
    "bild",
    "weight_g",
    "category",
    "packable",
    "waterproof",
    "quick_dry",
    "pack_location",
    "trip_types",
    "created_at",
    "updated_at",
];

/// Alles, was ein Ding ausmacht — ob es schon da ist oder erst gewuenscht.
///
/// Die Feldnamen folgen den TOML-Dateien, aus denen die Zeilen stammen, statt sie zu
/// uebersetzen: ein zweites Vokabular fuer dieselbe Sache ist die teure Version dieses Fehlers.
/// Die vier Listenfelder tragen `#[serde(default)]`, `id`, `kind` und `label` nicht.
///
/// Das ist der Vertrag fuer `POST /api/items`: sag, was es ist und wie es heisst, alles andere
/// ist freiwillig. Ein leeres `zustaende` von einem Formular zu verlangen waere Schikane; ein
/// Stueck ohne Namen anzulegen waere eine Zeile, die niemand wiederfindet.
///
/// Betrifft den Import NICHT — der liest `import::Roh` mit `deny_unknown_fields`, und dort ist
/// jedes Feld weiterhin genau so streng wie seit B25.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    /// `piece` = ein Ding (besessen oder ein konkretes Produkt). `slot` = ein BEDARF mit
    /// Zielmassen und noch ohne Produkt. Der Unterschied ueberlebt den Import, weil er der
    /// Grund ist, warum eine Wunschliste mehr ist als eine Einkaufsliste.
    pub kind: Kind,
    pub label: String,
    pub b: Option<i32>,
    pub t: Option<i32>,
    pub h: Option<i32>,
    pub h_min: Option<i32>,
    pub b_aufgeklappt: Option<i32>,
    pub t_ausgeklappt: Option<i32>,
    pub laenge: Option<i32>,
    pub anzahl: Option<i32>,
    /// Benannte Zustaende eines Klappmoebels, z. B. `["zu", "ausgeklappt"]`.
    #[serde(default)]
    pub zustaende: Vec<String>,
    /// Welche Masse geschaetzt sind. Wandert unveraendert in jeden Pruefbericht.
    #[serde(default)]
    pub unsicher: Vec<String>,
    pub platzbedarf_zone: Option<i32>,
    pub platzbedarf_block: Option<i32>,
    /// In Cent, nicht Euro. `finance` rechnet in Cent, und die Wunschliste trifft dort auf ein
    /// Budget (PRD B29) — zwei Einheiten fuer denselben Betrag ist die Naht, an der das reisst.
    pub preis_cent: Option<i64>,
    pub kosten_min_cent: Option<i64>,
    pub kosten_max_cent: Option<i64>,
    pub link: Option<String>,
    pub artikelnummer: Option<String>,
    pub quelle: Option<String>,
    pub gemessen_am: Option<String>,
    /// `bring` / `weg` / … — was beim Umzug mit diesem Stueck passiert.
    pub mitnahme: Option<String>,
    /// Dringlichkeit eines Bedarfs: `pflicht`, `empfehlung`, `konzept`, `ersetzt`.
    pub prioritaet: Option<String>,
    /// Ein Slot, der aus einem anderen Eintrag hervorgeht.
    pub basiert_auf: Option<String>,
    /// Welche Eintraege dieses Produkt ueberfluessig macht. Eine Beziehung, als Liste
    /// gehalten und nicht als Tabelle, weil sie hoechstens zwei Eintraege lang ist und
    /// nichts darauf joint. Sobald B29 die Wunschliste auf das Budget trifft und daraus ein
    /// Join wird, wird es eine Tabelle.
    #[serde(default)]
    pub ersetzt: Vec<String>,
    #[serde(default)]
    pub varianten: Vec<String>,
    pub ziel: Option<String>,
    pub hinweis: Option<String>,
    pub begruendung: Option<String>,
    pub entscheidung_offen: Option<String>,

    // --- Was dieses Stueck an Platz verlangt (PRD Q61 / B26) ---
    //
    // Bis 2026-08-31 riet die Pruefung aus dem Namen: `bett*` war ein Bett, `schrank*` ein
    // Schrank, und mit der Vermutung kam jede Schwelle mit. Das ist einmal teuer danebengegangen
    // — `^couch` fing `couchtisch`, also wurde ein Couchtisch gegen die Regeln eines Sofas
    // geprueft, und gefunden wurde es erst, als ein echter Esstisch dazukam.
    //
    // Diese Felder sind der Ersatz. Sie stehen am STUECK, nicht an der Wohnung: `open_clear`
    // ist am eigenen Schrank gemessen und schlaegt die Faustregel, und wie viele Seiten ein
    // Bett braucht, ist eine Aussage ueber die Nutzung. Ein Feld, das ein Ding beschreibt, das
    // mir gehoert, hat nichts in einer Datei zu suchen, die eine Wohnung beschreibt, die ich
    // miete.
    //
    // Alle optional, und leer heisst: der Name entscheidet weiter. Ein Umschalten am selben Tag
    // fuer alle 42 Zeilen waere ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand
    // die Zahlen dahinter geprueft hat.
    /// Welche Seite Tueren oder Schubladen braucht, in der Ausrichtung des Stuecks selbst.
    pub opens: Option<Seite>,
    /// Wie viel davor frei bleiben muss, in cm. Ohne `opens` gilt es fuer die beste Seite.
    pub open_clear: Option<i32>,
    /// Darf die sich oeffnende Seite an einer Wand liegen. `false` heisst: dort ist sie nutzlos.
    pub wall_ok: Option<bool>,
    /// Zweiter Zustand: Schlafsofa, Klapptisch. `dir` ist die Seite, `to` die Gesamttiefe
    /// AUSGEKLAPPT — nicht der Zuwachs, weil die Produktseite die Gesamttiefe nennt.
    pub expands_dir: Option<Seite>,
    pub expands_to: Option<i32>,
    /// Wie viele Seiten begehbar sein muessen. Ein Bett fuer eine Person braucht eine.
    pub access_sides: Option<i32>,
    /// Wie tief eine solche Seite sein muss, in cm.
    pub access_clear: Option<i32>,
    /// Ein Bild zu diesem Stueck, als Pfad UNTERHALB des privaten Asset-Verzeichnisses.
    ///
    /// Nur der relative Pfad, nie ein absoluter: die Zeilen liegen in der geteilten Datenbank
    /// und beschreiben ein Moebel, nicht diese Maschine. Wo `media/` liegt, erklaert die
    /// private Interior-Konfiguration.
    ///
    /// Ausgeliefert wird es ueber `GET /api/media/{*pfad}` und nur auf Anfrage — service.toml
    /// nennt genau das als Grund, warum diese Capability oeffentlich stehen darf: im Bundle
    /// steckt kein Foto.
    pub bild: Option<String>,
    /// Dieses Stueck soll frei im Raum stehen und teilt ihn.
    ///
    /// Betrifft nur die **Rangfolge** der Suche, nie ein Verdikt: `search::wandkontakt_cm`
    /// belohnt eine Wand im Ruecken, weil der Raeumungspruefer einen Esstisch mitten im Raum
    /// fuer genauso richtig haelt wie einen an der Wand. Ein Regal quer im Raum bekam damit 0 cm
    /// und sank — bestraft dafuer, dass es seine Aufgabe erfuellt. Gesetzt faellt es aus der
    /// Wandsumme heraus.
    ///
    /// Wie jedes Feld aus PRD Q61 ist es eine Aussage ueber das Stueck und nicht ueber die
    /// Wohnung, und wie sie alle ist es freiwillig: wer nichts erklaert, wird weiter an der
    /// Wand gemessen.
    pub raumtrenner: Option<bool>,
    /// Kommt zerlegt an oder laesst sich zum Tragen zerlegen.
    ///
    /// Betrifft genau eine Frage: `einbringung::durch_die_tuer`. Ein 140 cm breites Bett
    /// passt nicht durch eine 100 cm breite Tuer und steht trotzdem in jedem Schlafzimmer —
    /// es kommt in Teilen herein. Ohne diese Zeile meldet die Pruefung dort einen Verstoss,
    /// der keiner ist, und wird deshalb nach dem dritten Mal nicht mehr gelesen.
    ///
    /// Freiwillig wie jedes Feld aus PRD Q61: wer nichts sagt, wird als ein Stueck getragen.
    /// Das ist die vorsichtige Richtung — eine falsche Warnung kostet ein Nachdenken, eine
    /// fehlende kostet den Schrank.
    pub zerlegbar: Option<bool>,

    // --- Ausruestung (PRD Q92 / B51) ---
    //
    // Sieben Felder, die nur ein `kind = "gear"` je fuellt. Sie stehen hier und nicht in einer
    // zweiten Tabelle, weil Q58 genau diese Frage schon entschieden hat: eine Gegenstandstabelle
    // fuer Moebel UND Ausruestung. Die Packliste in `trips` bindet gegen diese Spalten; ohne sie
    // wurde jedes Gewicht als null mit einem Grund ausgeliefert, und die Haelfte wurde vor dem
    // Merge zurueckgenommen (815750c) statt eine halbe Form in die Datei zu schreiben.
    //
    // Englisch benannt wie `opens`, `wall_ok` und `access_sides`: es ist das Vokabular, in dem
    // die Entscheidung getroffen wurde, und ein zweites fuer dieselbe Sache ist der teure Fehler,
    // vor dem der Kopf dieser Datei warnt.
    /// Gewicht in Gramm. Gramm und nicht Kilo, aus demselben Grund, aus dem Preise in Cent
    /// stehen: eine Packliste addiert, und eine Kommazahl addiert sich falsch.
    pub weight_g: Option<i64>,
    /// Wozu das Stueck gehoert — `schlafen`, `kochen`, `kleidung`, `elektronik`. Frei, weil die
    /// Liste aus den Notizen kommt und keine Regel darauf laeuft.
    pub category: Option<String>,
    /// Laesst sich klein zusammenlegen. Eine Aussage ueber das Stueck, keine ueber die Reise.
    pub packable: Option<bool>,
    pub waterproof: Option<bool>,
    pub quick_dry: Option<bool>,
    /// Wo es im Gepaeck liegt — `rucksack`, `koffer`, `am koerper`. Ordnet die gedruckte Liste.
    pub pack_location: Option<String>,
    /// Fuer welche Reisearten es in Frage kommt, z. B. `["hiking", "city"]`. Leer heisst: fuer
    /// jede. Liste und keine Tabelle, aus demselben Grund wie `ersetzt` daneben.
    #[serde(default)]
    pub trip_types: Vec<String>,

    /// Wie oft diese Zeile geschrieben wurde; bestehende Zeilen beginnen bei 1 (PRD §10 A5).
    ///
    /// Gehoert dem Server. Jedes Schreiben erhoeht sie im selben Statement, und ein Wert im
    /// Rumpf von `PUT`/`PATCH` wird nie geschrieben. Wer sie als Bedingung schicken will,
    /// schickt sie als `If-Match` — siehe README, Abschnitt Gleichzeitiges Bearbeiten.
    #[serde(default)]
    pub revision: i64,
}

impl Item {
    /// Wahr, wenn irgendein Mass geraten ist. Das Flag wandert in jeden Bericht, der auf
    /// diesem Moebel beruht — eine Schaetzung, die unterwegs zur Messung wird, ist der
    /// Fehler, gegen den dieses Feld existiert.
    pub fn is_uncertain(&self) -> bool {
        !self.unsicher.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Piece,
    Slot,
    /// Ausruestung: was mitreist statt zu stehen. Dieselbe Tabelle nach Q58, weil ein Zelt und
    /// ein Regal dieselben Fragen beantworten — was ist es, wie schwer, wem gehoert es.
    Gear,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Piece => "piece",
            Kind::Slot => "slot",
            Kind::Gear => "gear",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "slot" => Kind::Slot,
            "gear" => Kind::Gear,
            // Alles andere ist ein Stueck. Ein unbekanntes Wort still zu Ausruestung zu machen
            // waere die teurere Vermutung: es faellt dann aus jeder Moebelpruefung heraus.
            _ => Kind::Piece,
        }
    }
}

/// Besitzt er es, will er es, oder ist es weg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Owned,
    Wanted,
    Gone,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Owned => "owned",
            State::Wanted => "wanted",
            State::Gone => "gone",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        match s {
            "owned" => Some(State::Owned),
            "wanted" => Some(State::Wanted),
            "gone" => Some(State::Gone),
            _ => None,
        }
    }
}

/// Wo ein Stueck in einer Wohnung tatsaechlich steht.
///
/// **Nicht dasselbe wie ein Layout.** `flats/<id>/layouts/*.toml` sind Vorschlaege, ueber die
/// argumentiert wird — sie tragen datierte Begruendungen und gehoeren nach Q60 in Dateien.
/// Diese Tabelle haelt, wo etwas nach dem Einzug wirklich steht: eine Tatsache ohne Argument,
/// und damit eine Zeile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Placement {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
}

/// Die Spalten, die ein Schreiben aus einem `Item` setzt, in der Reihenfolge der Parameter
/// `?1` bis `?48` von [`write_params`]. `id` steht vorn und wird nie ueberschrieben.
///
/// Nicht dabei: `revision`, `created_at`, `updated_at`. Die drei setzt der Server, nie der
/// Rumpf — ein Client, der seine gelesene Revision zurueckschickt, darf damit nichts setzen.
const WRITE_COLUMNS: [&str; 48] = [
    "id",
    "kind",
    "label",
    "b",
    "t",
    "h",
    "h_min",
    "b_aufgeklappt",
    "t_ausgeklappt",
    "laenge",
    "anzahl",
    "zustaende",
    "unsicher",
    "platzbedarf_zone",
    "platzbedarf_block",
    "preis_cent",
    "kosten_min_cent",
    "kosten_max_cent",
    "link",
    "artikelnummer",
    "quelle",
    "gemessen_am",
    "mitnahme",
    "prioritaet",
    "basiert_auf",
    "ersetzt",
    "varianten",
    "ziel",
    "hinweis",
    "begruendung",
    "entscheidung_offen",
    "opens",
    "open_clear",
    "wall_ok",
    "expands_dir",
    "expands_to",
    "access_sides",
    "access_clear",
    "raumtrenner",
    "bild",
    "zerlegbar",
    "weight_g",
    "category",
    "packable",
    "waterproof",
    "quick_dry",
    "pack_location",
    "trip_types",
];

/// Die Werte zu [`WRITE_COLUMNS`], Stelle fuer Stelle.
fn write_params(it: &Item) -> Result<Vec<Box<dyn rusqlite::ToSql>>, Fehler> {
    Ok(vec![
        Box::new(it.id.clone()),
        Box::new(it.kind.as_str()),
        Box::new(it.label.clone()),
        Box::new(it.b),
        Box::new(it.t),
        Box::new(it.h),
        Box::new(it.h_min),
        Box::new(it.b_aufgeklappt),
        Box::new(it.t_ausgeklappt),
        Box::new(it.laenge),
        Box::new(it.anzahl),
        Box::new(serde_json::to_string(&it.zustaende)?),
        Box::new(serde_json::to_string(&it.unsicher)?),
        Box::new(it.platzbedarf_zone),
        Box::new(it.platzbedarf_block),
        Box::new(it.preis_cent),
        Box::new(it.kosten_min_cent),
        Box::new(it.kosten_max_cent),
        Box::new(it.link.clone()),
        Box::new(it.artikelnummer.clone()),
        Box::new(it.quelle.clone()),
        Box::new(it.gemessen_am.clone()),
        Box::new(it.mitnahme.clone()),
        Box::new(it.prioritaet.clone()),
        Box::new(it.basiert_auf.clone()),
        Box::new(serde_json::to_string(&it.ersetzt)?),
        Box::new(serde_json::to_string(&it.varianten)?),
        Box::new(it.ziel.clone()),
        Box::new(it.hinweis.clone()),
        Box::new(it.begruendung.clone()),
        Box::new(it.entscheidung_offen.clone()),
        Box::new(it.opens.map(|s| s.as_str())),
        Box::new(it.open_clear),
        Box::new(it.wall_ok),
        Box::new(it.expands_dir.map(|s| s.as_str())),
        Box::new(it.expands_to),
        Box::new(it.access_sides),
        Box::new(it.access_clear),
        Box::new(it.raumtrenner),
        Box::new(it.bild.clone()),
        Box::new(it.zerlegbar),
        Box::new(it.weight_g),
        Box::new(it.category.clone()),
        Box::new(it.packable),
        Box::new(it.waterproof),
        Box::new(it.quick_dry),
        Box::new(it.pack_location.clone()),
        Box::new(serde_json::to_string(&it.trip_types)?),
    ])
}

/// Was ein Schreiben mit erwarteter Revision ergab.
#[derive(Debug)]
pub enum Schreibergebnis {
    /// Geschrieben; die Zeile traegt jetzt diese Revision.
    Geschrieben(i64),
    /// Den Eintrag gibt es nicht.
    Fehlt,
    /// Jemand hat inzwischen geschrieben. Der aktuelle Stand, damit der Aufrufer ihn zeigen
    /// kann statt ihn zu ueberschreiben.
    Veraltet(Box<Item>, Option<State>),
}

pub struct Store {
    pool: sjel_store::Pool,
    prefix: String,
}

type Fehler = Box<dyn std::error::Error>;

/// Der Praefix wird in DDL und jedes Statement interpoliert, also wird er geprueft statt
/// gebunden — dieselbe Vorsichtsmassnahme wie in `capabilities/trips/src/store.rs`.
fn validate_prefix(prefix: &str) -> Result<(), Fehler> {
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || prefix.is_empty()
    {
        return Err("prefix must contain only ASCII letters, digits, or underscore".into());
    }
    Ok(())
}

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Fehler> {
        Self::open_with_prefix(database_path, "interior")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Result<Self, Fehler> {
        validate_prefix(prefix)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    fn conn(&self) -> Result<sjel_store::PooledClient, Fehler> {
        Ok(self.pool.get()?)
    }

    /// Eine Verbindung fuer eine Abfrage, die ueber den eigenen Praefix hinausgeht.
    ///
    /// Genau ein Konsument: `budget::monatssaldo` liest `finance_transaction_projection`. Das
    /// ist erlaubt und der Grund fuer die eine geteilte Datei; es ist nur nichts, das
    /// versehentlich passieren soll, deshalb hat es einen eigenen, benannten Weg.
    pub fn borrow_connection(&self) -> Result<sjel_store::PooledClient, Fehler> {
        self.conn()
    }

    /// Die Tabellen, wie sie sind — nicht die Geschichte, die zu ihnen gefuehrt hat. Die Datei
    /// beginnt leer, also gibt es keine ALTER-Kette zu bewahren (libs/sjel-store/README.md).
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Fehler> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_item (
                id                 TEXT PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN ('piece','slot','gear')),
                label              TEXT NOT NULL,
                b                  INTEGER,
                t                  INTEGER,
                h                  INTEGER,
                h_min              INTEGER,
                b_aufgeklappt      INTEGER,
                t_ausgeklappt      INTEGER,
                laenge             INTEGER,
                anzahl             INTEGER,
                zustaende          TEXT NOT NULL DEFAULT '[]',
                unsicher           TEXT NOT NULL DEFAULT '[]',
                platzbedarf_zone   INTEGER,
                platzbedarf_block  INTEGER,
                preis_cent         INTEGER,
                kosten_min_cent    INTEGER,
                kosten_max_cent    INTEGER,
                link               TEXT,
                artikelnummer      TEXT,
                quelle             TEXT,
                gemessen_am        TEXT,
                mitnahme           TEXT,
                prioritaet         TEXT,
                basiert_auf        TEXT,
                ersetzt            TEXT NOT NULL DEFAULT '[]',
                varianten          TEXT NOT NULL DEFAULT '[]',
                ziel               TEXT,
                hinweis            TEXT,
                begruendung        TEXT,
                entscheidung_offen TEXT,
                opens              TEXT,
                open_clear         INTEGER,
                wall_ok            INTEGER,
                expands_dir        TEXT,
                expands_to         INTEGER,
                access_sides       INTEGER,
                access_clear       INTEGER,
                raumtrenner        INTEGER,
                zerlegbar          INTEGER,
                bild               TEXT,
                weight_g           INTEGER,
                category           TEXT,
                packable           INTEGER,
                waterproof         INTEGER,
                quick_dry          INTEGER,
                pack_location      TEXT,
                trip_types         TEXT NOT NULL DEFAULT '[]',
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL,
                revision           INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS {prefix}_item_state (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
                state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
                since   TEXT NOT NULL,
                note    TEXT
            );
            CREATE TABLE IF NOT EXISTS {prefix}_placement (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
                flat    TEXT NOT NULL,
                x       INTEGER NOT NULL,
                y       INTEGER NOT NULL,
                rot     INTEGER NOT NULL DEFAULT 0,
                since   TEXT NOT NULL,
                UNIQUE (item_id, flat)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_state_item
                ON {prefix}_item_state(item_id, since DESC);
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_flat
                ON {prefix}_placement(flat);
            CREATE TABLE IF NOT EXISTS {prefix}_sync_operation (
                operation_id TEXT PRIMARY KEY,
                revision INTEGER NOT NULL,
                processed_at TEXT NOT NULL
            );
            ",
            prefix = prefix
        ))?;
        Self::add_column_if_missing(conn, prefix, "raumtrenner", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "zerlegbar", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "bild", "TEXT")?;
        // Ausruestung (B51). Die sieben Spalten zuerst, dann der CHECK — der Umbau kopiert sie
        // mit, also muessen sie vorher da sein.
        Self::add_column_if_missing(conn, prefix, "weight_g", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "category", "TEXT")?;
        Self::add_column_if_missing(conn, prefix, "packable", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "waterproof", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "quick_dry", "INTEGER")?;
        Self::add_column_if_missing(conn, prefix, "pack_location", "TEXT")?;
        Self::add_column_if_missing(conn, prefix, "trip_types", "TEXT NOT NULL DEFAULT '[]'")?;
        Self::widen_kind_check(conn, prefix)?;
        // Nach dem Umbau und nicht davor: `widen_kind_check` kopiert nur `ITEM_COLUMNS`, und
        // eine Datei, die den Umbau noch braucht, hat diese Spalte ohnehin nicht. `DEFAULT 1`
        // gibt jeder bestehenden Zeile die Revision 1 (PRD §10 A5).
        Self::add_column_if_missing(conn, prefix, "revision", "INTEGER NOT NULL DEFAULT 1")?;
        Ok(())
    }

    /// `ALTER TABLE ... ADD COLUMN`, idempotent, weil SQLite kein `IF NOT EXISTS` dafuer hat.
    ///
    /// Der Kommentar ueber `run_migration` sagt, eine Datei beginne leer und es gebe keine
    /// ALTER-Kette zu bewahren. Das galt bis 2026-08-31 und gilt seit B25 nicht mehr: die
    /// Tabelle traegt importierte Zeilen, und `CREATE TABLE IF NOT EXISTS` erreicht eine
    /// bestehende Tabelle nicht. Die Zeilen einfach neu zu importieren waere kein Ausweg —
    /// `{prefix}_item_state` haelt die Zustandsgeschichte, und die ist der Grund, aus dem
    /// B25 sie als eigene Tabelle angelegt hat.
    ///
    /// `pragma_table_info` ist die Sonde, die `capabilities/places/src/backfill.rs:396` fuer
    /// dieselbe Frage benutzt.
    fn add_column_if_missing(
        conn: &Connection,
        prefix: &str,
        column: &str,
        typ: &str,
    ) -> Result<(), Fehler> {
        let vorhanden: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![format!("{prefix}_item"), column],
            |row| row.get(0),
        )?;
        if vorhanden == 0 {
            conn.execute_batch(&format!(
                "ALTER TABLE {prefix}_item ADD COLUMN {column} {typ};"
            ))?;
        }
        Ok(())
    }

    /// Den `kind`-CHECK auf `gear` erweitern, indem die Tabelle neu gebaut wird.
    ///
    /// SQLite haelt einen CHECK im gespeicherten DDL-Text; `ALTER TABLE` kann ihn nicht
    /// anfassen. Der Zwoelf-Schritte-Weg aus der SQLite-Dokumentation ist der einzige: neue
    /// Tabelle, Zeilen kopieren, alte fallen lassen, umbenennen. Er laeuft nur, wenn der
    /// gespeicherte Text `gear` noch nicht nennt — eine leere oder frische Datei hat den
    /// weiten CHECK schon aus `CREATE TABLE` oben, und ein zweiter Lauf faende nichts zu tun.
    ///
    /// **Die Kinder werden gesichert und zurueckgeschrieben, und das ist der Kern.**
    /// `{prefix}_item_state` und `{prefix}_placement` zeigen mit `ON DELETE CASCADE` auf
    /// `{prefix}_item(id)`; ein DROP der Elterntabelle loescht sie deshalb mit. Der uebliche
    /// Ausweg — `PRAGMA foreign_keys = off` vor dem `BEGIN` — steht hier nicht offen: dieser
    /// Code laeuft in der Transaktion, die `sjel_store::migrate_once` schon geoeffnet hat, und
    /// die Pragma ist innerhalb einer Transaktion wirkungslos (libs/sjel-store/src/lib.rs:328).
    /// Also werden beide Tabellen vorher kopiert und hinterher wieder gefuellt, im selben
    /// Umlauf: faellt irgendetwas davon aus, nimmt der Rollback alles mit.
    ///
    /// Die Spaltenliste ist ausgeschrieben und nicht `SELECT *`, damit ein spaeteres Feld hier
    /// auffaellt statt still in der falschen Spalte zu landen.
    fn widen_kind_check(conn: &Connection, prefix: &str) -> Result<(), Fehler> {
        let ddl: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![format!("{prefix}_item")],
                |row| row.get(0),
            )
            .optional()?;
        let Some(ddl) = ddl else { return Ok(()) };
        if ddl.contains("'gear'") {
            return Ok(());
        }
        let columns = ITEM_COLUMNS.join(", ");
        conn.execute_batch(&format!(
            "CREATE TABLE {prefix}_item_neu (
                id                 TEXT PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN ('piece','slot','gear')),
                label              TEXT NOT NULL,
                b                  INTEGER,
                t                  INTEGER,
                h                  INTEGER,
                h_min              INTEGER,
                b_aufgeklappt      INTEGER,
                t_ausgeklappt      INTEGER,
                laenge             INTEGER,
                anzahl             INTEGER,
                zustaende          TEXT NOT NULL DEFAULT '[]',
                unsicher           TEXT NOT NULL DEFAULT '[]',
                platzbedarf_zone   INTEGER,
                platzbedarf_block  INTEGER,
                preis_cent         INTEGER,
                kosten_min_cent    INTEGER,
                kosten_max_cent    INTEGER,
                link               TEXT,
                artikelnummer      TEXT,
                quelle             TEXT,
                gemessen_am        TEXT,
                mitnahme           TEXT,
                prioritaet         TEXT,
                basiert_auf        TEXT,
                ersetzt            TEXT NOT NULL DEFAULT '[]',
                varianten          TEXT NOT NULL DEFAULT '[]',
                ziel               TEXT,
                hinweis            TEXT,
                begruendung        TEXT,
                entscheidung_offen TEXT,
                opens              TEXT,
                open_clear         INTEGER,
                wall_ok            INTEGER,
                expands_dir        TEXT,
                expands_to         INTEGER,
                access_sides       INTEGER,
                access_clear       INTEGER,
                raumtrenner        INTEGER,
                zerlegbar          INTEGER,
                bild               TEXT,
                weight_g           INTEGER,
                category           TEXT,
                packable           INTEGER,
                waterproof         INTEGER,
                quick_dry          INTEGER,
                pack_location      TEXT,
                trip_types         TEXT NOT NULL DEFAULT '[]',
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
             );
             INSERT INTO {prefix}_item_neu ({columns})
                 SELECT {columns} FROM {prefix}_item;
             CREATE TABLE {prefix}_state_sicherung AS SELECT * FROM {prefix}_item_state;
             CREATE TABLE {prefix}_placement_sicherung AS SELECT * FROM {prefix}_placement;
             DROP TABLE {prefix}_item;
             ALTER TABLE {prefix}_item_neu RENAME TO {prefix}_item;
             INSERT INTO {prefix}_item_state SELECT * FROM {prefix}_state_sicherung;
             INSERT INTO {prefix}_placement SELECT * FROM {prefix}_placement_sicherung;
             DROP TABLE {prefix}_state_sicherung;
             DROP TABLE {prefix}_placement_sicherung;"
        ))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Fehler> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// Anlegen oder aktualisieren. `created_at` ueberlebt ein Update — wann eine Zeile
    /// entstanden ist, ist eine andere Tatsache als wann sie zuletzt stimmte.
    ///
    /// Bedingungslos: der spaetere Schreiber gewinnt. Das ist der Weg fuer den Import und fuer
    /// jeden Aufrufer, der keine Revision nennt. Die Revision steigt trotzdem, im selben
    /// Statement, damit ein Client mit einer aelteren Revision danach sicher abgewiesen wird.
    /// Zurueck kommt die Revision, die die Zeile jetzt traegt.
    pub fn upsert_item(&self, it: &Item) -> Result<i64, Fehler> {
        let p = &self.prefix;
        let now = sjel_store::now_offset("'+0 seconds'");
        let spalten = WRITE_COLUMNS.join(", ");
        let platzhalter = (1..=WRITE_COLUMNS.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let uebernehmen = WRITE_COLUMNS[1..]
            .iter()
            .map(|c| format!("{c}=excluded.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let conn = self.conn()?;
        let revision = conn.query_row(
            &format!(
                "INSERT INTO {p}_item ({spalten}, revision, created_at, updated_at)
                 VALUES ({platzhalter}, 1, {now}, {now})
                 ON CONFLICT(id) DO UPDATE SET {uebernehmen},
                    revision = {p}_item.revision + 1, updated_at = {now}
                 RETURNING revision"
            ),
            rusqlite::params_from_iter(write_params(it)?.iter()),
            |row| row.get(0),
        )?;
        Ok(revision)
    }

    /// Ueberschreiben, aber nur, wenn die Zeile noch die Revision traegt, die der Aufrufer
    /// gelesen hat (PRD §10 A5, Q110: Telefon und Mac bearbeiten dieselben Eintraege).
    ///
    /// Vergleich und Schreiben sind EIN Statement: `UPDATE ... WHERE id = ? AND revision = ?`.
    /// Ein Lesen vorher und ein Schreiben danach waere genau das Fenster, in dem der zweite
    /// Schreiber still gewinnt. Ein einzelnes Statement im Autocommit nimmt die Schreibsperre
    /// schon beim Start, bevor es liest; das Upgrade-Problem der verzoegerten Transaktion
    /// (`sjel_store::write_transaction`, PRD 0.19) entsteht erst mit einem zweiten Statement
    /// davor und tritt hier nicht auf.
    ///
    /// Trifft das Statement keine Zeile, sagt erst das Lesen danach, warum: gibt es den Eintrag
    /// nicht, oder traegt er eine andere Revision. Dieses Lesen schreibt nichts und braucht
    /// deshalb keine Transaktion; es liefert den Stand, den der Aufrufer sehen muss.
    pub fn update_item_if_revision(
        &self,
        it: &Item,
        erwartet: i64,
    ) -> Result<Schreibergebnis, Fehler> {
        let p = &self.prefix;
        let now = sjel_store::now_offset("'+0 seconds'");
        let setzen = WRITE_COLUMNS
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, c)| format!("{c} = ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut werte = write_params(it)?;
        werte.push(Box::new(erwartet));
        let conn = self.conn()?;
        let neu: Option<i64> = conn
            .query_row(
                &format!(
                    "UPDATE {p}_item SET {setzen}, revision = revision + 1, updated_at = {now}
                     WHERE id = ?1 AND revision = ?{n}
                     RETURNING revision",
                    n = WRITE_COLUMNS.len() + 1
                ),
                rusqlite::params_from_iter(werte.iter()),
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        if let Some(revision) = neu {
            return Ok(Schreibergebnis::Geschrieben(revision));
        }
        Ok(match self.item(&it.id)? {
            Some((aktuell, zustand)) => Schreibergebnis::Veraltet(Box::new(aktuell), zustand),
            None => Schreibergebnis::Fehlt,
        })
    }

    /// Einen Zustandswechsel festhalten. Schreibt NICHT, wenn der aktuelle Zustand schon
    /// derselbe ist: ein wiederholter Import darf keine Geschichte erfinden.
    pub fn record_state(
        &self,
        item_id: &str,
        state: State,
        note: Option<&str>,
    ) -> Result<bool, Fehler> {
        if self.current_state(item_id)? == Some(state) {
            return Ok(false);
        }
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_item_state (item_id, state, since, note)
                 VALUES (?1, ?2, {now}, ?3)",
                p = p,
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![item_id, state.as_str(), note],
        )?;
        Ok(true)
    }

    /// Wie viele Eintraege es gibt. Die Frage, an der `interior import` entscheidet, ob es
    /// eine Migration ist oder ein Ueberschreiben (PRD Q64).
    /// Returns the canonical revision recorded for a previously applied sync operation.
    pub fn sync_operation_revision(&self, operation_id: &str) -> Result<Option<i64>, Fehler> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT revision FROM {}_sync_operation WHERE operation_id = ?1",
                    self.prefix
                ),
                params![operation_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Records an applied operation. The primary key makes a retry visible instead of applying
    /// the same mutation twice.
    pub fn record_sync_operation(&self, operation_id: &str, revision: i64) -> Result<(), Fehler> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {}_sync_operation (operation_id, revision, processed_at)
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                self.prefix
            ),
            params![operation_id, revision],
        )?;
        Ok(())
    }

    pub fn item_count(&self) -> Result<i64, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {p}_item"), [], |r| r.get(0))?)
    }

    /// Ein einzelner Eintrag mit seinem aktuellen Zustand.
    pub fn item(&self, id: &str) -> Result<Option<(Item, Option<State>)>, Fehler> {
        Ok(self.catalogue()?.remove(id))
    }

    /// Die ganze Zustandsgeschichte eines Eintrags, aelteste zuerst.
    ///
    /// Die Oberflaeche zeigt sie, weil sie der Grund ist, warum `interior_item_state` eine
    /// Tabelle ist und keine Spalte: „gekauft" ist eine Zeile mehr und kein ueberschriebenes
    /// Feld, und wer das nicht sieht, haelt die Trennung fuer Umstaendlichkeit.
    pub fn state_history(
        &self,
        item_id: &str,
    ) -> Result<Vec<(State, String, Option<String>)>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT state, since, note FROM {p}_item_state
             WHERE item_id = ?1 ORDER BY since ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![item_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (s, since, note) = r?;
            if let Some(st) = State::parse(&s) {
                out.push((st, since, note));
            }
        }
        Ok(out)
    }

    pub fn current_state(&self, item_id: &str) -> Result<Option<State>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let raw: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT state FROM {p}_item_state WHERE item_id = ?1
                     ORDER BY since DESC, id DESC LIMIT 1"
                ),
                params![item_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.as_deref().and_then(State::parse))
    }

    pub fn place(&self, pl: &Placement) -> Result<(), Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_placement (item_id, flat, x, y, rot, since)
                 VALUES (?1, ?2, ?3, ?4, ?5, {now})
                 ON CONFLICT(item_id, flat) DO UPDATE SET
                    x=excluded.x, y=excluded.y, rot=excluded.rot, since={now}",
                p = p,
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot],
        )?;
        Ok(())
    }

    pub fn placements(&self, flat: &str) -> Result<Vec<Placement>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT item_id, flat, x, y, rot FROM {p}_placement WHERE flat = ?1 ORDER BY item_id"
        ))?;
        let rows = stmt.query_map(params![flat], |row| {
            Ok(Placement {
                item_id: row.get(0)?,
                flat: row.get(1)?,
                x: row.get(2)?,
                y: row.get(3)?,
                rot: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Jedes Item mit seinem aktuellen Zustand. Der Katalog, den die Pruefung liest.
    pub fn catalogue(&self) -> Result<BTreeMap<String, (Item, Option<State>)>, Fehler> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT i.id, i.kind, i.label, i.b, i.t, i.h, i.h_min, i.b_aufgeklappt,
                    i.t_ausgeklappt, i.laenge, i.anzahl, i.zustaende, i.unsicher,
                    i.platzbedarf_zone, i.platzbedarf_block, i.preis_cent, i.kosten_min_cent,
                    i.kosten_max_cent, i.link, i.artikelnummer, i.quelle, i.gemessen_am,
                    i.mitnahme, i.prioritaet, i.basiert_auf, i.ersetzt, i.varianten, i.ziel,
                    i.hinweis, i.begruendung, i.entscheidung_offen,
                    i.opens, i.open_clear, i.wall_ok, i.expands_dir, i.expands_to,
                    i.access_sides, i.access_clear, i.raumtrenner, i.bild, i.zerlegbar,
                    i.weight_g, i.category, i.packable, i.waterproof, i.quick_dry,
                    i.pack_location, i.trip_types, i.revision,
                    (SELECT s.state FROM {p}_item_state s
                      WHERE s.item_id = i.id ORDER BY s.since DESC, s.id DESC LIMIT 1)
             FROM {p}_item i ORDER BY i.id"
        ))?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(1)?;
            let seite = |i: usize| -> rusqlite::Result<Option<Seite>> {
                Ok(row
                    .get::<_, Option<String>>(i)?
                    .as_deref()
                    .and_then(Seite::parse))
            };
            let state: Option<String> = row.get(49)?;
            Ok((
                Item {
                    id: row.get(0)?,
                    kind: Kind::parse(&kind),
                    label: row.get(2)?,
                    b: row.get(3)?,
                    t: row.get(4)?,
                    h: row.get(5)?,
                    h_min: row.get(6)?,
                    b_aufgeklappt: row.get(7)?,
                    t_ausgeklappt: row.get(8)?,
                    laenge: row.get(9)?,
                    anzahl: row.get(10)?,
                    zustaende: sjel_store::json_column(row, 11)?,
                    unsicher: sjel_store::json_column(row, 12)?,
                    platzbedarf_zone: row.get(13)?,
                    platzbedarf_block: row.get(14)?,
                    preis_cent: row.get(15)?,
                    kosten_min_cent: row.get(16)?,
                    kosten_max_cent: row.get(17)?,
                    link: row.get(18)?,
                    artikelnummer: row.get(19)?,
                    quelle: row.get(20)?,
                    gemessen_am: row.get(21)?,
                    mitnahme: row.get(22)?,
                    prioritaet: row.get(23)?,
                    basiert_auf: row.get(24)?,
                    ersetzt: sjel_store::json_column(row, 25)?,
                    varianten: sjel_store::json_column(row, 26)?,
                    ziel: row.get(27)?,
                    hinweis: row.get(28)?,
                    begruendung: row.get(29)?,
                    entscheidung_offen: row.get(30)?,
                    opens: seite(31)?,
                    open_clear: row.get(32)?,
                    wall_ok: row.get(33)?,
                    expands_dir: seite(34)?,
                    expands_to: row.get(35)?,
                    access_sides: row.get(36)?,
                    access_clear: row.get(37)?,
                    raumtrenner: row.get(38)?,
                    bild: row.get(39)?,
                    zerlegbar: row.get(40)?,
                    weight_g: row.get(41)?,
                    category: row.get(42)?,
                    packable: row.get(43)?,
                    waterproof: row.get(44)?,
                    quick_dry: row.get(45)?,
                    pack_location: row.get(46)?,
                    trip_types: sjel_store::json_column(row, 47)?,
                    revision: row.get(48)?,
                },
                state.as_deref().and_then(State::parse),
            ))
        })?;
        let mut out = BTreeMap::new();
        for r in rows {
            let (item, state) = r?;
            out.insert(item.id.clone(), (item, state));
        }
        Ok(out)
    }
}
