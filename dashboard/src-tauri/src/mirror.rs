//! The phone's copy of the Mac's data, for when the Mac is not reachable.
//!
//! ## What this adds to the store that is already here
//!
//! [`crate::sync`] keeps the last answer to each GET the phone has *already made*, keyed by its URL
//! and query string. That is enough to reopen a page you have visited, and it is nothing on a phone
//! that has never opened the page — a filter you have not selected, a project you have not tapped, a
//! sort that is not the one you saw. This module holds a **whole table set** instead, built on the
//! Mac by `sjel mirror build` (`research/offline-on-the-phone.md`, route R1) and fetched as one
//! file, so the device can answer questions it never asked while the Mac was awake.
//!
//! The two are not a migration of each other yet. What the copy holds is the Mac's decision
//! (`PHONE_SET` in `tools/sjel-cli/src/mirror.rs`); this module never asks for a table set, because
//! a second decision point would be a second answer to one question.
//!
//! ## Why the digest is the whole point
//!
//! A partial database that opens is indistinguishable from a complete one at the point of use, so
//! "the fetch finished" is not evidence of anything. [`Mirror::sync`] refuses a copy whose bytes or
//! sha256 disagree with the manifest, refuses one that fails `PRAGMA integrity_check`, and refuses
//! one whose per-table row counts are not what the manifest claims. The file is written beside its
//! destination and renamed into place, so a reader never sees half a copy, and the manifest is
//! written only after the file it describes.
//!
//! ## Reading it
//!
//! Read-only, always: `SQLITE_OPEN_READ_ONLY`, so no code path can try a write against a copy that
//! would then stop being the Mac's. The queries are named rather than a general SQL command on
//! purpose — the WebView can call any command with any argument, and a `mirror_query(sql)` here
//! would be a database console reachable from a page.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use crate::mac_bridge::now_ms;
use crate::sync::{Outgoing, Reply, SendError, Transport};

/// The copy, in `<app_data_dir>/mirror/`.
const DIR: &str = "mirror";
pub const MIRROR_FILE: &str = "mirror.db";
/// The copy while it is being written: it is renamed over [`MIRROR_FILE`] once it has passed every
/// check, so a reader gets the previous copy whole or the new one whole.
const PART_FILE: &str = "mirror.db.part";
/// What the copy is, fetched with it.
const MANIFEST_FILE: &str = "mirror-manifest.json";

/// The Mac's manifest. The same path a browser on the tailnet reads.
const MANIFEST_PATH: &str = "/sjel-status/api/sjel-status/mirror/manifest";
const COPY_PATH: &str = "/sjel-status/api/sjel-status/mirror";
/// Ask the Mac to build a fresh copy. Bodyless: the caller asks for a rebuild, never for contents.
const REBUILD_PATH: &str = "/sjel-status/api/sjel-status/mirror/rebuild";

/// A manifest is a few hundred bytes; this is the bound that says so.
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
/// The copy's bound. The phone set measured 11.1 MB on 2026-10-21 and 87% of it was note bodies, so
/// this leaves the notes room to grow and still refuses a surprise rather than filling the device
/// with it.
const MAX_COPY_BYTES: usize = 256 * 1024 * 1024;

/// How old a stored copy may be before a sync asks the Mac to rebuild before fetching. A rebuild
/// answers with the new manifest, so the fetch that follows is of the copy that was just built.
pub const REBUILD_AFTER_MS: i64 = 12 * 60 * 60 * 1000;

/// One table's contribution, as the Mac's manifest reports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub rows: u64,
}

/// What the Mac says its copy holds. `path` is the Mac's own path and is deliberately never used
/// here: the device opens its own file, and a manifest that named a path the device followed would
/// be a way to read something else.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub built_at: String,
    /// `ok`, or what `PRAGMA integrity_check` said on the Mac.
    pub integrity: String,
    pub foreign_key_violations: u64,
    pub source: String,
    pub mode: String,
    pub tables: Vec<Table>,
    pub rows: u64,
    pub bytes: u64,
    pub sha256: String,
    pub path: String,
}

/// The manifest as the device keeps it: the Mac's, plus when this device received it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Stored {
    manifest: Manifest,
    fetched_at_ms: i64,
}

/// What the device holds, for the status line.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CopyStatus {
    pub built_at: String,
    /// The Mac's `built_at` as unix milliseconds, when it parses. A copy whose build time cannot be
    /// read is reported as stale rather than fresh: "I cannot date this" must not read as "this is
    /// current".
    pub built_at_ms: Option<i64>,
    pub age_ms: Option<i64>,
    /// True when the copy is older than [`REBUILD_AFTER_MS`], so the next sync rebuilds first.
    pub stale: bool,
    pub fetched_at_ms: i64,
    pub bytes: u64,
    pub rows: u64,
    pub tables: Vec<Table>,
    pub sha256: String,
    /// True when the file on disk matches the digest its manifest records. False is not a slow copy,
    /// it is a copy to throw away — [`Mirror::sync`] refetches it and the reader refuses it.
    pub verified: bool,
    pub integrity: String,
    pub foreign_key_violations: u64,
}

/// What the device holds, or why it holds nothing usable.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MirrorStatus {
    pub copy: Option<CopyStatus>,
    /// Set when a copy exists on disk and does not match its manifest, or a copy exists with no
    /// manifest. Empty when there is simply no copy yet.
    pub problem: Option<String>,
}

/// What one sync did.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncOutcome {
    /// `unchanged`, `fetched` or `rebuilt and fetched`.
    pub action: String,
    /// The bytes that crossed the network for the copy itself. Zero for `unchanged`.
    pub transferred: u64,
    /// Why a rebuild that was due did not happen. The fetch still runs, against whatever the Mac
    /// last built — an hour-old copy is worth more than a failed sync.
    pub rebuild_error: Option<String>,
    pub status: MirrorStatus,
}

/// One record from the copy: the columns every project-shaped row has.
///
/// Not the route's full shape. `/projects/api/tasks` also answers `uri`, `projects`, `data_class`
/// and `fields`, which it reads from vault frontmatter the copy does not carry. A reader that
/// showed those from here would invent them, so this says what it has and nothing else.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Record {
    pub path: String,
    pub project: String,
    pub kind: String,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: i64,
    pub archived: bool,
    pub content: String,
}

/// One project, counted the way `capabilities/projects` counts it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectSummary {
    pub name: String,
    pub archived: bool,
    pub open: usize,
    pub done: usize,
    pub notes: usize,
}

/// The device's copy directory.
pub struct Mirror {
    dir: PathBuf,
}

impl Mirror {
    /// The copy lives beside the phone's store, in its own directory.
    pub fn open<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data directory is not available: {e}"))?
            .join(DIR);
        Ok(Self { dir })
    }

    fn path(&self) -> PathBuf {
        self.dir.join(MIRROR_FILE)
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    fn stored(&self) -> Option<Stored> {
        let body = std::fs::read_to_string(self.manifest_path()).ok()?;
        serde_json::from_str(&body).ok()
    }

    /// What the device holds now. The copy's digest is checked on every call: 11 MB hashes in tens
    /// of milliseconds, and a status line that called an unverified copy ready would be the one
    /// place this design could lie.
    pub fn status(&self) -> MirrorStatus {
        let Some(stored) = self.stored() else {
            // A copy with no manifest cannot be checked, and an unchecked copy is not one this
            // module will claim: say so rather than `None`, which reads as "never fetched".
            let orphan = self.path().is_file();
            return MirrorStatus {
                copy: None,
                problem: if orphan {
                    Some(format!(
                        "{} exists with no manifest beside it — fetch again",
                        self.path().display()
                    ))
                } else {
                    None
                },
            };
        };
        let verified = digest_of(&self.path())
            .map(|digest| digest == stored.manifest.sha256)
            .unwrap_or(false);
        let built_at_ms = parse_rfc3339_ms(&stored.manifest.built_at);
        let age_ms = built_at_ms.and_then(|built| now_ms().checked_sub(built));
        let stale = age_ms.map_or(true, |age| age > REBUILD_AFTER_MS);
        MirrorStatus {
            problem: if verified {
                None
            } else {
                Some(
                    "the copy does not match the digest its manifest records — fetch again"
                        .to_owned(),
                )
            },
            copy: Some(CopyStatus {
                built_at: stored.manifest.built_at.clone(),
                built_at_ms,
                age_ms,
                stale,
                fetched_at_ms: stored.fetched_at_ms,
                bytes: stored.manifest.bytes,
                rows: stored.manifest.rows,
                tables: stored.manifest.tables.clone(),
                sha256: stored.manifest.sha256.clone(),
                verified,
                integrity: stored.manifest.integrity.clone(),
                foreign_key_violations: stored.manifest.foreign_key_violations,
            }),
        }
    }

    /// Fetch the copy, unless what the device holds is already it.
    ///
    /// `rebuild` asks the Mac to build a fresh copy first when the stored one is older than
    /// [`REBUILD_AFTER_MS`], or older than the device can tell, or there is none. It is a switch
    /// rather than always-on so a sync made on every page open does not spawn a build on the Mac
    /// each time; the caller decides.
    pub async fn sync<T: Transport>(
        &self,
        transport: &T,
        rebuild: bool,
    ) -> Result<SyncOutcome, String> {
        let before = self.status();
        let mut rebuild_error = None;
        let mut rebuilt = false;
        let mut manifest = fetch_manifest(transport).await?;

        let due = before
            .copy
            .as_ref()
            .map_or(true, |copy| !copy.verified || copy.stale);
        if rebuild && due {
            match ask_rebuild(transport).await {
                Ok(fresh) => {
                    manifest = fresh;
                    rebuilt = true;
                }
                Err(error) => rebuild_error = Some(error),
            }
        }

        // Already what the Mac says? Then nothing crosses the network. Compared against the digest
        // of the file on disk, not against the manifest that is stored, so a corrupted copy
        // refetches instead of reporting itself current.
        if let Some(copy) = &before.copy {
            if copy.verified && copy.sha256 == manifest.sha256 {
                return Ok(SyncOutcome {
                    action: "unchanged".to_owned(),
                    transferred: 0,
                    rebuild_error,
                    status: before,
                });
            }
        }

        let body = fetch_copy(transport).await?;
        let digest = sha256_hex(&body);
        if digest != manifest.sha256 {
            return Err(format!(
                "the fetch does not match its manifest: sha256 {digest}, manifest says {} — nothing was stored",
                manifest.sha256
            ));
        }
        if body.len() as u64 != manifest.bytes {
            return Err(format!(
                "the fetch is {} bytes, the manifest says {} — nothing was stored",
                body.len(),
                manifest.bytes
            ));
        }
        let transferred = body.len() as u64;
        self.store(&body, &manifest)?;
        Ok(SyncOutcome {
            action: if rebuilt {
                "rebuilt and fetched".to_owned()
            } else {
                "fetched".to_owned()
            },
            transferred,
            rebuild_error,
            status: self.status(),
        })
    }

    /// Write the copy beside its destination, check it there, and move it into place.
    ///
    /// The order is the safety property: the file lands first and the manifest second, so a reader
    /// that finds a manifest always finds the copy it describes. Anything that fails removes the
    /// partial file rather than leaving it to look like a copy.
    fn store(&self, body: &[u8], manifest: &Manifest) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let part = self.dir.join(PART_FILE);
        std::fs::write(&part, body).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("writing {}: {e}", part.display())
        })?;
        // Checked where it will be read from, not only in memory: the write is the step that can
        // produce a file that opens and disagrees with what was verified.
        if let Err(error) = check_copy(&part, manifest) {
            let _ = std::fs::remove_file(&part);
            return Err(error);
        }
        if let Err(error) = std::fs::rename(&part, self.path()) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("moving the copy into place: {error}"));
        }
        let stored = Stored {
            manifest: manifest.clone(),
            fetched_at_ms: now_ms(),
        };
        let body =
            serde_json::to_vec_pretty(&stored).map_err(|e| format!("unwritable manifest: {e}"))?;
        std::fs::write(self.manifest_path(), body)
            .map_err(|e| format!("writing {}: {e}", self.manifest_path().display()))
    }

    /// The copy, opened read-only, or the reason there is not one worth opening.
    fn connection(&self) -> Result<Connection, String> {
        let status = self.status();
        match &status.copy {
            Some(copy) if copy.verified => {}
            _ => {
                return Err(status
                    .problem
                    .unwrap_or_else(|| "no copy has been fetched yet".to_owned()))
            }
        }
        Connection::open_with_flags(self.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("opening the copy: {e}"))
    }

    /// Open tasks, or done ones, ordered as `capabilities/projects` orders them: not-done first,
    /// then by due date, then priority, then title.
    pub fn tasks(&self, status: Option<&str>) -> Result<Vec<Record>, String> {
        if let Some(status) = status {
            if !matches!(status, "open" | "done") {
                return Err(format!("invalid status '{status}'"));
            }
        }
        let clause = match status {
            Some("open") => "AND done = 0",
            Some("done") => "AND done = 1",
            _ => "",
        };
        self.select(&format!(
            "SELECT {COLUMNS} FROM projects_records
             WHERE kind = 'task' AND archived = 0 {clause}
             ORDER BY done, due IS NULL, due, priority, title"
        ))
    }

    /// Every record of one kind, ordered by path — the copy of `/projects/api/records?kind=`.
    pub fn records(&self, kind: &str) -> Result<Vec<Record>, String> {
        if !matches!(kind, "project" | "task" | "note") {
            return Err(format!("invalid kind '{kind}'"));
        }
        self.select(&format!(
            "SELECT {COLUMNS} FROM projects_records
             WHERE kind = '{kind}' AND archived = 0
             ORDER BY path"
        ))
    }

    /// One project's records, ordered by path.
    pub fn project_records(&self, name: &str) -> Result<Vec<Record>, String> {
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM projects_records
                 WHERE project = ?1 AND archived = 0
                 ORDER BY path"
            ))
            .map_err(|e| format!("reading the copy: {e}"))?;
        let rows = statement
            .query_map([name], row_to_record)
            .map_err(|e| format!("reading the copy: {e}"))?;
        collect(rows)
    }

    /// Every project as `capabilities/projects` counts it: open tasks, done tasks, notes.
    ///
    /// An archived project keeps its own row, because the route summarises `(name, archived)` pairs
    /// rather than names — filtering here would quietly merge two projects into one.
    pub fn projects(&self) -> Result<Vec<ProjectSummary>, String> {
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(
                "SELECT project, archived, kind, done, COUNT(*)
                 FROM projects_records
                 GROUP BY project, archived, kind, done
                 ORDER BY project",
            )
            .map_err(|e| format!("reading the copy: {e}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| format!("reading the copy: {e}"))?;
        let mut summaries: Vec<ProjectSummary> = Vec::new();
        for row in rows {
            let (name, archived, kind, done, count) =
                row.map_err(|e| format!("reading the copy: {e}"))?;
            let count = usize::try_from(count).unwrap_or(0);
            let index = match summaries
                .iter()
                .position(|s| s.name == name && s.archived == archived)
            {
                Some(index) => index,
                None => {
                    summaries.push(ProjectSummary {
                        name,
                        archived,
                        open: 0,
                        done: 0,
                        notes: 0,
                    });
                    summaries.len() - 1
                }
            };
            let summary = &mut summaries[index];
            match (kind.as_str(), done) {
                ("task", false) => summary.open += count,
                ("task", true) => summary.done += count,
                _ => summary.notes += count,
            }
        }
        Ok(summaries)
    }

    fn select(&self, sql: &str) -> Result<Vec<Record>, String> {
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(sql)
            .map_err(|e| format!("reading the copy: {e}"))?;
        let rows = statement
            .query_map([], row_to_record)
            .map_err(|e| format!("reading the copy: {e}"))?;
        collect(rows)
    }
}

/// The columns every record query selects, in the order [`row_to_record`] reads them.
const COLUMNS: &str = "path, project, kind, title, done, due, priority, archived, content";

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        path: row.get(0)?,
        project: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        done: row.get(4)?,
        due: row.get(5)?,
        priority: row.get(6)?,
        archived: row.get(7)?,
        content: row.get(8)?,
    })
}

fn collect<P, F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<P>, String>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<P>,
{
    rows.collect::<rusqlite::Result<Vec<P>>>()
        .map_err(|e| format!("reading the copy: {e}"))
}

// ─── The fetch ────────────────────────────────────────────────────────────────

/// `GET` the manifest: what the Mac says its copy holds.
async fn fetch_manifest<T: Transport>(transport: &T) -> Result<Manifest, String> {
    let reply = get_or_post(transport, "GET", MANIFEST_PATH).await?;
    if !is_success(reply.status) {
        return Err(format!(
            "the Mac has no copy to fetch ({}): {}",
            reply.status,
            server_message(&reply.body)
        ));
    }
    serde_json::from_slice(&reply.body).map_err(|e| format!("the manifest is unreadable: {e}"))
}

/// `POST` the rebuild route: the answer is the fresh manifest.
async fn ask_rebuild<T: Transport>(transport: &T) -> Result<Manifest, String> {
    let reply = get_or_post(transport, "POST", REBUILD_PATH).await?;
    if !is_success(reply.status) {
        return Err(format!(
            "the rebuild was refused ({}): {}",
            reply.status,
            server_message(&reply.body)
        ));
    }
    serde_json::from_slice(&reply.body)
        .map_err(|e| format!("the fresh manifest is unreadable: {e}"))
}

/// `GET` the copy itself.
async fn fetch_copy<T: Transport>(transport: &T) -> Result<Vec<u8>, String> {
    let reply = get_or_post(transport, "GET", COPY_PATH).await?;
    if !is_success(reply.status) {
        return Err(format!(
            "the Mac would not serve the copy ({}): {}",
            reply.status,
            server_message(&reply.body)
        ));
    }
    Ok(reply.body)
}

/// A bodyless request to one of the three mirror paths. No header is set: the transport signs the
/// request and the tailnet identity is injected on the way in, exactly as every other call.
async fn get_or_post<T: Transport>(
    transport: &T,
    method: &str,
    path: &str,
) -> Result<Reply, String> {
    let request = Outgoing {
        method: method.to_owned(),
        path: path.to_owned(),
        headers: Vec::new(),
        body: None,
    };
    let max_bytes = if path == COPY_PATH {
        MAX_COPY_BYTES
    } else {
        MAX_MANIFEST_BYTES
    };
    transport
        .send(&request, max_bytes)
        .await
        .map_err(SendError::message)
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The `error` field of a JSON answer, or the body as it is. Every route in this family answers
/// `{"error": …}`, and its sentence is worth more than the status code alone.
fn server_message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_owned())
}

/// The copy's own checks, run on the written file before it is moved into place: the database must
/// open, pass `PRAGMA integrity_check`, and hold exactly the rows the manifest claims for every
/// table it names.
///
/// The digest already says the bytes are the Mac's. This says the Mac's bytes are a database, which
/// the digest cannot: a copy built from a source that changed under it would hash correctly and
/// still be wrong.
fn check_copy(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("the fetched copy does not open: {e}"))?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|e| format!("integrity_check on the fetched copy failed: {e}"))?;
    if integrity != "ok" {
        return Err(format!(
            "the fetched copy failed integrity_check ({integrity}) — refused"
        ));
    }
    for table in &manifest.tables {
        check_identifier(&table.name)?;
        let rows: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", table.name),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("the fetched copy has no usable '{}': {e}", table.name))?;
        if rows < 0 || rows as u64 != table.rows {
            return Err(format!(
                "the fetched copy holds {rows} rows of '{}', the manifest says {} — refused",
                table.name, table.rows
            ));
        }
    }
    Ok(())
}

/// A table name arrives from the Mac inside a quoted identifier, so it is refused unless it is a
/// plain identifier: a manifest must not be able to close the quote this module opens.
fn check_identifier(name: &str) -> Result<(), String> {
    let plain = !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if plain {
        Ok(())
    } else {
        Err(format!(
            "the manifest names '{name}', which is not a table identifier — refused"
        ))
    }
}

/// The digest of a file's bytes: the same sha256 the Mac writes into the manifest, so a device can
/// compare without trusting anything but the bytes it holds.
fn digest_of(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The Mac writes `built_at` as RFC 3339 in UTC (`2026-10-21T21:21:30Z`). This reads that shape and
/// nothing else: a date this device cannot read is reported as stale rather than fresh, so no
/// dependency is pulled in for one field.
fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // What follows the seconds is a fraction and an offset. Only UTC is accepted, because that is
    // what the producer writes; a local-time offset would need a conversion this does not do.
    let rest = text.get(19..)?;
    let (fraction_ms, rest) = match rest.split_once('.') {
        Some((before, after)) => {
            if !before.is_empty() {
                return None;
            }
            let fraction_len = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            if fraction_len == 0 {
                return None;
            }
            // Three digits are milliseconds; more are truncated rather than rounded, because a
            // rounded date is a date the owner cannot reproduce.
            let digits = &after[..fraction_len];
            let millis = format!("{digits:0<3}");
            let millis = millis.get(..3)?.parse::<i64>().ok()?;
            (millis, after[fraction_len..].to_owned())
        }
        None => (0, rest.to_owned()),
    };
    if rest != "Z" && rest != "+00:00" {
        return None;
    }
    Some(
        days_from_civil(year, month, day) * 86_400_000
            + hour * 3_600_000
            + minute * 60_000
            + second * 1000
            + fraction_ms,
    )
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

// ─── Commands ─────────────────────────────────────────────────────────────────

/// What the device holds, without going near the network.
#[tauri::command]
pub async fn mirror_status<R: Runtime>(app: AppHandle<R>) -> Result<MirrorStatus, String> {
    Ok(Mirror::open(&app)?.status())
}

/// Fetch the copy, asking the Mac to rebuild first when the stored one is older than
/// [`REBUILD_AFTER_MS`]. The page calls this on open; a failure leaves whatever is already stored,
/// and the status line keeps saying how old that is.
#[tauri::command]
pub async fn mirror_sync<R: Runtime>(
    app: AppHandle<R>,
    rebuild: Option<bool>,
) -> Result<SyncOutcome, String> {
    let mirror = Mirror::open(&app)?;
    let transport = crate::mac_bridge::transport(&app)?;
    mirror.sync(&transport, rebuild.unwrap_or(true)).await
}

/// Open tasks from the copy, or done ones. The Mac is never consulted.
#[tauri::command]
pub async fn mirror_tasks<R: Runtime>(
    app: AppHandle<R>,
    status: Option<String>,
) -> Result<Vec<Record>, String> {
    Mirror::open(&app)?.tasks(status.as_deref())
}

/// Every record of one kind from the copy.
#[tauri::command]
pub async fn mirror_records<R: Runtime>(
    app: AppHandle<R>,
    kind: String,
) -> Result<Vec<Record>, String> {
    Mirror::open(&app)?.records(&kind)
}

/// Every project the copy knows, counted.
#[tauri::command]
pub async fn mirror_projects<R: Runtime>(app: AppHandle<R>) -> Result<Vec<ProjectSummary>, String> {
    Mirror::open(&app)?.projects()
}

/// One project's records from the copy.
#[tauri::command]
pub async fn mirror_project_records<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<Vec<Record>, String> {
    Mirror::open(&app)?.project_records(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const SCHEMA: &str = "CREATE TABLE projects_records (
            path TEXT PRIMARY KEY,
            project TEXT NOT NULL,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            done INTEGER NOT NULL,
            due TEXT,
            priority INTEGER NOT NULL,
            archived INTEGER NOT NULL,
            content TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE calendar_entries (id TEXT PRIMARY KEY);";

    /// A source database the way the Mac's copy looks: two tables, the projects one holding a
    /// note, a task and a project.
    fn source(dir: &Path) -> Vec<u8> {
        source_with(dir, "body")
    }

    /// The same rows with a different note body, so the digest changes and the row counts do not.
    fn source_with(dir: &Path, note_body: &str) -> Vec<u8> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = dir.join(format!("source-{}.db", NEXT.fetch_add(1, Ordering::SeqCst)));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO projects_records VALUES
                ('p/a.md', 'Alpha', 'project', 'Alpha', 0, NULL, 2, 0, '# Alpha', '2026-10-01'),
                ('p/b.md', 'Alpha', 'task', 'Second', 0, '2026-11-02', 1, 0, 'later', '2026-10-02'),
                ('p/c.md', 'Alpha', 'task', 'First', 0, '2026-10-30', 2, 0, 'sooner', '2026-10-03'),
                ('p/d.md', 'Alpha', 'task', 'Done one', 1, NULL, 3, 0, 'x', '2026-10-04'),
                ('p/e.md', 'Beta', 'note', 'A note', 0, NULL, 2, 0, ?1, '2026-10-05'),
                ('p/f.md', 'Beta', 'task', 'Archived', 0, NULL, 2, 1, 'x', '2026-10-06');",
            [note_body],
        )
        .unwrap();
        conn.execute("INSERT INTO calendar_entries VALUES ('e1')", [])
            .unwrap();
        drop(conn);
        std::fs::read(&path).unwrap()
    }

    fn manifest_for(bytes: &[u8]) -> Manifest {
        Manifest {
            built_at: "2026-10-21T21:21:30Z".to_owned(),
            integrity: "ok".to_owned(),
            foreign_key_violations: 0,
            source: "/somewhere/axon.db".to_owned(),
            mode: "selected".to_owned(),
            tables: vec![
                Table {
                    name: "projects_records".to_owned(),
                    rows: 6,
                },
                Table {
                    name: "calendar_entries".to_owned(),
                    rows: 1,
                },
            ],
            rows: 7,
            bytes: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            path: "/somewhere/mirror.db".to_owned(),
        }
    }

    struct Fake {
        manifest: Mutex<Reply>,
        copy: Mutex<Reply>,
        rebuild: Mutex<Option<Reply>>,
        sent: Mutex<Vec<Outgoing>>,
    }

    impl Fake {
        fn new(manifest: Manifest, copy: Vec<u8>) -> Self {
            Self {
                manifest: Mutex::new(json_reply(200, &manifest)),
                copy: Mutex::new(Reply {
                    status: 200,
                    content_type: Some("application/octet-stream".to_owned()),
                    body: copy,
                }),
                rebuild: Mutex::new(None),
                sent: Mutex::new(Vec::new()),
            }
        }

        fn rebuild_answers(&self, manifest: Manifest) {
            *self.rebuild.lock().unwrap() = Some(json_reply(200, &manifest));
        }

        fn sent(&self) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .map(|o| format!("{} {}", o.method, o.path))
                .collect()
        }
    }

    fn json_reply(status: u16, value: &Manifest) -> Reply {
        Reply {
            status,
            content_type: Some("application/json".to_owned()),
            body: serde_json::to_vec(value).unwrap(),
        }
    }

    impl Transport for Fake {
        async fn send(&self, request: &Outgoing, max_bytes: usize) -> Result<Reply, SendError> {
            self.sent.lock().unwrap().push(request.clone());
            let reply = if request.path == MANIFEST_PATH {
                self.manifest.lock().unwrap().clone()
            } else if request.path == COPY_PATH {
                self.copy.lock().unwrap().clone()
            } else if request.path == REBUILD_PATH {
                self.rebuild
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| SendError::Failed("no rebuild configured".to_owned()))?
            } else {
                return Err(SendError::Failed(format!(
                    "unexpected path {}",
                    request.path
                )));
            };
            if reply.body.len() > max_bytes {
                return Err(SendError::Failed("too large".to_owned()));
            }
            Ok(reply)
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "sjel-mirror-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn mirror(dir: &TempDir) -> Mirror {
        Mirror {
            dir: dir.path().join("mirror"),
        }
    }

    /// Tauri commands ask for the app handle; the copy itself only needs a directory, so the tests
    /// drive `Mirror` directly and the commands stay thin.
    fn fresh(dir: &TempDir) -> (Mirror, Fake) {
        let bytes = source(dir.path());
        let manifest = manifest_for(&bytes);
        let fake = Fake::new(manifest, bytes);
        (mirror(dir), fake)
    }

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn a_first_sync_stores_the_copy_and_verifies_it() {
        let dir = TempDir::new();
        let (mirror, fake) = fresh(&dir);
        let outcome = block(mirror.sync(&fake, false)).unwrap();
        assert_eq!(outcome.action, "fetched");
        assert!(outcome.transferred > 0);
        let status = outcome.status;
        let copy = status.copy.expect("a copy was stored");
        assert!(copy.verified, "the stored copy matches its digest");
        assert_eq!(copy.rows, 7);
        assert_eq!(copy.tables.len(), 2);
        assert!(status.problem.is_none());
        assert!(mirror.path().is_file());
        assert!(
            !mirror.dir.join(PART_FILE).exists(),
            "the part file is gone"
        );
    }

    #[test]
    fn a_second_sync_sends_nothing() {
        let dir = TempDir::new();
        let (mirror, fake) = fresh(&dir);
        block(mirror.sync(&fake, false)).unwrap();
        let before = fake.sent().len();
        let outcome = block(mirror.sync(&fake, false)).unwrap();
        assert_eq!(outcome.action, "unchanged");
        assert_eq!(outcome.transferred, 0);
        assert_eq!(fake.sent().len(), before + 1, "only the manifest is read");
    }

    #[test]
    fn a_copy_that_does_not_match_its_digest_is_refused_and_not_stored() {
        let dir = TempDir::new();
        let (mirror, fake) = fresh(&dir);
        {
            let mut copy = fake.copy.lock().unwrap();
            copy.body.push(b'x');
        }
        let error = block(mirror.sync(&fake, false)).unwrap_err();
        assert!(error.contains("does not match its manifest"), "{error}");
        assert!(!mirror.path().exists(), "nothing was stored");
        assert!(mirror.status().copy.is_none());
    }

    #[test]
    fn a_copy_with_the_wrong_row_count_is_refused() {
        let dir = TempDir::new();
        let bytes = source(dir.path());
        let mut manifest = manifest_for(&bytes);
        manifest.tables[0].rows = 5;
        let fake = Fake::new(manifest, bytes);
        let mirror = mirror(&dir);
        let error = block(mirror.sync(&fake, false)).unwrap_err();
        assert!(error.contains("the manifest says 5"), "{error}");
        assert!(!mirror.path().exists(), "nothing was stored");
        assert!(
            !mirror.dir.join(PART_FILE).exists(),
            "no partial file is left"
        );
    }

    #[test]
    fn a_stale_copy_is_rebuilt_before_it_is_fetched() {
        let dir = TempDir::new();
        let (mirror, fake) = fresh(&dir);
        // Stored first, then dated far enough back that the next sync asks for a rebuild.
        block(mirror.sync(&fake, false)).unwrap();
        let mut old = mirror.stored().unwrap();
        old.manifest.built_at = "2026-10-01T00:00:00Z".to_owned();
        std::fs::write(mirror.manifest_path(), serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(mirror.status().copy.unwrap().stale);

        // A different note body: the digest changes, the row counts do not, so the fetch has to run
        // rather than short-circuit and the copy still passes its own checks.
        let rebuilt = source_with(dir.path(), "a body written after the first copy");
        let fresh_manifest = Manifest {
            built_at: "2026-10-22T00:00:00Z".to_owned(),
            ..manifest_for(&rebuilt)
        };
        fake.rebuild_answers(fresh_manifest.clone());
        *fake.copy.lock().unwrap() = Reply {
            status: 200,
            content_type: None,
            body: rebuilt,
        };
        *fake.manifest.lock().unwrap() = json_reply(200, &fresh_manifest);
        let outcome = block(mirror.sync(&fake, true)).unwrap();
        assert_eq!(outcome.action, "rebuilt and fetched");
        assert_eq!(
            outcome.status.copy.unwrap().built_at,
            "2026-10-22T00:00:00Z"
        );
    }

    #[test]
    fn a_failed_rebuild_still_fetches_and_says_why() {
        let dir = TempDir::new();
        let (mirror, fake) = fresh(&dir);
        let outcome = block(mirror.sync(&fake, true)).unwrap();
        assert_eq!(outcome.action, "fetched");
        assert!(
            outcome.rebuild_error.is_some(),
            "the refusal is reported, not swallowed"
        );
        assert!(outcome.status.copy.unwrap().verified);
    }

    #[test]
    fn reading_the_copy_answers_the_projects_shapes_in_the_macs_order() {
        let dir = TempDir::new();
        let bytes = source(dir.path());
        let mirror = mirror(&dir);
        mirror.store(&bytes, &manifest_for(&bytes)).unwrap();

        let open = mirror.tasks(Some("open")).unwrap();
        let titles: Vec<&str> = open.iter().map(|r| r.title.as_str()).collect();
        // Due date first, regardless of priority, and the archived task is not here.
        assert_eq!(titles, vec!["First", "Second"]);
        assert_eq!(open[0].path, "p/c.md");

        assert_eq!(mirror.tasks(Some("done")).unwrap().len(), 1);
        assert_eq!(mirror.tasks(None).unwrap().len(), 3);
        assert!(mirror.tasks(Some("archived")).is_err());

        let notes = mirror.records("note").unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].project, "Beta");
        assert!(mirror.records("everything").is_err());

        let projects = mirror.projects().unwrap();
        let alpha = projects.iter().find(|p| p.name == "Alpha").unwrap();
        assert_eq!((alpha.open, alpha.done, alpha.notes), (2, 1, 1));
        let beta = projects.iter().find(|p| p.name == "Beta").unwrap();
        assert_eq!((beta.open, beta.done, beta.notes), (0, 0, 1));

        let records = mirror.project_records("Alpha").unwrap();
        assert_eq!(records.len(), 4);
        assert!(records.windows(2).all(|w| w[0].path <= w[1].path));
    }

    /// The whole device path against the copy this Mac actually built, rather than a three-row
    /// fixture: real manifest, real 11 MB file, real digest and integrity check.
    ///
    /// Ignored by default because it needs a machine with a copy and its directory says where.
    /// `SJEL_MIRROR_DIR=<sjel mirror path's dir>` and `cargo test --lib -- --ignored`.
    #[test]
    #[ignore = "needs a built copy and SJEL_MIRROR_DIR"]
    fn the_real_copy_fetches_verifies_and_reads() {
        let Ok(dir) = std::env::var("SJEL_MIRROR_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let bytes = std::fs::read(&manifest.path).unwrap();
        let fake = Fake::new(manifest.clone(), bytes);
        let temp = TempDir::new();
        let mirror = mirror(&temp);
        let outcome = block(mirror.sync(&fake, false)).unwrap();
        assert_eq!(outcome.action, "fetched");
        let copy = outcome.status.copy.unwrap();
        assert!(copy.verified);
        assert_eq!(copy.rows, manifest.rows);
        assert_eq!(copy.bytes, manifest.bytes);
        assert!(copy.built_at_ms.is_some(), "the Mac's build time parses");
        let projects = mirror.projects().unwrap();
        assert!(!projects.is_empty());
        // Every record belongs to exactly one summary, archived ones included: the counts add up to
        // the table the manifest says was copied, which is the check that nothing was dropped.
        let summarised: usize = projects.iter().map(|p| p.open + p.done + p.notes).sum();
        let records = manifest
            .tables
            .iter()
            .find(|t| t.name == "projects_records")
            .expect("the copy holds projects_records");
        assert_eq!(summarised as u64, records.rows);
        let open = mirror.tasks(Some("open")).unwrap();
        assert!(!open.is_empty(), "the phone can show what is due");
        assert!(open.iter().all(|t| !t.done && !t.archived));
    }

    #[test]
    fn a_copy_with_no_manifest_is_reported_rather_than_claimed() {
        let dir = TempDir::new();
        let mirror = mirror(&dir);
        std::fs::create_dir_all(mirror.path().parent().unwrap()).unwrap();
        std::fs::write(mirror.path(), b"not really a database").unwrap();
        let status = mirror.status();
        assert!(status.copy.is_none());
        assert!(status.problem.unwrap().contains("no manifest"));
        assert!(
            mirror.tasks(None).is_err(),
            "an unverified copy is not read"
        );
    }

    #[test]
    fn a_tampered_copy_is_not_verified_and_is_not_read() {
        let dir = TempDir::new();
        let bytes = source(dir.path());
        let mirror = mirror(&dir);
        mirror.store(&bytes, &manifest_for(&bytes)).unwrap();
        assert!(mirror.status().copy.unwrap().verified);
        std::fs::write(mirror.path(), b"changed").unwrap();
        let status = mirror.status();
        assert!(!status.copy.unwrap().verified);
        assert!(mirror.tasks(None).is_err());
    }

    #[test]
    fn a_manifest_table_that_is_not_an_identifier_is_refused() {
        assert!(check_identifier("projects_records").is_ok());
        assert!(check_identifier("a\"; DROP TABLE projects_records; --").is_err());
        assert!(check_identifier("1table").is_err());
        assert!(check_identifier("").is_err());
    }

    #[test]
    fn the_macs_build_time_reads_and_a_foreign_one_does_not() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-10-21T21:21:30Z"),
            Some(1_792_617_690_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-21T23:21:30+02:00"),
            None,
            "a local offset is refused rather than converted"
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-21T21:21:30.250Z"),
            Some(1_792_617_690_250)
        );
        assert_eq!(parse_rfc3339_ms("yesterday"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-21T21:21:30Z"), None);
    }
}
