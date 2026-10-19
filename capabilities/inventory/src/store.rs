//! The inventory as rows in the one shared SQLite file.
//!
//! State is its own table: a wish that gets bought is a second row, not an overwritten field, and
//! that span is what joins the wishlist to finance.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A side of a piece, in the piece's own orientation. `interior` reads the same text from the
/// column, so the stored word is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[serde(alias = "nord")]
    North,
    #[serde(alias = "sued")]
    South,
    #[serde(alias = "ost")]
    East,
    West,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::North => "north",
            Side::South => "south",
            Side::East => "east",
            Side::West => "west",
        }
    }

    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "north" | "nord" => Some(Side::North),
            "south" | "sued" => Some(Side::South),
            "east" | "ost" => Some(Side::East),
            "west" => Some(Side::West),
            _ => None,
        }
    }

    /// The same side after the piece is rotated clockwise by `rot` degrees (y grows south).
    /// Only multiples of 90 move it, matching `footprint`, which swaps sizes only near 90°.
    pub fn rotated(self, rot: i32) -> Side {
        const CLOCKWISE: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];
        if rot.rem_euclid(90) != 0 {
            return self;
        }
        let steps = (rot.rem_euclid(360) / 90) as usize;
        let i = CLOCKWISE.iter().position(|s| *s == self).unwrap_or(0);
        CLOCKWISE[(i + steps) % 4]
    }
}

/// Field and column names of the German schema, and their English names. One table for both,
/// because a column and its JSON field share a name. Old databases are renamed in place, and old
/// clients (a phone with a queued edit, an overlay TOML file) are still understood.
pub const LEGACY_FIELDS: [(&str, &str); 33] = [
    ("b", "width"),
    ("t", "depth"),
    ("h", "height"),
    ("h_min", "height_min"),
    ("b_aufgeklappt", "width_unfolded"),
    ("t_ausgeklappt", "depth_unfolded"),
    ("laenge", "length"),
    ("anzahl", "count"),
    ("zustaende", "fold_states"),
    ("unsicher", "estimated"),
    ("platzbedarf_zone", "clearance_zone"),
    ("platzbedarf_block", "clearance_block"),
    ("preis_cent", "price_cents"),
    ("kosten_min_cent", "cost_min_cents"),
    ("kosten_max_cent", "cost_max_cents"),
    ("artikelnummer", "sku"),
    ("quelle", "source"),
    ("gemessen_am", "measured_at"),
    ("mitnahme", "on_move"),
    ("prioritaet", "priority"),
    ("basiert_auf", "based_on"),
    ("ersetzt", "replaces"),
    ("varianten", "variants"),
    ("ziel", "target"),
    ("hinweis", "notes"),
    ("begruendung", "reason"),
    ("entscheidung_offen", "open_question"),
    ("raumtrenner", "room_divider"),
    ("zerlegbar", "disassembles"),
    ("bild", "image"),
    ("groesse", "size"),
    ("farbe", "color"),
    ("saison", "seasons"),
];

/// Stored words of the German vocabulary and their English replacements, per column.
const LEGACY_VALUES: [(&str, &str, &str); 15] = [
    ("opens", "nord", "north"),
    ("opens", "sued", "south"),
    ("opens", "ost", "east"),
    ("expands_dir", "nord", "north"),
    ("expands_dir", "sued", "south"),
    ("expands_dir", "ost", "east"),
    ("priority", "gesetzt", "decided"),
    ("priority", "pflicht", "required"),
    ("priority", "empfehlung", "recommended"),
    ("priority", "kandidat", "candidate"),
    ("priority", "konzept", "concept"),
    ("priority", "zurueckgestellt", "deferred"),
    ("priority", "verworfen", "rejected"),
    ("priority", "ersetzt", "replaced"),
    ("on_move", "offen", "undecided"),
];

/// Field names in an `estimated` list, in English. `modell` (the product model is unknown) is
/// the one word there that is no field.
pub fn english_estimated(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .map(|n| match n.as_str() {
            "modell" => "model".to_string(),
            _ => LEGACY_FIELDS
                .iter()
                .find(|(old, _)| *old == n)
                .map_or(n.clone(), |(_, new)| (*new).to_string()),
        })
        .collect()
}

/// The English word for a legacy priority or on-move word; anything else is returned unchanged.
pub fn english_value(column: &str, value: &str) -> String {
    LEGACY_VALUES
        .iter()
        .find(|(c, old, _)| *c == column && *old == value)
        .map_or_else(|| value.to_string(), |(_, _, new)| (*new).to_string())
}

/// Every column of `{prefix}_item`, in the order `widen_kind_check` recreates them. Spelled out so
/// a copy cannot silently shift values into the wrong column.
const ITEM_COLUMNS: [&str; 54] = [
    "id",
    "kind",
    "label",
    "width",
    "depth",
    "height",
    "height_min",
    "width_unfolded",
    "depth_unfolded",
    "length",
    "count",
    "fold_states",
    "estimated",
    "clearance_zone",
    "clearance_block",
    "price_cents",
    "cost_min_cents",
    "cost_max_cents",
    "link",
    "sku",
    "source",
    "measured_at",
    "on_move",
    "priority",
    "based_on",
    "replaces",
    "variants",
    "target",
    "notes",
    "reason",
    "open_question",
    "opens",
    "open_clear",
    "wall_ok",
    "expands_dir",
    "expands_to",
    "access_sides",
    "access_clear",
    "room_divider",
    "disassembles",
    "image",
    "weight_g",
    "category",
    "packable",
    "waterproof",
    "quick_dry",
    "pack_location",
    "trip_types",
    "size",
    "color",
    "seasons",
    "traits",
    "created_at",
    "updated_at",
];

/// One thing, owned or wanted. Lengths are in cm, money in cents.
///
/// The contract for `POST /api/items`: `id`, `kind` and `label` are required, everything else is
/// optional.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    /// `piece` is a thing, `slot` is a need with target sizes and no product yet.
    pub kind: Kind,
    pub label: String,
    #[serde(alias = "b")]
    pub width: Option<i32>,
    #[serde(alias = "t")]
    pub depth: Option<i32>,
    #[serde(alias = "h")]
    pub height: Option<i32>,
    #[serde(alias = "h_min")]
    pub height_min: Option<i32>,
    #[serde(alias = "b_aufgeklappt")]
    pub width_unfolded: Option<i32>,
    #[serde(alias = "t_ausgeklappt")]
    pub depth_unfolded: Option<i32>,
    #[serde(alias = "laenge")]
    pub length: Option<i32>,
    #[serde(alias = "anzahl")]
    pub count: Option<i32>,
    /// Named states of a folding piece, e.g. `["closed", "unfolded"]`.
    #[serde(default, alias = "zustaende")]
    pub fold_states: Vec<String>,
    /// Which dimensions are guesses. Carried into every report that uses them.
    #[serde(default, alias = "unsicher")]
    pub estimated: Vec<String>,
    #[serde(alias = "platzbedarf_zone")]
    pub clearance_zone: Option<i32>,
    #[serde(alias = "platzbedarf_block")]
    pub clearance_block: Option<i32>,
    #[serde(alias = "preis_cent")]
    pub price_cents: Option<i64>,
    #[serde(alias = "kosten_min_cent")]
    pub cost_min_cents: Option<i64>,
    #[serde(alias = "kosten_max_cent")]
    pub cost_max_cents: Option<i64>,
    pub link: Option<String>,
    #[serde(alias = "artikelnummer")]
    pub sku: Option<String>,
    /// Where the measurements came from.
    #[serde(alias = "quelle")]
    pub source: Option<String>,
    #[serde(alias = "gemessen_am")]
    pub measured_at: Option<String>,
    /// What happens to the piece on a move: `bring`, `leave`, `undecided`.
    #[serde(alias = "mitnahme")]
    pub on_move: Option<String>,
    /// Urgency of a need: `decided`, `required`, `recommended`, `candidate`, `concept`,
    /// `deferred`, `rejected`, `replaced`.
    #[serde(alias = "prioritaet")]
    pub priority: Option<String>,
    /// A slot derived from another entry.
    #[serde(alias = "basiert_auf")]
    pub based_on: Option<String>,
    /// Entries this product makes redundant.
    #[serde(default, alias = "ersetzt")]
    pub replaces: Vec<String>,
    #[serde(default, alias = "varianten")]
    pub variants: Vec<String>,
    #[serde(alias = "ziel")]
    pub target: Option<String>,
    #[serde(alias = "hinweis")]
    pub notes: Option<String>,
    #[serde(alias = "begruendung")]
    pub reason: Option<String>,
    #[serde(alias = "entscheidung_offen")]
    pub open_question: Option<String>,

    // What the piece needs around it. All optional: an empty field leaves the name heuristic in
    // charge, so no verdict changes before someone checked the numbers.
    /// The side that needs doors or drawers opened.
    pub opens: Option<Side>,
    /// Clearance in front of that side, in cm. Without `opens` it applies to the best side.
    pub open_clear: Option<i32>,
    /// Whether the opening side may face a wall.
    pub wall_ok: Option<bool>,
    /// A second, unfolded state: the side, and the total depth unfolded (not the increase).
    pub expands_dir: Option<Side>,
    pub expands_to: Option<i32>,
    /// How many sides must stay walkable, and how deep each must be.
    pub access_sides: Option<i32>,
    pub access_clear: Option<i32>,
    /// A path below the private asset root, served by `interior` on request only.
    #[serde(alias = "bild")]
    pub image: Option<String>,
    /// Meant to stand free and divide the room. Affects only search ranking, never a verdict.
    #[serde(alias = "raumtrenner")]
    pub room_divider: Option<bool>,
    /// Comes apart for carrying, so the door check does not apply to its assembled size.
    #[serde(alias = "zerlegbar")]
    pub disassembles: Option<bool>,

    // Gear, read by trips' packing lists.
    /// Grams, so a packing list adds integers.
    pub weight_g: Option<i64>,
    /// Free text, e.g. `kleidung`, `elektronik`; nothing computes on it.
    pub category: Option<String>,
    pub packable: Option<bool>,
    pub waterproof: Option<bool>,
    pub quick_dry: Option<bool>,
    pub pack_location: Option<String>,
    /// Trip kinds it suits, e.g. `["hiking", "city"]`. Empty means all.
    #[serde(default)]
    pub trip_types: Vec<String>,

    // Clothing. Free text, as printed on the label.
    #[serde(alias = "groesse")]
    pub size: Option<String>,
    #[serde(alias = "farbe")]
    pub color: Option<String>,
    #[serde(default, alias = "saison")]
    pub seasons: Vec<String>,
    /// What it does on a trip: `style`, `rain`, `warm`, …
    #[serde(default)]
    pub traits: Vec<String>,

    /// Owned by the server and raised by every write. A client that wants a conditional write
    /// sends it back as `If-Match`.
    #[serde(default)]
    pub revision: i64,
}

impl Item {
    pub fn is_uncertain(&self) -> bool {
        !self.estimated.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Piece,
    Slot,
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
            // An unknown word as gear would drop it from every furniture check.
            _ => Kind::Piece,
        }
    }
}

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

/// Columns a write sets, in the order of [`write_params`]. `revision`, `created_at` and
/// `updated_at` belong to the server.
const WRITE_COLUMNS: [&str; 52] = [
    "id",
    "kind",
    "label",
    "width",
    "depth",
    "height",
    "height_min",
    "width_unfolded",
    "depth_unfolded",
    "length",
    "count",
    "fold_states",
    "estimated",
    "clearance_zone",
    "clearance_block",
    "price_cents",
    "cost_min_cents",
    "cost_max_cents",
    "link",
    "sku",
    "source",
    "measured_at",
    "on_move",
    "priority",
    "based_on",
    "replaces",
    "variants",
    "target",
    "notes",
    "reason",
    "open_question",
    "opens",
    "open_clear",
    "wall_ok",
    "expands_dir",
    "expands_to",
    "access_sides",
    "access_clear",
    "room_divider",
    "image",
    "disassembles",
    "weight_g",
    "category",
    "packable",
    "waterproof",
    "quick_dry",
    "pack_location",
    "trip_types",
    "size",
    "color",
    "seasons",
    "traits",
];

fn write_params(it: &Item) -> Result<Vec<Box<dyn rusqlite::ToSql>>, Error> {
    Ok(vec![
        Box::new(it.id.clone()),
        Box::new(it.kind.as_str()),
        Box::new(it.label.clone()),
        Box::new(it.width),
        Box::new(it.depth),
        Box::new(it.height),
        Box::new(it.height_min),
        Box::new(it.width_unfolded),
        Box::new(it.depth_unfolded),
        Box::new(it.length),
        Box::new(it.count),
        Box::new(serde_json::to_string(&it.fold_states)?),
        Box::new(serde_json::to_string(&it.estimated)?),
        Box::new(it.clearance_zone),
        Box::new(it.clearance_block),
        Box::new(it.price_cents),
        Box::new(it.cost_min_cents),
        Box::new(it.cost_max_cents),
        Box::new(it.link.clone()),
        Box::new(it.sku.clone()),
        Box::new(it.source.clone()),
        Box::new(it.measured_at.clone()),
        Box::new(it.on_move.clone()),
        Box::new(it.priority.clone()),
        Box::new(it.based_on.clone()),
        Box::new(serde_json::to_string(&it.replaces)?),
        Box::new(serde_json::to_string(&it.variants)?),
        Box::new(it.target.clone()),
        Box::new(it.notes.clone()),
        Box::new(it.reason.clone()),
        Box::new(it.open_question.clone()),
        Box::new(it.opens.map(|s| s.as_str())),
        Box::new(it.open_clear),
        Box::new(it.wall_ok),
        Box::new(it.expands_dir.map(|s| s.as_str())),
        Box::new(it.expands_to),
        Box::new(it.access_sides),
        Box::new(it.access_clear),
        Box::new(it.room_divider),
        Box::new(it.image.clone()),
        Box::new(it.disassembles),
        Box::new(it.weight_g),
        Box::new(it.category.clone()),
        Box::new(it.packable),
        Box::new(it.waterproof),
        Box::new(it.quick_dry),
        Box::new(it.pack_location.clone()),
        Box::new(serde_json::to_string(&it.trip_types)?),
        Box::new(it.size.clone()),
        Box::new(it.color.clone()),
        Box::new(serde_json::to_string(&it.seasons)?),
        Box::new(serde_json::to_string(&it.traits)?),
    ])
}

/// The result of a write against an expected revision.
#[derive(Debug)]
pub enum WriteOutcome {
    /// Written; the row now carries this revision.
    Written(i64),
    Missing,
    /// Someone wrote in between. The current row, so the caller can show it instead of
    /// overwriting it.
    Stale(Box<Item>, Option<State>),
}

/// A group of wishes sharing a common constraint or goal (e.g. "travel-friendly, fewer but better").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WishGroup {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// A wish group viewed with its member items and aggregate costs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WishGroupView {
    pub id: String,
    pub title: String,
    pub constraint_text: String,
    pub created_at: String,
    pub updated_at: String,
    pub items: Vec<Item>,
    pub items_count: usize,
    pub total_low_cents: i64,
    pub total_high_cents: i64,
}

pub struct Store {
    pool: sjel_store::Pool,
    prefix: String,
}

type Error = Box<dyn std::error::Error>;

/// The prefix is interpolated into DDL, so it is validated instead of bound.
fn validate_prefix(prefix: &str) -> Result<(), Error> {
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
    pub fn open(database_path: &Path) -> Result<Self, Error> {
        Self::open_with_prefix(database_path, "inventory")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Result<Self, Error> {
        validate_prefix(prefix)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    fn conn(&self) -> Result<sjel_store::PooledClient, Error> {
        Ok(self.pool.get()?)
    }

    /// A connection for a query beyond this prefix: `budget::monthly_balance` reads finance.
    pub fn borrow_connection(&self) -> Result<sjel_store::PooledClient, Error> {
        self.conn()
    }

    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Error> {
        // Before anything else: an old file has German columns, and `CREATE TABLE IF NOT EXISTS`
        // and `add_column_if_missing` would otherwise add empty English ones beside them.
        Self::rename_legacy_columns(conn, &format!("{prefix}_item"))?;
        Self::rename_legacy_columns(conn, "interior_item")?;
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_item (
                id                 TEXT PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN ('piece','slot','gear')),
                label              TEXT NOT NULL,
                width              INTEGER,
                depth              INTEGER,
                height             INTEGER,
                height_min         INTEGER,
                width_unfolded     INTEGER,
                depth_unfolded     INTEGER,
                length             INTEGER,
                count              INTEGER,
                fold_states        TEXT NOT NULL DEFAULT '[]',
                estimated          TEXT NOT NULL DEFAULT '[]',
                clearance_zone     INTEGER,
                clearance_block    INTEGER,
                price_cents        INTEGER,
                cost_min_cents     INTEGER,
                cost_max_cents     INTEGER,
                link               TEXT,
                sku                TEXT,
                source             TEXT,
                measured_at        TEXT,
                on_move            TEXT,
                priority           TEXT,
                based_on           TEXT,
                replaces           TEXT NOT NULL DEFAULT '[]',
                variants           TEXT NOT NULL DEFAULT '[]',
                target             TEXT,
                notes              TEXT,
                reason             TEXT,
                open_question      TEXT,
                opens              TEXT,
                open_clear         INTEGER,
                wall_ok            INTEGER,
                expands_dir        TEXT,
                expands_to         INTEGER,
                access_sides       INTEGER,
                access_clear       INTEGER,
                room_divider       INTEGER,
                disassembles       INTEGER,
                image              TEXT,
                weight_g           INTEGER,
                category           TEXT,
                packable           INTEGER,
                waterproof         INTEGER,
                quick_dry          INTEGER,
                pack_location      TEXT,
                trip_types         TEXT NOT NULL DEFAULT '[]',
                size               TEXT,
                color              TEXT,
                seasons            TEXT NOT NULL DEFAULT '[]',
                traits             TEXT NOT NULL DEFAULT '[]',
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
            CREATE INDEX IF NOT EXISTS {prefix}_idx_state_item
                ON {prefix}_item_state(item_id, since DESC);
            CREATE TABLE IF NOT EXISTS {prefix}_sync_operation (
                operation_id TEXT PRIMARY KEY,
                revision INTEGER NOT NULL,
                processed_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {prefix}_wish_group (
                id              TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                constraint_text TEXT NOT NULL DEFAULT '',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS {prefix}_wish_group_item (
                group_id TEXT NOT NULL REFERENCES {prefix}_wish_group(id) ON DELETE CASCADE,
                item_id  TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
                PRIMARY KEY (group_id, item_id)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_wish_group_item
                ON {prefix}_wish_group_item(item_id);
            ",
            prefix = prefix
        ))?;
        // Columns added after the first schema, oldest first. All before `widen_kind_check`,
        // which copies `ITEM_COLUMNS` and needs each of them to exist.
        for (column, typ) in [
            ("room_divider", "INTEGER"),
            ("disassembles", "INTEGER"),
            ("image", "TEXT"),
            ("weight_g", "INTEGER"),
            ("category", "TEXT"),
            ("packable", "INTEGER"),
            ("waterproof", "INTEGER"),
            ("quick_dry", "INTEGER"),
            ("pack_location", "TEXT"),
            ("trip_types", "TEXT NOT NULL DEFAULT '[]'"),
            ("size", "TEXT"),
            ("color", "TEXT"),
            ("seasons", "TEXT NOT NULL DEFAULT '[]'"),
            ("traits", "TEXT NOT NULL DEFAULT '[]'"),
        ] {
            Self::add_column_if_missing(conn, prefix, column, typ)?;
        }
        Self::widen_kind_check(conn, prefix)?;
        // After the rebuild, which does not copy it.
        Self::add_column_if_missing(conn, prefix, "revision", "INTEGER NOT NULL DEFAULT 1")?;
        Self::move_from_interior(conn)?;
        Self::translate_legacy_values(conn, prefix)?;
        Ok(())
    }

    /// German column names to English, in place, so ids, revisions and state history survive.
    fn rename_legacy_columns(conn: &Connection, table: &str) -> Result<(), Error> {
        let existing = Self::columns_of(conn, table)?;
        for (old, new) in LEGACY_FIELDS {
            if existing.iter().any(|c| c == old) && !existing.iter().any(|c| c == new) {
                conn.execute_batch(&format!(
                    "ALTER TABLE {table} RENAME COLUMN {old} TO {new};"
                ))?;
            }
        }
        Ok(())
    }

    /// German vocabulary words in stored values to English. A no-op once translated.
    fn translate_legacy_values(conn: &Connection, prefix: &str) -> Result<(), Error> {
        for (column, old, new) in LEGACY_VALUES {
            conn.execute(
                &format!("UPDATE {prefix}_item SET {column} = ?2 WHERE {column} = ?1"),
                params![old, new],
            )?;
        }
        // `estimated` lists field names, so it follows the field rename.
        let rows: Vec<(String, String)> = conn
            .prepare(&format!(
                "SELECT id, estimated FROM {prefix}_item WHERE estimated <> '[]'"
            ))?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, raw) in rows {
            let names: Vec<String> = serde_json::from_str(&raw)?;
            let english = english_estimated(names.clone());
            if english != names {
                conn.execute(
                    &format!("UPDATE {prefix}_item SET estimated = ?2 WHERE id = ?1"),
                    params![id, serde_json::to_string(&english)?],
                )?;
            }
        }
        Ok(())
    }

    /// One-time move of the rows out of `interior_item`, where they lived before inventory was
    /// its own capability. The old table is dropped afterwards so there is one truth.
    ///
    /// `interior_placement` loses its foreign key first: it pointed at `interior_item` with
    /// `ON DELETE CASCADE`, and dropping the parent would delete every placement.
    fn move_from_interior(conn: &Connection) -> Result<(), Error> {
        let old: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'interior_item'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if old.is_none() {
            return Ok(());
        }

        Self::detach_placement_from_interior(conn)?;

        // Only the columns both tables have; an older file lacks some, and the rest default.
        let present = Self::columns_of(conn, "interior_item")?;
        let mut columns: Vec<&str> = ITEM_COLUMNS
            .iter()
            .copied()
            .filter(|c| present.iter().any(|v| v == c))
            .collect();
        if present.iter().any(|v| v == "revision") {
            columns.push("revision");
        }
        let list = columns.join(", ");
        conn.execute_batch(&format!(
            "INSERT INTO inventory_item ({list}) SELECT {list} FROM interior_item;
             INSERT INTO inventory_item_state (item_id, state, since, note)
                 SELECT item_id, state, since, note FROM interior_item_state;
             DROP TABLE interior_item_state;
             DROP TABLE interior_item;"
        ))?;
        Ok(())
    }

    fn detach_placement_from_interior(conn: &Connection) -> Result<(), Error> {
        let ddl: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'interior_placement'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(ddl) = ddl else { return Ok(()) };
        if !ddl.contains("REFERENCES interior_item") {
            return Ok(());
        }
        conn.execute_batch(
            "CREATE TABLE interior_placement_new (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                flat    TEXT NOT NULL,
                x       INTEGER NOT NULL,
                y       INTEGER NOT NULL,
                rot     INTEGER NOT NULL DEFAULT 0,
                since   TEXT NOT NULL,
                UNIQUE (item_id, flat)
             );
             INSERT INTO interior_placement_new (id, item_id, flat, x, y, rot, since)
                 SELECT id, item_id, flat, x, y, rot, since FROM interior_placement;
             DROP TABLE interior_placement;
             ALTER TABLE interior_placement_new RENAME TO interior_placement;
             CREATE INDEX IF NOT EXISTS interior_idx_placement_flat ON interior_placement(flat);",
        )?;
        Ok(())
    }

    /// Column names of a table in table order; empty when the table does not exist.
    fn columns_of(conn: &Connection, table: &str) -> Result<Vec<String>, Error> {
        let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
        let rows = stmt.query_map(params![table], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// `ALTER TABLE ... ADD COLUMN`, idempotent, because SQLite has no `IF NOT EXISTS` for it.
    fn add_column_if_missing(
        conn: &Connection,
        prefix: &str,
        column: &str,
        typ: &str,
    ) -> Result<(), Error> {
        let present: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![format!("{prefix}_item"), column],
            |row| row.get(0),
        )?;
        if present == 0 {
            conn.execute_batch(&format!(
                "ALTER TABLE {prefix}_item ADD COLUMN {column} {typ};"
            ))?;
        }
        Ok(())
    }

    /// Widens the `kind` CHECK to include `gear` by rebuilding the table, the only way SQLite
    /// allows. Runs only while the stored DDL lacks `gear`.
    ///
    /// The state rows are copied out and back: they cascade on the item table, and
    /// `PRAGMA foreign_keys = off` has no effect inside the migration's transaction
    /// (libs/sjel-store/src/lib.rs). A failure anywhere rolls the whole rebuild back.
    fn widen_kind_check(conn: &Connection, prefix: &str) -> Result<(), Error> {
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
            "CREATE TABLE {prefix}_item_new (
                id                 TEXT PRIMARY KEY,
                kind               TEXT NOT NULL CHECK (kind IN ('piece','slot','gear')),
                label              TEXT NOT NULL,
                width              INTEGER,
                depth              INTEGER,
                height             INTEGER,
                height_min         INTEGER,
                width_unfolded     INTEGER,
                depth_unfolded     INTEGER,
                length             INTEGER,
                count              INTEGER,
                fold_states        TEXT NOT NULL DEFAULT '[]',
                estimated          TEXT NOT NULL DEFAULT '[]',
                clearance_zone     INTEGER,
                clearance_block    INTEGER,
                price_cents        INTEGER,
                cost_min_cents     INTEGER,
                cost_max_cents     INTEGER,
                link               TEXT,
                sku                TEXT,
                source             TEXT,
                measured_at        TEXT,
                on_move            TEXT,
                priority           TEXT,
                based_on           TEXT,
                replaces           TEXT NOT NULL DEFAULT '[]',
                variants           TEXT NOT NULL DEFAULT '[]',
                target             TEXT,
                notes              TEXT,
                reason             TEXT,
                open_question      TEXT,
                opens              TEXT,
                open_clear         INTEGER,
                wall_ok            INTEGER,
                expands_dir        TEXT,
                expands_to         INTEGER,
                access_sides       INTEGER,
                access_clear       INTEGER,
                room_divider       INTEGER,
                disassembles       INTEGER,
                image              TEXT,
                weight_g           INTEGER,
                category           TEXT,
                packable           INTEGER,
                waterproof         INTEGER,
                quick_dry          INTEGER,
                pack_location      TEXT,
                trip_types         TEXT NOT NULL DEFAULT '[]',
                size               TEXT,
                color              TEXT,
                seasons            TEXT NOT NULL DEFAULT '[]',
                traits             TEXT NOT NULL DEFAULT '[]',
                created_at         TEXT NOT NULL,
                updated_at         TEXT NOT NULL
             );
             INSERT INTO {prefix}_item_new ({columns})
                 SELECT {columns} FROM {prefix}_item;
             CREATE TABLE {prefix}_state_backup AS SELECT * FROM {prefix}_item_state;
             DROP TABLE {prefix}_item;
             ALTER TABLE {prefix}_item_new RENAME TO {prefix}_item;
             INSERT INTO {prefix}_item_state SELECT * FROM {prefix}_state_backup;
             DROP TABLE {prefix}_state_backup;"
        ))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Error> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// Insert or update; the later writer wins. `created_at` survives an update. Returns the
    /// revision the row now carries, raised in the same statement so an older client is refused.
    pub fn upsert_item(&self, it: &Item) -> Result<i64, Error> {
        let p = &self.prefix;
        let now = sjel_store::now_offset("'+0 seconds'");
        let columns = WRITE_COLUMNS.join(", ");
        let placeholders = (1..=WRITE_COLUMNS.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let take = WRITE_COLUMNS[1..]
            .iter()
            .map(|c| format!("{c}=excluded.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let conn = self.conn()?;
        let revision = conn.query_row(
            &format!(
                "INSERT INTO {p}_item ({columns}, revision, created_at, updated_at)
                 VALUES ({placeholders}, 1, {now}, {now})
                 ON CONFLICT(id) DO UPDATE SET {take},
                    revision = {p}_item.revision + 1, updated_at = {now}
                 RETURNING revision"
            ),
            rusqlite::params_from_iter(write_params(it)?.iter()),
            |row| row.get(0),
        )?;
        Ok(revision)
    }

    /// Overwrite only while the row still carries the revision the caller read.
    ///
    /// Compare and write are one statement, so there is no window in which a second writer wins
    /// silently. When it matches no row, a read afterwards tells missing from stale.
    pub fn update_item_if_revision(&self, it: &Item, expected: i64) -> Result<WriteOutcome, Error> {
        let p = &self.prefix;
        let now = sjel_store::now_offset("'+0 seconds'");
        let set = WRITE_COLUMNS
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, c)| format!("{c} = ?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut values = write_params(it)?;
        values.push(Box::new(expected));
        let conn = self.conn()?;
        let written: Option<i64> = conn
            .query_row(
                &format!(
                    "UPDATE {p}_item SET {set}, revision = revision + 1, updated_at = {now}
                     WHERE id = ?1 AND revision = ?{n}
                     RETURNING revision",
                    n = WRITE_COLUMNS.len() + 1
                ),
                rusqlite::params_from_iter(values.iter()),
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        if let Some(revision) = written {
            return Ok(WriteOutcome::Written(revision));
        }
        Ok(match self.item(&it.id)? {
            Some((current, state)) => WriteOutcome::Stale(Box::new(current), state),
            None => WriteOutcome::Missing,
        })
    }

    /// Append a state change. Writes nothing when the current state is already `state`, so a
    /// repeated import does not invent history.
    pub fn record_state(
        &self,
        item_id: &str,
        state: State,
        note: Option<&str>,
    ) -> Result<bool, Error> {
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

    /// The revision recorded for a sync operation that was already applied.
    pub fn sync_operation_revision(&self, operation_id: &str) -> Result<Option<i64>, Error> {
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
    pub fn record_sync_operation(&self, operation_id: &str, revision: i64) -> Result<(), Error> {
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

    pub fn item_count(&self) -> Result<i64, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {p}_item"), [], |r| r.get(0))?)
    }

    pub fn item(&self, id: &str) -> Result<Option<(Item, Option<State>)>, Error> {
        Ok(self.catalogue()?.remove(id))
    }

    /// The whole state history of an entry, oldest first.
    pub fn state_history(
        &self,
        item_id: &str,
    ) -> Result<Vec<(State, String, Option<String>)>, Error> {
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

    pub fn current_state(&self, item_id: &str) -> Result<Option<State>, Error> {
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

    /// Every item with its current state.
    pub fn catalogue(&self) -> Result<BTreeMap<String, (Item, Option<State>)>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT i.id, i.kind, i.label, i.width, i.depth, i.height, i.height_min,
                    i.width_unfolded, i.depth_unfolded, i.length, i.count, i.fold_states,
                    i.estimated, i.clearance_zone, i.clearance_block, i.price_cents,
                    i.cost_min_cents, i.cost_max_cents, i.link, i.sku, i.source, i.measured_at,
                    i.on_move, i.priority, i.based_on, i.replaces, i.variants, i.target,
                    i.notes, i.reason, i.open_question,
                    i.opens, i.open_clear, i.wall_ok, i.expands_dir, i.expands_to,
                    i.access_sides, i.access_clear, i.room_divider, i.image, i.disassembles,
                    i.weight_g, i.category, i.packable, i.waterproof, i.quick_dry,
                    i.pack_location, i.trip_types, i.size, i.color, i.seasons, i.traits, i.revision,
                    (SELECT s.state FROM {p}_item_state s
                      WHERE s.item_id = i.id ORDER BY s.since DESC, s.id DESC LIMIT 1)
             FROM {p}_item i ORDER BY i.id"
        ))?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(1)?;
            let side = |i: usize| -> rusqlite::Result<Option<Side>> {
                Ok(row
                    .get::<_, Option<String>>(i)?
                    .as_deref()
                    .and_then(Side::parse))
            };
            let state: Option<String> = row.get(53)?;
            Ok((
                Item {
                    id: row.get(0)?,
                    kind: Kind::parse(&kind),
                    label: row.get(2)?,
                    width: row.get(3)?,
                    depth: row.get(4)?,
                    height: row.get(5)?,
                    height_min: row.get(6)?,
                    width_unfolded: row.get(7)?,
                    depth_unfolded: row.get(8)?,
                    length: row.get(9)?,
                    count: row.get(10)?,
                    fold_states: sjel_store::json_column(row, 11)?,
                    estimated: sjel_store::json_column(row, 12)?,
                    clearance_zone: row.get(13)?,
                    clearance_block: row.get(14)?,
                    price_cents: row.get(15)?,
                    cost_min_cents: row.get(16)?,
                    cost_max_cents: row.get(17)?,
                    link: row.get(18)?,
                    sku: row.get(19)?,
                    source: row.get(20)?,
                    measured_at: row.get(21)?,
                    on_move: row.get(22)?,
                    priority: row.get(23)?,
                    based_on: row.get(24)?,
                    replaces: sjel_store::json_column(row, 25)?,
                    variants: sjel_store::json_column(row, 26)?,
                    target: row.get(27)?,
                    notes: row.get(28)?,
                    reason: row.get(29)?,
                    open_question: row.get(30)?,
                    opens: side(31)?,
                    open_clear: row.get(32)?,
                    wall_ok: row.get(33)?,
                    expands_dir: side(34)?,
                    expands_to: row.get(35)?,
                    access_sides: row.get(36)?,
                    access_clear: row.get(37)?,
                    room_divider: row.get(38)?,
                    image: row.get(39)?,
                    disassembles: row.get(40)?,
                    weight_g: row.get(41)?,
                    category: row.get(42)?,
                    packable: row.get(43)?,
                    waterproof: row.get(44)?,
                    quick_dry: row.get(45)?,
                    pack_location: row.get(46)?,
                    trip_types: sjel_store::json_column(row, 47)?,
                    size: row.get(48)?,
                    color: row.get(49)?,
                    seasons: sjel_store::json_column(row, 50)?,
                    traits: sjel_store::json_column(row, 51)?,
                    revision: row.get(52)?,
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

    /// The categories that occur, with counts. Read from the rows, never declared, so a new
    /// category needs no code change. Empty and whitespace-only values do not count.
    pub fn categories(&self) -> Result<Vec<(String, i64)>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT TRIM(category), COUNT(*) FROM {p}_item
             WHERE category IS NOT NULL AND TRIM(category) <> ''
             GROUP BY TRIM(category)
             ORDER BY COUNT(*) DESC, TRIM(category) ASC"
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Create or update a wish group with its title and constraint.
    pub fn create_or_update_wish_group(
        &self,
        id: &str,
        title: &str,
        constraint_text: &str,
    ) -> Result<WishGroup, Error> {
        let p = &self.prefix;
        let now = sjel_store::now_offset("'+0 seconds'");
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_wish_group (id, title, constraint_text, created_at, updated_at)
                 VALUES (?1, ?2, ?3, {now}, {now})
                 ON CONFLICT(id) DO UPDATE SET
                    title = excluded.title,
                    constraint_text = excluded.constraint_text,
                    updated_at = {now}"
            ),
            params![id, title, constraint_text],
        )?;
        Ok(WishGroup {
            id: id.to_string(),
            title: title.to_string(),
            constraint_text: constraint_text.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        })
    }

    /// Delete a wish group and its item memberships.
    pub fn delete_wish_group(&self, id: &str) -> Result<bool, Error> {
        let conn = self.conn()?;
        let count = conn.execute(
            &format!("DELETE FROM {}_wish_group WHERE id = ?1", self.prefix),
            params![id],
        )?;
        Ok(count > 0)
    }

    /// Set the items assigned to a wish group.
    pub fn set_wish_group_items(&self, group_id: &str, item_ids: &[String]) -> Result<(), Error> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            &format!("DELETE FROM {}_wish_group_item WHERE group_id = ?1", self.prefix),
            params![group_id],
        )?;
        for item_id in item_ids {
            tx.execute(
                &format!(
                    "INSERT OR IGNORE INTO {}_wish_group_item (group_id, item_id) VALUES (?1, ?2)",
                    self.prefix
                ),
                params![group_id, item_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Return all wish groups with their members and cost totals.
    pub fn wish_groups(&self) -> Result<Vec<WishGroupView>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, title, constraint_text, created_at, updated_at FROM {}_wish_group ORDER BY title",
            self.prefix
        ))?;
        let groups: Vec<WishGroup> = stmt
            .query_map([], |row| {
                Ok(WishGroup {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    constraint_text: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::new();
        for g in groups {
            let mut item_stmt = conn.prepare(&format!(
                "SELECT item_id FROM {}_wish_group_item WHERE group_id = ?1 ORDER BY item_id",
                self.prefix
            ))?;
            let ids: Vec<String> = item_stmt
                .query_map(params![g.id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;

            let mut items = Vec::new();
            for id in &ids {
                if let Some((it, _)) = self.item(id)? {
                    items.push(it);
                }
            }
            let total_low: i64 = items
                .iter()
                .map(|i| i.price_cents.or(i.cost_min_cents).unwrap_or(0))
                .sum();
            let total_high: i64 = items
                .iter()
                .map(|i| i.price_cents.or(i.cost_max_cents).unwrap_or(0))
                .sum();
            let count = items.len();
            out.push(WishGroupView {
                id: g.id,
                title: g.title,
                constraint_text: g.constraint_text,
                created_at: g.created_at,
                updated_at: g.updated_at,
                items,
                items_count: count,
                total_low_cents: total_low,
                total_high_cents: total_high,
            });
        }
        Ok(out)
    }
}

/// Spellings of one category that differ only in case or whitespace, most frequent first.
/// Reported, never normalised: silently lower-casing would change what someone typed.
pub fn collisions(categories: &[(String, i64)]) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    for (name, count) in categories {
        let key = name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        groups.entry(key).or_default().push((name.clone(), *count));
    }
    let mut out: Vec<Vec<String>> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            g.into_iter().map(|(name, _)| name).collect()
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(name: &str, count: i64) -> (String, i64) {
        (name.to_string(), count)
    }

    #[test]
    fn two_spellings_of_one_category_are_reported() {
        let k = vec![
            category("kleidung", 5),
            category("Kleidung", 2),
            category("kleidung ", 1),
            category("schlafen", 3),
        ];
        assert_eq!(
            collisions(&k),
            vec![vec![
                "kleidung".to_string(),
                "Kleidung".to_string(),
                "kleidung ".to_string()
            ]]
        );
    }

    #[test]
    fn a_single_spelling_is_no_collision() {
        assert!(collisions(&[category("kleidung", 5)]).is_empty());
        assert!(collisions(&[]).is_empty());
    }

    #[test]
    fn only_the_same_word_collides() {
        let k = vec![
            category("Kochen", 1),
            category("kochen", 1),
            category("kochen-und-backen", 1),
        ];
        assert_eq!(collisions(&k).len(), 1);
        assert_eq!(collisions(&k)[0], vec!["Kochen", "kochen"]);
    }

    /// A file written by the German schema keeps every value and its history through the rename.
    #[test]
    fn a_german_schema_is_renamed_without_losing_a_value() {
        let path = std::env::temp_dir().join(format!("inventory-legacy-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            // The current schema with every column renamed back: what an old file looks like.
            let conn = Connection::open(&path).unwrap();
            Store::run_migration(&conn, "inventory").unwrap();
            for (old, new) in LEGACY_FIELDS {
                conn.execute_batch(&format!(
                    "ALTER TABLE inventory_item RENAME COLUMN {new} TO {old};"
                ))
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO inventory_item (id, kind, label, b, preis_cent, groesse, prioritaet,
                    expands_dir, mitnahme, saison, unsicher, created_at, updated_at, revision)
                 VALUES ('mantel','piece','Mantel',60,19900,'M','pflicht','sued','offen',
                    '[\"winter\"]','[\"h\",\"groesse\",\"modell\"]','2026-01-01','2026-01-01',4);
                 INSERT INTO inventory_item_state (item_id, state, since) VALUES
                    ('mantel','wanted','2026-01-01');",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let (item, state) = store.item("mantel").unwrap().unwrap();
        assert_eq!(item.width, Some(60));
        assert_eq!(item.price_cents, Some(19900));
        assert_eq!(item.size.as_deref(), Some("M"));
        assert_eq!(item.priority.as_deref(), Some("required"));
        assert_eq!(item.expands_dir, Some(Side::South));
        assert_eq!(item.on_move.as_deref(), Some("undecided"));
        assert_eq!(item.seasons, vec!["winter".to_string()]);
        assert_eq!(item.estimated, vec!["height", "size", "model"]);
        assert_eq!(item.revision, 4);
        assert_eq!(state, Some(State::Wanted));
        let _ = std::fs::remove_file(&path);
    }
}
