//! One-time import of `inventory/*.toml` into the tables. After it, the tables are the truth and
//! the files stay as a record of origin.
//!
//! `deny_unknown_fields` everywhere: an earlier reader silently dropped nine fields for years,
//! and an import that does the same loses them for good. German key names from older files are
//! accepted as aliases.

use crate::store::{Item, Kind, Side, State, Store};
use serde::Deserialize;
use std::path::Path;

type Error = Box<dyn std::error::Error>;

/// One row as written in the files.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    label: String,
    #[serde(default, alias = "b")]
    width: Option<i32>,
    #[serde(default, alias = "t")]
    depth: Option<i32>,
    #[serde(default, alias = "h")]
    height: Option<i32>,
    #[serde(default, alias = "h_min")]
    height_min: Option<i32>,
    #[serde(default, alias = "b_aufgeklappt")]
    width_unfolded: Option<i32>,
    #[serde(default, alias = "t_ausgeklappt")]
    depth_unfolded: Option<i32>,
    #[serde(default, alias = "laenge")]
    length: Option<i32>,
    #[serde(default, alias = "anzahl")]
    count: Option<i32>,
    #[serde(default, alias = "zustaende")]
    fold_states: Vec<String>,
    #[serde(default, alias = "unsicher")]
    estimated: Vec<String>,
    #[serde(default, alias = "platzbedarf_zone")]
    clearance_zone: Option<i32>,
    #[serde(default, alias = "platzbedarf_block")]
    clearance_block: Option<i32>,
    #[serde(default, alias = "preis_eur")]
    price_eur: Option<f64>,
    /// `[min, max]` in euro: an estimate is a range.
    #[serde(default, alias = "kosten_schaetzung_eur")]
    cost_estimate_eur: Option<[f64; 2]>,
    #[serde(default)]
    link: Option<String>,
    /// A number in the files, kept as text: a leading zero would be lost.
    #[serde(default, alias = "artikelnummer")]
    sku: Option<toml::Value>,
    #[serde(default, alias = "quelle")]
    source: Option<String>,
    /// The wishlist file's name for the same thing as `source`; merged into one column.
    #[serde(default, alias = "quelle_masse")]
    measurement_source: Option<String>,
    #[serde(default, alias = "gemessen_am")]
    measured_at: Option<toml::value::Datetime>,
    #[serde(default, alias = "mitnahme")]
    on_move: Option<String>,
    /// Urgency in the files; becomes `priority`.
    #[serde(default, alias = "status")]
    priority: Option<String>,
    #[serde(default, alias = "basiert_auf")]
    based_on: Option<String>,
    #[serde(default, alias = "ersetzt")]
    replaces: Vec<String>,
    #[serde(default, alias = "varianten_rueckwand")]
    variants: Vec<String>,
    #[serde(default, alias = "ziel")]
    target: Option<String>,
    #[serde(default, alias = "hinweis")]
    notes: Option<String>,
    #[serde(default, alias = "begruendung")]
    reason: Option<String>,
    #[serde(default, alias = "entscheidung_offen")]
    open_question: Option<String>,
    #[serde(default)]
    opens: Option<Side>,
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
    #[serde(default, alias = "raumtrenner")]
    room_divider: Option<bool>,
    #[serde(default, alias = "zerlegbar")]
    disassembles: Option<bool>,
    #[serde(default, alias = "bild")]
    image: Option<String>,
}

/// `expands = { dir = "south", to = 145 }`: the total depth unfolded, not the increase.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expands {
    dir: Side,
    to: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedFile {
    #[serde(default)]
    item: Vec<Raw>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WishlistFile {
    #[serde(default)]
    slot: Vec<Raw>,
    #[serde(default, alias = "produkt")]
    product: Vec<Raw>,
}

/// Rounded, not truncated: 49.99 € is 4999 cents, and `as i64` alone gives 4998.
fn cents(eur: f64) -> i64 {
    (eur * 100.0).round() as i64
}

impl Raw {
    fn into_item(self, kind: Kind) -> Item {
        let english =
            |column: &str, v: Option<String>| v.map(|v| crate::store::english_value(column, &v));
        Item {
            id: self.id,
            kind,
            label: self.label,
            width: self.width,
            depth: self.depth,
            height: self.height,
            height_min: self.height_min,
            width_unfolded: self.width_unfolded,
            depth_unfolded: self.depth_unfolded,
            length: self.length,
            count: self.count,
            fold_states: self.fold_states,
            estimated: crate::store::english_estimated(self.estimated),
            clearance_zone: self.clearance_zone,
            clearance_block: self.clearance_block,
            price_cents: self.price_eur.map(cents),
            cost_min_cents: self.cost_estimate_eur.map(|k| cents(k[0])),
            cost_max_cents: self.cost_estimate_eur.map(|k| cents(k[1])),
            link: self.link,
            sku: self.sku.map(|v| match v {
                toml::Value::String(s) => s,
                other => other.to_string(),
            }),
            source: self.source.or(self.measurement_source),
            measured_at: self.measured_at.map(|d| d.to_string()),
            on_move: english("on_move", self.on_move),
            priority: english("priority", self.priority),
            based_on: self.based_on,
            replaces: self.replaces,
            variants: self.variants,
            target: self.target,
            notes: self.notes,
            reason: self.reason,
            open_question: self.open_question,
            opens: self.opens,
            open_clear: self.open_clear,
            wall_ok: self.wall_ok,
            expands_dir: self.expands.as_ref().map(|e| e.dir),
            expands_to: self.expands.as_ref().map(|e| e.to),
            access_sides: self.access_sides,
            access_clear: self.access_clear,
            room_divider: self.room_divider,
            disassembles: self.disassembles,
            image: self.image,
            // These files describe furniture. Gear arrives through `trips gear import`, clothing
            // through `inventory add` and the dashboard.
            ..Item::default()
        }
    }
}

/// What a run did, as numbers, so an import cannot report "ok" having written nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub gear: usize,
    pub pieces: usize,
    pub slots: usize,
    pub state_changes: usize,
}

/// A need another entry replaced is no longer wanted. `priority` means urgency and `state` means
/// ownership; this is the one place one becomes the other.
fn state_for(priority: Option<&str>, default: State) -> State {
    match priority {
        Some("replaced") => State::Gone,
        _ => default,
    }
}

/// Reads `<dir>/owned.toml` and `<dir>/wishlist.toml` into the tables. Repeatable: the same files
/// twice give the same rows and no second state change.
pub fn inventory(store: &Store, dir: &Path) -> Result<Report, Error> {
    let owned: OwnedFile = read(&dir.join("owned.toml"))?;
    let wish: WishlistFile = read(&dir.join("wishlist.toml"))?;
    let mut report = Report::default();

    for (raw, kind, default) in owned
        .item
        .into_iter()
        .map(|r| (r, Kind::Piece, State::Owned))
        .chain(
            wish.slot
                .into_iter()
                .map(|r| (r, Kind::Slot, State::Wanted)),
        )
        .chain(
            wish.product
                .into_iter()
                .map(|r| (r, Kind::Piece, State::Wanted)),
        )
    {
        let item = raw.into_item(kind);
        let state = state_for(item.priority.as_deref(), default);
        store.upsert_item(&item)?;
        if store.record_state(&item.id, state, Some("imported from inventory/*.toml"))? {
            report.state_changes += 1;
        }
        match kind {
            Kind::Piece => report.pieces += 1,
            Kind::Slot => report.slots += 1,
            Kind::Gear => report.gear += 1,
        }
    }
    Ok(report)
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}
