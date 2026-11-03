//! Where pieces stand in a flat, in the shared SQLite file. The items themselves belong to
//! `inventory` and are read through its store, so there is one schema and one migration.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub use sjel_inventory::store::{Item, Kind, Side, State};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Placement {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub since: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlacementHistory {
    pub item_id: String,
    pub flat: String,
    pub x: i32,
    pub y: i32,
    pub rot: i32,
    pub revision: i64,
    pub source_revision_id: Option<String>,
    pub source_observation_id: Option<String>,
    pub validation: Option<serde_json::Value>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoomPlanSpace {
    pub space_id: String,
    pub flat: String,
    pub label: String,
    pub created_at: String,
    #[serde(default)]
    pub revision_ids: Vec<String>,
    #[serde(default)]
    pub alignments: Vec<RoomPlanAlignment>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoomPlanAlignment {
    pub space_id: String,
    pub revision_id: String,
    pub reference_revision_id: String,
    pub report: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoomPlanObjectLink {
    pub space_id: String,
    pub revision_id: String,
    pub observation_id: String,
    pub item_id: String,
    pub created_at: String,
}

pub struct Store {
    pool: sjel_store::Pool,
    prefix: String,
    items: sjel_inventory::store::Store,
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
        Self::open_with_prefix(database_path, "interior")
    }

    /// Opens inventory's store on the same file too: it creates and migrates the item tables, so
    /// `interior check` works on a file inventory never opened.
    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Result<Self, Error> {
        validate_prefix(prefix)?;
        let items = sjel_inventory::store::Store::open(database_path)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
            items,
        })
    }

    fn conn(&self) -> Result<sjel_store::PooledClient, Error> {
        Ok(self.pool.get()?)
    }

    /// A connection for a query beyond this prefix: `budget::monthly_balance` reads finance.
    pub fn borrow_connection(&self) -> Result<sjel_store::PooledClient, Error> {
        self.conn()
    }

    /// `item_id` has no foreign key: the items belong to another capability, and a key across
    /// two prefixes would make one capability's deletes fail on the other's rows.
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Error> {
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
                revision INTEGER NOT NULL DEFAULT 0,
                UNIQUE (item_id, flat)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_flat
                ON {prefix}_placement(flat);
            CREATE TABLE IF NOT EXISTS {prefix}_roomplan_space (
                space_id   TEXT PRIMARY KEY,
                flat       TEXT NOT NULL,
                label      TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_roomplan_space_flat
                ON {prefix}_roomplan_space(flat, created_at);
            CREATE TABLE IF NOT EXISTS {prefix}_roomplan_space_revision (
                space_id   TEXT NOT NULL REFERENCES {prefix}_roomplan_space(space_id) ON DELETE CASCADE,
                revision_id TEXT NOT NULL,
                added_at   TEXT NOT NULL,
                PRIMARY KEY (space_id, revision_id)
            );
            CREATE TABLE IF NOT EXISTS {prefix}_roomplan_alignment (
                space_id             TEXT NOT NULL REFERENCES {prefix}_roomplan_space(space_id) ON DELETE CASCADE,
                revision_id          TEXT NOT NULL,
                reference_revision_id TEXT NOT NULL,
                report_json          TEXT NOT NULL,
                created_at           TEXT NOT NULL,
                PRIMARY KEY (space_id, revision_id)
            );
            CREATE TRIGGER IF NOT EXISTS {prefix}_roomplan_alignment_no_update
                BEFORE UPDATE ON {prefix}_roomplan_alignment
                BEGIN SELECT RAISE(ABORT, 'RoomPlan alignment history is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS {prefix}_roomplan_alignment_no_delete
                BEFORE DELETE ON {prefix}_roomplan_alignment
                BEGIN SELECT RAISE(ABORT, 'RoomPlan alignment history is append-only'); END;
            CREATE TABLE IF NOT EXISTS {prefix}_roomplan_object_link (
                space_id       TEXT NOT NULL REFERENCES {prefix}_roomplan_space(space_id) ON DELETE CASCADE,
                revision_id    TEXT NOT NULL,
                observation_id TEXT NOT NULL,
                item_id        TEXT NOT NULL,
                created_at     TEXT NOT NULL,
                PRIMARY KEY (space_id, revision_id, observation_id),
                UNIQUE (space_id, revision_id, item_id)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_roomplan_link_item
                ON {prefix}_roomplan_object_link(space_id, item_id);
            "
        ))?;
        let has_revision = {
            let mut columns = conn.prepare(&format!("PRAGMA table_info({prefix}_placement)"))?;
            let rows = columns.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|column| column == "revision")
        };
        if !has_revision {
            conn.execute_batch(&format!(
                "ALTER TABLE {prefix}_placement ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;"
            ))?;
        }
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_placement_history (
                id                    INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id               TEXT NOT NULL,
                flat                  TEXT NOT NULL,
                x                     INTEGER NOT NULL,
                y                     INTEGER NOT NULL,
                rot                   INTEGER NOT NULL,
                revision              INTEGER NOT NULL,
                source_revision_id    TEXT,
                source_observation_id TEXT,
                validation_json       TEXT,
                recorded_at           TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_history_item
                ON {prefix}_placement_history(flat, item_id, revision);
            CREATE TRIGGER IF NOT EXISTS {prefix}_placement_history_no_update
                BEFORE UPDATE ON {prefix}_placement_history
                BEGIN SELECT RAISE(ABORT, 'placement history is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS {prefix}_placement_history_no_delete
                BEFORE DELETE ON {prefix}_placement_history
                BEGIN SELECT RAISE(ABORT, 'placement history is append-only'); END;
            INSERT INTO {prefix}_placement_history
                (item_id, flat, x, y, rot, revision, source_revision_id,
                 source_observation_id, recorded_at)
            SELECT p.item_id, p.flat, p.x, p.y, p.rot, p.revision, NULL, NULL, p.since
            FROM {prefix}_placement p
            WHERE NOT EXISTS (
                SELECT 1 FROM {prefix}_placement_history h
                WHERE h.item_id = p.item_id AND h.flat = p.flat AND h.revision = p.revision
            );
            "
        ))?;
        let has_validation = {
            let mut columns =
                conn.prepare(&format!("PRAGMA table_info({prefix}_placement_history)"))?;
            let rows = columns.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|column| column == "validation_json")
        };
        if !has_validation {
            conn.execute_batch(&format!(
                "ALTER TABLE {prefix}_placement_history ADD COLUMN validation_json TEXT;"
            ))?;
        }
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Error> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    pub fn place(&self, pl: &Placement) -> Result<(), Error> {
        self.write_placement(pl, None, None, None, None).map(|_| ())
    }

    pub fn place_if_revision(
        &self,
        pl: &Placement,
        expected_revision: i64,
        source_revision_id: &str,
        source_observation_id: &str,
        validation: Option<&serde_json::Value>,
    ) -> Result<Placement, Error> {
        self.write_placement(
            pl,
            Some(expected_revision),
            Some(source_revision_id),
            Some(source_observation_id),
            validation,
        )
    }

    fn write_placement(
        &self,
        pl: &Placement,
        expected_revision: Option<i64>,
        source_revision_id: Option<&str>,
        source_observation_id: Option<&str>,
        validation: Option<&serde_json::Value>,
    ) -> Result<Placement, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        let changed = if let Some(expected) = expected_revision {
            let updated = tx.execute(
                &format!(
                    "UPDATE {p}_placement SET x = ?3, y = ?4, rot = ?5,
                     revision = revision + 1, since = {now}
                     WHERE item_id = ?1 AND flat = ?2 AND revision = ?6",
                    now = sjel_store::now_offset("'+0 seconds'")
                ),
                params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot, expected],
            )?;
            if updated == 0 && expected == 0 {
                tx.execute(
                    &format!(
                        "INSERT INTO {p}_placement (item_id, flat, x, y, rot, since, revision)
                         SELECT ?1, ?2, ?3, ?4, ?5, {now}, 1
                         WHERE NOT EXISTS (SELECT 1 FROM {p}_placement WHERE item_id = ?1 AND flat = ?2)",
                        now = sjel_store::now_offset("'+0 seconds'")
                    ),
                    params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot],
                )?
            } else {
                updated
            }
        } else {
            tx.execute(
                &format!(
                    "INSERT INTO {p}_placement (item_id, flat, x, y, rot, since, revision)
                     VALUES (?1, ?2, ?3, ?4, ?5, {now}, 1)
                     ON CONFLICT(item_id, flat) DO UPDATE SET
                        x = excluded.x, y = excluded.y, rot = excluded.rot,
                        revision = {p}_placement.revision + 1, since = {now}",
                    now = sjel_store::now_offset("'+0 seconds'")
                ),
                params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot],
            )?
        };
        if changed != 1 {
            let current: Option<i64> = tx
                .query_row(
                    &format!("SELECT revision FROM {p}_placement WHERE item_id = ?1 AND flat = ?2"),
                    params![pl.item_id, pl.flat],
                    |row| row.get(0),
                )
                .optional()?;
            return Err(format!(
                "placement revision conflict: expected {}, current {}",
                expected_revision.unwrap_or_default(),
                current.unwrap_or_default()
            )
            .into());
        }
        let placement = tx.query_row(
            &format!(
                "SELECT item_id, flat, x, y, rot, revision, since FROM {p}_placement
                 WHERE item_id = ?1 AND flat = ?2"
            ),
            params![pl.item_id, pl.flat],
            |row| {
                Ok(Placement {
                    item_id: row.get(0)?,
                    flat: row.get(1)?,
                    x: row.get(2)?,
                    y: row.get(3)?,
                    rot: row.get(4)?,
                    revision: row.get(5)?,
                    since: row.get(6)?,
                })
            },
        )?;
        tx.execute(
            &format!(
                "INSERT INTO {p}_placement_history
                 (item_id, flat, x, y, rot, revision, source_revision_id,
                  source_observation_id, validation_json, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                placement.item_id,
                placement.flat,
                placement.x,
                placement.y,
                placement.rot,
                placement.revision,
                source_revision_id,
                source_observation_id,
                validation.map(serde_json::Value::to_string),
                placement.since
            ],
        )?;
        tx.commit()?;
        Ok(placement)
    }

    pub fn placements(&self, flat: &str) -> Result<Vec<Placement>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT item_id, flat, x, y, rot, revision, since FROM {p}_placement WHERE flat = ?1 ORDER BY item_id"
        ))?;
        let rows = stmt.query_map(params![flat], |row| {
            Ok(Placement {
                item_id: row.get(0)?,
                flat: row.get(1)?,
                x: row.get(2)?,
                y: row.get(3)?,
                rot: row.get(4)?,
                revision: row.get(5)?,
                since: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn placement_history(
        &self,
        flat: &str,
        item_id: &str,
    ) -> Result<Vec<PlacementHistory>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT item_id, flat, x, y, rot, revision, source_revision_id,
                    source_observation_id, validation_json, recorded_at
             FROM {p}_placement_history WHERE flat = ?1 AND item_id = ?2
             ORDER BY revision DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![flat, item_id], |row| {
            Ok(PlacementHistory {
                item_id: row.get(0)?,
                flat: row.get(1)?,
                x: row.get(2)?,
                y: row.get(3)?,
                rot: row.get(4)?,
                revision: row.get(5)?,
                source_revision_id: row.get(6)?,
                source_observation_id: row.get(7)?,
                validation: row
                    .get::<_, Option<String>>(8)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                recorded_at: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_roomplan_space(
        &self,
        space_id: &str,
        flat: &str,
        label: &str,
        revision_id: &str,
    ) -> Result<(), Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO {p}_roomplan_space (space_id, flat, label, created_at)
                 VALUES (?1, ?2, ?3, {now})",
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![space_id, flat, label],
        )?;
        let existing_flat: Option<String> = tx
            .query_row(
                &format!("SELECT flat FROM {p}_roomplan_space WHERE space_id = ?1"),
                params![space_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing_flat.as_deref() != Some(flat) {
            return Err("RoomPlan space ID is already used by another flat".into());
        }
        let other_space: Option<String> = tx
            .query_row(
                &format!("SELECT space_id FROM {p}_roomplan_space_revision WHERE revision_id = ?1"),
                params![revision_id],
                |row| row.get(0),
            )
            .optional()?;
        if other_space
            .as_deref()
            .is_some_and(|existing| existing != space_id)
        {
            return Err("Scan revision is already assigned to another room".into());
        }
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO {p}_roomplan_space_revision (space_id, revision_id, added_at)
                 VALUES (?1, ?2, {now})",
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![space_id, revision_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_roomplan_revision(
        &self,
        space_id: &str,
        flat: &str,
        revision_id: &str,
        reference_revision_id: Option<&str>,
        alignment_report: Option<&serde_json::Value>,
    ) -> Result<(), Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let other_space: Option<String> = conn
            .query_row(
                &format!("SELECT space_id FROM {p}_roomplan_space_revision WHERE revision_id = ?1"),
                params![revision_id],
                |row| row.get(0),
            )
            .optional()?;
        if other_space
            .as_deref()
            .is_some_and(|existing| existing != space_id)
        {
            return Err("Scan revision is already assigned to another room".into());
        }
        let tx = conn.unchecked_transaction()?;
        let changed = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO {p}_roomplan_space_revision (space_id, revision_id, added_at)
                 SELECT space_id, ?3, {now} FROM {p}_roomplan_space WHERE space_id = ?1 AND flat = ?2",
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![space_id, flat, revision_id],
        )?;
        let exists: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {p}_roomplan_space_revision WHERE space_id = ?1 AND revision_id = ?2)"),
            params![space_id, revision_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err("RoomPlan space was not found for this flat".into());
        }
        if changed == 1 {
            if let (Some(reference_revision_id), Some(report)) =
                (reference_revision_id, alignment_report)
            {
                tx.execute(
                    &format!(
                        "INSERT INTO {p}_roomplan_alignment
                         (space_id, revision_id, reference_revision_id, report_json, created_at)
                         VALUES (?1, ?2, ?3, ?4, {now})",
                        now = sjel_store::now_offset("'+0 seconds'")
                    ),
                    params![
                        space_id,
                        revision_id,
                        reference_revision_id,
                        report.to_string()
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn roomplan_spaces(&self, flat: &str) -> Result<Vec<RoomPlanSpace>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT space_id, flat, label, created_at FROM {p}_roomplan_space WHERE flat = ?1 ORDER BY created_at, space_id"
        ))?;
        let rows = stmt.query_map(params![flat], |row| {
            Ok(RoomPlanSpace {
                space_id: row.get(0)?,
                flat: row.get(1)?,
                label: row.get(2)?,
                created_at: row.get(3)?,
                revision_ids: Vec::new(),
                alignments: Vec::new(),
            })
        })?;
        let mut spaces = rows.collect::<Result<Vec<_>, _>>()?;
        for space in &mut spaces {
            let mut revisions = conn.prepare(&format!(
                "SELECT revision_id FROM {p}_roomplan_space_revision WHERE space_id = ?1 ORDER BY added_at, revision_id"
            ))?;
            space.revision_ids = revisions
                .query_map(params![space.space_id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut alignments = conn.prepare(&format!(
                "SELECT space_id, revision_id, reference_revision_id, report_json, created_at
                 FROM {p}_roomplan_alignment WHERE space_id = ?1 ORDER BY created_at, revision_id"
            ))?;
            space.alignments = alignments
                .query_map(params![space.space_id], |row| {
                    let report: String = row.get(3)?;
                    Ok(RoomPlanAlignment {
                        space_id: row.get(0)?,
                        revision_id: row.get(1)?,
                        reference_revision_id: row.get(2)?,
                        report: serde_json::from_str(&report).unwrap_or(serde_json::Value::Null),
                        created_at: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(spaces)
    }

    pub fn link_roomplan_object(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> Result<(), Error> {
        let catalogue = self.items.catalogue()?;
        let Some((item, state)) = catalogue.get(item_id) else {
            return Err("Inventory item was not found".into());
        };
        if item.kind == Kind::Slot || matches!(state, Some(State::Wanted | State::Gone)) {
            return Err("RoomPlan observations can link only to owned inventory items".into());
        }
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_roomplan_object_link (space_id, revision_id, observation_id, item_id, created_at)
                 SELECT ?1, ?2, ?3, ?4, {now}
                 WHERE EXISTS (SELECT 1 FROM {p}_roomplan_space_revision WHERE space_id = ?1 AND revision_id = ?2)
                 ON CONFLICT(space_id, revision_id, observation_id) DO UPDATE SET
                    item_id = excluded.item_id, created_at = excluded.created_at",
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![space_id, revision_id, observation_id, item_id],
        )?;
        let exists: bool = conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {p}_roomplan_object_link WHERE space_id = ?1 AND revision_id = ?2 AND observation_id = ?3 AND item_id = ?4)"),
            params![space_id, revision_id, observation_id, item_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err("Revision is not registered in this RoomPlan space".into());
        }
        Ok(())
    }

    pub fn roomplan_object_links(&self, space_id: &str) -> Result<Vec<RoomPlanObjectLink>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT space_id, revision_id, observation_id, item_id, created_at
             FROM {p}_roomplan_object_link WHERE space_id = ?1 ORDER BY revision_id, observation_id"
        ))?;
        let rows = stmt.query_map(params![space_id], |row| {
            Ok(RoomPlanObjectLink {
                space_id: row.get(0)?,
                revision_id: row.get(1)?,
                observation_id: row.get(2)?,
                item_id: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every item with its current state.
    pub fn catalogue(&self) -> Result<BTreeMap<String, (Item, Option<State>)>, Error> {
        self.items.catalogue()
    }
}
