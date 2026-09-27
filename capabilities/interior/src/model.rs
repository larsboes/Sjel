//! Das gemessene Modell: Wohnung, Regeln, Inventar, Layouts.
//!
//! Alles in Zentimetern, Ursprung innere Nordwest-Ecke, x nach Osten, y nach Sueden — genau
//! wie `room.toml` es festlegt. Meter kommen in diesem Programm nirgends vor.
//!
//! Die Daten liegen im privaten Overlay unter `<overlay>/data/interior` und werden nur
//! gelesen. Sie sind auf Papier mit einem Bandmass entstanden; dieser Code ist nicht befugt,
//! sie zu korrigieren.
//!
//! Aufteilung nach Lebensdauer, nicht nach Dateigroesse:
//!   inventory/        Moebel, die jeden Umzug ueberleben
//!   flats/<id>/       Geometrie und Regeln, die ihn nicht ueberleben

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Pt = [i32; 2];

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{path}: {detail}")]
    Config { path: PathBuf, detail: String },
    #[error("SJEL_PERSONAL_ROOT ist nicht gesetzt — ohne Overlay gibt es keine Wohnungsdaten")]
    NoOverlay,
    #[error("{0}")]
    Missing(String),
    #[error("Inventar nicht lesbar: {0}")]
    Store(String),
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ModelError> {
    let text = std::fs::read_to_string(path).map_err(|source| ModelError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| ModelError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// `<overlay>/data/interior`. Kein Fallback auf ein Verzeichnis im Repo: eine Planung gegen
/// erfundene Masse waere schlimmer als gar keine.
pub fn data_dir() -> Result<PathBuf, ModelError> {
    axon_config::overlay_data_dir("interior").ok_or(ModelError::NoOverlay)
}

/// Private, human-facing interior assets such as photos and RoomPlan captures.
///
/// The machine model stays in `data/interior`; the asset root may be declared by the
/// private overlay as `assets_root` in `config/interior.json`. A missing file, or a file
/// without `assets_root`, falls back to `data/interior`, which keeps the public test
/// shape self-contained. A file that exists but cannot be read or parsed is an error:
/// the README's rule is "bricht ab statt zu raten", and a silent fallback would serve
/// photos from a directory nobody chose.
pub fn assets_dir() -> Result<PathBuf, ModelError> {
    let fallback = data_dir()?;
    match axon_config::overlay_config("interior.json") {
        Some(config) => assets_dir_from(&config, fallback),
        None => Ok(fallback),
    }
}

/// `assets_dir` without the environment, so the refusals are testable.
fn assets_dir_from(config: &Path, fallback: PathBuf) -> Result<PathBuf, ModelError> {
    let text = match std::fs::read_to_string(config) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(fallback),
        Err(source) => {
            return Err(ModelError::Read {
                path: config.to_path_buf(),
                source,
            })
        }
    };
    let malformed = |detail: String| ModelError::Config {
        path: config.to_path_buf(),
        detail,
    };
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| malformed(format!("kein gueltiges JSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| malformed("erwartet ein JSON-Objekt".into()))?;
    match object.get("assets_root") {
        None => Ok(fallback),
        Some(serde_json::Value::String(root)) if !root.trim().is_empty() => {
            Ok(axon_config::expand_tilde(root))
        }
        Some(other) => Err(malformed(format!(
            "assets_root muss ein nicht leerer Pfad sein, steht aber als {other}"
        ))),
    }
}

// ---------------------------------------------------------------- Wohnung

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Opening {
    pub id: String,
    pub wand: String,
    /// Absolute Koordinate entlang der Wandachse, kein Versatz zum Wandanfang.
    pub von: i32,
    pub bis: i32,
    pub breite: i32,
    #[serde(default)]
    pub typ: Option<String>,
    #[serde(default)]
    pub freihaltezone: Option<i32>,
    #[serde(default)]
    pub sperrflaeche: Option<Zone>,
    /// Ist dies die Tuer, durch die man die Wohnung betritt?
    ///
    /// Die Wohnung sagt es, nicht der Code. `badtuer` und `eingangstuer` fuehren beide
    /// `typ = "tuer"`, und sie am Namen zu unterscheiden waere derselbe Fehler, den B26a mit
    /// der nach Norden festverdrahteten Kuechen-Anlaufzone geschlossen hat. Fehlt die Angabe,
    /// laeuft R6 nicht und der Bericht sagt, dass sie fehlt.
    #[serde(default)]
    pub eingang: Option<bool>,
    /// Unter- und Oberkante der Verglasung ueber dem Boden, in cm.
    ///
    /// Nur fuer `sonne.rs`, und dort unverzichtbar: die Ausdehnung eines Lichtflecks haengt
    /// linear an beiden. Eine Fensterhoehe anzunehmen hiesse, den Schattenwurf auf den
    /// Zentimeter genau aus einer Erfindung zu rechnen.
    #[serde(default)]
    pub glas_von_cm: Option<i32>,
    #[serde(default)]
    pub glas_bis_cm: Option<i32>,
    #[serde(default)]
    pub schwenk: Option<String>,
    #[serde(default)]
    pub schwenk_nach_innen: Option<bool>,
    #[serde(default)]
    pub notiz: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Zone {
    pub x: [i32; 2],
    pub y: [i32; 2],
}

impl Zone {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x[0],
            y: self.y[0],
            w: self.x[1] - self.x[0],
            d: self.y[1] - self.y[0],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Wall {
    pub von: Pt,
    pub bis: Pt,
    pub laenge: i32,
    #[serde(default)]
    pub frei: bool,
    #[serde(default)]
    pub notiz: Option<String>,
}

impl Wall {
    /// Waagerecht heisst: die Wand laeuft entlang x. Wird gebraucht, um eine Oeffnung auf die
    /// richtige Achse zu legen — `west` und `sued_hauptraum` laufen entgegen der Kantenrichtung
    /// des Polygons, deshalb wird die Achse aus den Koordinaten bestimmt und nicht angenommen.
    pub fn horizontal(&self) -> bool {
        (self.bis[0] - self.von[0]).abs() > (self.bis[1] - self.von[1]).abs()
    }
}

/// Welche Seite eines Rechtecks gemeint ist. Deutsch wie der Rest des Modells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Seite {
    Nord,
    Sued,
    Ost,
    West,
}

impl Seite {
    pub fn as_str(self) -> &'static str {
        match self {
            Seite::Nord => "nord",
            Seite::Sued => "sued",
            Seite::Ost => "ost",
            Seite::West => "west",
        }
    }

    pub fn parse(s: &str) -> Option<Seite> {
        match s {
            "nord" => Some(Seite::Nord),
            "sued" => Some(Seite::Sued),
            "ost" => Some(Seite::Ost),
            "west" => Some(Seite::West),
            _ => None,
        }
    }

    /// Dieselbe Seite, nachdem das Stueck gedreht wurde.
    ///
    /// `rot` zaehlt im Uhrzeigersinn, und y waechst nach Sueden — auf dem Plan ist das die
    /// Richtung, in die ein Zeiger dreht. Nord wird also zu Ost, Ost zu Sued, und so weiter.
    /// Nur Vielfache von 90 Grad; alles andere bleibt, wo es ist, weil `footprint` eine
    /// Grundflaeche auch nur bei nahe 90 Grad tauscht und zwei verschiedene Rundungen fuer
    /// dieselbe Drehung die Art von Fehler sind, die niemand sucht.
    pub fn gedreht(self, rot: i32) -> Seite {
        const IM_UHRZEIGERSINN: [Seite; 4] = [Seite::Nord, Seite::Ost, Seite::Sued, Seite::West];
        let schritte = rot.rem_euclid(360) / 90;
        if rot.rem_euclid(90) != 0 {
            return self;
        }
        let i = IM_UHRZEIGERSINN
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0);
        IM_UHRZEIGERSINN[(i + schritte as usize) % 4]
    }
}

/// Der Platz, den ein Mensch vor einem festen Einbau braucht, damit dieser benutzbar bleibt.
///
/// `seite` ist Geometrie und steht deshalb hier. `abstand` ist der NAME einer Schwelle in
/// `rules.toml` und nicht die Zahl selbst: wie viel Platz man vor einer Kuechenzeile braucht,
/// ist eine Norm und keine Eigenschaft dieser Wohnung. Eine hier ausgeschriebene Zahl waere
/// eine zweite Fassung derselben Regel, und zwei Fassungen stimmen genau bis zur ersten
/// Aenderung ueberein.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Anlaufzone {
    pub seite: Seite,
    pub abstand: String,
}

/// Ein Weg, der begehbar bleiben muss.
///
/// `von` und `nach` nennen Wegpunkte: eine Oeffnung, die kein Fenster ist, oder ein festes
/// Moebel mit `anlaufzone`. Bis 2026-08-30 stand diese Liste als Literal in `clearance.rs` und
/// nannte drei Oeffnungen dieser einen Wohnung beim Namen (PRD B26a). Eine Wohnung mit einem
/// Flur, zwei Zimmern oder ohne Badtuer bekam damit gar keine Route gemessen und meldete
/// nichts — nicht einmal einen Fehler. Welche Wege zaehlen, weiss die Wohnung.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Route {
    pub von: String,
    pub nach: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FixedFurniture {
    pub id: String,
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub tiefe: Option<i32>,
    #[serde(default)]
    pub laenge: Option<i32>,
    #[serde(default)]
    pub enthaelt: Vec<String>,
    /// Ohne diese Angabe ist der Einbau nur ein Hindernis: er belegt Flaeche, verlangt aber
    /// keinen freien Platz davor und taugt nicht als Ziel eines Laufwegs.
    #[serde(default)]
    pub anlaufzone: Option<Anlaufzone>,
}

impl FixedFurniture {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x[0],
            y: self.y[0],
            w: self.x[1] - self.x[0],
            d: self.y[1] - self.y[0],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Hauptraum {
    pub polygon: Vec<Pt>,
    /// 0 bedeutet "nicht gemessen". Siehe `todo.offen` — es gibt hier keinen Platzhalterwert,
    /// weil eine erfundene Raumhoehe still in jede Hoehenpruefung durchschlaegt.
    #[serde(default)]
    pub hoehe: i32,
    #[serde(default)]
    pub flaeche_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Terrasse {
    #[serde(default)]
    pub geschaetzt: bool,
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    pub flaeche_geschaetzt_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Todo {
    #[serde(default)]
    pub offen: Vec<String>,
}

/// Wo die Wohnung auf der Erde steht und wie ihr Plan zur Himmelsrichtung liegt.
///
/// Ohne diesen Block rechnet `sonne.rs` nicht. Das ist Absicht: ein erfundener Standort
/// ergaebe einen Lichtfleck auf den Zentimeter genau, der auf nichts beruht.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Lage {
    /// Geographische Breite in Grad, Nord positiv.
    pub breite: f64,
    /// Geographische Laenge in Grad, Ost positiv.
    pub laenge: f64,
    /// Der Abstand der Ortszeit zu UTC in Stunden, **einschliesslich Sommerzeit**.
    ///
    /// Eine Zahl und keine Zeitzonendatenbank: diese Capability haengt an keiner solchen, und
    /// wer die Uhrzeiten als Sommerzeit meint, traegt hier die Sommerzeit ein.
    pub utc_offset_h: f64,
    /// Welche Kompassrichtung im Plan nach oben zeigt (Richtung -y), in Grad.
    ///
    /// 0 heisst: oben ist Norden. Ohne diese Zahl waere jede Himmelsrichtung im Plan geraten,
    /// und `room.toml` haelt bereits `nord`, `ost`, `sued`, `west` als reine PLANrichtungen.
    #[serde(default)]
    pub nordrichtung_grad: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Room {
    pub flat: FlatMeta,
    #[serde(default)]
    pub lage: Option<Lage>,
    pub hauptraum: Hauptraum,
    #[serde(default)]
    pub zonen: BTreeMap<String, ZoneNamed>,
    pub bad: Option<BadRoom>,
    #[serde(default)]
    pub terrasse: Option<Terrasse>,
    pub waende: BTreeMap<String, Wall>,
    #[serde(default)]
    pub oeffnungen: Vec<Opening>,
    #[serde(default)]
    pub fix_moebel: Vec<FixedFurniture>,
    /// Welche Wege gemessen werden. Leer heisst: keiner — und das ist eine Aussage, keine
    /// Voreinstellung. Ein stiller Standard waere hier genau der Weg, auf dem eine Wohnung
    /// besteht, weil niemand nachgesehen hat, ob jemand durch sie hindurchkommt.
    #[serde(default)]
    pub routen: Vec<Route>,
    #[serde(default)]
    pub todo: Todo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlatMeta {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ZoneNamed {
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    pub flaeche_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BadRoom {
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    pub flaeche_m2: f64,
}

impl Room {
    /// Aus dem Polygon gerechnet, nicht aus der Datei gelesen — so kann die angegebene Flaeche
    /// nicht unbemerkt von der Geometrie abdriften, aus der jede Pruefung sie ableitet.
    pub fn area_m2(&self) -> f64 {
        crate::geometry::polygon_area_m2(&self.hauptraum.polygon)
    }

    /// Die aeusseren Masse des Hauptraums, in derselben Sprache wie ein Moebel: `b` nach Osten,
    /// `t` nach Sueden.
    ///
    /// Aus dem Polygon gerechnet und nicht aus `[flat.aufmass...]` gelesen, aus demselben Grund
    /// wie `area_m2`: die angegebene Zahl kann von der Geometrie abdriften, aus der jede
    /// Pruefung rechnet. Sie steht hier, damit die Oberflaeche die Raummasse anzeigen kann,
    /// ohne sich eine zweite Fassung des Polygons zu halten.
    pub fn masse(&self) -> (i32, i32) {
        let spanne = |f: fn(&Pt) -> i32| {
            let werte = self.hauptraum.polygon.iter().map(f);
            werte.clone().max().unwrap_or(0) - werte.min().unwrap_or(0)
        };
        (spanne(|p| p[0]), spanne(|p| p[1]))
    }

    /// Wo eine Oeffnung physisch sitzt. `von`/`bis` sind absolute Koordinaten entlang der
    /// Wandachse; fuer nord/ost/west faellt das mit einem Versatz zusammen, fuer `badtuer`
    /// nicht — als Versatz gelesen laege sie ausserhalb ihrer eigenen Wand.
    pub fn opening_span(&self, o: &Opening) -> Option<(Pt, Pt)> {
        let w = self.waende.get(&o.wand)?;
        Some(if w.horizontal() {
            ([o.von, w.von[1]], [o.bis, w.von[1]])
        } else {
            ([w.von[0], o.von], [w.von[0], o.bis])
        })
    }
}

// ---------------------------------------------------------------- Regeln

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Laufwege {
    pub haupt_soll: i32,
    pub haupt_min: i32,
    pub neben_min: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Lichtkorridor {
    pub x: [i32; 2],
    pub y: [i32; 2],
    pub max_hoehe: i32,
}

/// Wie schwer ein Verstoss wiegt. `hart` blockiert, `weich` warnt.
///
/// Steht hier und nicht in `clearance.rs`, weil die Wohnung sie deklariert und der Pruefer sie
/// nur nachschlaegt. Bis 2026-08-31 war es umgekehrt: `clearance.rs` trug die Schwere an jeder
/// der 21 Ausgabestellen als Literal, waehrend `rules.toml` ein `schwere`-Feld fuehrte, das
/// nichts las. Der Modulkopf dort behauptete trotzdem "Schwere folgt rules.toml".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hart,
    Weich,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Regel {
    pub id: String,
    pub schwere: String,
    pub text: String,
}

impl Regel {
    /// Die deklarierte Schwere, oder ein Fehler.
    ///
    /// Ein unbekanntes Wort ist absichtlich kein stillschweigendes `weich`: eine Wohnung, die
    /// sich vertippt, bekaeme sonst eine Regel, die nie blockiert, und der Bericht saehe aus
    /// wie einer ueber eine Wohnung, in der alles erlaubt ist.
    pub fn severity(&self) -> Result<Severity, ModelError> {
        match self.schwere.as_str() {
            "hart" => Ok(Severity::Hart),
            "weich" => Ok(Severity::Weich),
            other => Err(ModelError::Missing(format!(
                "rules.toml: Regel `{}` hat die Schwere `{other}` — erlaubt sind `hart` und `weich`",
                self.id
            ))),
        }
    }
}

/// Was die Wohnung ueber Sonne am Arbeitsplatz sagt (Regel R9).
///
/// Eigener Block und keine Zeile in `[abstaende]`: dort stehen Zentimeter, und das hier sind
/// Stunden. Eine Schwelle in der falschen Einheit ist die Sorte Zahl, die beim Lesen stimmt
/// und beim Rechnen nicht.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Sonnenregel {
    /// Wie viele der geprueften Stunden ein Schreibtisch hoechstens in direkter Sonne liegen
    /// darf. Der Nenner ist `sonne::gepruefte_stunden()`.
    pub max_stunden_am_schreibtisch: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rules {
    /// Nur gesetzt, wenn die Wohnung R9 fuehrt. Fehlt sie, laeuft die Regel nicht und sagt das.
    #[serde(default)]
    pub sonne: Option<Sonnenregel>,
    pub laufwege: Laufwege,
    /// Absichtlich eine Map: jede Distanz wird ueber ihren Namen gelesen, damit eine Regel,
    /// die einen Wert erfindet statt ihn nachzuschlagen, beim Lesen auffaellt.
    pub abstaende: BTreeMap<String, i32>,
    pub lichtkorridor: Lichtkorridor,
    #[serde(default)]
    pub regeln: Vec<Regel>,
}

impl Rules {
    pub fn abstand(&self, key: &str) -> Result<i32, ModelError> {
        self.abstaende
            .get(key)
            .copied()
            .ok_or_else(|| ModelError::Missing(format!("rules.toml kennt keinen Abstand `{key}`")))
    }

    /// Die deklarierte Regel zu einer Kennung, oder ein Fehler.
    ///
    /// Dieselbe Form wie `abstand`, aus demselben Grund: eine Regel, die ihren Text oder ihre
    /// Schwere erfindet, statt sie nachzuschlagen, faellt beim Lesen auf. Der Pruefer ruft das
    /// an jeder Stelle, an der er eine R-Kennung ausgibt — eine Kennung, die die Wohnung nicht
    /// fuehrt, ist damit ein Fehler und kein Verstoss ohne Text.
    pub fn regel(&self, id: &str) -> Result<&Regel, ModelError> {
        self.regeln.iter().find(|r| r.id == id).ok_or_else(|| {
            let bekannt: Vec<&str> = self.regeln.iter().map(|r| r.id.as_str()).collect();
            ModelError::Missing(format!(
                "rules.toml deklariert keine Regel `{id}` — deklariert sind: {}",
                if bekannt.is_empty() {
                    "keine".to_string()
                } else {
                    bekannt.join(", ")
                }
            ))
        })
    }

    /// Deklarierte Kennungen, die diese Maschine nicht pruefen kann.
    ///
    /// **Kein Fehler, sondern ein Bericht.** Die reale Wohnung fuehrt R5 (Schreibtisch weder
    /// frontal noch mit dem Ruecken zur Verglasung) und R6 (der Blick vom Eingang faellt nicht
    /// zuerst aufs Bett) — beides Hausregeln, die hier niemand prueft. Bis 2026-08-31 fielen
    /// sie stumm heraus, und ein Bericht ohne sie sah aus wie ein vollstaendiger. Er nennt sie
    /// jetzt, damit die Luecke sichtbar bleibt, statt sich als Bestehen auszugeben.
    pub fn nicht_geprueft(&self, geprueft: &[&str]) -> Vec<&Regel> {
        self.regeln
            .iter()
            .filter(|r| !geprueft.contains(&r.id.as_str()))
            .collect()
    }
}

// ---------------------------------------------------------------- Inventar

/// Der Katalog ist keine Datei mehr. Seit PRD B25 (2026-08-31) sind Moebel Zeilen in der
/// geteilten SQLite-Datei; `inventory/*.toml` war die Migrationsquelle und ist jetzt das
/// Herkunftsprotokoll. Q60 zieht die Grenze: ein Moebel ist eine Zeile, ein Raum nicht.
pub type Catalogue = BTreeMap<String, crate::store::Item>;

// ---------------------------------------------------------------- Layout

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlacedItem {
    #[serde(rename = "ref")]
    pub reference: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub rot: i32,
    /// Ueberschreibt die Katalogmasse — fuer den zweiten Zustand eines Klappmoebels.
    #[serde(default)]
    pub size: Option<[i32; 2]>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Layout {
    pub name: String,
    #[serde(default, rename = "item")]
    pub items: Vec<PlacedItem>,
    #[serde(skip_deserializing, default)]
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub d: i32,
}

impl Rect {
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.d && o.y < self.y + self.d
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.d
    }
}

/// Grundflaeche nach Drehung. Eine Drehung nahe 90 Grad tauscht Breite und Tiefe; alles
/// andere laesst sie stehen. `size` im Layout schlaegt den Katalog.
pub fn footprint(p: &PlacedItem, cat: &Catalogue) -> Result<(i32, i32, Option<i32>), ModelError> {
    let item = cat.get(&p.reference);
    let (b, t) = match p.size {
        Some([b, t]) => (Some(b), Some(t)),
        None => (item.and_then(|i| i.b), item.and_then(|i| i.t)),
    };
    let (b, t) = match (b, t) {
        (Some(b), Some(t)) => (b, t),
        _ => {
            return Err(ModelError::Missing(format!(
                "kein Grundriss fuer `{}` — weder im Inventar noch als size: im Layout",
                p.reference
            )))
        }
    };
    let swap = ((p.rot.rem_euclid(180)) - 90).abs() < 45;
    let h = item.and_then(|i| i.h);
    Ok(if swap { (t, b, h) } else { (b, t, h) })
}

// ---------------------------------------------------------------- Laden

pub struct Model {
    pub room: Room,
    pub rules: Rules,
    pub catalogue: Catalogue,
    /// Besitz je Eintrag, getrennt gehalten statt als Feld auf dem Item: der Zustand hat eine
    /// Geschichte (`interior_item_state`) und das Item hat keine.
    pub states: BTreeMap<String, crate::store::State>,
    pub flat_dir: PathBuf,
}

/// Welche Wohnungen unter `data/interior/flats/` liegen.
pub fn flats() -> Result<Vec<String>, ModelError> {
    let dir = data_dir()?.join("flats");
    let rd = std::fs::read_dir(&dir).map_err(|source| ModelError::Read { path: dir, source })?;
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

/// Die Wohnung, gegen die gerechnet wird: `SJEL_INTERIOR_FLAT`, sonst die einzige vorhandene.
///
/// Gibt es mehrere und keine ist gewaehlt, ist das ein Fehler und keine Vorauswahl. Sobald ein
/// zweiter Raum zum Vergleich existiert (PRD B28), waere ein stiller Standard genau der Weg, auf
/// dem ein Plan der falschen Wohnung als der richtige durchgeht.
pub fn default_flat() -> Result<String, ModelError> {
    if let Ok(v) = axon_config::env_var("SJEL_INTERIOR_FLAT") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let all = flats()?;
    match all.len() {
        0 => Err(ModelError::Missing(
            "keine Wohnung unter data/interior/flats/ — ohne Geometrie gibt es nichts zu rechnen"
                .into(),
        )),
        1 => Ok(all.into_iter().next().unwrap()),
        _ => Err(ModelError::Missing(format!(
            "mehrere Wohnungen ({}) — waehle eine mit SJEL_INTERIOR_FLAT oder --flat",
            all.join(", ")
        ))),
    }
}

impl Model {
    pub fn load(flat: &str) -> Result<Self, ModelError> {
        let root = data_dir()?;
        let flat_dir = root.join("flats").join(flat);
        let room: Room = read_toml(&flat_dir.join("room.toml"))?;
        let rules: Rules = read_toml(&flat_dir.join("rules.toml"))?;

        // Der Katalog kommt aus der Datenbank, nicht mehr aus `inventory/*.toml` (B25). Der
        // Pfad ist derselbe, den jede andere Capability nimmt: SJEL_DB_PATH, sonst das
        // Overlay. Ein leerer Katalog ist kein Fehler beim Laden — er faellt dort auf, wo
        // ein Layout ein Stueck nennt, das es nicht gibt, und das ist die Stelle mit dem
        // besseren Fehlertext.
        let store = crate::store::Store::open(&axon_config::database_path())
            .map_err(|e| ModelError::Store(e.to_string()))?;
        let rows = store
            .catalogue()
            .map_err(|e| ModelError::Store(e.to_string()))?;

        let mut catalogue = Catalogue::new();
        let mut states = BTreeMap::new();
        for (id, (item, state)) in rows {
            if let Some(s) = state {
                states.insert(id.clone(), s);
            }
            catalogue.insert(id, item);
        }
        Ok(Model {
            room,
            rules,
            catalogue,
            states,
            flat_dir,
        })
    }

    /// Die Wunschliste: alles, was `wanted` ist, in Katalogreihenfolge. Der Anschluss, den
    /// PRD B29 an `finance` legt — ein gewuenschtes Stueck traegt einen Preis, und Geld
    /// gehoert `finance`.
    pub fn wishlist(&self) -> Vec<&crate::store::Item> {
        self.catalogue
            .values()
            .filter(|i| self.states.get(&i.id) == Some(&crate::store::State::Wanted))
            .collect()
    }

    pub fn layouts_dir(&self) -> PathBuf {
        self.flat_dir.join("layouts")
    }

    pub fn load_layout(&self, name: &str) -> Result<Layout, ModelError> {
        let path = self.layouts_dir().join(format!("{name}.toml"));
        let mut l: Layout = read_toml(&path)?;
        l.id = name.to_string();
        Ok(l)
    }

    pub fn layout_names(&self) -> Result<Vec<String>, ModelError> {
        let dir = self.layouts_dir();
        let rd =
            std::fs::read_dir(&dir).map_err(|source| ModelError::Read { path: dir, source })?;
        let mut out: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(str::to_string))?
            })
            .collect();
        out.sort();
        Ok(out)
    }

    /// Jedes Moebel im Katalog, dessen Masse nicht gemessen sind. Wandert unveraendert in jeden
    /// Pruefbericht.
    pub fn uncertainties(&self) -> Vec<(&str, &str, &[String])> {
        let mut v: Vec<_> = self
            .catalogue
            .values()
            .filter(|i| i.is_uncertain())
            .map(|i| (i.id.as_str(), i.label.as_str(), i.unsicher.as_slice()))
            .collect();
        v.sort_by_key(|(id, _, _)| *id);
        v
    }
}

#[cfg(test)]
mod assets_dir_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("interior-assets-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// No file and no key both keep the default: an overlay without asset photos is legal.
    #[test]
    fn a_missing_file_or_key_keeps_the_default() {
        let dir = scratch("missing");
        let fallback = PathBuf::from("/overlay/data/interior");
        assert_eq!(
            assets_dir_from(&dir.join("interior.json"), fallback.clone()).unwrap(),
            fallback
        );
        let config = dir.join("interior-no-key.json");
        std::fs::write(&config, r#"{"obsidian": {}}"#).unwrap();
        assert_eq!(
            assets_dir_from(&config, fallback.clone()).unwrap(),
            fallback
        );
    }

    /// The overlay's key is `assets_root`; it wins over the default.
    #[test]
    fn the_declared_assets_root_is_used() {
        let dir = scratch("declared");
        let config = dir.join("interior.json");
        std::fs::write(&config, r#"{"assets_root": "/private/assets"}"#).unwrap();
        assert_eq!(
            assets_dir_from(&config, PathBuf::from("/fallback")).unwrap(),
            PathBuf::from("/private/assets")
        );
    }

    /// "bricht ab statt zu raten": a file that exists and does not parse is an error.
    #[test]
    fn an_unparsable_or_malformed_file_is_an_error() {
        let dir = scratch("malformed");
        for (name, body) in [
            ("broken.json", "{ assets_root = "),
            ("array.json", "[]"),
            ("number.json", r#"{"assets_root": 7}"#),
            ("blank.json", r#"{"assets_root": "  "}"#),
        ] {
            let config = dir.join(name);
            std::fs::write(&config, body).unwrap();
            let outcome = assets_dir_from(&config, PathBuf::from("/fallback"));
            assert!(
                matches!(outcome, Err(ModelError::Config { .. })),
                "{name} must be refused, got {outcome:?}"
            );
        }
    }

    /// A path that exists but is not readable as a file is an error, not the default.
    #[test]
    fn an_unreadable_file_is_an_error() {
        let dir = scratch("unreadable");
        let outcome = assets_dir_from(&dir, PathBuf::from("/fallback"));
        assert!(
            matches!(outcome, Err(ModelError::Read { .. })),
            "{outcome:?}"
        );
    }
}
