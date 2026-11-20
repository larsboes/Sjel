//! Markdown files are authoritative for Projects records. The database indexes their content.
//!
//! A write from Sjel writes the file first (whole file, temp-plus-rename, mode 0600) and the index
//! second, and refuses when the file holds bytes the index has not seen: an edit made in another
//! editor is never overwritten. [`Projects::reindex`] adopts such edits, drops rows whose file was
//! deleted, and refuses a file whose frontmatter lowers its folder's data class.

use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[cfg(not(feature = "turso-wal"))]
use engine::TransactionBehavior;
use serde::Serialize;
#[cfg(feature = "turso-wal")]
use sjel_native_wal::{Pool as Store, TransactionBehavior, NOW};
#[cfg(not(feature = "turso-wal"))]
use sjel_turso::{libsql as engine, Store, NOW};
#[cfg(feature = "turso-wal")]
use turso as engine;

use crate::ops::SaveError;
use crate::record::{check_attachment, Kind, Record};

pub const PREFIX: &str = "projects";
type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[cfg(not(feature = "turso-wal"))]
type WriteTransaction<'conn> = engine::Transaction;
#[cfg(feature = "turso-wal")]
type WriteTransaction<'conn> = engine::transaction::Transaction<'conn>;

async fn begin_write(conn: &mut engine::Connection) -> Fallible<WriteTransaction<'_>> {
    Ok(conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?)
}

pub struct Projects {
    db: Store,
    #[cfg(feature = "turso-wal")]
    database: PathBuf,
    root: PathBuf,
    pub(crate) source_config: Option<PathBuf>,
}

pub use projects_api::mail_evidence::MailProposalInput;

#[derive(Debug, Serialize)]
pub struct MailProposal {
    pub id: i64,
    #[serde(flatten)]
    pub input: MailProposalInput,
    pub version: i64,
    pub status: String,
}

#[derive(Debug)]
/// A reviewer's exact view of the project record a link decision applies to.
pub struct InventoryLinkTarget<'a> {
    pub path: &'a str,
    pub project: &'a str,
    pub base: &'a str,
    pub revision: &'a str,
    pub add: bool,
}

#[derive(Debug, Serialize)]
pub struct InventoryLink {
    pub record_path: String,
    pub item_ref: String,
    pub verified_revision: i64,
    pub reviewed_at: String,
    pub provenance: &'static str,
}

pub struct MailDecisionOutcome {
    pub proposal: MailProposal,
    pub target_path: Option<String>,
    pub render_error: Option<String>,
}

/// One mail thread's prefix, `mail:v1:<thread>:`, so a withdrawal never spans threads.
pub fn mail_thread_prefix(prefix: &str) -> bool {
    prefix
        .strip_prefix("mail:v1:")
        .and_then(|rest| rest.strip_suffix(':'))
        .is_some_and(|thread| {
            !thread.is_empty()
                && thread.len() <= 120
                && thread
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
}

/// What `attach` did with one file.
#[derive(Debug, PartialEq, Eq)]
pub enum Attached {
    New,
    Unchanged,
    /// A different file of that name exists; nothing was written.
    Differs,
}

/// What `verify` found different between the index and the files.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drift {
    /// Rows whose file is missing or holds other bytes.
    pub stale: Vec<String>,
    /// Markdown files under the root with no row.
    pub orphaned: Vec<String>,
}

/// What [`Projects::reindex`] changed in the index.
#[derive(Debug, Default, Serialize)]
pub struct Reindexed {
    /// Rows that took their file's newer bytes.
    pub adopted: Vec<String>,
    /// Files that had no row.
    pub added: Vec<String>,
    /// Rows whose file is gone.
    pub dropped: Vec<String>,
    /// Files the index refuses, with the reason. Their previous row, if any, stays.
    pub refused: Vec<(String, String)>,
    /// Files that are not records (unreadable, or a path no record can take).
    pub skipped: Vec<(String, String)>,
}

/// A file holds bytes the index has not seen, so a write from Sjel would overwrite an edit made
/// elsewhere. `projects reindex` adopts the edit.
#[derive(Debug)]
pub struct NewerFile(pub String);

impl std::fmt::Display for NewerFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: the file changed outside Sjel; reindex before writing",
            self.0
        )
    }
}

impl std::error::Error for NewerFile {}

impl Projects {
    /// Open the index and name the directory that holds the record files.
    ///
    /// # Errors
    /// The database cannot be opened or migrated.
    pub async fn open(database: &Path, root: PathBuf) -> Fallible<Self> {
        #[cfg(not(feature = "turso-wal"))]
        let db = Store::open(database, PREFIX, migrate).await?;
        #[cfg(feature = "turso-wal")]
        let db = {
            let database = database.to_owned();
            tokio::task::spawn_blocking(move || {
                sjel_native_wal::open_pool(&database, PREFIX, migrate)
                    .map_err(|error| error.to_string())
            })
            .await??
        };
        Ok(Self {
            db,
            #[cfg(feature = "turso-wal")]
            database: database.canonicalize()?,
            root,
            source_config: None,
        })
    }

    /// Write a closed snapshot of the committed index.
    ///
    /// # Errors
    /// The owner cannot open a reader or create the snapshot.
    #[cfg(feature = "turso-wal")]
    pub fn snapshot(&self, target: &Path) -> Result<(), String> {
        let conn = self.db.get().map_err(|error| error.to_string())?;
        sjel_native_wal::snapshot(&conn, &self.database, target).map_err(|error| error.to_string())
    }

    /// Select the deployment's read-only project task sources.
    pub fn with_sources(mut self, path: Option<PathBuf>) -> Self {
        self.source_config = path;
        self
    }

    /// `<overlay>/data/projects`, or nothing when no overlay is configured.
    pub fn default_root() -> Option<PathBuf> {
        sjel_config::overlay_data_dir(PREFIX)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn ping(&self) -> Fallible<()> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT 1", ()).await?;
        rows.next()
            .await?
            .ok_or_else(|| std::io::Error::other("`SELECT 1` returned no row"))?;
        Ok(())
    }

    pub async fn stage_mail_proposal(
        &self,
        input: &MailProposalInput,
    ) -> Fallible<Option<MailProposal>> {
        input.validate()?;
        if crate::record::is_domain(&input.project) {
            return Err("mail evidence requires a project, not a domain".into());
        }
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let mut rows = tx
            .query(
                "SELECT 1 FROM projects_records WHERE project = ?1 AND archived = 0 LIMIT 1",
                [input.project.as_str()],
            )
            .await?;
        if rows.next().await?.is_none() {
            return Err("no live project".into());
        }
        drop(rows);
        let payload = serde_json::to_string(input)?;
        tx.execute("INSERT INTO projects_mail_proposals (source_id, line_id, event, project, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(source_id, line_id, event, project) DO NOTHING", (input.source_id.as_str(), input.line_id.as_str(), input.event.as_str(), input.project.as_str(), payload.as_str())).await?;
        let mut rows = tx.query("SELECT id, payload, version, status FROM projects_mail_proposals WHERE source_id = ?1 AND line_id = ?2 AND event = ?3 AND project = ?4", (input.source_id.as_str(), input.line_id.as_str(), input.event.as_str(), input.project.as_str())).await?;
        let row = rows.next().await?.ok_or("proposal missing")?;
        let proposal = proposal_from_row(&row)?;
        drop(rows);
        tx.commit().await?;
        if proposal.input != *input {
            return Ok(None);
        }
        Ok(Some(proposal))
    }

    /// Delete every pending or rejected proposal from one thread, for example after the thread
    /// became c3. Approved proposals stay: a reviewer already accepted their evidence.
    pub async fn withdraw_mail_thread(&self, source_prefix: &str) -> Fallible<u64> {
        if !mail_thread_prefix(source_prefix) {
            return Err("source_prefix must be mail:v1:<thread>:".into());
        }
        let conn = self.db.connect().await?;
        // substr, not LIKE: `_` is a LIKE wildcard and is valid in a thread ID.
        Ok(conn.execute("DELETE FROM projects_mail_proposals WHERE substr(source_id, 1, length(?1)) = ?1 AND status IN ('pending', 'rejected')", [source_prefix]).await?)
    }

    pub async fn mail_proposals(&self, before: i64) -> Fallible<Vec<MailProposal>> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT id, payload, version, status FROM projects_mail_proposals WHERE id < ?1 ORDER BY id DESC LIMIT 500", [before]).await?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            result.push(proposal_from_row(&row)?);
        }
        Ok(result)
    }

    pub async fn decide_mail_proposal(
        &self,
        id: i64,
        version: i64,
        decision: &str,
        target: Option<(&str, &str)>,
    ) -> Fallible<Option<MailDecisionOutcome>> {
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let mut rows = tx.query("SELECT id, payload, version, status FROM projects_mail_proposals WHERE id = ?1 AND version = ?2 AND status = 'pending'", (id, version)).await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let mut proposal = proposal_from_row(&row)?;
        drop(rows);
        let mut updated = None;
        let status = match (decision, target) {
            ("approve", Some((path, base))) => {
                let mut rows = tx.query("SELECT path, content FROM projects_records WHERE path = ?1 AND project = ?2 AND archived = 0 AND content = ?3", (path, proposal.input.project.as_str(), base)).await?;
                let Some(row) = rows.next().await? else {
                    return Ok(None);
                };
                let record = record_from_row(&row)?;
                drop(rows);
                let record_class = crate::ops::classify_record(&record).value;
                if record.domain
                    || record.archived
                    || record.project != proposal.input.project
                    || record_class.as_str() < proposal.input.data_class.as_str()
                {
                    return Ok(None);
                }
                let fence_len = proposal
                    .input
                    .evidence
                    .split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
                    .max(3);
                let fence = "`".repeat(fence_len);
                let content = format!(
                    "{}\n\n### Mail evidence\n\nSource: {}\n\nLine: {}\n\n{}\n{}\n{}\n",
                    base,
                    serde_json::to_string(&proposal.input.source_id)?,
                    serde_json::to_string(&proposal.input.line_id)?,
                    fence,
                    proposal.input.evidence,
                    fence
                );
                let record = Record::from_content(path, content)?;
                let count = tx.execute(&format!("UPDATE projects_records SET content = ?1, updated_at = {} WHERE path = ?2 AND project = ?3 AND archived = 0 AND content = ?4", NOW), (record.content.as_str(), path, proposal.input.project.as_str(), base)).await?;
                if count != 1 {
                    return Ok(None);
                }
                tx.execute("INSERT INTO projects_mail_applications (proposal_id, target_path) VALUES (?1, ?2)", (id, path)).await?;
                updated = Some(record);
                "approved_applied"
            }
            ("reject", None) => "rejected",
            _ => return Err("invalid decision or target".into()),
        };
        let changed = tx.execute("UPDATE projects_mail_proposals SET status = ?1, version = version + 1 WHERE id = ?2 AND version = ?3 AND status = 'pending'", (status, id, version)).await?;
        if changed != 1 {
            return Ok(None);
        }
        if let Some((path, base)) = target.filter(|_| updated.is_some()) {
            self.check_file(path, Some(base))?;
        }
        tx.commit().await?;
        proposal.status = status.to_string();
        proposal.version += 1;
        let target_path = updated.as_ref().map(|record| record.path.clone());
        let render_error = updated
            .as_ref()
            .and_then(|record| self.render(record).err())
            .map(|error| error.to_string());
        Ok(Some(MailDecisionOutcome {
            proposal,
            target_path,
            render_error,
        }))
    }

    pub async fn inventory_links(
        &self,
        item_ref: Option<&str>,
        path: Option<&str>,
    ) -> Fallible<Vec<(InventoryLink, Record)>> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT l.record_path, l.item_ref, l.verified_revision, l.reviewed_at, r.path, r.content FROM projects_inventory_links l JOIN projects_records r ON r.path = l.record_path WHERE (?1 IS NULL OR l.item_ref = ?1) AND (?2 IS NULL OR l.record_path = ?2) ORDER BY l.record_path, l.item_ref", (item_ref, path)).await?;
        let mut links = Vec::new();
        while let Some(row) = rows.next().await? {
            let record = Record::from_content(&row.get::<String>(4)?, row.get(5)?)?;
            links.push((
                InventoryLink {
                    record_path: row.get(0)?,
                    item_ref: row.get(1)?,
                    verified_revision: row.get(2)?,
                    reviewed_at: row.get(3)?,
                    provenance: "operator_review",
                },
                record,
            ));
        }
        Ok(links)
    }

    pub async fn decide_inventory_link(
        &self,
        target: InventoryLinkTarget<'_>,
        item_ref: &str,
        verified_revision: Option<i64>,
    ) -> Fallible<Option<bool>> {
        let InventoryLinkTarget {
            path,
            project,
            base,
            revision,
            add,
        } = target;
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let mut rows = tx.query("SELECT path, content FROM projects_records WHERE path = ?1 AND project = ?2 AND archived = 0 AND kind = 'project' AND content = ?3 AND updated_at = ?4", (path, project, base, revision)).await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let record = record_from_row(&row)?;
        drop(rows);
        if record.domain
            || record.archived
            || record.kind != Kind::Project
            || record.project != project
        {
            return Ok(None);
        }
        let changed = if add {
            let item_revision = verified_revision.ok_or("missing Inventory verification")?;
            tx.execute(&format!("INSERT INTO projects_inventory_links (record_path, item_ref, verified_revision, reviewed_at) VALUES (?1, ?2, ?3, {}) ON CONFLICT(record_path,item_ref) DO NOTHING", NOW), (path, item_ref, item_revision)).await?
        } else {
            tx.execute(
                "DELETE FROM projects_inventory_links WHERE record_path = ?1 AND item_ref = ?2",
                (path, item_ref),
            )
            .await?
        };
        tx.commit().await?;
        Ok(Some(changed != 0))
    }

    /// Write one record's file, then its row.
    ///
    /// # Errors
    /// [`NewerFile`] when the file holds bytes the index has not seen; the write or the index
    /// update failed. A failed index update leaves the file written, and `reindex` adopts it.
    pub async fn put(&self, record: &Record) -> Fallible<()> {
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let prior = Self::get_timed_on(&tx, &record.path)
            .await?
            .map(|(r, _)| r.content);
        Self::index_on(&tx, record).await?;
        self.check_file(&record.path, prior.as_deref())?;
        self.render(record)?;
        tx.commit().await?;
        Ok(())
    }

    /// Checks the base under the index writer lock and keeps it through the file replacement.
    pub(crate) async fn save_checked(&self, record: &Record, base: Option<&str>) -> Fallible<()> {
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let path = record.path.as_str();
        let prior = Self::get_timed_on(&tx, path).await?.map(|(r, _)| r.content);
        let disk = match fs::read_to_string(self.root.join(path)) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(content) = disk.filter(|content| Some(content) != prior.as_ref()) {
            return Err(SaveError::Stale(content).into());
        }
        match (prior.as_deref(), base) {
            (Some(current), Some(base)) if current == base => {}
            (Some(current), Some(_)) => return Err(SaveError::Stale(current.to_owned()).into()),
            (Some(_), None) => return Err(SaveError::Refused(format!("{path} exists")).into()),
            (None, Some(_)) => return Err(SaveError::Refused(format!("no record {path:?}")).into()),
            (None, None) if !record.domain => {
                let mut rows = tx.query(
                    "SELECT 1 FROM projects_records WHERE project = ?1 AND archived = 0 LIMIT 1",
                    [record.project.as_str()],
                ).await?;
                if rows.next().await?.is_none() {
                    return Err(SaveError::Refused(format!(
                        "no live project named {:?}",
                        record.project
                    ))
                    .into());
                }
            }
            (None, None) => {}
        }
        Self::index_on(&tx, record).await?;
        self.check_file(path, prior.as_deref())?;
        self.render(record)?;
        tx.commit().await?;
        Ok(())
    }

    /// Refuse when the file at `path` is not the bytes `prior` names: a different file, a file
    /// where no row is, or no file where a row is.
    fn check_file(&self, path: &str, prior: Option<&str>) -> Fallible<()> {
        let on_disk = match fs::read(self.root.join(path)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if on_disk.as_deref() == prior.map(str::as_bytes) {
            Ok(())
        } else {
            Err(NewerFile(path.to_string()).into())
        }
    }

    async fn index_on(tx: &engine::Connection, record: &Record) -> Fallible<()> {
        tx.execute(
            &format!(
                "INSERT INTO projects_records
                   (path, project, kind, title, done, due, priority, archived, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, {now})
                 ON CONFLICT(path) DO UPDATE SET project = excluded.project, kind = excluded.kind,
                   title = excluded.title, done = excluded.done, due = excluded.due,
                   priority = excluded.priority, archived = excluded.archived,
                   content = excluded.content, updated_at = excluded.updated_at",
                now = NOW
            ),
            (
                record.path.as_str(),
                record.project.as_str(),
                record.kind.as_str(),
                record.title.as_str(),
                record.done,
                record.due.as_deref(),
                record.priority,
                record.archived,
                record.content.as_str(),
            ),
        )
        .await?;
        Ok(())
    }

    /// Bring the index to the files: adopt newer bytes, add new files, drop rows whose file is
    /// gone, and refuse a file whose frontmatter declares a class that is invalid or below its
    /// folder floor.
    ///
    /// # Errors
    /// The root is not a directory, it holds no Markdown file while rows exist (an unmounted or
    /// emptied root must not empty the index), or the index cannot be written.
    pub async fn reindex(&self) -> Fallible<Reindexed> {
        if !self.root.is_dir() {
            return Err(format!(
                "{}: not a directory; nothing reindexed",
                self.root.display()
            )
            .into());
        }
        let rows: std::collections::HashMap<String, String> = self
            .all()
            .await?
            .into_iter()
            .map(|r| (r.path, r.content))
            .collect();
        let tree = markdown_root::MarkdownRoot::declare(&self.root)?;
        let files = tree.markdown_files_recursive()?;
        if files.is_empty() && !rows.is_empty() {
            return Err(format!(
                "{}: no Markdown file while the index holds {} rows; nothing reindexed",
                self.root.display(),
                rows.len()
            )
            .into());
        }
        let mut report = Reindexed::default();
        let mut seen = std::collections::HashSet::new();
        for file in files {
            let Some(id) = tree.relative_id(&file) else {
                continue;
            };
            seen.insert(id.clone());
            let content = match fs::read_to_string(&file) {
                Ok(content) => content,
                Err(e) => {
                    report.skipped.push((id, format!("unreadable: {e}")));
                    continue;
                }
            };
            if rows.get(&id) == Some(&content) {
                continue;
            }
            let record = match Record::from_content(&id, content) {
                Ok(record) => record,
                Err(reason) => {
                    report.skipped.push((id, reason));
                    continue;
                }
            };
            if let Some(reason) = crate::ops::class_refusal(&record) {
                report.refused.push((id, reason));
                continue;
            }
            let mut conn = self.db.connect().await?;
            let tx = begin_write(&mut conn).await?;
            match fs::read(&file) {
                Ok(content) if content == record.content.as_bytes() => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    report.skipped.push((id, format!("unreadable: {error}")));
                    continue;
                }
            }
            Self::index_on(&tx, &record).await?;
            tx.commit().await?;
            if rows.contains_key(&id) {
                report.adopted.push(id);
            } else {
                report.added.push(id);
            }
        }
        let mut gone: Vec<&String> = rows.keys().filter(|path| !seen.contains(*path)).collect();
        gone.sort();
        for path in gone {
            let mut conn = self.db.connect().await?;
            let tx = begin_write(&mut conn).await?;
            if self.root.join(path).try_exists()? {
                continue;
            }
            tx.execute(
                "DELETE FROM projects_inventory_links WHERE record_path = ?1",
                [path.as_str()],
            )
            .await?;
            tx.execute(
                "DELETE FROM projects_records WHERE path = ?1",
                [path.as_str()],
            )
            .await?;
            tx.commit().await?;
            report.dropped.push(path.clone());
        }
        Ok(report)
    }

    /// Delete one record and its file. Returns whether a row was there.
    ///
    /// # Errors
    /// [`NewerFile`] when the file changed outside Sjel; the delete or the file removal failed.
    pub async fn remove(&self, path: &str) -> Fallible<bool> {
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        let prior = Self::get_timed_on(&tx, path).await?.map(|(r, _)| r.content);
        tx.execute(
            "DELETE FROM projects_inventory_links WHERE record_path = ?1",
            [path],
        )
        .await?;
        let removed = tx
            .execute("DELETE FROM projects_records WHERE path = ?1", [path])
            .await?
            > 0;
        self.check_file(path, prior.as_deref())?;
        match fs::remove_file(self.root.join(path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Move one record to a new path: the row takes the new path and every column the new path
    /// derives, the file is written from the row's content, and its inventory links follow.
    /// Returns whether a row was there.
    ///
    /// # Errors
    /// The new path is invalid or occupied, or the source changed outside Sjel.
    pub async fn move_record(&self, from: &str, to: &str) -> Fallible<bool> {
        let mut conn = self.db.connect().await?;
        let tx = begin_write(&mut conn).await?;
        if Self::get_timed_on(&tx, to).await?.is_some() {
            return Err(format!("{to}: a record is already there").into());
        }
        let Some((record, _)) = Self::get_timed_on(&tx, from).await? else {
            return Ok(false);
        };
        let moved =
            Record::from_content(to, record.content).map_err(|why| format!("{to}: {why}"))?;
        tx.execute(
            "UPDATE projects_inventory_links SET record_path = ?1 WHERE record_path = ?2",
            [to, from],
        )
        .await?;
        // Every column is derived from the path and the bytes, so a loose archive note's project
        // follows the new title too.
        tx.execute(
            "UPDATE projects_records
                SET path = ?1, project = ?2, kind = ?3, title = ?4, done = ?5, due = ?6,
                    priority = ?7, archived = ?8
              WHERE path = ?9",
            (
                to,
                moved.project.as_str(),
                moved.kind.as_str(),
                moved.title.as_str(),
                moved.done,
                moved.due.as_deref(),
                moved.priority,
                moved.archived,
                from,
            ),
        )
        .await?;
        self.check_file(from, Some(&moved.content))?;
        self.check_file(to, None)?;
        self.render(&moved)?;
        match fs::remove_file(self.root.join(from)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn get(&self, path: &str) -> Fallible<Option<Record>> {
        Ok(self.get_timed(path).await?.map(|(record, _)| record))
    }

    /// One record with the moment its row was last written.
    ///
    /// The time is the row's `updated_at`, which changes on every `put`, so a replica can ask
    /// "did this change since I last read it?" without asking for the content.
    pub async fn get_timed(&self, path: &str) -> Fallible<Option<(Record, String)>> {
        let conn = self.db.connect().await?;
        Self::get_timed_on(&conn, path).await
    }

    async fn get_timed_on(
        conn: &engine::Connection,
        path: &str,
    ) -> Fallible<Option<(Record, String)>> {
        let mut rows = conn
            .query(
                "SELECT path, content, updated_at FROM projects_records WHERE path = ?1",
                [path],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let record = record_from_row(&row)?;
        Ok(Some((record, row.get(2)?)))
    }

    /// Every record, ordered by path.
    pub async fn all(&self) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records ORDER BY path",
            Vec::new(),
        )
        .await
    }

    pub async fn by_kind(&self, kind: Kind) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records WHERE kind = ?1 ORDER BY path",
            vec![kind.as_str().into()],
        )
        .await
    }

    pub async fn by_project(&self, project: &str) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records WHERE project = ?1 ORDER BY path",
            vec![project.into()],
        )
        .await
    }

    /// One project's records, each with the moment its row was last written.
    ///
    /// A replica reads this with every content left out and then fetches only the records whose
    /// time moved, instead of re-reading a project of 183 notes that did not change.
    pub async fn by_project_timed(&self, project: &str) -> Fallible<Vec<(Record, String)>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT path, content, updated_at FROM projects_records WHERE project = ?1 ORDER BY path",
                vec![engine::Value::from(project)],
            )
            .await?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await? {
            let record = record_from_row(&row)?;
            records.push((record, row.get(2)?));
        }
        Ok(records)
    }

    async fn query(&self, sql: &str, params: Vec<engine::Value>) -> Fallible<Vec<Record>> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query(sql, params).await?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await? {
            records.push(record_from_row(&row)?);
        }
        Ok(records)
    }

    /// Store an attachment: a binary file of a project, kept as a file in the tree.
    ///
    /// Attachments are not rows. 986 MB of PDFs and images came with the vault import
    /// (measured 2026-10-18); in the shared database they would grow every backup of it. The tree
    /// is a backup path already. An existing file is never overwritten.
    ///
    /// # Errors
    /// The path is not an attachment path, or the write failed.
    pub fn attach(&self, path: &str, bytes: &[u8]) -> Fallible<Attached> {
        check_attachment(path)?;
        match fs::read(self.root.join(path)) {
            Ok(current) if current == bytes => return Ok(Attached::Unchanged),
            Ok(_) => return Ok(Attached::Differs),
            Err(_) => {}
        }
        self.write_file(path, bytes)?;
        Ok(Attached::New)
    }

    /// Every attachment path, sorted.
    pub fn attachments(&self) -> Fallible<Vec<String>> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let tree = markdown_root::MarkdownRoot::declare(&self.root)?;
        Ok(tree
            .files_recursive()?
            .iter()
            .filter_map(|f| tree.relative_id(f))
            .filter(|id| check_attachment(id).is_ok())
            .collect())
    }

    /// The bytes of one attachment, or nothing when there is none at that path.
    ///
    /// # Errors
    /// The path is not an attachment path.
    pub fn attachment(&self, path: &str) -> Fallible<Option<Vec<u8>>> {
        check_attachment(path)?;
        Ok(fs::read(self.root.join(path)).ok())
    }

    fn render(&self, record: &Record) -> Fallible<()> {
        self.write_file(&record.path, record.content.as_bytes())
    }

    /// Whole file, temp-plus-rename, mode 0600.
    fn write_file(&self, path: &str, bytes: &[u8]) -> Fallible<()> {
        let target = self.root.join(path);
        let dir = target.parent().ok_or("record path has no folder")?;
        fs::create_dir_all(dir)?;
        let name = target.file_name().ok_or("record path has no file name")?;
        let temp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &target)?;
        Ok(())
    }

    /// Compare the index with the files, byte for byte.
    pub async fn verify(&self) -> Fallible<Drift> {
        let records = self.all().await?;
        let mut drift = Drift::default();
        for record in &records {
            match fs::read(self.root.join(&record.path)) {
                Ok(bytes) if bytes == record.content.as_bytes() => {}
                _ => drift.stale.push(record.path.clone()),
            }
        }
        let known: std::collections::HashSet<&str> =
            records.iter().map(|r| r.path.as_str()).collect();
        if self.root.is_dir() {
            let tree = markdown_root::MarkdownRoot::declare(&self.root)?;
            for file in tree.markdown_files_recursive()? {
                if let Some(id) = tree.relative_id(&file) {
                    if !known.contains(id.as_str()) {
                        drift.orphaned.push(id);
                    }
                }
            }
        }
        Ok(drift)
    }
}

fn proposal_from_row(row: &engine::Row) -> Fallible<MailProposal> {
    let payload: String = row.get(1)?;
    Ok(MailProposal {
        id: row.get(0)?,
        input: serde_json::from_str(&payload)?,
        version: row.get(2)?,
        status: row.get(3)?,
    })
}

/// `path, content` is the whole row a record needs: every other column is derived from the
/// content when the record is read, so the file stays the single source of the fields.
fn record_from_row(row: &engine::Row) -> Fallible<Record> {
    let path: String = row.get(0)?;
    let content: String = row.get(1)?;
    Ok(Record::from_content(&path, content)?)
}

const PROJECTS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS projects_records (
            path TEXT PRIMARY KEY,
            project TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('project', 'task', 'note')),
            title TEXT NOT NULL,
            done INTEGER NOT NULL,
            due TEXT,
            priority INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 3),
            archived INTEGER NOT NULL,
            content TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS projects_records_project ON projects_records (project);
        CREATE INDEX IF NOT EXISTS projects_records_kind ON projects_records (kind, done);
        CREATE TABLE IF NOT EXISTS projects_inventory_links (
            record_path TEXT NOT NULL, item_ref TEXT NOT NULL,
            verified_revision INTEGER NOT NULL, reviewed_at TEXT NOT NULL,
            PRIMARY KEY (record_path, item_ref)
        );
        CREATE INDEX IF NOT EXISTS projects_inventory_links_target ON projects_inventory_links (item_ref);";
const MAIL_PROPOSAL_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS projects_mail_proposals (
            id INTEGER PRIMARY KEY,
            source_id TEXT NOT NULL, line_id TEXT NOT NULL, event TEXT NOT NULL,
            project TEXT NOT NULL, payload TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'rejected', 'approved_pending_action', 'approved_applied')),
            UNIQUE (source_id, line_id, event, project)
        );";
const MAIL_APPLICATION_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS projects_mail_applications (
        proposal_id INTEGER PRIMARY KEY REFERENCES projects_mail_proposals(id),
        target_path TEXT NOT NULL
    )";
const MAIL_PROPOSAL_DDL: &str =
    "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'projects_mail_proposals'";
const RENAME_PROPOSAL_TABLE: &str =
    "ALTER TABLE projects_mail_proposals RENAME TO projects_mail_proposals_old";
const MOVE_PROPOSALS: &str =
    "INSERT INTO projects_mail_proposals SELECT * FROM projects_mail_proposals_old;
     DROP TABLE projects_mail_proposals_old;";

#[cfg(not(feature = "turso-wal"))]
async fn migrate(conn: engine::Connection) -> Fallible<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    tx.execute_batch(PROJECTS_SCHEMA).await?;
    tx.execute_batch(MAIL_PROPOSAL_SCHEMA).await?;
    let mut rows = tx.query(MAIL_PROPOSAL_DDL, ()).await?;
    let sql: String = rows
        .next()
        .await?
        .ok_or("proposal schema missing")?
        .get(0)?;
    drop(rows);
    if !sql.contains("approved_applied") {
        tx.execute(RENAME_PROPOSAL_TABLE, ()).await?;
        tx.execute_batch(MAIL_PROPOSAL_SCHEMA).await?;
        tx.execute_batch(MOVE_PROPOSALS).await?;
    }
    tx.execute(MAIL_APPLICATION_SCHEMA, ()).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(feature = "turso-wal")]
fn migrate(conn: &sjel_native_wal::Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(PROJECTS_SCHEMA)?;
    conn.execute_batch(MAIL_PROPOSAL_SCHEMA)?;
    let sql: String = conn.query_row(MAIL_PROPOSAL_DDL, (), |row| row.get(0))?;
    if !sql.contains("approved_applied") {
        conn.execute(RENAME_PROPOSAL_TABLE, ())?;
        conn.execute_batch(MAIL_PROPOSAL_SCHEMA)?;
        conn.execute_batch(MOVE_PROPOSALS)?;
    }
    conn.execute(MAIL_APPLICATION_SCHEMA, ())?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) async fn scratch(name: &str) -> (Projects, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sjel-projects-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("sjel.db"), dir.join("rendered"))
            .await
            .unwrap();
        (projects, dir)
    }

    #[tokio::test]
    async fn migration_preserves_previous_reviewed_proposals() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-projects-mail-migration-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sjel.db");
        #[cfg(not(feature = "turso-wal"))]
        let conn = sjel_turso::open_local(&path).await.unwrap();
        #[cfg(feature = "turso-wal")]
        let conn = engine::Builder::new_local(path.to_str().unwrap())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        conn.execute_batch("CREATE TABLE projects_mail_proposals (id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, line_id TEXT NOT NULL, event TEXT NOT NULL, project TEXT NOT NULL, payload TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1, status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'rejected', 'approved_pending_action')), UNIQUE (source_id, line_id, event, project)); INSERT INTO projects_mail_proposals VALUES (1, 'm', 'l', 'project_evidence', 'P', '{\"source_id\":\"m\",\"line_id\":\"l\",\"event\":\"project_evidence\",\"project\":\"P\",\"title\":\"t\",\"evidence\":\"e\",\"data_class\":\"c1\",\"task_suggestion\":null}', 2, 'approved_pending_action');").await.unwrap();
        drop(conn);
        let store = Projects::open(&path, dir.join("rendered")).await.unwrap();
        let proposals = store.mail_proposals(i64::MAX).await.unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "approved_pending_action");
        assert_eq!(proposals[0].version, 2);
        fs::remove_dir_all(dir).unwrap();
    }

    fn task(path: &str, done: bool) -> Record {
        Record::from_content(
            path,
            format!("---\ntype: task\ndone: {done}\npriority: 1\n---\n\nbody\n"),
        )
        .unwrap()
    }

    #[cfg(feature = "turso-wal")]
    #[tokio::test]
    async fn the_owner_snapshot_restores_a_committed_record() {
        let (projects, dir) = scratch("snapshot").await;
        let record = task("Example/Tasks/Review.md", true);
        projects.put(&record).await.unwrap();
        let target = dir.join("restored.db");
        projects.snapshot(&target).unwrap();
        let restored = Projects::open(&target, dir.join("isolated")).await.unwrap();
        let recovered = restored.get(&record.path).await.unwrap().unwrap();
        assert_eq!(recovered.content, record.content);
        assert!(recovered.done);
        assert!(!restored.root().exists());
        fs::remove_dir_all(dir).unwrap();
    }

    /// A write and its file change together.
    #[tokio::test]
    async fn every_put_renders_the_same_bytes() {
        let (projects, dir) = scratch("render").await;
        let first = task("Waschmaschine/Tasks/Schlauch.md", false);
        projects.put(&first).await.unwrap();
        let file = projects.root().join(&first.path);
        assert_eq!(fs::read_to_string(&file).unwrap(), first.content);
        let second = task("Waschmaschine/Tasks/Schlauch.md", true);
        projects.put(&second).await.unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), second.content);
        assert_eq!(projects.verify().await.unwrap(), Drift::default());
        fs::remove_dir_all(dir).unwrap();
    }

    /// A loose archive note's project is its own name, so the move carries it.
    #[tokio::test]
    async fn a_move_takes_every_derived_column_with_it() {
        let (projects, dir) = scratch("move-columns").await;
        let loose = Record::from_content(
            "Archive/Alt.md",
            "---\ntype: task\ndone: true\npriority: 1\ndue: 2026-11-01\n---\n\nx\n".into(),
        )
        .unwrap();
        projects.put(&loose).await.unwrap();
        assert!(projects
            .move_record("Archive/Alt.md", "Archive/Neu.md")
            .await
            .unwrap());
        let moved = projects.get("Archive/Neu.md").await.unwrap().unwrap();
        assert_eq!(
            (
                moved.project.as_str(),
                moved.archived,
                moved.done,
                moved.priority
            ),
            ("Neu", true, true, 1)
        );
        assert_eq!(moved.due.as_deref(), Some("2026-11-01"));
        assert!(!projects.root().join("Archive/Alt.md").exists());
        assert_eq!(projects.verify().await.unwrap(), Drift::default());
        // A name that is taken, and a title that is not a path, are refused without a write.
        assert!(projects
            .move_record("Archive/Neu.md", "Archive/Neu.md")
            .await
            .is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Rows rebuilt from the files equal the live rows.
    #[tokio::test]
    async fn a_store_rebuilt_from_the_files_is_identical() {
        let (projects, dir) = scratch("roundtrip").await;
        projects.put(&task("A/Tasks/One.md", false)).await.unwrap();
        projects
            .put(&task("Archive/B/Tasks/Two.md", true))
            .await
            .unwrap();
        let rebuilt = Projects::open(&dir.join("rebuilt.db"), dir.join("unused"))
            .await
            .unwrap();
        let tree = markdown_root::MarkdownRoot::declare(projects.root()).unwrap();
        for file in tree.markdown_files_recursive().unwrap() {
            let id = tree.relative_id(&file).unwrap();
            rebuilt
                .put(&Record::from_content(&id, fs::read_to_string(file).unwrap()).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(rebuilt.all().await.unwrap(), projects.all().await.unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_attachment_is_written_once_and_never_overwritten() {
        let (projects, dir) = scratch("attach").await;
        assert_eq!(
            projects.attach("A/Assets/x.png", b"png").unwrap(),
            Attached::New
        );
        assert_eq!(
            projects.attach("A/Assets/x.png", b"png").unwrap(),
            Attached::Unchanged
        );
        assert_eq!(
            projects.attach("A/Assets/x.png", b"other").unwrap(),
            Attached::Differs
        );
        assert_eq!(
            projects.attachment("A/Assets/x.png").unwrap().unwrap(),
            b"png"
        );
        assert!(projects.attach("A/x.md", b"note").is_err());
        assert!(projects.attachment("../secret.png").is_err());
        projects.put(&task("A/Tasks/One.md", false)).await.unwrap();
        assert_eq!(projects.attachments().unwrap(), vec!["A/Assets/x.png"]);
        assert_eq!(
            projects.verify().await.unwrap(),
            Drift::default(),
            "attachments are not orphans"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_edit_elsewhere_is_adopted_and_never_overwritten() {
        let (projects, dir) = scratch("drift").await;
        let record = task("A/Tasks/One.md", false);
        projects.put(&record).await.unwrap();
        let edited = "---\ntype: task\ndone: true\npriority: 1\n---\n\nedited in another editor\n";
        fs::write(projects.root().join(&record.path), edited).unwrap();
        fs::write(projects.root().join("A/stray.md"), "x").unwrap();
        let drift = projects.verify().await.unwrap();
        assert_eq!(drift.stale, vec!["A/Tasks/One.md"]);
        assert_eq!(drift.orphaned, vec!["A/stray.md"]);

        let overwrite = projects
            .put(&task("A/Tasks/One.md", false))
            .await
            .unwrap_err();
        assert!(
            overwrite.downcast_ref::<NewerFile>().is_some(),
            "{overwrite}"
        );
        assert!(projects.remove("A/Tasks/One.md").await.is_err());
        assert_eq!(
            fs::read_to_string(projects.root().join(&record.path)).unwrap(),
            edited
        );

        let report = projects.reindex().await.unwrap();
        assert_eq!(report.adopted, vec!["A/Tasks/One.md"]);
        assert_eq!(report.added, vec!["A/stray.md"]);
        assert!(projects.get("A/Tasks/One.md").await.unwrap().unwrap().done);
        assert_eq!(projects.verify().await.unwrap(), Drift::default());

        assert!(projects.remove("A/Tasks/One.md").await.unwrap());
        assert!(!projects.root().join("A/Tasks/One.md").exists());
        assert!(!projects.remove("A/Tasks/One.md").await.unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn reindex_drops_deleted_files_and_refuses_a_lowered_class() {
        let (projects, dir) = scratch("reindex").await;
        projects.put(&task("A/Tasks/One.md", false)).await.unwrap();
        let finance = Record::from_content(
            "Atlas/Finance/Depot.md",
            "---\ntype: note\n---\n\nbalance\n".into(),
        )
        .unwrap();
        projects.put(&finance).await.unwrap();

        let lowered = "---\ntype: note\nclass: c1\n---\n\nbalance\n";
        fs::write(projects.root().join(&finance.path), lowered).unwrap();
        fs::remove_file(projects.root().join("A/Tasks/One.md")).unwrap();
        let report = projects.reindex().await.unwrap();
        assert_eq!(report.dropped, vec!["A/Tasks/One.md"]);
        assert_eq!(report.refused.len(), 1);
        assert_eq!(report.refused[0].0, "Atlas/Finance/Depot.md");
        assert!(report.refused[0].1.contains("below the folder floor c2"));
        assert_eq!(
            projects.get(&finance.path).await.unwrap().unwrap().content,
            finance.content,
            "a refused file leaves the previous row"
        );
        assert!(projects.get("A/Tasks/One.md").await.unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_empty_or_missing_root_never_empties_the_index() {
        let (projects, dir) = scratch("empty-root").await;
        projects.put(&task("A/Tasks/One.md", false)).await.unwrap();
        fs::remove_file(projects.root().join("A/Tasks/One.md")).unwrap();
        assert!(projects.reindex().await.is_err());
        fs::remove_dir_all(projects.root()).unwrap();
        assert!(projects.reindex().await.is_err());
        assert!(projects.get("A/Tasks/One.md").await.unwrap().is_some());
        fs::remove_dir_all(dir).unwrap();
    }
}
