//! `sjel mirror` — a read-only copy of this machine's data, for a device that is not here.
//!
//! The phone cannot hold an embedded replica of `axon.db`: a replica's primary must own its
//! database file, and this machine's file is opened directly by eighteen capability processes. So
//! the first thing a phone can have is a **copy**, refreshed when asked — [`research/offline-on-the-phone.md`]
//! route R1. It moves no data between files, changes no capability, and closes off none of the
//! larger routes.
//!
//! Consistency is the whole job. The source is opened read-only and every table is copied inside
//! one read transaction, so the copy is a moment rather than a smear of moments while capabilities
//! keep writing. The file is built beside its destination and renamed into place, so a phone
//! fetching during a rebuild sees the previous copy whole or the new one whole, never half of each.
//!
//! [`research/offline-on-the-phone.md`]: ../../../research/offline-on-the-phone.md

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The tables the phone gets by default: what is worth reading when this machine is unreachable,
/// and nothing that should not travel. `--all` exists for a deliberate "everything" and is not a
/// default, because a copy of the whole database carries comms, documents and finance with it.
const PHONE_SET: &[&str] = &[
    "projects_records",
    "calendar_entries",
    "calendar_rhythms",
    "entities_entities",
    "entities_facts",
    "places_places",
    "places_person_places",
];

const HELP: &str = "\
sjel mirror — a read-only copy of this machine's data, for a device that is not here.

  build [--all | --tables a,b,c] [--out <path>]
      Build the copy. Defaults to the phone set (projects, calendar, entities, places) at
      <overlay>/data/mirror/mirror.db. Copied inside one read transaction and renamed into place,
      so a reader never sees half a build. --all copies every table in the database.

  status [--json]
      When it was built, from where, what it holds, how big, and its digest.

  path
      Print where the copy lives, whether or not it has been built.

The copy is read-only by intent: nothing writes to it. Writes made on the device do not come back.
The setting the phone reads is the manifest, which is what a fetch should compare before shipping a
whole database over the tailnet.";

/// One table's contribution, so the manifest can be read without opening the copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    name: String,
    rows: u64,
}

/// What a build produced. Written beside the copy as `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub built_at: String,
    /// `ok`, or what `PRAGMA integrity_check` said instead.
    pub integrity: String,
    /// Foreign keys the copy breaks. Zero for `--all`; a subset is expected to break some, because
    /// a table whose parent was not selected still declares the reference.
    pub foreign_key_violations: u64,
    pub source: String,
    pub mode: String,
    pub tables: Vec<Table>,
    pub rows: u64,
    pub bytes: u64,
    pub sha256: String,
    pub path: String,
}

pub fn run(rest: &[String]) -> ExitCode {
    let mut words: Vec<String> = Vec::new();
    let mut all = false;
    let mut tables: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut json = false;

    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--all" => all = true,
            "--tables" => match iter.next() {
                Some(value) => tables = Some(value.clone()),
                None => return usage("--tables needs a comma-separated list"),
            },
            "--out" => match iter.next() {
                Some(value) => out = Some(PathBuf::from(value)),
                None => return usage("--out needs a path"),
            },
            other if other.starts_with('-') => return usage(&format!("unknown flag '{other}'")),
            other => words.push(other.to_owned()),
        }
    }

    if all && tables.is_some() {
        return usage("--all and --tables say different things; pick one");
    }

    match words.first().map_or("status", String::as_str) {
        "build" => {
            let wanted: Option<Vec<String>> = match (&tables, all) {
                (Some(list), _) => Some(
                    list.split(',')
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty())
                        .collect(),
                ),
                (None, true) => None,
                (None, false) => Some(PHONE_SET.iter().map(|name| (*name).to_owned()).collect()),
            };
            match build(wanted.as_deref(), out.as_deref()) {
                Ok(manifest) => {
                    if json {
                        print_json(&manifest);
                    } else {
                        println!(
                            "{}: {} table(s), {} row(s), {} written",
                            manifest.path,
                            manifest.tables.len(),
                            manifest.rows,
                            bytes(manifest.bytes)
                        );
                        println!("sha256: {}", manifest.sha256);
                    }
                    ExitCode::SUCCESS
                }
                Err(message) => {
                    eprintln!("sjel mirror build: {message}");
                    ExitCode::FAILURE
                }
            }
        }
        "status" => match status() {
            Ok(Some(manifest)) => {
                if json {
                    print_json(&manifest);
                } else {
                    println!("{}", manifest.path);
                    println!("built {} from {}", manifest.built_at, manifest.source);
                    println!(
                        "{} table(s), {} row(s), {}",
                        manifest.tables.len(),
                        manifest.rows,
                        bytes(manifest.bytes)
                    );
                    println!(
                        "integrity {}, {} foreign key(s) broken by the selection",
                        manifest.integrity, manifest.foreign_key_violations
                    );
                    println!("sha256: {}", manifest.sha256);
                }
                ExitCode::SUCCESS
            }
            Ok(None) => {
                if json {
                    print_json(&serde_json::json!({ "built": false }));
                } else {
                    println!("no copy yet — run `sjel mirror build`");
                }
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("sjel mirror status: {message}");
                ExitCode::FAILURE
            }
        },
        "path" => match destination() {
            Ok(path) => {
                println!("{}", path.display());
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("sjel mirror path: {message}");
                ExitCode::FAILURE
            }
        },
        other => usage(&format!("unknown verb '{other}'")),
    }
}

/// Where the copy and its manifest live: `<overlay>/data/mirror/`.
fn destination() -> Result<PathBuf, String> {
    let dir = sjel_config::overlay_data_dir("mirror")
        .ok_or("no overlay resolved, so there is nowhere to put a copy")?;
    Ok(dir.join("mirror.db"))
}

/// The manifest beside the copy named `mirror.db`.
pub fn manifest_path(database: &Path) -> PathBuf {
    database.with_file_name("manifest.json")
}

/// Build the copy. `None` means every table.
fn build(tables: Option<&[String]>, out: Option<&Path>) -> Result<Manifest, String> {
    let source = sjel_config::database_path();
    if !source.is_file() {
        return Err(format!(
            "{} is not a file, so there is nothing to copy",
            source.display()
        ));
    }
    let target = match out {
        Some(path) => path.to_path_buf(),
        None => destination()?,
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    // Built beside the destination, then renamed: a device fetching during a rebuild gets the
    // previous copy whole or the new one whole, never half of each.
    let staged = target.with_extension("db.building");
    let _ = std::fs::remove_file(&staged);

    let wanted = names_to_copy(&source, tables)?;
    let checks = copy_tables(&source, &staged, &wanted)?;
    if checks.integrity != "ok" {
        return Err(format!(
            "the copy failed its own integrity check: {}",
            checks.integrity
        ));
    }

    let bytes = std::fs::metadata(&staged)
        .map_err(|e| format!("cannot measure {}: {e}", staged.display()))?
        .len();
    let digest = sha256_of(&staged)?;
    std::fs::rename(&staged, &target)
        .map_err(|e| format!("cannot move the copy into place: {e}"))?;

    let rows = wanted.iter().map(|t| t.rows).sum();
    let manifest = Manifest {
        built_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        integrity: checks.integrity.clone(),
        foreign_key_violations: checks.foreign_key_violations,
        source: source.display().to_string(),
        mode: match tables {
            Some(_) => "selected".to_owned(),
            None => "all".to_owned(),
        },
        tables: wanted,
        rows,
        bytes,
        sha256: digest,
        path: target.display().to_string(),
    };
    let body = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("cannot render the manifest: {e}"))?;
    std::fs::write(manifest_path(&target), body)
        .map_err(|e| format!("cannot write the manifest: {e}"))?;
    Ok(manifest)
}

/// Which tables to copy, and how many rows each holds.
///
/// `None` means every table. A named table that does not exist is an error rather than a silent
/// omission: a copy missing the table the phone needs is worse than no copy, because it looks like
/// a success.
fn names_to_copy(source: &Path, tables: Option<&[String]>) -> Result<Vec<Table>, String> {
    let conn = rusqlite::Connection::open_with_flags(
        source,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("cannot open {} read-only: {e}", source.display()))?;

    let mut present: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| format!("cannot read the schema: {e}"))?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("cannot read the schema: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("cannot read the schema: {e}"))?;

    let wanted: Vec<String> = match tables {
        Some(list) => {
            for name in list {
                if !present.iter().any(|p| p == name) {
                    return Err(format!("no table named '{name}' in {}", source.display()));
                }
            }
            list.to_vec()
        }
        None => {
            present.sort();
            present
        }
    };

    wanted
        .into_iter()
        .map(|name| {
            let rows: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\"")),
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("cannot count {name}: {e}"))?;
            let rows = u64::try_from(rows).unwrap_or(0);
            Ok(Table { name, rows })
        })
        .collect()
}

/// Copy each table's schema and rows into a fresh file, inside one read transaction.
struct Checks {
    integrity: String,
    foreign_key_violations: u64,
}

fn copy_tables(source: &Path, target: &Path, tables: &[Table]) -> Result<Checks, String> {
    // A copy is built fresh: an existing file would answer "table already exists" on the first
    // `CREATE TABLE`, and half-copying into a stale file is the failure this refuses.
    let _ = std::fs::remove_file(target);
    let _ = std::fs::remove_file(target.with_extension("db-wal"));
    let _ = std::fs::remove_file(target.with_extension("db-shm"));
    let conn = rusqlite::Connection::open(target)
        .map_err(|e| format!("cannot create {}: {e}", target.display()))?;
    // The source is attached to a connection whose main schema IS the copy, so the schema text
    // read out of `sqlite_master` can be executed as-is: it lands in the copy, not in the source.
    conn.execute(
        &format!(
            "ATTACH DATABASE '{}' AS src",
            source.display().to_string().replace('\'', "''")
        ),
        [],
    )
    .map_err(|e| format!("cannot attach {}: {e}", source.display()))?;

    // One transaction on both databases: every table is copied from the same moment, so the copy
    // is consistent even while eighteen capabilities keep writing the source.
    // A copy is a snapshot, not a live database being written: a selected subset breaks the
    // references of tables it did not take, and refusing that would refuse the phone set. The
    // checks below report what was broken instead of pretending nothing was.
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(|e| format!("cannot relax constraints on the copy: {e}"))?;
    conn.execute_batch("BEGIN")
        .map_err(|e| format!("cannot start a transaction: {e}"))?;
    let outcome = (|| -> Result<(), String> {
        for table in tables {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM src.sqlite_master WHERE type = 'table' AND name = ?1",
                    [&table.name],
                    |row| row.get(0),
                )
                .map_err(|e| format!("cannot read the schema of {}: {e}", table.name))?;
            conn.execute_batch(&sql)
                .map_err(|e| format!("cannot create {}: {e}", table.name))?;
            conn.execute(
                &format!(
                    "INSERT INTO \"{}\" SELECT * FROM src.\"{}\"",
                    table.name.replace('"', "\"\""),
                    table.name.replace('"', "\"\"")
                ),
                [],
            )
            .map_err(|e| format!("cannot copy {}: {e}", table.name))?;
            // Indexes, so a read on the phone is not a table scan. Failing to rebuild one is not
            // worth refusing the copy over: the data is the point and the index is speed.
            let mut indexes = conn
                .prepare(
                    "SELECT sql FROM src.sqlite_master WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL",
                )
                .map_err(|e| format!("cannot read the indexes of {}: {e}", table.name))?;
            let statements: Vec<String> = indexes
                .query_map([&table.name], |row| row.get::<_, String>(0))
                .map_err(|e| format!("cannot read the indexes of {}: {e}", table.name))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("cannot read the indexes of {}: {e}", table.name))?;
            for statement in statements {
                if let Err(error) = conn.execute_batch(&statement) {
                    eprintln!("sjel mirror: index for {} skipped: {error}", table.name);
                }
            }
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("cannot commit the copy: {e}"))?;
            let integrity: String = conn
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap_or_else(|e| format!("integrity_check refused to run: {e}"));
            let foreign_key_violations: u64 = conn
                .prepare("PRAGMA foreign_key_check")
                .and_then(|mut stmt| {
                    stmt.query_map([], |_| Ok(()))?
                        .collect::<Result<Vec<_>, _>>()
                })
                .map(|rows| rows.len() as u64)
                .unwrap_or(0);
            Ok(Checks {
                integrity,
                foreign_key_violations,
            })
        }
        Err(message) => {
            let _ = conn.execute_batch("ROLLBACK");
            let _ = std::fs::remove_file(target);
            Err(message)
        }
    }
}

fn status() -> Result<Option<Manifest>, String> {
    let path = destination()?;
    let body = match std::fs::read_to_string(manifest_path(&path)) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read the manifest: {error}")),
    };
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|error| format!("the manifest is not readable: {error}"))
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.1} MB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1} kB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}");
    eprintln!(
        "Usage: sjel mirror [build|status|path] [--all|--tables a,b,c] [--out <path>] [--json]"
    );
    ExitCode::from(2)
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => eprintln!("sjel mirror: cannot render JSON: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sjel-mirror-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A source with two tables, one index, and a row in each.
    fn seeded(dir: &Path) -> PathBuf {
        let path = dir.join("source.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE projects_records (id TEXT PRIMARY KEY, title TEXT);
             CREATE INDEX projects_records_title ON projects_records(title);
             CREATE TABLE comms_feed_items (id TEXT PRIMARY KEY, body TEXT);
             INSERT INTO projects_records VALUES ('a', 'a task'), ('b', 'another');
             INSERT INTO comms_feed_items VALUES ('c', 'a mail');",
        )
        .unwrap();
        path
    }

    #[test]
    fn a_selected_copy_holds_the_named_tables_rows_and_indexes() {
        let dir = scratch("selected");
        let source = seeded(&dir);
        let target = dir.join("mirror.db");

        let tables = names_to_copy(&source, Some(&["projects_records".to_owned()])).unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows, 2);
        let checks = copy_tables(&source, &target, &tables).unwrap();
        assert_eq!(checks.integrity, "ok");

        let copy = rusqlite::Connection::open(&target).unwrap();
        let rows: i64 = copy
            .query_row("SELECT COUNT(*) FROM projects_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
        // The table nobody asked for is absent, not empty.
        let other: i64 = copy
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='comms_feed_items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other, 0);
        let indexes: i64 = copy
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='projects_records_title'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 1, "an index is what makes a phone read fast");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every table when none is named, which is what `--all` asks for.
    #[test]
    fn a_full_copy_holds_every_table() {
        let dir = scratch("all");
        let source = seeded(&dir);
        let target = dir.join("mirror.db");
        let tables = names_to_copy(&source, None).unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["comms_feed_items", "projects_records"]);
        let checks = copy_tables(&source, &target, &tables).unwrap();
        assert_eq!(checks.integrity, "ok");
        let copy = rusqlite::Connection::open(&target).unwrap();
        let rows: i64 = copy
            .query_row("SELECT COUNT(*) FROM comms_feed_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A named table that is not there is refused: a copy that silently lacks the table the phone
    /// needed looks exactly like a successful one.
    #[test]
    fn a_table_that_is_not_there_is_refused_by_name() {
        let dir = scratch("missing");
        let source = seeded(&dir);
        let err = names_to_copy(&source, Some(&["projects_taskz".to_owned()])).unwrap_err();
        assert!(err.contains("no table named 'projects_taskz'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The digest is of the file that was written, so a fetch can decide whether it needs the copy.
    #[test]
    fn the_digest_changes_when_the_content_does() {
        let dir = scratch("digest");
        let source = seeded(&dir);
        let target = dir.join("mirror.db");
        let tables = names_to_copy(&source, None).unwrap();
        let checks = copy_tables(&source, &target, &tables).unwrap();
        assert_eq!(checks.integrity, "ok");
        let first = sha256_of(&target).unwrap();

        let conn = rusqlite::Connection::open(&source).unwrap();
        conn.execute("INSERT INTO projects_records VALUES ('c', 'third')", [])
            .unwrap();
        let tables = names_to_copy(&source, None).unwrap();
        let checks = copy_tables(&source, &target, &tables).unwrap();
        assert_eq!(checks.integrity, "ok");
        assert_ne!(first, sha256_of(&target).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
