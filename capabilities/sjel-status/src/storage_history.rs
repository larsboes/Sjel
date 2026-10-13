//! The disk's own history: one sample per hour, so "is this growing" has an answer.
//!
//! `tools/storage report` answers "what is filling the disk right now". It cannot answer
//! "was it filling faster last week", because a report is a point and growth is a slope.
//! The machine already had the measurement — `host-watch` reads it every hour — but kept no
//! record of it: `capabilities/host-watch/README.md` states its contract as "most runs print
//! one line and write nothing, and that silence is the product". So the record is kept here,
//! by the one surface that is always up, on its own hourly timer.
//!
//! The tables are `storage_samples` and `storage_absent` in the shared database, under this
//! capability's own prefix (`capabilities/store/README.md`, "one file, table prefixes").
//! This file owns the prefix, the sampler is their only writer, and `status/storage.rs` reads
//! them back.
//!
//! ## Why the hour is the primary key
//!
//! A restart inside the same hour must not add a second row. `hour` is a canonical UTC
//! stamp truncated to the hour, and the insert is `INSERT OR REPLACE` on it, so a process
//! that comes back at 10:31 overwrites 10:00 rather than appending. A trend drawn from rows
//! that arrive in bursts would show growth that is an artefact of restarts.
//!
//! The wait between samples is anchored on the wall clock — sleep until the next `:00` — and not
//! on a monotonic hour. On Darwin a monotonic `Instant` does not advance while the system is
//! asleep, so `sleep(3600s)` on a laptop that sleeps nightly stretches silently: this machine was
//! measured awake 11 h 02 m between two samples 11 h 58 m apart, with every hour key in between
//! missing. Anchoring on the boundary puts each row in the hour it belongs to and cannot write
//! two rows for one hour. [`until_next_hour`] is that rule, and it is a function so the boundary
//! case (a full hour, never a zero sleep) can be tested without a clock.
//!
//! ## The second table, and why absence is not a column
//!
//! The sample answers "how much". It cannot answer "since when has this class been gone",
//! for two reasons that are the same reason: a named absence is a *set*, not a scalar, and a
//! date needs the hour it started. So the names go in their own table — `storage_absent`,
//! one row per name per hour, written in the same transaction as the sample it belongs to,
//! because the two are one observation and a sample that kept a size without the names that
//! were gone would be a partial reading. It is the same 90-day window and the same hourly key.
//!
//! `open_runs` turns the table into "absent since", and its rule is the whole point of the
//! shape: a run breaks on a *sample* in which the name was present, never on a missing hour.
//! The machine is off at night and sometimes for days, and a power cut cannot prove a path came
//! back — so a run that spanned one still reads as continuous, and restarting the clock there
//! would make every long absence look newer than it is.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// How long an hourly sample is kept. 90 days is ~2160 rows, well under a megabyte, and
/// enough for the question a person actually asks ("has this grown since last month").
/// Older rows are deleted on each sample, so the table cannot grow without bound.
pub const RETENTION_DAYS: i64 = 90;

/// How long until the next wall-clock hour boundary.
///
/// The cadence is anchored on the clock rather than a plain `sleep(3600s)`, and that is the
/// whole reason this is a function. On Darwin a monotonic `Instant` does not advance while the
/// system is asleep, so an hour-long monotonic sleep silently stretches on a laptop that sleeps
/// nightly: on 2026-10-12 this machine was awake 11 h 02 m between two samples 11 h 58 m apart,
/// and every hour key in between was simply missing. Sleeping until the next `:00` puts the row
/// in the hour it belongs to, cannot write two rows for one hour, and makes the row count mean
/// the count of awake hours rather than the count of timer expiries.
fn until_next_hour(now: SystemTime) -> Duration {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    // Exactly on the boundary the answer is a full hour, not zero: a zero sleep would spin.
    Duration::from_secs(3600 - seconds % 3600)
}

/// The `hour` column's format: `sjel_store::NOW`'s stamp truncated to the hour.
///
/// Written once, because this value is a primary key that a reader compares against, not a
/// label: the sampler's key and the reader's key cannot be allowed to drift by a `%f`.
const HOUR_FORMAT: &str = "%Y-%m-%d %H:00:00.000+00:00";

const PREFIX: &str = "storage";

fn now_hour(conn: &sjel_store::rusqlite::Connection) -> Result<String, String> {
    conn.query_row(
        &format!("SELECT strftime('{HOUR_FORMAT}','now')"),
        [],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// One hourly reading of the volume.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Sample {
    /// The hour this sample belongs to, canonical UTC. Primary key; see the module doc.
    pub hour: String,
    /// When the reading was actually taken.
    pub at: String,
    pub used: i64,
    pub free: i64,
    pub total: i64,
    pub named: i64,
    pub unattributed: i64,
    pub reclaimable: i64,
    pub state: String,
}

/// One name the policy declares that the machine no longer answers for.
///
/// `kind` is a contract with the reader, not a lookup key: it says which of the policy's
/// three lists the name came from, because a class name, a protected path and a service name
/// can be the same string and mean three different things. The three values below are the
/// whole vocabulary and the panel groups on them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Absence {
    /// `class` · `protected` · `service`.
    pub kind: String,
    pub name: String,
}

/// The measurement, before it is stamped and stored. Kept apart from [`Sample`] because the
/// hour and the clock are SQLite's to decide, not this process's.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measurement {
    pub used: i64,
    pub free: i64,
    pub total: i64,
    pub named: i64,
    pub unattributed: i64,
    pub reclaimable: i64,
    pub state: String,
    /// Every name this reading found gone. Empty is the ordinary case.
    pub absent: Vec<Absence>,
}

impl Measurement {
    /// Read the fields out of `tools/storage report --json`.
    ///
    /// A report from a build before the reconciliation fields has no `named_bytes`, and the
    /// honest response is to record nothing rather than a zero: a sample that says "0 GB
    /// named" on a day the tool could not compute it would draw a cliff in the trend that
    /// never happened.
    pub(crate) fn from_report(report: &serde_json::Value) -> Result<Self, String> {
        let disk = report
            .get("disk")
            .ok_or_else(|| "report has no `disk`".to_string())?;
        let int = |value: &serde_json::Value, key: &str| -> Result<i64, String> {
            value
                .get(key)
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| format!("report has no `{key}`"))
        };
        let reclaimable = report
            .get("classes")
            .and_then(serde_json::Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.get("applicable").and_then(serde_json::Value::as_bool) == Some(true)
                    })
                    .filter_map(|row| row.get("bytes").and_then(serde_json::Value::as_i64))
                    .sum()
            })
            .unwrap_or(0);
        Ok(Self {
            used: int(disk, "used")?,
            free: int(disk, "free")?,
            total: int(disk, "total")?,
            named: int(report, "named_bytes")?,
            unattributed: int(report, "unattributed_bytes")?,
            reclaimable,
            state: report
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            absent: absent_from_report(report),
        })
    }
}

/// The names the report says the machine answered "gone" for, in a stable order.
///
/// Each kind's own word decides, and nothing else does. A class or a protection is absent only
/// on an explicit `present: false` — a report from a build before that field is not saying
/// "gone", it is not being asked, and recording it as absence would open a run on the first
/// sample after an upgrade for every path the machine has. A service is absent only on
/// `presence == "absent"`; `unverifiable` is the probe saying it could not tell (an unreadable
/// uid, a probe that will not run), which `tools/storage/src/service.rs` is explicit about and
/// which must not be written down as a death.
fn absent_from_report(report: &serde_json::Value) -> Vec<Absence> {
    let rows = |key: &str| -> Vec<serde_json::Value> {
        report
            .get(key)
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut absent = Vec::new();
    let mut push = |kind: &str, name: Option<&str>| {
        if let Some(name) = name {
            absent.push(Absence {
                kind: kind.to_string(),
                name: name.to_string(),
            });
        }
    };
    for row in rows("classes") {
        if matches!(row.get("present"), Some(serde_json::Value::Bool(false))) {
            push("class", row.get("name").and_then(serde_json::Value::as_str));
        }
    }
    for row in rows("protected") {
        if matches!(row.get("present"), Some(serde_json::Value::Bool(false))) {
            push(
                "protected",
                row.get("path").and_then(serde_json::Value::as_str),
            );
        }
    }
    for row in rows("expected_service") {
        if row.get("presence").and_then(serde_json::Value::as_str) == Some("absent") {
            push(
                "service",
                row.get("name").and_then(serde_json::Value::as_str),
            );
        }
    }
    absent.sort();
    absent.dedup();
    absent
}

pub(crate) struct StorageHistory {
    pool: sjel_store::Pool,
}

impl StorageHistory {
    /// Open and migrate. The sampler's constructor; a reader that only wants rows uses
    /// [`snapshot`], which tolerates a machine that has never sampled.
    pub(crate) fn open(database: &Path) -> Result<Self, String> {
        let pool = sjel_store::open_pool(database, PREFIX, |conn| migrate(conn, PREFIX))
            .map_err(|e| e.to_string())?;
        Ok(Self { pool })
    }

    /// Store one reading, replacing this hour's row if a restart already wrote one, and drop
    /// anything past the retention window. One transaction, because the sample, the names that
    /// were gone and the sweep are one decision: a commit that kept the size and lost the
    /// absences would record a full disk with nothing missing from it.
    pub(crate) fn record(&self, m: &Measurement) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let hour = now_hour(&conn)?;
        drop(conn);
        self.record_at(&hour, m)
    }

    /// Record one observation under a caller-chosen hour.
    ///
    /// [`record`](Self::record) is the only production caller and takes the hour from SQLite's
    /// clock. The hour is a parameter because no test can make `now` be three days ago, and
    /// both rules worth testing are about hours: a run that spans a gap in them, and a restart
    /// inside one.
    fn record_at(&self, hour: &str, m: &Measurement) -> Result<(), String> {
        let mut conn = self.pool.get().map_err(|e| e.to_string())?;
        let tx = sjel_store::write_transaction(&mut conn).map_err(|e| e.to_string())?;
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO {PREFIX}_samples
                 (hour, at, used, free, total, named, unattributed, reclaimable, state)
                 VALUES (?1, {now}, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                now = sjel_store::NOW
            ),
            sjel_store::rusqlite::params![
                hour,
                m.used,
                m.free,
                m.total,
                m.named,
                m.unattributed,
                m.reclaimable,
                m.state,
            ],
        )
        .map_err(|e| e.to_string())?;
        // This hour's absence set is replaced, not merged. A class that came back during the
        // hour must not leave the earlier reading's row saying it was still gone, and
        // `INSERT OR REPLACE` alone cannot express that: it would add rows, never remove one.
        tx.execute(
            &format!("DELETE FROM {PREFIX}_absent WHERE hour = ?1"),
            sjel_store::rusqlite::params![hour],
        )
        .map_err(|e| e.to_string())?;
        for absence in &m.absent {
            tx.execute(
                &format!("INSERT INTO {PREFIX}_absent (hour, kind, name) VALUES (?1, ?2, ?3)"),
                sjel_store::rusqlite::params![hour, absence.kind, absence.name],
            )
            .map_err(|e| e.to_string())?;
        }
        // One cutoff, one transaction, both tables. Rows swept from one and kept in the other
        // would leave the absence table naming hours no sample covers.
        let cutoff = sjel_store::now_offset(&format!("'-{RETENTION_DAYS} days'"));
        for table in ["samples", "absent"] {
            tx.execute(
                &format!("DELETE FROM {PREFIX}_{table} WHERE hour < {cutoff}"),
                [],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Everything the endpoint serves: the samples, and the absence runs that are still open.
pub(crate) struct Snapshot {
    pub samples: Vec<Sample>,
    pub absent_since: Vec<AbsentSince>,
}

/// One name that was missing in the newest sample and has been missing in every sample since.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AbsentSince {
    /// `class` · `protected` · `service`; see [`Absence`].
    pub kind: String,
    pub name: String,
    /// The oldest hour of the still-open run, canonical like [`Sample::hour`].
    pub since: String,
    /// True when the run reaches the oldest sample kept. The name was already gone when the
    /// record began, so `since` is a floor and not the hour it died — the difference between
    /// "gone since Tuesday" and "gone since at least Tuesday".
    pub capped: bool,
}

/// Read the whole history, oldest first.
///
/// Read-only, and a machine that has never sampled has no table — that is "no history", not
/// an error, the same rule `host_watch`'s reader applies. A database that cannot be opened at
/// all is a real fault and is reported. One pool for both tables, so the samples and the
/// absences read against them are one moment of the file rather than two.
pub(crate) fn snapshot(database: &Path) -> Result<Snapshot, String> {
    let pool = sjel_store::pool_for(database).map_err(|e| e.to_string())?;
    let conn = pool.get().map_err(|e| e.to_string())?;
    let samples = read_samples(&conn)?;
    let absent = read_absent(&conn)?;
    Ok(Snapshot {
        absent_since: open_runs(&samples, &absent),
        samples,
    })
}

fn read_samples(conn: &sjel_store::rusqlite::Connection) -> Result<Vec<Sample>, String> {
    let mut statement = match conn.prepare(&format!(
        "SELECT hour, at, used, free, total, named, unattributed, reclaimable, state
         FROM {PREFIX}_samples ORDER BY hour ASC"
    )) {
        Ok(statement) => statement,
        Err(error) if error.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let rows = statement
        .query_map([], |row| {
            Ok(Sample {
                hour: row.get(0)?,
                at: row.get(1)?,
                used: row.get(2)?,
                free: row.get(3)?,
                total: row.get(4)?,
                named: row.get(5)?,
                unattributed: row.get(6)?,
                reclaimable: row.get(7)?,
                state: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Every absence row, oldest first.
///
/// A missing table is a machine whose sjel-status predates it — no absence is known, which is
/// not an error, and is why the sampler's own table had to be additive rather than a change to
/// an existing one.
fn read_absent(conn: &sjel_store::rusqlite::Connection) -> Result<Vec<(String, Absence)>, String> {
    let mut statement = match conn.prepare(&format!(
        "SELECT hour, kind, name FROM {PREFIX}_absent ORDER BY hour ASC, kind ASC, name ASC"
    )) {
        Ok(statement) => statement,
        Err(error) if error.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Absence {
                    kind: row.get(1)?,
                    name: row.get(2)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// The still-open absence run per name, from samples and absence rows, both oldest first.
///
/// A run breaks on a sample in which the name was present and on nothing else. Hours with no
/// sample are not evidence either way: the machine was off, and a power cut cannot prove a path
/// came back. `capped` marks the other edge — a run reaching the oldest sample kept, where the
/// record starts and so `since` is a floor rather than the hour the name died.
fn open_runs(samples: &[Sample], absent: &[(String, Absence)]) -> Vec<AbsentSince> {
    let Some(oldest) = samples.first().map(|sample| sample.hour.as_str()) else {
        return Vec::new();
    };
    let mut gone_at: BTreeMap<&str, BTreeSet<(&str, &str)>> = BTreeMap::new();
    let mut names: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (hour, absence) in absent {
        let key = (absence.kind.as_str(), absence.name.as_str());
        names.insert(key);
        gone_at.entry(hour.as_str()).or_default().insert(key);
    }
    // Walked in hour order, so `open` holds the hour each still-missing name was first missing
    // in. A name whose run ended is dropped, and a later absence starts a fresh one.
    let mut open: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    for sample in samples {
        let gone = gone_at.get(sample.hour.as_str());
        for name in &names {
            match (
                gone.is_some_and(|set| set.contains(name)),
                open.contains_key(name),
            ) {
                (true, false) => {
                    open.insert(*name, sample.hour.as_str());
                }
                (false, true) => {
                    open.remove(name);
                }
                _ => {}
            }
        }
    }
    open.into_iter()
        .map(|((kind, name), since)| AbsentSince {
            kind: kind.to_string(),
            name: name.to_string(),
            since: since.to_string(),
            capped: since == oldest,
        })
        .collect()
}

/// Start the hourly sampler. Returns immediately; the loop runs on the runtime.
///
/// A failure to open the database is said once at startup rather than discovered as a
/// history that never appears — the same choice the backup policy loop makes.
pub(crate) fn spawn(database: PathBuf, root: PathBuf) {
    tokio::spawn(async move {
        let history = match StorageHistory::open(&database) {
            Ok(history) => history,
            Err(error) => {
                eprintln!("[sjel-status] storage history not started: {error}");
                return;
            }
        };
        loop {
            match sample_once(&root).await {
                Ok(measurement) => {
                    if let Err(error) = history.record(&measurement) {
                        eprintln!("[sjel-status] storage sample not recorded: {error}");
                    }
                }
                Err(error) => eprintln!("[sjel-status] storage sample failed: {error}"),
            }
            tokio::time::sleep(until_next_hour(SystemTime::now())).await;
        }
    });
}

/// Run the tool the same way the handler does — through the launcher, so the overlay is
/// resolved and a missing release binary is built — and turn its report into a measurement.
async fn sample_once(root: &Path) -> Result<Measurement, String> {
    let out = tokio::process::Command::new(root.join("tools/storage/storage"))
        .arg("report")
        .arg("--json")
        .current_dir(root)
        .output()
        .await
        .map_err(|e| format!("could not run tools/storage: {e}"))?;
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        format!(
            "tools/storage did not emit JSON (exit {:?}): {e}",
            out.status.code()
        )
    })?;
    Measurement::from_report(&report)
}

fn migrate(
    conn: &sjel_store::rusqlite::Connection,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(&format!(
        "
        CREATE TABLE IF NOT EXISTS {prefix}_samples (
            hour         TEXT PRIMARY KEY,
            at           TEXT NOT NULL,
            used         INTEGER NOT NULL,
            free         INTEGER NOT NULL,
            total        INTEGER NOT NULL,
            named        INTEGER NOT NULL,
            unattributed INTEGER NOT NULL,
            reclaimable  INTEGER NOT NULL,
            state        TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_{prefix}_samples_at
            ON {prefix}_samples(at);

        -- One row per gone name per hour, written with the sample it belongs to. The primary
        -- key is the only index: a reader takes the whole table and decides in [`open_runs`],
        -- and an index nothing queries is a cost with no reader.
        CREATE TABLE IF NOT EXISTS {prefix}_absent (
            hour TEXT NOT NULL,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            PRIMARY KEY (hour, kind, name)
        );
        "
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report shaped like the tool's, with the two reconciliation fields.
    fn report() -> serde_json::Value {
        serde_json::json!({
            "disk": { "used": 271_100_000_000u64, "free": 164_000_000_000u64, "total": 460_000_000_000u64, "target": "/System/Volumes/Data" },
            "state": "ok",
            "named_bytes": 40_698_048_512u64,
            "unattributed_bytes": 202_032_476_160u64,
            "classes": [
                { "name": "a", "bytes": 1_000_000_000u64, "applicable": true, "flagged": false, "measured": true },
                { "name": "b", "bytes": 5_000_000_000u64, "applicable": false, "flagged": true, "measured": true },
                { "name": "c", "bytes": 2_000_000_000u64, "applicable": true, "flagged": false, "measured": true },
            ],
        })
    }

    #[test]
    fn a_report_becomes_the_measurement_the_chart_draws() {
        let m = Measurement::from_report(&report()).expect("the report parses");
        assert_eq!(m.used, 271_100_000_000);
        assert_eq!(m.named, 40_698_048_512);
        assert_eq!(m.unattributed, 202_032_476_160);
        assert_eq!(m.state, "ok");
    }

    #[test]
    fn reclaimable_counts_applicable_classes_only() {
        // The number `apply` would actually free; the report-only class must not inflate it.
        let m = Measurement::from_report(&report()).unwrap();
        assert_eq!(m.reclaimable, 3_000_000_000);
    }

    #[test]
    fn a_report_without_the_reconciliation_fields_is_refused_rather_than_zeroed() {
        // A sample that recorded 0 named bytes on a day the tool could not compute it would
        // draw a cliff in the trend that never happened.
        let mut old = report();
        old.as_object_mut().unwrap().remove("named_bytes");
        let error = Measurement::from_report(&old).expect_err("refused");
        assert!(error.contains("named_bytes"), "{error}");
    }

    /// A throwaway database per test. The temp directory goes with the process, so a panic
    /// cannot leave a fixture behind.
    fn temp_database(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-status-storage-history-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{tag}.db"))
    }

    fn measurement(used: i64) -> Measurement {
        measurement_with(used, Vec::new())
    }

    fn measurement_with(used: i64, absent: Vec<Absence>) -> Measurement {
        Measurement {
            used,
            free: 1,
            total: 2,
            named: 3,
            unattributed: 4,
            reclaimable: 5,
            state: "ok".into(),
            absent,
        }
    }

    fn absence(kind: &str, name: &str) -> Absence {
        Absence {
            kind: kind.into(),
            name: name.into(),
        }
    }

    /// A canonical hour a test controls, so a run can be built across days.
    fn hour(day: u32, at: u32) -> String {
        format!("2026-10-{day:02} {at:02}:00:00.000+00:00")
    }

    /// A report with one absence of each kind, one present thing of each kind, and two names
    /// that are not absence at all.
    fn report_with_absence() -> serde_json::Value {
        let mut report = report();
        report["classes"] = serde_json::json!([
            { "name": "gone", "bytes": 0, "applicable": true, "flagged": false, "measured": true, "present": false },
            { "name": "here", "bytes": 10, "applicable": true, "flagged": false, "measured": true, "present": true },
            { "name": "unasked", "bytes": 0, "applicable": true, "flagged": false, "measured": true },
        ]);
        report["protected"] = serde_json::json!([
            { "path": "/gone", "bytes": 0, "reason": "written when it existed", "present": false },
            { "path": "/here", "bytes": 1, "reason": "a reason", "present": true },
        ]);
        report["expected_service"] = serde_json::json!([
            { "name": "stopped", "kind": "launchagent", "note": "", "presence": "absent", "running": false, "reason": null },
            { "name": "unreadable", "kind": "launchagent", "note": "", "presence": "unverifiable", "running": null, "reason": "no uid" },
            { "name": "up", "kind": "launchagent", "note": "", "presence": "running", "running": true, "reason": null },
        ]);
        report
    }

    #[test]
    fn a_second_sample_in_the_same_hour_replaces_the_first() {
        // The restart case. Two rows in one hour would make a trend that rises on every
        // restart, which is an artefact and not a measurement.
        let db = temp_database("replace");
        let history = StorageHistory::open(&db).unwrap();
        history.record(&measurement(10)).unwrap();
        history.record(&measurement(20)).unwrap();
        let rows = snapshot(&db).unwrap().samples;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].used, 20, "the fresher reading wins");
        assert_eq!(rows[0].hour.len(), 29, "canonical stamp: {}", rows[0].hour);
    }

    #[test]
    fn a_machine_that_has_never_sampled_reports_no_history_not_an_error() {
        let db = temp_database("empty");
        // A pool with no migration run over it: the table does not exist yet.
        let snap = snapshot(&db).unwrap();
        assert_eq!(snap.samples, Vec::new());
        assert_eq!(snap.absent_since, Vec::new());
    }

    #[test]
    fn a_recorded_sample_round_trips_through_the_table() {
        let db = temp_database("roundtrip");
        let history = StorageHistory::open(&db).unwrap();
        history.record(&measurement(42)).unwrap();
        let rows = snapshot(&db).unwrap().samples;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].used, 42);
        assert_eq!(rows[0].reclaimable, 5);
        assert_eq!(rows[0].state, "ok");
    }

    #[test]
    fn every_kind_of_absence_the_machine_answered_for_is_recorded() {
        let m = Measurement::from_report(&report_with_absence()).unwrap();
        assert_eq!(
            m.absent,
            vec![
                absence("class", "gone"),
                absence("protected", "/gone"),
                absence("service", "stopped"),
            ],
            "a dead class, a dead protected path and a dead service are all absences"
        );
    }

    #[test]
    fn a_report_that_never_asked_whether_a_path_is_there_records_no_absence() {
        // The upgrade case: the first sample after a new build must not open a run for every
        // class the machine owns, just because the field arrived on the report.
        let m = Measurement::from_report(&report()).unwrap();
        assert_eq!(m.absent, Vec::new());
    }

    #[test]
    fn an_unverifiable_service_is_not_written_down_as_absent() {
        let m = Measurement::from_report(&report_with_absence()).unwrap();
        assert!(
            !m.absent.iter().any(|a| a.name == "unreadable"),
            "`unverifiable` is the probe refusing to answer, not a death"
        );
    }

    #[test]
    fn the_next_sample_is_scheduled_on_the_wall_clock_hour() {
        let at = |hour: u64, minute: u64, second: u64| {
            UNIX_EPOCH + Duration::from_secs(hour * 3600 + minute * 60 + second)
        };
        assert_eq!(
            until_next_hour(at(0, 16, 25)),
            Duration::from_secs(43 * 60 + 35)
        );
        assert_eq!(until_next_hour(at(3, 59, 59)), Duration::from_secs(1));
        // On the boundary the answer is a full hour, never a zero sleep that would spin.
        assert_eq!(until_next_hour(at(3, 0, 0)), Duration::from_secs(3600));
    }

    #[test]
    fn a_name_that_comes_back_ends_the_run_and_a_new_one_starts_when_it_goes_again() {
        let db = temp_database("comes-back");
        let history = StorageHistory::open(&db).unwrap();
        history
            .record_at(
                &hour(1, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        history
            .record_at(
                &hour(1, 11),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        history.record_at(&hour(1, 12), &measurement(1)).unwrap();
        history
            .record_at(
                &hour(2, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        let since = snapshot(&db).unwrap().absent_since;
        assert_eq!(since.len(), 1, "{since:?}");
        assert_eq!(since[0].since, hour(2, 10), "the run that is still open");
        assert!(!since[0].capped);
    }

    #[test]
    fn a_power_cut_does_not_restart_the_clock_on_an_absence() {
        // Three days with no sample at all cannot prove the path came back, so it is one run.
        // This is the rule the second table's shape exists for.
        let db = temp_database("gap");
        let history = StorageHistory::open(&db).unwrap();
        history.record_at(&hour(1, 9), &measurement(1)).unwrap();
        history
            .record_at(
                &hour(1, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        history
            .record_at(
                &hour(4, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        let since = snapshot(&db).unwrap().absent_since;
        assert_eq!(
            since[0].since,
            hour(1, 10),
            "not when the machine came back"
        );
        assert!(!since[0].capped, "a sample before it had the class present");
    }

    #[test]
    fn a_run_that_reaches_the_oldest_kept_sample_reports_its_date_as_a_floor() {
        let db = temp_database("capped");
        let history = StorageHistory::open(&db).unwrap();
        history
            .record_at(
                &hour(1, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        history
            .record_at(
                &hour(1, 11),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        let since = snapshot(&db).unwrap().absent_since;
        assert_eq!(since.len(), 1, "{since:?}");
        assert_eq!(since[0].since, hour(1, 10));
        assert!(
            since[0].capped,
            "the record begins here, so it may have died before it"
        );
    }

    #[test]
    fn a_re_reading_inside_the_same_hour_replaces_that_hour_s_absence_set() {
        // A restart inside the hour, by which time the class was back. Merging rather than
        // replacing would leave the hour saying it was still gone.
        let db = temp_database("same-hour");
        let history = StorageHistory::open(&db).unwrap();
        history
            .record_at(
                &hour(1, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        history.record_at(&hour(1, 10), &measurement(1)).unwrap();
        let snap = snapshot(&db).unwrap();
        assert_eq!(snap.samples.len(), 1);
        assert_eq!(snap.absent_since, Vec::new());
    }

    #[test]
    fn retention_sweeps_absence_rows_with_the_samples_they_belong_to() {
        let db = temp_database("retention");
        let history = StorageHistory::open(&db).unwrap();
        history
            .record_at(
                "2020-01-01 10:00:00.000+00:00",
                &measurement_with(1, vec![absence("class", "ancient")]),
            )
            .unwrap();
        history.record_at(&hour(1, 10), &measurement(1)).unwrap();
        let pool = sjel_store::pool_for(&db).unwrap();
        let conn = pool.get().unwrap();
        let rows = read_absent(&conn).unwrap();
        assert!(rows.is_empty(), "the old hour is past the window: {rows:?}");
    }

    #[test]
    fn a_database_from_before_the_absence_table_reports_no_absence_rather_than_failing() {
        // The upgrade path: samples exist, the second table does not. "No absence is known"
        // is the honest answer — an error would blank the panel, and "everything is gone"
        // would be a lie.
        let db = temp_database("old-table");
        let history = StorageHistory::open(&db).unwrap();
        history
            .record_at(
                &hour(1, 10),
                &measurement_with(1, vec![absence("class", "gone")]),
            )
            .unwrap();
        let pool = sjel_store::pool_for(&db).unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch("DROP TABLE storage_absent").unwrap();
        }
        let snap = snapshot(&db).unwrap();
        assert_eq!(snap.samples.len(), 1);
        assert_eq!(snap.absent_since, Vec::new());
    }
}
