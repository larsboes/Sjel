//! The `backup_` tables in the shared store: targets, runs, verifications, timing policy.
//!
//! Writes stay inside this prefix (ISA anti-claim A1). Reading another prefix is a plain
//! SELECT, the same usage `capabilities/store/README.md` chose one database for.
//!
//! Every timestamp is TEXT in `sjel_store::NOW`'s one canonical format, because two widths
//! in one column stop `ORDER BY` being time order. Where the *maths* needs seconds —
//! "is this target due" — SQLite converts at the query, so the stored column keeps the one
//! format and no second representation is introduced.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The canonical timestamp expression. `sjel_store::NOW`, never a literal here.
const NOW: &str = sjel_store::NOW;

/// A declared target, as `tools/backup-all.sh --targets-json` reports it: derived from the
/// capabilities' manifests and resolved through the private overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TargetDecl {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub host: String,
    /// `true`/`false` for a local path that does or does not exist, `unchecked` for a host,
    /// `unknown` when there was nothing to check. Never a guess that a target is reachable.
    #[serde(default)]
    pub present: String,
    #[serde(default)]
    pub declared_by: Vec<String>,
}

/// A target plus what it has proved. `verified_verdict` is `None` until a rehearsal passed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TargetRow {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub host: String,
    pub present: String,
    pub declared_by: Vec<String>,
    pub seen_at: String,
    pub verified_at: Option<String>,
    pub verified_verdict: Option<String>,
    pub verified_detail: Option<String>,
    pub interval_hours: Option<i64>,
}

/// What an archive was, recorded by the producer. Never recomputed later: a digest
/// recomputed from bytes that may have changed since is a different claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveIdentity {
    pub name: String,
    pub bytes: i64,
    pub sha256: String,
}

/// One attempt. `exit_code` is `None` while it is still running.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRow {
    pub id: i64,
    pub capability: String,
    pub target: String,
    pub started_at: String,
    pub started_epoch: i64,
    pub finished_at: Option<String>,
    pub finished_epoch: Option<i64>,
    pub exit_code: Option<i64>,
    pub archive: Option<ArchiveIdentity>,
    pub detail: String,
    pub log_path: String,
}

impl RunRow {
    /// A run that finished non-zero. The distinction the receipts could not make: a
    /// finished run is not a landed backup.
    pub fn failed(&self) -> bool {
        matches!(self.exit_code, Some(code) if code != 0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationRow {
    pub id: i64,
    pub target: String,
    pub capability: String,
    pub at: String,
    pub verdict: String,
    pub detail: String,
    pub archive: Option<ArchiveIdentity>,
}

pub struct BackupStore {
    pool: sjel_store::Pool,
    prefix: String,
}

impl BackupStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, "backup")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Fallible<Self> {
        validate_prefix(prefix)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    pub(crate) fn conn(&self) -> Fallible<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The cheapest statement that proves this store can reach its database.
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_targets (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                path TEXT NOT NULL DEFAULT '',
                host TEXT NOT NULL DEFAULT '',
                present TEXT NOT NULL DEFAULT 'unknown',
                declared_by TEXT NOT NULL DEFAULT '[]',
                seen_at TEXT NOT NULL,
                verified_at TEXT,
                verified_verdict TEXT,
                verified_detail TEXT
            );

            -- One row per attempt, including the ones that fail. The rolling receipt
            -- answers \"when did a backup last land\"; this answers \"what did the last run
            -- do\", which is the question a failure leaves no other trace of.
            CREATE TABLE IF NOT EXISTS {prefix}_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                capability TEXT NOT NULL,
                target TEXT NOT NULL,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                exit_code INTEGER,
                archive_name TEXT,
                archive_bytes INTEGER,
                archive_sha256 TEXT,
                detail TEXT NOT NULL DEFAULT '',
                log_path TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS {prefix}_runs_by_capability
                ON {prefix}_runs (capability, started_at);

            -- A rehearsal outcome per target. F3's claim is that a target is offered as
            -- verified only when one of these says so.
            CREATE TABLE IF NOT EXISTS {prefix}_verifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                target TEXT NOT NULL,
                capability TEXT NOT NULL,
                at TEXT NOT NULL,
                verdict TEXT NOT NULL CHECK (verdict IN ('verified','failed','unchecked')),
                detail TEXT NOT NULL DEFAULT '',
                archive_name TEXT,
                archive_sha256 TEXT
            );

            -- The interval, as data. NULL means off, which is a value the operator can
            -- choose rather than an absence with a different meaning.
            CREATE TABLE IF NOT EXISTS {prefix}_policy (
                target TEXT PRIMARY KEY,
                interval_hours INTEGER,
                updated_at TEXT NOT NULL
            );
            "
        ))?;
        Ok(())
    }
}

fn validate_prefix(prefix: &str) -> Fallible<()> {
    let shaped = !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if shaped {
        Ok(())
    } else {
        Err(format!("invalid table prefix {prefix:?}").into())
    }
}

impl BackupStore {
    /// Upsert the declared targets, preserving what each has already proved.
    ///
    /// The declarations come from the tools on every status call, so a target that is no
    /// longer declared disappears from the surface while its verification history stays —
    /// deleting history because a manifest changed would erase the evidence for the
    /// question the following day asks.
    pub fn refresh_targets(&self, decls: &[TargetDecl]) -> Fallible<()> {
        let conn = self.conn()?;
        for decl in decls {
            let declared_by = serde_json::to_string(&decl.declared_by)?;
            conn.execute(
                &format!(
                    "INSERT INTO {}_targets (id, kind, path, host, present, declared_by, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, {NOW})
                     ON CONFLICT(id) DO UPDATE SET
                        kind = excluded.kind,
                        path = excluded.path,
                        host = excluded.host,
                        present = excluded.present,
                        declared_by = excluded.declared_by,
                        seen_at = excluded.seen_at",
                    self.prefix
                ),
                params![
                    decl.id,
                    decl.kind,
                    decl.path,
                    decl.host,
                    decl.present,
                    declared_by
                ],
            )?;
        }
        Ok(())
    }

    pub fn targets(&self) -> Fallible<Vec<TargetRow>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT t.id, t.kind, t.path, t.host, t.present, t.declared_by, t.seen_at,
                    t.verified_at, t.verified_verdict, t.verified_detail, p.interval_hours
               FROM {p}_targets t
               LEFT JOIN {p}_policy p ON p.target = t.id
              ORDER BY t.id",
            p = self.prefix
        ))?;
        let rows = statement.query_map([], |row| {
            let declared: String = row.get(5)?;
            Ok(TargetRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                path: row.get(2)?,
                host: row.get(3)?,
                present: row.get(4)?,
                declared_by: serde_json::from_str(&declared).unwrap_or_default(),
                seen_at: row.get(6)?,
                verified_at: row.get(7)?,
                verified_verdict: row.get(8)?,
                verified_detail: row.get(9)?,
                interval_hours: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn start_run(&self, capability: &str, target: &str, log_path: &str) -> Fallible<i64> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {}_runs (capability, target, started_at, log_path)
                 VALUES (?1, ?2, {NOW}, ?3)",
                self.prefix
            ),
            params![capability, target, log_path],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Close a run. `archive` is what the producer recorded, or `None` when the run
    /// produced nothing to record — which is itself the answer for a failed run.
    pub fn finish_run(
        &self,
        id: i64,
        exit_code: i64,
        archive: Option<&ArchiveIdentity>,
        detail: &str,
    ) -> Fallible<()> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "UPDATE {}_runs
                    SET finished_at = {NOW}, exit_code = ?2,
                        archive_name = ?3, archive_bytes = ?4, archive_sha256 = ?5,
                        detail = ?6
                  WHERE id = ?1",
                self.prefix
            ),
            params![
                id,
                exit_code,
                archive.map(|a| a.name.as_str()),
                archive.map(|a| a.bytes),
                archive.map(|a| a.sha256.as_str()),
                detail,
            ],
        )?;
        Ok(())
    }

    fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
        let name: Option<String> = row.get(8)?;
        Ok(RunRow {
            id: row.get(0)?,
            capability: row.get(1)?,
            target: row.get(2)?,
            started_at: row.get(3)?,
            started_epoch: row.get(4)?,
            finished_at: row.get(5)?,
            finished_epoch: row.get(6)?,
            exit_code: row.get(7)?,
            archive: name.map(|name| ArchiveIdentity {
                name,
                bytes: row.get::<_, Option<i64>>(9).ok().flatten().unwrap_or(0),
                sha256: row
                    .get::<_, Option<String>>(10)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            }),
            detail: row.get(11)?,
            log_path: row.get(12)?,
        })
    }

    /// The epoch columns are converted at the SELECT, so the stored column keeps the one
    /// canonical TEXT format while a surface can still order and compare numerically.
    const RUN_COLUMNS: &'static str = "id, capability, target, started_at, \
         CAST(strftime('%s', started_at) AS INTEGER), finished_at, \
         CAST(strftime('%s', finished_at) AS INTEGER), exit_code, \
         archive_name, archive_bytes, archive_sha256, detail, log_path";

    pub fn runs(&self, limit: i64) -> Fallible<Vec<RunRow>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {} FROM {}_runs ORDER BY id DESC LIMIT ?1",
            Self::RUN_COLUMNS,
            self.prefix
        ))?;
        let rows = statement.query_map(params![limit], Self::run_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The newest attempt per capability — the row that answers "did the last run work",
    /// which a receipt cannot answer for a run that failed.
    pub fn latest_runs(&self) -> Fallible<Vec<RunRow>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT {} FROM {}_runs r
              WHERE r.id = (SELECT max(id) FROM {}_runs WHERE capability = r.capability)
              ORDER BY r.capability",
            Self::RUN_COLUMNS,
            self.prefix,
            self.prefix
        ))?;
        let rows = statement.query_map([], Self::run_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn running_count(&self) -> Fallible<i64> {
        let conn = self.conn()?;
        let count = conn.query_row(
            &format!(
                "SELECT count(*) FROM {}_runs WHERE finished_at IS NULL",
                self.prefix
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Is a run for this capability still in flight?
    ///
    /// Asked before starting another, because two runs on one capability are not a wasted
    /// fork: for a SQLite contract both drive the same maintenance hold, and the first to
    /// finish resumes the capability out from under the second — which is how a "coherent
    /// cold snapshot" stops being either.
    ///
    /// Bounded by a staleness window. A row is only left unfinished by a process that died
    /// mid-run, and such a row must not refuse every future run: the guard is against a
    /// concurrent writer, and a run from six hours ago has no writer.
    pub fn running_for(&self, capability: &str) -> Fallible<bool> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            &format!(
                "SELECT count(*) FROM {p}_runs
                  WHERE capability = ?1 AND finished_at IS NULL
                    AND started_at > datetime('now', '-6 hours')",
                p = self.prefix
            ),
            params![capability],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn record_verification(
        &self,
        target: &str,
        capability: &str,
        verdict: &str,
        detail: &str,
        archive: Option<&ArchiveIdentity>,
    ) -> Fallible<()> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_verifications
                    (target, capability, at, verdict, detail, archive_name, archive_sha256)
                 VALUES (?1, ?2, {NOW}, ?3, ?4, ?5, ?6)",
                p = self.prefix
            ),
            params![
                target,
                capability,
                verdict,
                detail,
                archive.map(|a| a.name.as_str()),
                archive.map(|a| a.sha256.as_str()),
            ],
        )?;
        // The current answer lives on the target row, so a reader that asks "is this target
        // verified" does not have to reconstruct it; the rows above are the history of how
        // it got there.
        conn.execute(
            &format!(
                "UPDATE {}_targets
                    SET verified_at = {NOW}, verified_verdict = ?2, verified_detail = ?3
                  WHERE id = ?1",
                self.prefix
            ),
            params![target, verdict, detail],
        )?;
        Ok(())
    }

    pub fn latest_verification(&self, target: &str) -> Fallible<Option<VerificationRow>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                &format!(
                    "SELECT id, target, capability, at, verdict, detail, archive_name, archive_sha256
                       FROM {}_verifications WHERE target = ?1 ORDER BY id DESC LIMIT 1",
                    self.prefix
                ),
                params![target],
                |row| {
                    let name: Option<String> = row.get(6)?;
                    Ok(VerificationRow {
                        id: row.get(0)?,
                        target: row.get(1)?,
                        capability: row.get(2)?,
                        at: row.get(3)?,
                        verdict: row.get(4)?,
                        detail: row.get(5)?,
                        archive: name.map(|name| ArchiveIdentity {
                            name,
                            bytes: 0,
                            sha256: row.get::<_, Option<String>>(7).ok().flatten().unwrap_or_default(),
                        }),
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Set the interval for a target. `None` is `off`, stored as NULL so "off" is a value
    /// the operator chose rather than a row that happens to be missing.
    pub fn set_policy(&self, target: &str, interval_hours: Option<i64>) -> Fallible<()> {
        if let Some(hours) = interval_hours {
            if hours < crate::policy::MIN_INTERVAL_HOURS {
                return Err(format!(
                    "interval_hours must be at least {}",
                    crate::policy::MIN_INTERVAL_HOURS
                )
                .into());
            }
        }
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {p}_policy (target, interval_hours, updated_at)
                 VALUES (?1, ?2, {NOW})
                 ON CONFLICT(target) DO UPDATE SET
                    interval_hours = excluded.interval_hours,
                    updated_at = excluded.updated_at",
                p = self.prefix
            ),
            params![target, interval_hours],
        )?;
        Ok(())
    }

    /// Every target with an interval set, and that interval. The policy loop's input.
    pub fn active_policies(&self) -> Fallible<Vec<(String, i64)>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT target, interval_hours FROM {}_policy
              WHERE interval_hours IS NOT NULL ORDER BY target",
            self.prefix
        ))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// When a capability last landed a run against a target, as epoch seconds.
    ///
    /// Converted at the query rather than stored: the column keeps the one canonical TEXT
    /// format, and `strftime` is what turns it into a number for the due arithmetic.
    pub fn last_success_epoch(&self, capability: &str, target: &str) -> Fallible<Option<i64>> {
        let conn = self.conn()?;
        let epoch = conn
            .query_row(
                &format!(
                    "SELECT CAST(strftime('%s', max(started_at)) AS INTEGER)
                       FROM {}_runs
                      WHERE capability = ?1 AND target = ?2 AND finished_at IS NOT NULL
                        AND exit_code = 0",
                    self.prefix
                ),
                params![capability, target],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        Ok(epoch)
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;

    fn store(name: &str) -> (BackupStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("backup-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        let store = BackupStore::open(&path).expect("the store opens");
        (store, path)
    }

    fn decl(id: &str, kind: &str, present: &str, caps: &[&str]) -> TargetDecl {
        TargetDecl {
            id: id.to_string(),
            kind: kind.to_string(),
            path: format!("/tmp/{id}"),
            host: String::new(),
            present: present.to_string(),
            declared_by: caps.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn a_failed_run_is_recorded_and_is_not_a_success() {
        let (store, _path) = store("failed-run");
        store
            .refresh_targets(&[decl("t", "local", "true", &["store"])])
            .unwrap();
        let id = store.start_run("store", "t", "/tmp/log").unwrap();
        store
            .finish_run(
                id,
                1,
                None,
                "icloud-item: upload failed: Couldn’t access your iCloud account",
            )
            .unwrap();

        let runs = store.latest_runs().unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].failed(), "a non-zero exit must read as failed");
        assert!(
            runs[0].detail.contains("iCloud account"),
            "the reason survives the run: {}",
            runs[0].detail
        );

        // The defect this crate exists for: a failed run must not defer the next attempt,
        // because it has no successful archive behind it.
        assert_eq!(store.last_success_epoch("store", "t").unwrap(), None);
        let policies = vec![("t".to_string(), 24_i64)];
        let contracts = vec![("store".to_string(), "t".to_string())];
        let due = crate::policy::due_runs(
            &policies,
            &contracts,
            |_, _| store.last_success_epoch("store", "t").unwrap(),
            1_000_000,
            false,
        );
        assert_eq!(due.len(), 1, "a failed run leaves the target due");
    }

    #[test]
    fn a_successful_run_records_its_archive_and_defers_the_next() {
        let (store, _path) = store("successful-run");
        store
            .refresh_targets(&[decl("t", "local", "true", &["store"])])
            .unwrap();
        let id = store.start_run("store", "t", "/tmp/log").unwrap();
        let archive = ArchiveIdentity {
            name: "store-20260929T200000Z.tar.gz".into(),
            bytes: 47_208_702,
            sha256: "a".repeat(64),
        };
        store.finish_run(id, 0, Some(&archive), "").unwrap();

        let runs = store.latest_runs().unwrap();
        assert_eq!(runs[0].archive.as_ref(), Some(&archive));
        assert!(store.last_success_epoch("store", "t").unwrap().is_some());
    }

    #[test]
    fn verification_updates_the_target_and_keeps_its_history() {
        let (store, _path) = store("verification");
        store
            .refresh_targets(&[decl("t", "local", "true", &["store"])])
            .unwrap();
        store
            .record_verification("t", "store", "failed", "the bytes were not there", None)
            .unwrap();
        store
            .record_verification("t", "store", "verified", "hashed and restored", None)
            .unwrap();

        let target = &store.targets().unwrap()[0];
        assert_eq!(target.verified_verdict.as_deref(), Some("verified"));
        assert!(target.verified_at.is_some());
        // The failure is still on the record: a target that once failed and now passes is
        // not the same fact as one that never failed.
        let latest = store.latest_verification("t").unwrap().unwrap();
        assert_eq!(latest.verdict, "verified");
        let conn = store.conn().unwrap();
        let count: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {}_verifications", store.prefix),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn policy_round_trips_and_off_is_a_value() {
        let (store, _path) = store("policy");
        store
            .refresh_targets(&[decl("t", "local", "true", &["store"])])
            .unwrap();
        assert!(store.active_policies().unwrap().is_empty());

        store.set_policy("t", Some(24)).unwrap();
        assert_eq!(
            store.active_policies().unwrap(),
            vec![("t".to_string(), 24)]
        );
        assert_eq!(store.targets().unwrap()[0].interval_hours, Some(24));

        // Off is stored, not deleted: the target still reports "off" rather than "no policy".
        store.set_policy("t", None).unwrap();
        assert!(store.active_policies().unwrap().is_empty());
        assert_eq!(store.targets().unwrap()[0].interval_hours, None);

        assert!(
            store.set_policy("t", Some(0)).is_err(),
            "0 hours is not an interval"
        );
    }

    #[test]
    fn a_target_that_stops_being_declared_keeps_its_verification() {
        let (store, _path) = store("undeclared");
        store
            .refresh_targets(&[decl("t", "local", "true", &["store"])])
            .unwrap();
        store
            .record_verification("t", "store", "verified", "hashed and restored", None)
            .unwrap();
        store.refresh_targets(&[]).unwrap();

        let target = &store.targets().unwrap()[0];
        assert_eq!(target.present, "true");
        assert_eq!(target.verified_verdict.as_deref(), Some("verified"));
    }
}

#[cfg(test)]
mod staleness_tests {
    use super::*;

    /// A row left unfinished by a process that died must not refuse every future run.
    #[test]
    fn an_abandoned_run_stops_counting_after_the_window() {
        let dir = std::env::temp_dir().join(format!("backup-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stale.db");
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        let store = BackupStore::open(&path).unwrap();
        let id = store.start_run("store", "t", "/tmp/log").unwrap();
        assert!(
            store.running_for("store").unwrap(),
            "a fresh run is in flight"
        );

        // Backdate the row past the window rather than waiting six hours for it.
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {}_runs SET started_at = datetime('now', '-7 hours') WHERE id = ?1",
                    store.prefix()
                ),
                params![id],
            )
            .unwrap();
        assert!(
            !store.running_for("store").unwrap(),
            "an abandoned run must not block the next one forever"
        );
    }
}
