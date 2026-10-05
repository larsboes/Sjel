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
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

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

    // --- Kleidung (2026-10-05) ---
    //
    // Drei Felder, und die Entscheidung dahinter ist dieselbe wie bei der Ausruestung: sie
    // stehen hier und nicht in einer zweiten Tabelle, weil Q58 genau diese Frage entschieden
    // hat — eine Gegenstandstabelle fuer alles, was mir gehoert. Ein Hemd ist eine Zeile mit
    // einer Groesse, kein eigenes Schema.
    //
    // Alle drei sind freiwillig. Ein Kleidungsstueck ohne Groesse ist keine kaputte Zeile:
    // eine Groesse, die niemand nachgeschlagen hat, waere schlimmer als keine.
    /// Wie es im Etikett steht — `M`, `42`, `60x60`. Freier Text und keine Aufzaehlung: eine
    /// erfundene Skala (`S..XXL`) waere die naechste Marke, die nicht hineinpasst.
    pub groesse: Option<String>,
    /// Was es beschreibt — `weiss`, `dunkelblau`, `gestreift`. Frei, aus demselben Grund.
    pub farbe: Option<String>,
    /// Wann es getragen wird, z. B. `["ganzjahr"]`, `["winter"]`. Liste wie `trip_types`:
    /// ein Mantel ist nicht Winter ODER Uebergang, er ist beides.
    #[serde(default)]
    pub saison: Vec<String>,

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

/// Der Praefix der Item-Tabellen.
///
/// Sie gehoeren `capabilities/inventory` (ISA F13) und werden dort angelegt und migriert;
/// diese Capability liest und schreibt sie ueber die geteilte Datei, weil sie ohne sie
/// keinen Plan rechnen kann. Der eigene Praefix dieser Capability traegt nur noch
/// `interior_placement`.
const ITEM_PREFIX: &str = "inventory";

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
    /// Nur noch die **eigene** Tabelle.
    ///
    /// `inventory_item` und `inventory_item_state` gehoeren `capabilities/inventory` und werden
    /// dort angelegt und migriert (ISA F13). Eine zweite DDL hier waere eine zweite Wahrheit
    /// ueber dieselben Spalten — und die vergisst man beim naechsten Feld, weil nichts sie
    /// meldet.
    ///
    /// `item_id` traegt **keinen** Fremdschluessel mehr. Die Zeilen gehoeren einer anderen
    /// Capability, und ein FK ueber zwei Praefixe liesse jeden Loeschvorgang der einen an einer
    /// Zeile der anderen scheitern. `trips/src/pack.rs` begruendet dieselbe Entscheidung fuer
    /// seinen `item_ref`.
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Fehler> {
        // Die Item-Tabellen, falls sie fehlen.
        //
        // **Ein Bootstrap und keine zweite Wahrheit.** Sie gehoeren `capabilities/inventory`
        // und werden dort angelegt und migriert — samt der ALTER-Kette fuer Dateien, die vor
        // B51 oder vor den Kleidungsspalten geschrieben wurden. Hier steht nur das Anlegen mit
        // dem heutigen Stand, und zwar aus einem Grund: `interior` wird als CLI benutzt
        // (`interior check` ist ein Gate) und in Tests, und beide laufen auf einer Datei, in
        // der `inventory` noch nie geoeffnet wurde. Eine zweite ALTER-Kette hier waere die
        // Drift; ein `CREATE TABLE IF NOT EXISTS` ist es nicht — auf jeder Datei, die
        // `inventory` kennt, tut es nichts.
        //
        // Was die beiden verbindet, sind die **Spaltennamen**, und die stehen in
        // `catalogue()` ausgeschrieben. Eine umbenannte oder entfernte Spalte faellt dort
        // sofort um; eine neue in `inventory` stoert hier nichts.
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {item}_item (
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
                groesse            TEXT,
                farbe              TEXT,
                saison             TEXT NOT NULL DEFAULT '[]',
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL,
                revision           INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS {item}_item_state (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL REFERENCES {item}_item(id) ON DELETE CASCADE,
                state   TEXT NOT NULL CHECK (state IN ('owned','wanted','gone')),
                since   TEXT NOT NULL,
                note    TEXT
            );
            CREATE INDEX IF NOT EXISTS {item}_idx_state_item
                ON {item}_item_state(item_id, since DESC);
            ",
            item = ITEM_PREFIX
        ))?;
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_placement (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                flat    TEXT NOT NULL,
                x       INTEGER NOT NULL,
                y       INTEGER NOT NULL,
                rot     INTEGER NOT NULL DEFAULT 0,
                since   TEXT NOT NULL,
                UNIQUE (item_id, flat)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_flat
                ON {prefix}_placement(flat);
            ",
            prefix = prefix
        ))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Fehler> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
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
        let p = ITEM_PREFIX;
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
                    i.pack_location, i.trip_types, i.groesse, i.farbe, i.saison, i.revision,
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
            let state: Option<String> = row.get(52)?;
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
                    groesse: row.get(48)?,
                    farbe: row.get(49)?,
                    saison: sjel_store::json_column(row, 50)?,
                    revision: row.get(51)?,
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
