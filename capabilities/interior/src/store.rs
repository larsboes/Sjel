//! Where pieces stand in a flat, in the shared SQLite file. The items themselves belong to
//! `inventory` and are read through its store, so there is one schema and one migration.

use rusqlite::{params, Connection};
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
                UNIQUE (item_id, flat)
            );
            CREATE INDEX IF NOT EXISTS {prefix}_idx_placement_flat
                ON {prefix}_placement(flat);
            "
        ))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<(), Error> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    pub fn place(&self, pl: &Placement) -> Result<(), Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_placement (item_id, flat, x, y, rot, since)
                 VALUES (?1, ?2, ?3, ?4, ?5, {now})
                 ON CONFLICT(item_id, flat) DO UPDATE SET
                    x=excluded.x, y=excluded.y, rot=excluded.rot, since={now}",
                now = sjel_store::now_offset("'+0 seconds'")
            ),
            params![pl.item_id, pl.flat, pl.x, pl.y, pl.rot],
        )?;
        Ok(())
    }

    pub fn placements(&self, flat: &str) -> Result<Vec<Placement>, Error> {
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

    /// Every item with its current state.
    pub fn catalogue(&self) -> Result<BTreeMap<String, (Item, Option<State>)>, Error> {
        self.items.catalogue()
    }
}
