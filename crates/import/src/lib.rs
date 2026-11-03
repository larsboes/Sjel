//! One import for Sjel. Walks a folder, sorts every file to the capability that
//! owns it (`route`), hands it over through that capability's HTTP contract, and
//! records one outcome per file in the ledger. Source files are only read.

pub mod route;
pub mod server;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

pub use route::Route;

const PREFIX: &str = "import";
/// documents refuses larger uploads (`capabilities/documents/src/library.rs` `MAX_BYTES`).
const DOCUMENTS_MAX_BYTES: u64 = 128 * 1024 * 1024;

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The owner stored new bytes.
    Imported,
    /// The owner already held these bytes, under this or another name.
    Present,
    /// Sorted, but the owner has no write contract yet.
    Pending,
    /// Not personal data, or refused by the owner.
    Skipped,
    /// The handover failed; the next run retries it.
    Failed,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Present => "present",
            Self::Pending => "pending",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// A settled file is not read again while its size and mtime stay the same.
    fn settled(name: &str) -> bool {
        matches!(name, "imported" | "present" | "skipped")
    }
}

/// The documents capability as the import sees it.
pub trait Documents {
    /// The id and filename of a record that holds these bytes.
    ///
    /// # Errors
    /// The capability is unreachable or answers with an error.
    fn holding(&self, sha256: &str) -> Result<Option<(String, String)>, String>;
    /// Store the bytes and return the record id.
    ///
    /// # Errors
    /// `Handover::Refused` for a deliberate refusal, `Handover::Failed` otherwise.
    fn upload(&self, filename: &str, bytes: &[u8]) -> Result<String, Handover>;
}

/// Why an upload did not store the file.
#[derive(Debug)]
pub enum Handover {
    Refused(String),
    Failed(String),
}

/// One file of a plan.
#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub route: Route,
    pub byte_len: u64,
    pub modified_ns: i64,
}

/// Walk `root` and sort every regular file. Symlinks are listed as skipped, never followed.
///
/// # Errors
/// `root` is not a directory or a directory cannot be read.
pub fn plan(root: &Path) -> Fallible<Vec<Entry>> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(format!("{} is not a directory", root.display()).into());
    }
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir)? {
            let path = item?.path();
            let metadata = fs::symlink_metadata(&path)?;
            let relative = path.strip_prefix(root).unwrap_or(&path);
            if metadata.is_dir() {
                // A skipped folder is listed once, never entered: node_modules can hold 100,000 files.
                match route::skip_folder(relative) {
                    Some(reason) => entries.push(entry(path, Route::Skip(reason), &metadata)),
                    None => pending.push(path),
                }
                continue;
            }
            let route = if metadata.file_type().is_symlink() {
                Route::Skip("symlink")
            } else if !metadata.is_file() {
                Route::Skip("not a regular file")
            } else {
                let is_csv = path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"));
                let header = is_csv.then(|| first_line(&path)).flatten();
                route::route(relative, header.as_deref())
            };
            entries.push(entry(path, route, &metadata));
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn entry(path: PathBuf, route: Route, metadata: &fs::Metadata) -> Entry {
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as i64);
    Entry {
        path,
        route,
        byte_len: metadata.len(),
        modified_ns,
    }
}

fn first_line(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader, Read};
    let mut line = String::new();
    BufReader::new(fs::File::open(path).ok()?.take(4096))
        .read_line(&mut line)
        .ok()?;
    Some(line)
}

/// Counts of a plan: files and bytes per route, and files per skip reason.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlanTally {
    pub routes: BTreeMap<&'static str, (usize, u64)>,
    pub skipped: BTreeMap<&'static str, usize>,
}

/// Sum a plan for display.
pub fn tally(entries: &[Entry]) -> PlanTally {
    let mut tally = PlanTally::default();
    for entry in entries {
        let slot = tally.routes.entry(entry.route.name()).or_default();
        slot.0 += 1;
        slot.1 += entry.byte_len;
        if let Route::Skip(reason) = entry.route {
            *tally.skipped.entry(reason).or_default() += 1;
        }
    }
    tally
}

/// The per-file ledger in the shared database.
pub struct Ledger {
    pool: sjel_store::Pool,
}

/// One ledger row as `status` reports it.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tally {
    pub route: String,
    pub outcome: String,
    pub detail: Option<String>,
}

impl Ledger {
    /// # Errors
    /// The database cannot be opened or migrated.
    pub fn open(database: &Path) -> Fallible<Self> {
        let pool = sjel_store::open_pool(database, PREFIX, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS import_files (
                    path TEXT PRIMARY KEY,
                    byte_len INTEGER NOT NULL,
                    modified_ns INTEGER NOT NULL,
                    sha256 TEXT,
                    route TEXT NOT NULL,
                    outcome TEXT NOT NULL CHECK (outcome IN ('imported', 'present', 'pending', 'skipped', 'failed')),
                    detail TEXT,
                    target_id TEXT,
                    recorded_at TEXT NOT NULL
                );",
            )?;
            Ok(())
        })?;
        Ok(Self { pool })
    }

    /// Unchanged bytes on the same route. A file whose route changed with `src/route.rs` is
    /// sorted again, so a fixed rule reaches files an earlier run skipped.
    fn settled(&self, entry: &Entry, path: &str) -> Fallible<bool> {
        let row: Option<(i64, i64, String, String)> = self
            .pool
            .get()?
            .query_row(
                "SELECT byte_len, modified_ns, outcome, route FROM import_files WHERE path = ?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(row.is_some_and(|(byte_len, modified_ns, outcome, route)| {
            byte_len == entry.byte_len as i64
                && modified_ns == entry.modified_ns
                && Outcome::settled(&outcome)
                && route == entry.route.name()
        }))
    }

    fn record(
        &self,
        entry: &Entry,
        path: &str,
        sha256: Option<&str>,
        outcome: Outcome,
        detail: Option<&str>,
        target_id: Option<&str>,
    ) -> Fallible<()> {
        self.pool.get()?.execute(
            "INSERT INTO import_files (path, byte_len, modified_ns, sha256, route, outcome, detail, target_id, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET byte_len = excluded.byte_len, modified_ns = excluded.modified_ns,
               sha256 = excluded.sha256, route = excluded.route, outcome = excluded.outcome,
               detail = excluded.detail, target_id = excluded.target_id, recorded_at = excluded.recorded_at",
            params![path, entry.byte_len as i64, entry.modified_ns, sha256, entry.route.name(), outcome.name(), detail, target_id, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Counts per route, outcome and detail for every recorded file under `root`.
    ///
    /// # Errors
    /// The read fails.
    pub fn status(&self, root: &Path) -> Fallible<BTreeMap<Tally, usize>> {
        let prefix = format!("{}/", root.display().to_string().trim_end_matches('/'));
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT route, outcome, CASE WHEN outcome IN ('skipped', 'pending', 'failed') THEN detail END, count(*)
             FROM import_files WHERE substr(path, 1, length(?1)) = ?1 GROUP BY 1, 2, 3",
        )?;
        let rows = stmt.query_map([prefix], |row| {
            Ok((
                Tally {
                    route: row.get(0)?,
                    outcome: row.get(1)?,
                    detail: row.get(2)?,
                },
                row.get::<_, i64>(3)? as usize,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Counts of one run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub unchanged: usize,
    pub outcomes: BTreeMap<&'static str, usize>,
}

/// Hand every planned file to its owner and record the outcome.
/// `progress` receives every file; `None` means unchanged since the last run.
///
/// # Errors
/// The ledger cannot be read or written. A failed handover is recorded, not returned.
pub fn run(
    entries: &[Entry],
    ledger: &Ledger,
    documents: &dyn Documents,
    mut progress: impl FnMut(&Entry, Option<Outcome>, Option<&str>),
) -> Fallible<Summary> {
    let mut summary = Summary::default();
    for entry in entries {
        let Some(path) = entry.path.to_str() else {
            progress(entry, None, None);
            continue;
        };
        if ledger.settled(entry, path)? {
            summary.unchanged += 1;
            progress(entry, None, None);
            continue;
        }
        let (sha256, outcome, detail, target) = match entry.route {
            Route::Skip(reason) => (None, Outcome::Skipped, Some(reason.to_owned()), None),
            Route::Media | Route::Finance | Route::Notes => (
                None,
                Outcome::Pending,
                Some(format!("{} has no import contract yet", entry.route.name())),
                None,
            ),
            Route::Documents if entry.byte_len > DOCUMENTS_MAX_BYTES => (
                None,
                Outcome::Skipped,
                Some("larger than the 128 MiB documents limit".to_owned()),
                None,
            ),
            Route::Documents => hand_to_documents(entry, documents),
        };
        ledger.record(
            entry,
            path,
            sha256.as_deref(),
            outcome,
            detail.as_deref(),
            target.as_deref(),
        )?;
        *summary.outcomes.entry(outcome.name()).or_default() += 1;
        progress(entry, Some(outcome), detail.as_deref());
    }
    Ok(summary)
}

type Handled = (Option<String>, Outcome, Option<String>, Option<String>);

fn hand_to_documents(entry: &Entry, documents: &dyn Documents) -> Handled {
    let bytes = match fs::read(&entry.path) {
        Ok(bytes) => bytes,
        Err(error) => return (None, Outcome::Failed, Some(format!("read: {error}")), None),
    };
    let sha256 = hex(&Sha256::digest(&bytes));
    let filename = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document");
    match documents.holding(&sha256) {
        Ok(Some((id, held_as))) => {
            let detail = (held_as != filename).then(|| format!("held as {held_as}"));
            return (Some(sha256), Outcome::Present, detail, Some(id));
        }
        Ok(None) => {}
        Err(error) => return (Some(sha256), Outcome::Failed, Some(error), None),
    }
    match documents.upload(filename, &bytes) {
        Ok(id) => (Some(sha256), Outcome::Imported, None, Some(id)),
        Err(Handover::Refused(reason)) => (Some(sha256), Outcome::Skipped, Some(reason), None),
        Err(Handover::Failed(error)) => (Some(sha256), Outcome::Failed, Some(error), None),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// documents over the shell proxy, with the deployment token.
pub struct DocumentsHttp {
    base: String,
    bearer: Option<String>,
    client: reqwest::blocking::Client,
}

const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("import");

impl DocumentsHttp {
    /// `SJEL_STATUS_URL` (the shell proxy) or its loopback default, as the assistant resolves it.
    ///
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn from_deployment() -> Fallible<Self> {
        let shell =
            std::env::var("SJEL_STATUS_URL").unwrap_or_else(|_| "http://127.0.0.1:8082".to_owned());
        Ok(Self {
            base: format!("{}/documents/api/documents", shell.trim_end_matches('/')),
            bearer: sjel_server::InboundAuth::from_deployment().bearer_header(),
            client: sjel_http::client(PURPOSE, std::time::Duration::from_secs(300))?,
        })
    }

    fn url(&self, params: &[(&str, &str)]) -> Result<reqwest::Url, String> {
        reqwest::Url::parse_with_params(&self.base, params).map_err(|error| error.to_string())
    }

    fn authorized(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match &self.bearer {
            Some(bearer) => request.header(reqwest::header::AUTHORIZATION, bearer),
            None => request,
        }
    }
}

fn body_error(response: reqwest::blocking::Response) -> String {
    let status = response.status().as_u16();
    let body = response.text().unwrap_or_default();
    let reason = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_owned))
        .unwrap_or(body);
    format!("documents answered HTTP {status}: {reason}")
}

impl Documents for DocumentsHttp {
    fn holding(&self, sha256: &str) -> Result<Option<(String, String)>, String> {
        let response = self
            .authorized(
                self.client
                    .get(self.url(&[("sha256", sha256), ("limit", "1")])?),
            )
            .send()
            .map_err(|error| format!("documents unreachable: {error}"))?;
        if !response.status().is_success() {
            return Err(body_error(response));
        }
        let value: serde_json::Value = response.json().map_err(|error| error.to_string())?;
        Ok(value["documents"].get(0).and_then(|document| {
            Some((
                document["id"].as_str()?.to_owned(),
                document["filename"].as_str()?.to_owned(),
            ))
        }))
    }

    fn upload(&self, filename: &str, bytes: &[u8]) -> Result<String, Handover> {
        let response = self
            .authorized(
                self.client
                    .post(
                        self.url(&[("filename", filename)])
                            .map_err(Handover::Failed)?,
                    )
                    .body(bytes.to_vec()),
            )
            .send()
            .map_err(|error| Handover::Failed(format!("documents unreachable: {error}")))?;
        let status = response.status();
        if status.is_client_error() && status != reqwest::StatusCode::UNAUTHORIZED {
            return Err(Handover::Refused(body_error(response)));
        }
        if !status.is_success() {
            return Err(Handover::Failed(body_error(response)));
        }
        let value: serde_json::Value = response
            .json()
            .map_err(|error| Handover::Failed(error.to_string()))?;
        value["document"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Handover::Failed("documents answered without a document id".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        held: RefCell<BTreeMap<String, String>>,
        uploads: RefCell<usize>,
    }

    impl Documents for Fake {
        fn holding(&self, sha256: &str) -> Result<Option<(String, String)>, String> {
            Ok(self
                .held
                .borrow()
                .get(sha256)
                .map(|name| (sha256.to_owned(), name.clone())))
        }
        fn upload(&self, filename: &str, bytes: &[u8]) -> Result<String, Handover> {
            if filename.contains("SECRET") {
                return Err(Handover::Refused("excluded".into()));
            }
            *self.uploads.borrow_mut() += 1;
            let sha256 = hex(&Sha256::digest(bytes));
            self.held
                .borrow_mut()
                .insert(sha256.clone(), filename.to_owned());
            Ok(sha256)
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sjel-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/Bescheid.pdf"), b"tax").unwrap();
        fs::write(dir.join("b/Bescheid Kopie.pdf"), b"tax").unwrap();
        fs::write(dir.join("b/SECRET.pdf"), b"pin").unwrap();
        fs::write(dir.join("b/photo.jpg"), b"jpg").unwrap();
        fs::write(dir.join("a/node_modules/pkg/index.js"), b"js").unwrap();
        dir
    }

    #[test]
    fn same_bytes_in_two_folders_are_stored_once_and_a_rerun_reads_nothing() {
        let root = scratch("run");
        let ledger = Ledger::open(&root.with_extension("db")).unwrap();
        let entries = plan(&root).unwrap();
        // node_modules is listed once as a folder, never entered.
        assert!(entries
            .iter()
            .all(|entry| !entry.path.ends_with("index.js")));
        let documents = Fake::default();
        let first = run(&entries, &ledger, &documents, |_, _, _| {}).unwrap();
        assert_eq!(*documents.uploads.borrow(), 1);
        assert_eq!(first.outcomes.get("imported"), Some(&1));
        assert_eq!(first.outcomes.get("present"), Some(&1));
        assert_eq!(first.outcomes.get("pending"), Some(&1));
        assert_eq!(first.outcomes.get("skipped"), Some(&2), "{first:?}");

        let entries = plan(&root).unwrap();
        let again = run(&entries, &ledger, &documents, |_, _, _| {}).unwrap();
        // Only the pending photo is looked at again; nothing is uploaded twice.
        assert_eq!(again.outcomes.get("pending"), Some(&1));
        assert_eq!(again.unchanged, entries.len() - 1);
        assert_eq!(*documents.uploads.borrow(), 1);

        let status = ledger.status(&root).unwrap();
        assert_eq!(status.values().sum::<usize>(), entries.len());

        // A rule change that moves a skipped file to a route sorts it again.
        let mut rerouted = plan(&root).unwrap();
        let secret = rerouted
            .iter_mut()
            .find(|e| e.path.ends_with("SECRET.pdf"))
            .unwrap();
        secret.route = Route::Notes;
        let after = run(&rerouted, &ledger, &documents, |_, _, _| {}).unwrap();
        assert_eq!(after.outcomes.get("pending"), Some(&2), "{after:?}");
        fs::remove_dir_all(&root).unwrap();
        for suffix in ["db", "db-wal", "db-shm"] {
            let _ = fs::remove_file(root.with_extension(suffix));
        }
    }
}
