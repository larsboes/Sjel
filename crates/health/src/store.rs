//! Two tables: `health_days` holds one value per metric per day; a later import of the same day
//! replaces it, because the Health app's export is cumulative. `health_imports` records each
//! import run. The `turso-wal` feature keeps them in Health's own owner file; otherwise they live
//! in the shared database.

use std::path::Path;

use serde::Serialize;
#[cfg(feature = "turso-wal")]
use sjel_native_wal::{self as engine, params, Pool as Db, TransactionBehavior, NOW};
#[cfg(all(feature = "libsql", not(feature = "turso-wal")))]
use sjel_turso::{libsql, libsql::params, Store as Db, NOW};
#[cfg(not(any(feature = "libsql", feature = "turso-wal")))]
compile_error!("sjel-health needs the `libsql` or the `turso-wal` feature");

use crate::export::{describe, DayValue};

pub const PREFIX: &str = "health";
pub type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Serialize, PartialEq)]
pub struct Day {
    pub metric: String,
    pub day: String,
    pub value: f64,
    pub unit: String,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct MetricInfo {
    pub metric: String,
    pub group: String,
    pub unit: String,
    pub days: i64,
    pub first_day: String,
    pub last_day: String,
    pub latest: f64,
}

#[derive(Debug, Serialize)]
pub struct WindowStat {
    pub metric: String,
    pub group: String,
    pub unit: String,
    pub days_with_data: i64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Serialize)]
pub struct Import {
    pub file_name: String,
    pub records: i64,
    pub values: i64,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    pub at: String,
}

#[cfg_attr(feature = "turso-wal", derive(Clone))]
pub struct HealthStore {
    db: Db,
}

impl HealthStore {
    /// # Errors
    /// The database cannot be opened or migrated.
    #[cfg(all(feature = "libsql", not(feature = "turso-wal")))]
    pub async fn open(path: &Path) -> Fallible<Self> {
        Ok(Self {
            db: Db::open(path, PREFIX, migrate).await?,
        })
    }

    /// # Errors
    /// The owner file cannot be opened or migrated.
    #[cfg(feature = "turso-wal")]
    pub async fn open(path: &Path) -> Fallible<Self> {
        let path = path.to_path_buf();
        let db = tokio::task::spawn_blocking(move || {
            engine::open_pool(&path, PREFIX, |conn| Ok(conn.execute_batch(SCHEMA)?))
                .map_err(|error| error.to_string())
        })
        .await??;
        Ok(Self { db })
    }

    /// Write a consistent copy of the owner file to `target`.
    ///
    /// # Errors
    /// The snapshot cannot be staged or published.
    #[cfg(feature = "turso-wal")]
    pub fn snapshot(&self, database: &Path, target: &Path) -> Fallible<()> {
        engine::snapshot(&self.db.get()?, database, target)
            .map_err(|error| error.to_string().into())
    }

    pub async fn ping(&self) -> Fallible<()> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT 1", ()).await?;
        rows.next().await?.ok_or("`SELECT 1` returned no row")?;
        Ok(())
    }

    /// Upsert every value in one transaction and record the run.
    pub async fn record(&self, file_name: &str, records: u64, values: &[DayValue]) -> Fallible<()> {
        self.write(file_name, records, values, false).await
    }

    /// [`Self::record`] for a full export: for each metric it carries, first delete every stored
    /// day in its day range, so a day the export no longer has does not survive.
    pub async fn replace(
        &self,
        file_name: &str,
        records: u64,
        values: &[DayValue],
    ) -> Fallible<()> {
        self.write(file_name, records, values, true).await
    }

    async fn write(
        &self,
        file_name: &str,
        records: u64,
        values: &[DayValue],
        replace: bool,
    ) -> Fallible<()> {
        #[cfg(all(feature = "libsql", not(feature = "turso-wal")))]
        let tx = self.db.begin().await?;
        #[cfg(feature = "turso-wal")]
        let mut conn = self.db.connect().await?;
        #[cfg(feature = "turso-wal")]
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        if replace {
            let mut ranges: std::collections::BTreeMap<&str, (&str, &str)> = Default::default();
            for value in values {
                let range = ranges
                    .entry(value.metric)
                    .or_insert((&value.day, &value.day));
                range.0 = range.0.min(value.day.as_str());
                range.1 = range.1.max(value.day.as_str());
            }
            for (metric, (first, last)) in ranges {
                tx.execute(
                    "DELETE FROM health_days WHERE metric = ?1 AND day BETWEEN ?2 AND ?3",
                    (metric, first, last),
                )
                .await?;
            }
        }
        for value in values {
            tx.execute(
                &format!(
                    "INSERT INTO health_days (metric, day, value, unit, source, imported_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, {NOW})
                     ON CONFLICT(metric, day) DO UPDATE SET value = excluded.value,
                         unit = excluded.unit, source = excluded.source,
                         imported_at = excluded.imported_at"
                ),
                params![
                    value.metric,
                    value.day.as_str(),
                    value.value,
                    value.unit,
                    value.source.as_str()
                ],
            )
            .await?;
        }
        let first = values.iter().map(|v| v.day.as_str()).min();
        let last = values.iter().map(|v| v.day.as_str()).max();
        tx.execute(
            &format!(
                "INSERT INTO health_imports (file_name, records, `values`, first_day, last_day, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, {NOW})"
            ),
            params![
                file_name,
                i64::try_from(records)?,
                i64::try_from(values.len())?,
                first,
                last
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Values between `from` and `to` inclusive, oldest first, optionally for one metric.
    pub async fn days(&self, metric: Option<&str>, from: &str, to: &str) -> Fallible<Vec<Day>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT metric, day, value, unit, source FROM health_days
                 WHERE day BETWEEN ?1 AND ?2 AND (?3 IS NULL OR metric = ?3)
                 ORDER BY day, metric",
                params![from, to, metric],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Day {
                metric: row.get(0)?,
                day: row.get(1)?,
                value: row.get(2)?,
                unit: row.get(3)?,
                source: row.get(4)?,
            });
        }
        Ok(out)
    }

    /// Every stored metric with its coverage and newest value.
    pub async fn metrics(&self) -> Fallible<Vec<MetricInfo>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT d.metric, d.unit, COUNT(*), MIN(d.day), MAX(d.day),
                        (SELECT value FROM health_days l WHERE l.metric = d.metric ORDER BY l.day DESC LIMIT 1)
                 FROM health_days d GROUP BY d.metric ORDER BY d.metric",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let metric: String = row.get(0)?;
            out.push(MetricInfo {
                group: describe(&metric)
                    .map_or("other", |(group, _)| group)
                    .to_owned(),
                metric,
                unit: row.get(1)?,
                days: row.get(2)?,
                first_day: row.get(3)?,
                last_day: row.get(4)?,
                latest: row.get(5)?,
            });
        }
        Ok(out)
    }

    /// Mean, min and max per metric over `from..=to`.
    pub async fn window(&self, from: &str, to: &str) -> Fallible<Vec<WindowStat>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT metric, unit, COUNT(*), AVG(value), MIN(value), MAX(value) FROM health_days
                 WHERE day BETWEEN ?1 AND ?2 GROUP BY metric ORDER BY metric",
                params![from, to],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let metric: String = row.get(0)?;
            out.push(WindowStat {
                group: describe(&metric)
                    .map_or("other", |(group, _)| group)
                    .to_owned(),
                metric,
                unit: row.get(1)?,
                days_with_data: row.get(2)?,
                mean: row.get(3)?,
                min: row.get(4)?,
                max: row.get(5)?,
            });
        }
        Ok(out)
    }

    pub async fn imports(&self) -> Fallible<Vec<Import>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT file_name, records, `values`, first_day, last_day, at
                 FROM health_imports ORDER BY id DESC LIMIT 50",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Import {
                file_name: row.get(0)?,
                records: row.get(1)?,
                values: row.get(2)?,
                first_day: row.get(3)?,
                last_day: row.get(4)?,
                at: row.get(5)?,
            });
        }
        Ok(out)
    }
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS health_days (
        metric TEXT NOT NULL,
        day TEXT NOT NULL,
        value REAL NOT NULL,
        unit TEXT NOT NULL,
        source TEXT NOT NULL,
        imported_at TEXT NOT NULL,
        PRIMARY KEY (metric, day)
    );
    CREATE INDEX IF NOT EXISTS health_days_by_day ON health_days (day);
    CREATE TABLE IF NOT EXISTS health_imports (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_name TEXT NOT NULL,
        records INTEGER NOT NULL,
        `values` INTEGER NOT NULL,
        first_day TEXT,
        last_day TEXT,
        at TEXT NOT NULL
    );";

#[cfg(all(feature = "libsql", not(feature = "turso-wal")))]
async fn migrate(conn: libsql::Connection) -> Fallible<()> {
    conn.execute_batch(SCHEMA).await?;
    Ok(())
}
