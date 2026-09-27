use super::*;

/// The hourly host watch's open findings, served for the dashboard's decision ladder.
///
/// ## Why this capability serves another capability's table
///
/// `host-watch` is a scheduled job: it runs, writes, and exits (`schedule = "1h"`, and
/// the manifest schema refuses a port on a scheduled job because nothing would be
/// listening on it). So it cannot serve its own findings, and something that is always
/// up has to. That is this process — the one `autostart = "true"` surface, and already
/// the machine's answer to "what is wrong here".
///
/// The precedent is next door: `/api/axon-status/backups` publishes receipts written by
/// `tools/backup.sh`, which is also a job with no server. Reading across a capability
/// boundary inside the one shared file is what that file is for
/// (`capabilities/store/README.md`, "cross-schema joins within one database are a single
/// connection").
///
/// Ownership does not move with the surface. `host-watch` owns the finding's content,
/// its lifecycle and its table; this reads and never writes.
///
/// ## Why a missing table is an empty list
///
/// The table is created by the first `tools/host-watch` run on a machine. A deployment
/// that has never scheduled the watch has no table, and the honest answer for it is "no
/// findings", not an error — an error here would put a red card on the dashboard of a
/// machine with nothing wrong. A database that cannot be opened at all is reported,
/// because that is a real fault.
#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub(crate) struct HostWatchFinding {
    pub(crate) id: String,
    /// The condition, e.g. `cpu:ApplicationsStorageExtension`. Stable across runs and
    /// across reboots: it names the command, never the pid.
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) note: String,
    pub(crate) first_seen: String,
    pub(crate) last_seen: String,
}

/// Open findings only, worst-persisting first.
///
/// Resolved rows stay in the table as history and are deliberately not served: this
/// endpoint feeds a list of things to decide about, and a condition that cleared has
/// nothing left to decide.
fn read_findings(database: &std::path::Path) -> Result<Vec<HostWatchFinding>, String> {
    let pool = sjel_store::pool_for(database).map_err(|e| e.to_string())?;
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut statement = match conn.prepare(
        "SELECT id, key, title, note, first_seen, last_seen
         FROM host_watch_findings
         WHERE status = 'open'
         ORDER BY first_seen ASC",
    ) {
        Ok(statement) => statement,
        // `no such table` is the never-run case above. Any other prepare failure is a
        // real fault and is reported rather than flattened into an empty list.
        Err(error) if error.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let rows = statement
        .query_map([], |row| {
            Ok(HostWatchFinding {
                id: row.get(0)?,
                key: row.get(1)?,
                title: row.get(2)?,
                note: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub(crate) async fn host_watch_handler() -> (StatusCode, Json<Value>) {
    let database = sjel_config::database_path();
    // Blocking file I/O off the async runtime, the same rule every other store reader in
    // this repo follows: a busy writer makes a read wait out `busy_timeout`, and that is
    // five seconds of a runtime worker other handlers need.
    match tokio::task::spawn_blocking(move || read_findings(&database)).await {
        Ok(Ok(findings)) => (StatusCode::OK, Json(json!({ "findings": findings }))),
        Ok(Err(error)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "read failed" })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("axon-status-hw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The case that decides whether a healthy machine shows a red card: a deployment
    /// that has never run the watch has no table, and that is zero findings.
    #[test]
    fn a_database_without_the_table_reports_no_findings() {
        let path = scratch("never-run");
        assert_eq!(
            read_findings(&path).expect("an empty database opens"),
            vec![]
        );
    }

    /// Resolved rows are history, not decisions. Serving them would keep a process that
    /// exited last month on the ladder forever.
    #[test]
    fn only_open_findings_are_served() {
        let path = scratch("open-only");
        let pool = sjel_store::pool_for(&path).unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE host_watch_findings (
                 id TEXT PRIMARY KEY, key TEXT NOT NULL, generation INTEGER NOT NULL,
                 status TEXT NOT NULL, title TEXT NOT NULL, note TEXT NOT NULL,
                 first_seen TEXT NOT NULL, last_seen TEXT NOT NULL, resolved_at TEXT);
             INSERT INTO host_watch_findings VALUES
                 ('cpu:Stuck~1','cpu:Stuck',1,'open','Stuck is busy','pid 1','2026-08-01','2026-08-02',NULL),
                 ('cpu:Gone~1','cpu:Gone',1,'resolved','Gone was busy','pid 2','2026-07-01','2026-07-02','2026-07-03');",
        )
        .unwrap();

        let served = read_findings(&path).expect("a readable database");
        assert_eq!(served.len(), 1, "a resolved finding leaked into the list");
        assert_eq!(served[0].key, "cpu:Stuck");
    }
}
