//! `tools/host-watch` — notice a runaway process, a filling disk or an unexpected wildcard
//! listener, once per run, and write what it found into the shared store.
//!
//! Ported from `tools/host-watch.ts` on 2026-10-04, its test with it. The launcher keeps the path
//! the docs name; `capabilities/host-watch/service.toml` names the built binary directly, so the
//! hourly launchd job starts neither an interpreter nor a shell.
//!
//! ## Notice, then hand off
//!
//! Two host conditions are invisible until they hurt, and both bit on 2026-08-15 (Axon#177): a
//! System Settings Storage pane got stuck at 08:35 and burned 3h29m of CPU before anyone
//! noticed; and the disk half was ALREADY solved — `tools/storage` answered it in seconds — but
//! nothing ran it, so it may as well not have existed. A tool nobody runs and a tool nobody
//! built are the same tool.
//!
//! So this writes a row per finding into its OWN table and NOTHING when the machine is healthy,
//! because a watcher that cries wolf gets muted and a muted watcher is worse than none. No
//! notification machinery: core Sjel has never had a notifier and does not grow one here.
//!
//! ## Who reads it
//!
//! Nothing is listening here — the manifest schema refuses a port on a scheduled job — so
//! `sjel-status` publishes these rows at `/api/sjel-status/host-watch` and the dashboard ranks
//! them on its decision ladder. Ownership does not move with the surface: this tool owns the
//! finding's content, its lifecycle and its table; `sjel-status` only reads.
//!
//! ## What it delegates
//!
//! The free-space half is `tools/storage report --json` and the exposure half is
//! `host-net check --json`, each invoked rather than reimplemented: they own their policy files,
//! their parsing and their exit codes, and re-deriving any of it here would be the second source
//! of truth their own documentation argues against. A non-zero exit from either is data, not a
//! failure — exit 1 is how each reports the loudest thing it can say.
//!
//! ## Exit codes
//!
//! 0 = checked (findings or not) · 1 = could not check · 2 = no policy.
//!
//! ## Differences from the TypeScript, all deliberate
//!
//! The three probes run concurrently here as they did under `Promise.all`. The `--json` payload
//! carries the same fields, in the struct's order rather than the object literal's. The host-net
//! binary is resolved through `CARGO_TARGET_DIR` when it is set, where the TypeScript always
//! looked in `<root>/target/release` — a custom target dir made that check silently report "not
//! built" while the binary existed.

mod pure;

use crate::paths::Paths;
use pure::{
    classify_processes, decide_emission, decide_resolutions, net_finding, parse_ps_output,
    storage_finding, Emission, Finding, FindingRow, NetReport, Proc, StorageReport, WatchPolicy,
};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const HELP: &str = "\
tools/host-watch — notice a runaway process or a filling disk, once per run.

  tools/host-watch              check, record findings
  tools/host-watch --dry-run    check and print; write nothing
  tools/host-watch --json       machine-readable findings
  tools/host-watch -h           this help

Policy: <overlay>/config/host-watch-policy.toml
";

/// Prefixes this capability's tables in the one shared SQLite file (PRD Q45): `host_watch` here
/// means the table `host_watch_findings`. Underscored, not hyphenated — the capability's
/// directory name is `host-watch` and a hyphen is not a legal bare identifier in SQL.
const PREFIX: &str = "host_watch";

/// `CREATE TABLE IF NOT EXISTS` on every run, which is what every Rust capability's migration
/// does too (`libs/sjel-store`) — a job that runs hourly and might be the first thing to touch a
/// fresh database cannot assume someone else went first.
///
/// The partial unique index is the contract: at most one OPEN finding per condition. A plain
/// unique index on `key` would refuse the second generation, which is precisely the history this
/// watcher needs to keep.
const DDL: &str = "\
CREATE TABLE IF NOT EXISTS host_watch_findings (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    generation INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','resolved')),
    title TEXT NOT NULL,
    note TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    resolved_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS host_watch_findings_one_open_per_key
    ON host_watch_findings (key) WHERE status = 'open';
";

fn fail(message: &str, code: u8) -> ExitCode {
    eprintln!("host-watch: {message}");
    ExitCode::from(code)
}

pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let dry_run = argv.iter().any(|a| a == "--dry-run");
    let as_json = argv.iter().any(|a| a == "--json");

    let root = match Paths::from_env() {
        Ok(p) => p.root,
        Err(e) => return fail(&e, 1),
    };

    // The overlay's own reader, not `Paths`: `sjel-config` is what resolves the overlay and the
    // database for every capability, and `tools/storage` reads its policy the same way. The
    // policy lives in the overlay, so this exits 2 before anything needs the database.
    let Some(policy_path) = sjel_config::overlay_config("host-watch-policy.toml") else {
        eprintln!(
            "host-watch: no overlay to resolve the policy from; set SJEL_PERSONAL_ROOT\n\
             See schemas/host-watch-policy.toml.example for the expected shape."
        );
        return ExitCode::from(2);
    };
    if !policy_path.is_file() {
        eprintln!(
            "host-watch: no policy at {}\n\
             See schemas/host-watch-policy.toml.example for the expected shape.",
            policy_path.display()
        );
        return ExitCode::from(2);
    }
    let policy: WatchPolicy = match std::fs::read_to_string(&policy_path)
        .map_err(|e| e.to_string())
        .and_then(|text| toml::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => return fail(&format!("{}: {e}", policy_path.display()), 1),
    };

    // All three at once, as `Promise.all` did: the wall cost is storage's `du`/`df` and it is
    // almost entirely I/O wait, so running them in series would add the two cheap ones to it.
    let (procs, storage, net) = std::thread::scope(|scope| {
        let ps = scope.spawn(run_ps);
        let st = scope.spawn(|| run_storage(&root));
        let hn = scope.spawn(|| run_host_net(&root));
        (ps.join(), st.join(), hn.join())
    });
    let procs: Vec<Proc> = match procs {
        Ok(Ok(procs)) => procs,
        _ => return fail("ps failed", 1),
    };
    let storage = storage.unwrap_or(None);
    let net = net.unwrap_or(None);

    let proc_findings = classify_processes(&procs, &policy);
    let mut findings: Vec<Finding> = proc_findings.iter().map(|f| f.finding()).collect();
    findings.extend(storage.as_ref().and_then(storage_finding));
    findings.extend(net_finding(net.as_ref()));

    if as_json {
        let mut values: Vec<serde_json::Value> = proc_findings
            .iter()
            .map(|f| serde_json::to_value(f).unwrap_or(serde_json::Value::Null))
            .collect();
        values.extend(
            findings[proc_findings.len()..]
                .iter()
                .map(|f| serde_json::to_value(f).unwrap_or(serde_json::Value::Null)),
        );
        let payload = serde_json::json!({ "checked": procs.len(), "findings": values });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_owned())
        );
        return ExitCode::SUCCESS;
    }

    if dry_run {
        println!(
            "host-watch: {} finding(s), writing nothing (--dry-run)",
            findings.len()
        );
        for f in &findings {
            println!("  {}", f.title);
        }
        return ExitCode::SUCCESS;
    }

    // A healthy run still writes, and that is the point of the closing half: it is the run with
    // NO findings that clears the rows the last one left open. Returning early here — which this
    // did while `tasks` owned the lifecycle — would mean a condition that cleared stayed on the
    // ladder until the operator noticed it themselves.
    let emitted = match emit(&findings) {
        Ok(e) => e,
        Err(e) => return fail(&e, 1),
    };
    let state = storage
        .as_ref()
        .and_then(|s| s.state.as_deref())
        .unwrap_or("unknown");
    if findings.is_empty() && emitted.resolved == 0 {
        println!(
            "host-watch: {} processes, disk {state} — nothing to report",
            procs.len()
        );
    } else {
        println!(
            "host-watch: {} finding(s) — {} new, {} still open, {} cleared",
            findings.len(),
            emitted.created,
            emitted.refreshed,
            emitted.resolved
        );
    }
    ExitCode::SUCCESS
}

// ── the three probes ──────────────────────────────────────────────────────────

/// `ps -Aceo pid,time,etime,comm`.
fn run_ps() -> Result<Vec<Proc>, String> {
    let out = Command::new("ps")
        .args(["-Aceo", "pid,time,etime,comm"])
        .output()
        .map_err(|e| format!("ps could not run: {e}"))?;
    if !out.status.success() {
        return Err("ps failed".to_owned());
    }
    Ok(parse_ps_output(&String::from_utf8_lossy(&out.stdout)))
}

/// storage is invoked rather than reimplemented: it owns the policy file, the `du`/`df`
/// arithmetic and the exit code, and re-deriving any of that here would be the second source of
/// truth its own header argues against. A non-zero exit is NOT a failure — it is how storage
/// reports free space below critical, which is the loudest thing it can say.
///
/// The launcher, not the binary: it sources `tools/lib/paths.sh`, which is what resolves the
/// overlay, and it builds the release binary if this is the first run after a checkout. The
/// `--json` shape it returns is the contract `tools/storage/src/main.rs` states.
fn run_storage(root: &Path) -> Option<StorageReport> {
    let launcher = root.join("tools").join("storage").join("storage");
    match Command::new(&launcher).args(["report", "--json"]).output() {
        Ok(out) => match serde_json::from_slice::<StorageReport>(&out.stdout) {
            Ok(report) => Some(report),
            Err(_) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let tail: String = stderr.trim().chars().take(200).collect();
                eprintln!(
                    "host-watch: storage --json unreadable (exit {}) {tail}",
                    out.status.code().unwrap_or(-1)
                );
                None
            }
        },
        Err(e) => {
            eprintln!("host-watch: storage --json could not run: {e}");
            None
        }
    }
}

/// host-net is invoked rather than reimplemented, the same call storage gets above: it owns the
/// netstat parsing, the scope rule and the overlay policy.
///
/// The built BINARY, never `capabilities/host-net/host-net`. That launcher builds on first use,
/// and an hourly scheduled job with the ability to start a cargo build is a surprise nobody asked
/// for. A host that has never built it files nothing and says so once.
fn run_host_net(root: &Path) -> Option<NetReport> {
    let bin = host_net_binary(root);
    if !bin.is_file() {
        eprintln!(
            "host-watch: {} not built — network exposure not checked",
            bin.display()
        );
        return None;
    }
    match Command::new(&bin).args(["check", "--json"]).output() {
        // `Command::output` drains both pipes, so the TypeScript's deadlock note is satisfied
        // by construction rather than by draining them together.
        Ok(out) => match serde_json::from_slice::<NetReport>(&out.stdout) {
            Ok(report) => Some(report),
            Err(_) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let tail: String = stderr.trim().chars().take(200).collect();
                eprintln!(
                    "host-watch: host-net check --json unreadable (exit {}) {tail}",
                    out.status.code().unwrap_or(-1)
                );
                None
            }
        },
        Err(e) => {
            eprintln!("host-watch: host-net check --json could not run: {e}");
            None
        }
    }
}

/// `<root>/target/release/host-net-cli`, or the same path under `CARGO_TARGET_DIR` when this
/// shell has one — the directory the workspace is actually built into, which `tools/lib/sjel-cli.sh`
/// resolves the same way.
fn host_net_binary(root: &Path) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    target.join("release").join("host-net-cli")
}

// ── the store ─────────────────────────────────────────────────────────────────

/// What this run changed, for the one line it prints.
struct Emitted {
    created: usize,
    refreshed: usize,
    resolved: usize,
}

/// Write this run's verdict: open what is new, refresh what persists, close what cleared.
///
/// One transaction, because a half-applied run is a lie about the machine — a finding resolved
/// with its replacement not yet written reads as "nothing wrong" for exactly as long as it takes
/// the next hour to arrive.
fn emit(findings: &[Finding]) -> Result<Emitted, String> {
    // `sjel_config::database_path` owns the resolution — `SJEL_DB_PATH`, then the overlay — so
    // a tool that opened a different file than the capabilities do would write findings nothing
    // reads. It is only reached with an overlay present or with the variable set, because the
    // policy above already failed otherwise, so its no-overlay scratch fallback cannot be hit.
    let path = sjel_config::database_path();
    let pool = sjel_store::open_pool(&path, PREFIX, |conn| {
        conn.execute_batch(DDL)?;
        Ok(())
    })
    .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut conn = pool.get().map_err(|e| e.to_string())?;

    let existing: Vec<FindingRow> = {
        let mut statement = conn
            .prepare(&format!(
                "SELECT id, key, generation, status FROM {PREFIX}_findings"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(FindingRow {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    generation: row.get(2)?,
                    status: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    // `sjel_store::NOW` is the canonical stamp, spelled the way the TypeScript spelled it.
    let now = sjel_store::NOW;
    let refresh = format!(
        "UPDATE {PREFIX}_findings SET title = ?1, note = ?2, last_seen = {now} WHERE id = ?3"
    );
    let create = format!(
        "INSERT INTO {PREFIX}_findings (id, key, generation, status, title, note, first_seen, last_seen)
         VALUES (?1, ?2, ?3, 'open', ?4, ?5, {now}, {now})"
    );
    let resolve = format!(
        "UPDATE {PREFIX}_findings SET status = 'resolved', resolved_at = {now} WHERE id = ?1"
    );

    let mut counts = Emitted {
        created: 0,
        refreshed: 0,
        resolved: 0,
    };
    let tx = sjel_store::write_transaction(&mut conn).map_err(|e| e.to_string())?;
    for finding in findings {
        match decide_emission(&finding.key, &existing) {
            Emission::Refresh { id } => {
                tx.execute(
                    &refresh,
                    sjel_store::rusqlite::params![finding.title, finding.note, id],
                )
                .map_err(|e| e.to_string())?;
                counts.refreshed += 1;
                println!("host-watch: still open — {}", finding.title);
            }
            Emission::Create { id, generation } => {
                tx.execute(
                    &create,
                    sjel_store::rusqlite::params![
                        id,
                        finding.key,
                        generation,
                        finding.title,
                        finding.note
                    ],
                )
                .map_err(|e| e.to_string())?;
                counts.created += 1;
                println!("host-watch: NEW — {}", finding.title);
            }
        }
    }
    for id in decide_resolutions(findings, &existing) {
        tx.execute(&resolve, sjel_store::rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        counts.resolved += 1;
        println!("host-watch: cleared — {id}");
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(counts)
}
