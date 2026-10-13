//! The measured model: flat, rules, inventory, layouts.
//!
//! Centimetres throughout, origin at the inner north-west corner, x east, y south, as `room.toml`
//! defines it. The data lives in the private overlay under `<overlay>/data/interior`, was measured
//! on paper with a tape, and is only read: this code may not correct it.

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
    #[error("SJEL_PERSONAL_ROOT is not set; without an overlay there is no flat data")]
    NoOverlay,
    #[error("{0}")]
    Missing(String),
    #[error("inventory not readable: {0}")]
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

/// `<overlay>/data/interior`. No fallback into the repository: planning against invented sizes is
/// worse than none.
pub fn data_dir() -> Result<PathBuf, ModelError> {
    sjel_config::overlay_data_dir("interior").ok_or(ModelError::NoOverlay)
}

/// Private, human-facing interior assets such as photos and RoomPlan captures.
///
/// The overlay may declare `assets_root` in `config/interior.json`; a missing file or key falls back
/// to `data/interior`. A file that exists but does not parse is an error rather than a guess, so no
/// photos are served from a directory nobody chose.
pub fn assets_dir() -> Result<PathBuf, ModelError> {
    let fallback = data_dir()?;
    match sjel_config::overlay_config("interior.json") {
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
        .map_err(|e| malformed(format!("not valid JSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| malformed("expected a JSON object".into()))?;
    match object.get("assets_root") {
        None => Ok(fallback),
        Some(serde_json::Value::String(root)) if !root.trim().is_empty() => {
            Ok(sjel_config::expand_tilde(root))
        }
        Some(other) => Err(malformed(format!(
            "assets_root must be a non-empty path, but is {other}"
        ))),
    }
}

// ---------------------------------------------------------------- Flat

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Opening {
    pub id: String,
    #[serde(alias = "wand")]
    pub wall: String,
    /// Absolute coordinate along the wall axis, not an offset from the wall start.
    #[serde(alias = "von")]
    pub from: i32,
    #[serde(alias = "bis")]
    pub to: i32,
    #[serde(alias = "breite")]
    pub width: i32,
    /// `door`, `glass_door` or `window`; German words in the file are translated on load.
    #[serde(default, alias = "typ")]
    pub kind: Option<String>,
    #[serde(default)]
    #[serde(alias = "freihaltezone")]
    pub keep_clear_zone: Option<i32>,
    #[serde(default)]
    #[serde(alias = "sperrflaeche")]
    pub blocked_area: Option<Zone>,
    /// Whether this is the door one enters the flat by. The flat says so, not the name: the
    /// bathroom and entrance doors are both doors. Without it R6 does not run and the report says so.
    #[serde(default)]
    #[serde(alias = "eingang")]
    pub entrance: Option<bool>,
    /// Lower and upper edge of the glazing above the floor, in cm. Only `sun.rs` needs them, and a
    /// light patch scales linearly with both, so none is assumed.
    #[serde(default)]
    #[serde(alias = "glas_von_cm")]
    pub glass_from_cm: Option<i32>,
    #[serde(default)]
    #[serde(alias = "glas_bis_cm")]
    pub glass_to_cm: Option<i32>,
    #[serde(default)]
    #[serde(alias = "schwenk")]
    pub swing: Option<String>,
    #[serde(default)]
    #[serde(alias = "schwenk_nach_innen")]
    pub swings_inward: Option<bool>,
    #[serde(default, alias = "notiz")]
    pub note: Option<String>,
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
    #[serde(alias = "von")]
    pub from: Pt,
    #[serde(alias = "bis")]
    pub to: Pt,
    #[serde(alias = "laenge")]
    pub length: i32,
    #[serde(default, alias = "frei")]
    pub free: bool,
    #[serde(default, alias = "notiz")]
    pub note: Option<String>,
}

impl Wall {
    /// Horizontal means the wall runs along x. Derived from the coordinates, because some walls run
    /// against the polygon's edge direction.
    pub fn horizontal(&self) -> bool {
        (self.to[0] - self.from[0]).abs() > (self.to[1] - self.from[1]).abs()
    }
}

pub use crate::store::Side;

/// The space a person needs in front of a fixture to use it. `threshold` names a value in
/// `rules.toml` rather than holding one, so the norm has one source.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApproachZone {
    #[serde(alias = "seite")]
    pub side: Side,
    #[serde(alias = "abstand")]
    pub threshold: String,
}

/// A path that must stay walkable between two waypoints: an opening that is not a window, or a
/// fixture with an `approach_zone`. Which paths count is the flat's to say.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Route {
    #[serde(alias = "von")]
    pub from: String,
    #[serde(alias = "nach")]
    pub to: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FixedFurniture {
    pub id: String,
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default, alias = "tiefe")]
    pub depth: Option<i32>,
    #[serde(default)]
    #[serde(alias = "laenge")]
    pub length: Option<i32>,
    #[serde(default, alias = "enthaelt")]
    pub contains: Vec<String>,
    /// Without it the fixture is only an obstacle and no walkway target.
    #[serde(default)]
    #[serde(alias = "anlaufzone")]
    pub approach_zone: Option<ApproachZone>,
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
pub struct MainRoom {
    pub polygon: Vec<Pt>,
    /// 0 means not measured; see `todo.open`. An invented ceiling height would leak into every height
    /// check.
    #[serde(default)]
    #[serde(alias = "hoehe")]
    pub height: i32,
    #[serde(default, alias = "flaeche_m2")]
    pub area_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Terrace {
    #[serde(default, alias = "geschaetzt")]
    pub estimated: bool,
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default)]
    #[serde(alias = "flaeche_geschaetzt_m2")]
    pub area_estimated_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Todo {
    #[serde(default, alias = "offen")]
    pub open: Vec<String>,
}

/// Where the flat is on Earth and how its plan is oriented. Without it `sun.rs` does not compute:
/// an invented location would give a centimetre-exact light patch based on nothing.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Location {
    /// Latitude in degrees, north positive.
    #[serde(alias = "breite")]
    pub latitude: f64,
    /// Longitude in degrees, east positive.
    #[serde(alias = "laenge")]
    pub longitude: f64,
    /// Offset of local time from UTC in hours, including daylight saving time. A number, not a time
    /// zone database.
    pub utc_offset_h: f64,
    /// The compass direction that points up in the plan (towards -y), in degrees; 0 means north. The
    /// wall names in `room.toml` are plan directions only.
    #[serde(default)]
    #[serde(alias = "nordrichtung_grad")]
    pub north_deg: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Room {
    pub flat: FlatMeta,
    #[serde(default)]
    #[serde(alias = "lage")]
    pub location: Option<Location>,
    #[serde(alias = "hauptraum")]
    pub main_room: MainRoom,
    #[serde(default)]
    #[serde(alias = "zonen")]
    pub zones: BTreeMap<String, ZoneNamed>,
    #[serde(alias = "bad")]
    pub bathroom: Option<Bathroom>,
    #[serde(default)]
    #[serde(alias = "terrasse")]
    pub terrace: Option<Terrace>,
    #[serde(alias = "waende")]
    pub walls: BTreeMap<String, Wall>,
    #[serde(default)]
    #[serde(alias = "oeffnungen")]
    pub openings: Vec<Opening>,
    #[serde(default)]
    #[serde(alias = "fix_moebel")]
    pub fixed_furniture: Vec<FixedFurniture>,
    /// Which paths are measured. Empty means none, stated rather than defaulted.
    #[serde(default)]
    #[serde(alias = "routen")]
    pub routes: Vec<Route>,
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
    #[serde(default, alias = "flaeche_m2")]
    pub area_m2: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Bathroom {
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(default, alias = "flaeche_m2")]
    pub area_m2: f64,
}

impl Room {
    /// Computed from the polygon, not read from the file, so the stated area cannot drift from the
    /// geometry every check uses.
    pub fn area_m2(&self) -> f64 {
        crate::geometry::polygon_area_m2(&self.main_room.polygon)
    }

    /// Outer dimensions of the main room as `(width, depth)`, computed from the polygon for the same
    /// reason as `area_m2`.
    pub fn dimensions(&self) -> (i32, i32) {
        let span = |f: fn(&Pt) -> i32| {
            let values = self.main_room.polygon.iter().map(f);
            values.clone().max().unwrap_or(0) - values.min().unwrap_or(0)
        };
        (span(|p| p[0]), span(|p| p[1]))
    }

    /// Where an opening physically sits. `from`/`to` are absolute coordinates along the wall axis, not
    /// an offset from the wall start.
    pub fn opening_span(&self, o: &Opening) -> Option<(Pt, Pt)> {
        let w = self.walls.get(&o.wall)?;
        Some(if w.horizontal() {
            ([o.from, w.from[1]], [o.to, w.from[1]])
        } else {
            ([w.from[0], o.from], [w.from[0], o.to])
        })
    }
}

// ---------------------------------------------------------------- Rules

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Walkways {
    #[serde(alias = "haupt_soll")]
    pub main_target: i32,
    #[serde(alias = "haupt_min")]
    pub main_min: i32,
    #[serde(alias = "neben_min")]
    pub side_min: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightCorridor {
    pub x: [i32; 2],
    pub y: [i32; 2],
    #[serde(alias = "max_hoehe")]
    pub max_height: i32,
}

/// How much a violation weighs: `hard` blocks, `soft` warns. The flat declares it; the checker only
/// looks it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    #[serde(alias = "schwere")]
    pub severity: String,
    pub text: String,
}

impl Rule {
    /// The declared severity, or an error. An unknown word is not silently `soft`, or a typo would
    /// make a rule that never blocks.
    pub fn level(&self) -> Result<Severity, ModelError> {
        match self.severity.as_str() {
            "hard" | "hart" => Ok(Severity::Hard),
            "soft" | "weich" => Ok(Severity::Soft),
            other => Err(ModelError::Missing(format!(
                "rules.toml: rule `{}` has severity `{other}`; allowed are `hard` and `soft`",
                self.id
            ))),
        }
    }
}

/// What the flat says about sun at the desk (rule R9). Its own block because these are hours,
/// not the centimetres of `[thresholds]`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SunRule {
    /// How many of the checked hours a desk may lie in direct sun at most; the denominator is
    /// `sun::checked_hours()`.
    #[serde(alias = "max_stunden_am_schreibtisch")]
    pub max_hours_at_desk: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rules {
    /// Set only when the flat has R9; otherwise the rule does not run and says so.
    #[serde(default)]
    #[serde(alias = "sonne")]
    pub sun: Option<SunRule>,
    #[serde(alias = "laufwege")]
    pub walkways: Walkways,
    /// A map on purpose: every distance is read by name, so a rule that invents a value shows up.
    #[serde(alias = "abstaende")]
    pub thresholds: BTreeMap<String, i32>,
    #[serde(alias = "lichtkorridor")]
    pub light_corridor: LightCorridor,
    #[serde(default, alias = "regeln")]
    pub rules: Vec<Rule>,
}

impl Rules {
    pub fn threshold(&self, key: &str) -> Result<i32, ModelError> {
        self.thresholds
            .get(key)
            .copied()
            .ok_or_else(|| ModelError::Missing(format!("rules.toml has no threshold `{key}`")))
    }

    /// The declared rule for an id, or an error: an id the flat does not declare is an error, not a
    /// violation without text.
    pub fn rule(&self, id: &str) -> Result<&Rule, ModelError> {
        self.rules.iter().find(|r| r.id == id).ok_or_else(|| {
            let known: Vec<&str> = self.rules.iter().map(|r| r.id.as_str()).collect();
            ModelError::Missing(format!(
                "rules.toml declares no rule `{id}`; declared are: {}",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ))
        })
    }

    /// Declared ids this checker cannot check. A report, not an error, so the gap stays visible instead
    /// of passing as a pass.
    pub fn unchecked(&self, checked: &[&str]) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| !checked.contains(&r.id.as_str()))
            .collect()
    }
}

// ---------------------------------------------------------------- Inventory

/// Furniture is rows in the shared store, read through inventory.
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
    /// Overrides the catalogue size, e.g. for the unfolded state of a folding piece.
    #[serde(default)]
    pub size: Option<[i32; 2]>,
    #[serde(default)]
    #[serde(alias = "typ")]
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

/// Footprint after rotation: near 90° width and depth swap. `size` in the layout beats the
/// catalogue.
pub fn footprint(p: &PlacedItem, cat: &Catalogue) -> Result<(i32, i32, Option<i32>), ModelError> {
    let item = cat.get(&p.reference);
    let (w, d) = match p.size {
        Some([w, d]) => (Some(w), Some(d)),
        None => (item.and_then(|i| i.width), item.and_then(|i| i.depth)),
    };
    let (w, d) = match (w, d) {
        (Some(w), Some(d)) => (w, d),
        _ => {
            return Err(ModelError::Missing(format!(
                "no footprint for `{}`: neither in the inventory nor as size: in the layout",
                p.reference
            )))
        }
    };
    let swap = ((p.rot.rem_euclid(180)) - 90).abs() < 45;
    let h = item.and_then(|i| i.height);
    Ok(if swap { (d, w, h) } else { (w, d, h) })
}

// ---------------------------------------------------------------- Loading

/// Opening kinds in English; older `room.toml` files use German words.
/// The kind may carry a suffix after the word (`glastuer_schiebe`), which is kept.
fn english_opening_kind(kind: &str) -> String {
    for (old, new) in [("glastuer", "glass_door"), ("tuer", "door"), ("fenster", "window")] {
        if let Some(rest) = kind.strip_prefix(old) {
            return format!("{new}{rest}");
        }
    }
    kind.to_string()
}

pub struct Model {
    pub room: Room,
    pub rules: Rules,
    pub catalogue: Catalogue,
    /// Ownership per item, kept apart from the item: the state has a history, the item does not.
    pub states: BTreeMap<String, crate::store::State>,
    pub flat_dir: PathBuf,
}

/// The flats under `data/interior/flats/`.
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

/// The flat to compute against: `SJEL_INTERIOR_FLAT`, else the only one. Several without a choice
/// is an error, never a default.
pub fn default_flat() -> Result<String, ModelError> {
    if let Ok(v) = sjel_config::env_var("SJEL_INTERIOR_FLAT") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let all = flats()?;
    match all.len() {
        0 => Err(ModelError::Missing(
            "no flat under data/interior/flats/; without geometry there is nothing to compute"
                .into(),
        )),
        1 => Ok(all.into_iter().next().unwrap()),
        _ => Err(ModelError::Missing(format!(
            "several flats ({}); choose one with SJEL_INTERIOR_FLAT or --flat",
            all.join(", ")
        ))),
    }
}

impl Model {
    pub fn load(flat: &str) -> Result<Self, ModelError> {
        let root = data_dir()?;
        let flat_dir = root.join("flats").join(flat);
        let mut room: Room = read_toml(&flat_dir.join("room.toml"))?;
        for o in &mut room.openings {
            o.kind = o.kind.take().map(|k| english_opening_kind(&k));
        }
        let rules: Rules = read_toml(&flat_dir.join("rules.toml"))?;

        // The catalogue comes from the store (SJEL_DB_PATH, else the overlay). An empty one is no error
        // here; it fails where a layout names a missing piece, with the better message.
        let store = crate::store::Store::open(&sjel_config::database_path())
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

    /// Everything `wanted`, in catalogue order.
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

    /// Every catalogue piece with unmeasured sizes; carried into every report.
    pub fn uncertainties(&self) -> Vec<(&str, &str, &[String])> {
        let mut v: Vec<_> = self
            .catalogue
            .values()
            .filter(|i| i.is_uncertain())
            .map(|i| (i.id.as_str(), i.label.as_str(), i.estimated.as_slice()))
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

    /// A file that exists and does not parse is an error, not a guess.
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
