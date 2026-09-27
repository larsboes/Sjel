//! Where the aggregate is kept. Table prefix `punctuality` in the one shared
//! SQLite file (PRD Q45), same arrangement scouting and transit already use.
//!
//! Only the aggregate lands here — roughly 400k summary rows, not the ~120M stop
//! records they were computed from. That is a backup decision as much as a storage one:
//! the shared file is in the backup set, so anything in this database is in every
//! backup forever, while the raw parquet is a cache that upstream can hand back at any
//! time.

use crate::ingest::CellKey;
use crate::stats::Cell;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use sjel_store::QueryAll;
use std::collections::HashMap;
use std::path::Path;

pub struct Store {
    /// Pooled like its six siblings, though this capability is the one that gains
    /// least from it: it opens a store twice per process, not once per request. It
    /// is here because the shared sjel-store crate owns both migration and pooling, so
    /// a consumer that wants its migration half carries its pool half too — and
    /// carrying the dependency while hand-rolling a second connection strategy
    /// beside it would be the worse of the two outcomes.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `punctuality` here means `punctuality_stop_stats` and its two siblings.
    prefix: String,
}

pub type Coverage = (String, String, i32, String);

#[derive(Debug, Clone)]
pub struct MonthRecord {
    pub month: String,
    pub source_oid: Option<String>,
    pub rows_read: i64,
    pub rows_skipped: i64,
}

#[derive(Debug, Clone)]
pub struct IngestedMonth {
    pub month: String,
    pub source_oid: Option<String>,
}

/// The projection `row_to_stat` reads, in its exact order.
///
/// Both queries interpolate this rather than spelling the list out, because the two
/// drifted: `station_stats` stopped at `cancel_rate` while `row_to_stat` reached for
/// `counts` at index 12. A projection narrower than the reader is not a compile error,
/// so it surfaced as a panic at request time, and the panic poisoned the server's store
/// lock for every later request (#175).
const STAT_COLUMNS: &str = "s.eva, st.station_name, s.train_type, s.hour, s.weekend, s.n, \
     s.canceled, s.mean_delay, s.p50, s.p90, s.share_late_6, s.cancel_rate, s.sum_delay, s.counts";

/// The dataset writes EVA numbers zero-padded to eight digits (`08000044`); HAFAS, and
/// therefore `capabilities/transit`, returns them unpadded (`8000044`). Joining the two
/// without this returns zero rows and looks exactly like "we have no data for that
/// station" — which is how it presented the first time.
pub fn normalize_eva(eva: &str) -> String {
    let digits = eva.trim_start_matches('0');
    format!("{digits:0>8}")
}

#[derive(Debug, Clone)]
pub struct StatRow {
    pub eva: String,
    pub station_name: Option<String>,
    pub train_type: String,
    pub hour: i16,
    pub weekend: bool,
    pub n: i64,
    pub canceled: i64,
    pub mean_delay: f32,
    pub p50: i16,
    pub p90: i16,
    pub share_late_6: f32,
    pub cancel_rate: f32,
    /// Exact running delay sum, kept so incremental merges do not round a mean back
    /// into a sum on every monthly update.
    pub sum_delay: i64,
    /// The stored histogram, one count per delay bucket.
    ///
    /// Whether it can answer an arbitrary threshold is
    /// [`Cell::share_at_least_from_counts`]'s call, not this field's: it returns
    /// `None` for an array of the wrong length. Under Postgres this was
    /// `Option<Vec<i32>>` because the column was retrofitted onto rows that
    /// predated it; no SQLite row does, so the column is NOT NULL and the outer
    /// option had one inhabitant.
    pub counts: Vec<i32>,
}

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "punctuality")
    }

    /// `prefix` is either the literal `"punctuality"` or a test-generated name — never
    /// user input. SQLite has no parametrized-identifier syntax for DDL, so prefixed
    /// names are built with `format!` below; that is safe because of where the name
    /// comes from, not because interpolating into SQL is safe in general.
    pub fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        validate_prefix(prefix)?;
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/sjel-store/README.md has why.
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool.
    ///
    /// Held across a whole transaction by `replace_stats`, which is the intended
    /// use of a checkout rather than a problem with one: the connection is this
    /// caller's until it is dropped, and r2d2 hands the next caller a different one.
    fn conn(&self) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The current shape of the three tables, not the history that produced them.
    ///
    /// The `ALTER TABLE ... ADD COLUMN IF NOT EXISTS counts` that retrofitted the
    /// histogram is folded into `CREATE TABLE`, and folded in as NOT NULL: SQLite has
    /// neither that ALTER form nor an alterable constraint, and no deployed SQLite file
    /// predates this migration, so there are no pre-retrofit rows for a nullable column
    /// to describe.
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_stations (
                eva          TEXT PRIMARY KEY,
                station_name TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {prefix}_stop_stats (
                eva           TEXT    NOT NULL,
                train_type    TEXT    NOT NULL,
                hour          INTEGER NOT NULL,
                weekend       INTEGER NOT NULL,
                n             INTEGER NOT NULL,
                canceled      INTEGER NOT NULL,
                mean_delay    REAL    NOT NULL,
                p50           INTEGER NOT NULL,
                p90           INTEGER NOT NULL,
                share_late_6  REAL    NOT NULL,
                cancel_rate   REAL    NOT NULL,
                sum_delay     INTEGER NOT NULL DEFAULT 0,
                -- The histogram itself, one count per delay bucket, as a JSON array:
                -- SQLite has no array type, and this is one of the two measured
                -- Postgres columns that had no native equivalent (PRD Q45).
                --
                -- Persisting seven scalars and dropping the array meant the only
                -- exceedance anyone could ever ask about was the six minutes that
                -- happened to get its own column, and transit's punctuality.rs said
                -- outright that transfer risk cannot be produced from this data.
                -- It always could; the array was thrown away on the way to disk.
                -- ~512 bytes a cell, and a new question is now a query rather than
                -- a re-ingest of every parquet file.
                counts        TEXT    NOT NULL,
                -- A rowid table, and that is measured rather than inherited.
                -- The four key columns are stored twice here, once in the rowid
                -- btree and once in the autoindex, which makes this the one table
                -- in the file big enough for WITHOUT ROWID to be worth pricing:
                -- 470,782 rows, 153.76 MB of table plus 12.06 MB of autoindex on
                -- a copy of the live file (2026-09-05, dbstat).
                --
                -- Rebuilt three ways from that copy: 167.15 MB as a rowid table,
                -- 178.86 MB WITHOUT ROWID filled in rowid order (7% worse, and
                -- that is the order a CREATE-INSERT-SELECT migration would use),
                -- 162.48 MB WITHOUT ROWID filled in key order (2.8% better). The
                -- `counts` cell is what decides it: rows this wide pack a key
                -- btree badly, so the autoindex it removes is nearly all it saves.
                -- 2.8% does not pay for a 470k-row table rebuild. Leave it.
                PRIMARY KEY (eva, train_type, hour, weekend)
            );

            -- What window produced the current contents. Without it, a table of numbers
            -- says nothing about which months it covers, and a partial re-ingest would
            -- be indistinguishable from a complete one.
            --
            -- INTEGER PRIMARY KEY AUTOINCREMENT, not the bare rowid alias BIGSERIAL
            -- would otherwise map to: `coverage` reads the newest run as MAX(id), and a
            -- plain rowid is reused after the highest row is deleted.
            CREATE TABLE IF NOT EXISTS {prefix}_ingest_runs (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                ran_at       TEXT NOT NULL DEFAULT ({now}),
                from_month   TEXT NOT NULL,
                to_month     TEXT NOT NULL,
                months       INTEGER NOT NULL,
                rows_read    INTEGER NOT NULL,
                rows_skipped INTEGER NOT NULL,
                cells        INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS {prefix}_ingest_months (
                month       TEXT PRIMARY KEY,
                source_oid  TEXT,
                rows_read   INTEGER NOT NULL,
                rows_skipped INTEGER NOT NULL
            );
            ",
            prefix = prefix,
            now = sjel_store::NOW
        ))?;

        // SQLite has no portable `ADD COLUMN IF NOT EXISTS`. Older deployments
        // already have the aggregate table, so add the exact merge sum only when the
        // migration sees that it is absent.
        let has_sum_delay = conn
            .prepare(&format!("PRAGMA table_info({prefix}_stop_stats)"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "sum_delay");
        if !has_sum_delay {
            conn.execute(
                &format!("ALTER TABLE {prefix}_stop_stats ADD COLUMN sum_delay INTEGER NOT NULL DEFAULT 0"),
                [],
            )?;
            conn.execute(
                &format!("UPDATE {prefix}_stop_stats SET sum_delay = CAST(ROUND(mean_delay * n) AS INTEGER)"),
                [],
            )?;
        }
        Ok(())
    }

    /// Replaces the aggregate wholesale inside one transaction.
    ///
    /// Replace rather than upsert, because the rows are a *function* of the ingested
    /// window: narrowing the window and merging into what was there would leave rows
    /// from months no longer covered, and nothing in the table would show it.
    pub fn replace_stats(
        &self,
        cells: &HashMap<CellKey, Cell>,
        stations: &HashMap<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        replace_stats_tx(&tx, &prefix, cells, stations)?;
        tx.commit()?;
        Ok(())
    }

    /// Rebuilds the aggregate and replaces the source-month ledger in one transaction.
    /// This is the correction path: a changed or removed upstream month cannot be
    /// subtracted from a materialized histogram, so the safe fallback is a rebuild.
    pub fn replace_stats_and_months(
        &self,
        cells: &HashMap<CellKey, Cell>,
        stations: &HashMap<String, String>,
        months: &[MonthRecord],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        replace_stats_tx(&tx, &prefix, cells, stations)?;
        tx.execute(&format!("DELETE FROM {prefix}_ingest_months"), [])?;
        insert_months(&tx, &prefix, months)?;
        tx.commit()?;
        Ok(())
    }

    /// Merges only new monthly cells into the materialized aggregate and records the
    /// source identities atomically. A retry cannot double-count a month because the
    /// caller only passes months absent from the ledger.
    pub fn merge_stats_and_months(
        &self,
        cells: &HashMap<CellKey, Cell>,
        stations: &HashMap<String, String>,
        months: &[MonthRecord],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let mut select = tx.prepare(&format!(
            "SELECT n, canceled, sum_delay, counts FROM {prefix}_stop_stats
             WHERE eva = ?1 AND train_type = ?2 AND hour = ?3 AND weekend = ?4"
        ))?;
        let mut upsert = tx.prepare(&format!(
            "INSERT INTO {prefix}_stop_stats
               (eva, train_type, hour, weekend, n, canceled, mean_delay, p50, p90,
                share_late_6, cancel_rate, sum_delay, counts)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT (eva, train_type, hour, weekend) DO UPDATE SET
                n = excluded.n, canceled = excluded.canceled,
                mean_delay = excluded.mean_delay, p50 = excluded.p50,
                p90 = excluded.p90, share_late_6 = excluded.share_late_6,
                cancel_rate = excluded.cancel_rate, sum_delay = excluded.sum_delay,
                counts = excluded.counts"
        ))?;
        for (key, cell) in cells {
            let old = select
                .query_row(
                    params![&key.eva, &key.train_type, key.hour as i16, key.weekend],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            sjel_store::json_column::<Vec<i32>>(row, 3)?,
                        ))
                    },
                )
                .optional()?;
            let mut merged = cell.clone();
            if let Some((n, canceled, sum, counts)) = old {
                let existing =
                    Cell::from_parts(&counts, u64::try_from(n)?, u64::try_from(canceled)?, sum)
                        .ok_or("malformed persisted punctuality histogram")?;
                merged.merge(&existing);
            }
            upsert.execute(params![
                &key.eva,
                &key.train_type,
                key.hour as i16,
                key.weekend,
                merged.n as i64,
                merged.canceled as i64,
                merged.mean() as f32,
                merged.quantile(0.5) as i16,
                merged.quantile(0.9) as i16,
                merged.share_at_least(6) as f32,
                merged.cancel_rate() as f32,
                merged.sum_delay(),
                serde_json::to_string(&merged.counts_i32())?,
            ])?;
        }
        drop(select);
        drop(upsert);
        upsert_stations(&tx, &prefix, stations)?;
        insert_months(&tx, &prefix, months)?;
        tx.commit()?;
        Ok(())
    }

    pub fn replace_month_manifest(
        &self,
        months: &[MonthRecord],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(&format!("DELETE FROM {prefix}_ingest_months"), [])?;
        insert_months(&tx, &prefix, months)?;
        tx.commit()?;
        Ok(())
    }

    pub fn month_manifest(&self) -> Result<Vec<IngestedMonth>, Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        Ok(self.conn()?.query_all(
            &format!("SELECT month, source_oid FROM {prefix}_ingest_months ORDER BY month"),
            [],
            |row| {
                Ok(IngestedMonth {
                    month: row.get(0)?,
                    source_oid: row.get(1)?,
                })
            },
        )?)
    }

    pub fn record_run(
        &self,
        from: &str,
        to: &str,
        months: i32,
        rows: i64,
        skipped: i64,
        cells: i32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        self.conn()?.execute(
            &format!(
                "INSERT INTO {prefix}_ingest_runs (from_month, to_month, months, rows_read, rows_skipped, cells)
                 VALUES (?1,?2,?3,?4,?5,?6)"
            ),
            params![&from, &to, months, rows, skipped, cells],
        )?;
        Ok(())
    }

    /// Statistics for one station, optionally narrowed to a train type, ordered by hour.
    /// `min_n` drops cells too thin to mean anything — a 100% late rate over three
    /// observations is noise wearing a percentage sign.
    pub fn station_stats(
        &self,
        eva: &str,
        train_type: Option<&str>,
        min_n: i64,
    ) -> Result<Vec<StatRow>, Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        let eva = normalize_eva(eva);
        // Projects `s.counts` for the same reason `stop_stats` does: `row_to_stat` reads
        // thirteen columns, so a projection that stops at twelve fails on the last `get`
        // instead of returning a row whose histogram is absent.
        let sql = format!(
            "SELECT {STAT_COLUMNS}
             FROM {prefix}_stop_stats s
             LEFT JOIN {prefix}_stations st ON st.eva = s.eva
             WHERE s.eva = ?1 AND (?2 IS NULL OR s.train_type = ?2) AND s.n >= ?3
             ORDER BY s.train_type, s.weekend, s.hour"
        );
        Ok(self
            .conn()?
            .query_all(&sql, params![&eva, &train_type, min_n], row_to_stat)?)
    }

    /// One cell, or `None` when it does not exist or is thinner than `min_n`.
    ///
    /// `None` deliberately does not degrade to a neighbouring hour or to the station's
    /// average. Both would answer a question nobody asked and would be indistinguishable
    /// from a real reading downstream.
    pub fn stop_stats(
        &self,
        eva: &str,
        train_type: &str,
        hour: i16,
        weekend: bool,
        min_n: i64,
    ) -> Result<Option<StatRow>, Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        let eva = normalize_eva(eva);
        Ok(self
            .conn()?
            .query_row(
                &format!(
                    "SELECT {STAT_COLUMNS}
                     FROM {prefix}_stop_stats s
                     LEFT JOIN {prefix}_stations st ON st.eva = s.eva
                     WHERE s.eva = ?1 AND s.train_type = ?2 AND s.hour = ?3 AND s.weekend = ?4
                       AND s.n >= ?5"
                ),
                params![&eva, &train_type, hour, weekend, min_n],
                row_to_stat,
            )
            .optional()?)
    }

    /// The window the current aggregate covers, from the most recent ingest run.
    /// `None` means nothing has been ingested — which a caller must be able to tell
    /// apart from "this train is never late".
    pub fn coverage(&self) -> Result<Option<Coverage>, Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        Ok(self
            .conn()?
            .query_row(
                &format!(
                    "SELECT from_month, to_month, cells, ran_at FROM {prefix}_ingest_runs
                     ORDER BY id DESC LIMIT 1"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }

    /// EVA numbers whose station name contains `needle`, case-insensitively.
    ///
    /// `LIKE` where Postgres had `ILIKE`. SQLite's LIKE already ignores case, but only
    /// for ASCII: `MÜNCHEN` does not match `München`, where `münchen` does. Adding ICU
    /// to buy that one letter is not worth a build-time dependency; the caller that hits
    /// it retypes the umlaut in lower case.
    pub fn find_stations(
        &self,
        needle: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let prefix = &self.prefix;
        Ok(self.conn()?.query_all(
            &format!(
                "SELECT eva, station_name FROM {prefix}_stations
                 WHERE station_name LIKE '%' || ?1 || '%' ORDER BY station_name LIMIT 25"
            ),
            params![&needle],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }
}

fn replace_stats_tx(
    tx: &Transaction<'_>,
    prefix: &str,
    cells: &HashMap<CellKey, Cell>,
    stations: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    tx.execute(&format!("DELETE FROM {prefix}_stop_stats"), [])?;
    let mut insert = tx.prepare(&format!(
        "INSERT INTO {prefix}_stop_stats
           (eva, train_type, hour, weekend, n, canceled, mean_delay, p50, p90,
            share_late_6, cancel_rate, sum_delay, counts)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"
    ))?;
    for (key, cell) in cells {
        insert.execute(params![
            &key.eva,
            &key.train_type,
            key.hour as i16,
            key.weekend,
            cell.n as i64,
            cell.canceled as i64,
            cell.mean() as f32,
            cell.quantile(0.5) as i16,
            cell.quantile(0.9) as i16,
            cell.share_at_least(6) as f32,
            cell.cancel_rate() as f32,
            cell.sum_delay(),
            serde_json::to_string(&cell.counts_i32())?,
        ])?;
    }
    drop(insert);
    upsert_stations(tx, prefix, stations)
}

fn upsert_stations(
    tx: &Transaction<'_>,
    prefix: &str,
    stations: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut station_insert = tx.prepare(&format!(
        "INSERT INTO {prefix}_stations (eva, station_name) VALUES (?1,?2)
         ON CONFLICT (eva) DO UPDATE SET station_name = excluded.station_name"
    ))?;
    for (eva, name) in stations {
        station_insert.execute(params![eva, name])?;
    }
    Ok(())
}

fn insert_months(
    tx: &Transaction<'_>,
    prefix: &str,
    months: &[MonthRecord],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut insert = tx.prepare(&format!(
        "INSERT INTO {prefix}_ingest_months (month, source_oid, rows_read, rows_skipped)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT (month) DO UPDATE SET source_oid = excluded.source_oid,
           rows_read = excluded.rows_read, rows_skipped = excluded.rows_skipped"
    ))?;
    for month in months {
        insert.execute(params![
            &month.month,
            &month.source_oid,
            month.rows_read,
            month.rows_skipped,
        ])?;
    }
    Ok(())
}

fn row_to_stat(r: &Row) -> rusqlite::Result<StatRow> {
    Ok(StatRow {
        eva: r.get(0)?,
        station_name: r.get(1)?,
        train_type: r.get(2)?,
        hour: r.get(3)?,
        weekend: r.get(4)?,
        n: r.get(5)?,
        canceled: r.get(6)?,
        mean_delay: r.get(7)?,
        p50: r.get(8)?,
        p90: r.get(9)?,
        share_late_6: r.get(10)?,
        cancel_rate: r.get(11)?,
        sum_delay: r.get(12)?,
        counts: sjel_store::json_column(r, 13)?,
    })
}

/// The prefix is interpolated into DDL and every statement, so it is checked
/// rather than trusted. Production passes the literal `punctuality`; only a test
/// passes anything else.
fn validate_prefix(prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ok = !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && prefix.chars().next().is_some_and(|c| !c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(format!("unsafe table prefix '{prefix}'").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `row_to_stat` reads indices 0..=12. A projection of any other width is not a
    /// compile error; it fails on the first request that reaches the missing index.
    #[test]
    fn the_projection_is_as_wide_as_row_to_stat_reads() {
        assert_eq!(STAT_COLUMNS.split(',').count(), 14);
    }

    #[test]
    fn eva_normalization_bridges_the_dataset_and_hafas() {
        // The two forms of the same station must land on one key.
        assert_eq!(normalize_eva("8000044"), "08000044");
        assert_eq!(normalize_eva("08000044"), "08000044");
        assert_eq!(normalize_eva("8000044"), normalize_eva("08000044"));
    }

    #[test]
    fn normalization_does_not_truncate_a_longer_id() {
        assert_eq!(normalize_eva("123456789"), "123456789");
    }

    #[test]
    fn an_all_zero_id_does_not_become_empty() {
        // trim_start_matches would eat the whole string; the pad has to put it back.
        assert_eq!(normalize_eva("0"), "00000000");
    }
}

/// Database-backed; the module name is the selector CI splits the suite on. It was
/// `postgres_tests` until PRD Q45 — the suite needs a temp file now, not a server.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::ingest::CellKey;

    /// A file per test, in a directory this process owns.
    fn open_test_store(suffix: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("punctuality-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        // The directory is named by pid, and a pid is recycled eventually. A previous
        // run's rows arriving in this one is the failure the old TRUNCATE prevented.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        Store::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    fn one_cell(eva: &str, train_type: &str, hour: u8, delays: &[i32]) -> (CellKey, Cell) {
        let mut cell = Cell::default();
        for d in delays {
            cell.record(*d, false);
        }
        (
            CellKey {
                eva: eva.to_string(),
                train_type: train_type.to_string(),
                hour,
                weekend: false,
            },
            cell,
        )
    }

    #[test]
    fn ping_reaches_the_database() {
        open_test_store("ping")
            .ping()
            .expect("a live store answers");
    }

    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        // A file where a directory has to be, which is what the readiness handler
        // turns into a 503 the way it did for a stopped container.
        let blocker =
            std::env::temp_dir().join(format!("punctuality-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            Store::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    /// The histogram is the reason the array column exists, and JSON text is the only
    /// shape SQLite has for it. A round trip that loses it puts the capability back to
    /// answering exactly one threshold.
    #[test]
    fn the_histogram_survives_the_round_trip() {
        let store = open_test_store("histogram");
        let (key, cell) = one_cell("08000044", "ICE", 8, &[0, 1, 6, 6, 20, -2]);
        let live = cell.share_at_least(6);

        let mut cells = HashMap::new();
        cells.insert(key, cell);
        let mut stations = HashMap::new();
        stations.insert("08000044".to_string(), "Frankfurt(Main)Hbf".to_string());
        store.replace_stats(&cells, &stations).unwrap();

        let row = store
            .stop_stats("8000044", "ICE", 8, false, 1)
            .unwrap()
            .expect("the cell was just written");
        assert_eq!(row.station_name.as_deref(), Some("Frankfurt(Main)Hbf"));
        assert_eq!(
            Cell::share_at_least_from_counts(&row.counts, 6),
            Some(live),
            "the stored array must answer what the live cell answers"
        );
    }

    /// Replace, not merge. Rows from a window no longer covered would be invisible
    /// in a table of numbers that says nothing about its own coverage.
    #[test]
    fn a_second_ingest_replaces_the_aggregate_rather_than_merging_into_it() {
        let store = open_test_store("replace");
        let mut stations = HashMap::new();
        stations.insert("08000105".to_string(), "Frankfurt(Main)Hbf".to_string());

        let mut first = HashMap::new();
        let (key, cell) = one_cell("08000105", "ICE", 7, &[0; 40]);
        first.insert(key, cell);
        store.replace_stats(&first, &stations).unwrap();

        let mut second = HashMap::new();
        let (key, cell) = one_cell("08000105", "IC", 7, &[0; 40]);
        second.insert(key, cell);
        store.replace_stats(&second, &stations).unwrap();

        let rows = store.station_stats("08000105", None, 1).unwrap();
        assert_eq!(rows.len(), 1, "the ICE row belonged to the old window");
        assert_eq!(rows[0].train_type, "IC");
        // The station upsert is the half that does merge -- it is a lookup table,
        // not a function of the window.
        assert_eq!(rows[0].station_name.as_deref(), Some("Frankfurt(Main)Hbf"));
    }

    #[test]
    fn incremental_months_merge_without_double_counting() {
        let store = open_test_store("incremental");
        let (key, first) = one_cell("08000105", "ICE", 7, &[0; 40]);
        store
            .replace_stats_and_months(
                &HashMap::from([(key.clone(), first)]),
                &HashMap::new(),
                &[MonthRecord {
                    month: "2026-01".into(),
                    source_oid: Some("one".into()),
                    rows_read: 40,
                    rows_skipped: 0,
                }],
            )
            .unwrap();

        let (_, second) = one_cell("08000105", "ICE", 7, &[10; 40]);
        store
            .merge_stats_and_months(
                &HashMap::from([(key, second)]),
                &HashMap::new(),
                &[MonthRecord {
                    month: "2026-02".into(),
                    source_oid: Some("two".into()),
                    rows_read: 40,
                    rows_skipped: 0,
                }],
            )
            .unwrap();

        let row = store
            .stop_stats("8000105", "ICE", 7, false, 1)
            .unwrap()
            .unwrap();
        assert_eq!(row.n, 80);
        assert_eq!(row.sum_delay, 400);
        assert!((row.mean_delay - 5.0).abs() < 0.01);
        assert_eq!(store.month_manifest().unwrap().len(), 2);
    }

    /// `?2 IS NULL OR train_type = ?2` is the translation of Postgres's
    /// `$2::text IS NULL OR ...`, and it is the whole of the "no filter" path.
    #[test]
    fn a_null_train_type_means_every_train_type() {
        let store = open_test_store("null_filter");
        let mut cells = HashMap::new();
        for train_type in ["ICE", "IC", "RE"] {
            let (key, cell) = one_cell("08000152", train_type, 9, &[0; 40]);
            cells.insert(key, cell);
        }
        store.replace_stats(&cells, &HashMap::new()).unwrap();

        assert_eq!(store.station_stats("8000152", None, 1).unwrap().len(), 3);
        assert_eq!(
            store
                .station_stats("8000152", Some("ICE"), 1)
                .unwrap()
                .len(),
            1
        );
        // min_n is the thinness cut, and it applies to the same rows.
        assert!(store.station_stats("8000152", None, 41).unwrap().is_empty());
    }

    /// Nothing ingested must read as "no data", never as "never late". The newest
    /// run wins, which is what the AUTOINCREMENT id is for.
    #[test]
    fn coverage_is_absent_until_a_run_is_recorded_then_reports_the_newest() {
        let store = open_test_store("coverage");
        assert!(store.coverage().unwrap().is_none());

        store
            .record_run("2025-12", "2026-02", 3, 1_000, 10, 42)
            .unwrap();
        store
            .record_run("2026-01", "2026-06", 6, 2_000, 20, 84)
            .unwrap();

        let (from, to, cells, ran_at) = store.coverage().unwrap().unwrap();
        assert_eq!(
            (from.as_str(), to.as_str(), cells),
            ("2026-01", "2026-06", 84)
        );
        assert!(!ran_at.is_empty());
    }

    /// The padded/unpadded EVA split is the bug that presented as "no data for that
    /// station", so it is pinned on a real query rather than on the helper alone.
    #[test]
    fn a_station_is_found_by_either_spelling_of_its_eva() {
        let store = open_test_store("eva_forms");
        let mut cells = HashMap::new();
        let (key, cell) = one_cell("08000044", "ICE", 12, &[0; 40]);
        cells.insert(key, cell);
        let mut stations = HashMap::new();
        stations.insert("08000044".to_string(), "Aachen Hbf".to_string());
        store.replace_stats(&cells, &stations).unwrap();

        assert_eq!(store.station_stats("8000044", None, 1).unwrap().len(), 1);
        assert_eq!(store.station_stats("08000044", None, 1).unwrap().len(), 1);
        // LIKE stands in for ILIKE: case-insensitive over ASCII.
        assert_eq!(store.find_stations("aachen").unwrap().len(), 1);
        assert_eq!(store.find_stations("AACHEN").unwrap().len(), 1);
    }
}
