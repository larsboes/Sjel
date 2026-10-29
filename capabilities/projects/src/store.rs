//! The shared store holds every record; each write re-renders that record's file in the same call.
//!
//! The database is canonical. The rendered tree under `<overlay>/data/projects/` is the second
//! copy Principle 8 asks for, written the way finance writes its decisions: whole file,
//! temp-plus-rename, mode 0600.
//!
//! The database is LibSQL through `sjel-turso`, so every read and write is awaited. The rendered
//! tree stays ordinary synchronous file I/O: it is a few KB per record on the overlay, and
//! `markdown_root` walks directories with no async form. Keeping that split means the reactor
//! waits on the database and never on a temp-file rename.

use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sjel_turso::{libsql, Store};

use crate::record::{check_attachment, Kind, Record};

pub const PREFIX: &str = "projects";
type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Projects {
    db: Store,
    root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MailProposalInput {
    pub source_id: String,
    pub line_id: String,
    pub event: String,
    pub project: String,
    pub title: String,
    pub evidence: String,
    pub data_class: String,
    pub task_suggestion: Option<String>,
}

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

impl MailProposalInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, max) in [
            (&self.source_id, 512),
            (&self.line_id, 512),
            (&self.project, 255),
            (&self.title, 255),
            (&self.evidence, 2000),
        ] {
            if value.trim().is_empty() || value.len() > max || value.contains('\0') {
                return Err("proposal field is empty or exceeds its limit");
            }
        }
        if self.event != "project_evidence"
            || !["c0", "c1", "c2"].contains(&self.data_class.as_str())
            || crate::record::is_domain(&self.project)
        {
            return Err("invalid event, data class, or project");
        }
        if self
            .task_suggestion
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 1000 || s.contains('\0'))
        {
            return Err("invalid task suggestion");
        }
        Ok(())
    }
}

/// What `attach` did with one file.
#[derive(Debug, PartialEq, Eq)]
pub enum Attached {
    New,
    Unchanged,
    /// A different file of that name exists; nothing was written.
    Differs,
}

/// What `verify` found wrong between the store and the rendered tree.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drift {
    /// Rows whose file is missing or holds other bytes.
    pub stale: Vec<String>,
    /// Markdown files under the root with no row.
    pub orphaned: Vec<String>,
}

impl Projects {
    /// Open the store and name the directory records render into.
    ///
    /// # Errors
    /// The database cannot be opened or migrated.
    pub async fn open(database: &Path, root: PathBuf) -> Fallible<Self> {
        let db = Store::open(database, PREFIX, migrate).await?;
        Ok(Self { db, root })
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
        let tx = self.db.begin().await?;
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
        let tx = self.db.begin().await?;
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
                let count = tx.execute(&format!("UPDATE projects_records SET content = ?1, updated_at = {} WHERE path = ?2 AND project = ?3 AND archived = 0 AND content = ?4", sjel_turso::NOW), (record.content.as_str(), path, proposal.input.project.as_str(), base)).await?;
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
        let tx = self.db.begin().await?;
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
            tx.execute(&format!("INSERT INTO projects_inventory_links (record_path, item_ref, verified_revision, reviewed_at) VALUES (?1, ?2, ?3, {}) ON CONFLICT(record_path,item_ref) DO NOTHING", sjel_turso::NOW), (path, item_ref, item_revision)).await?
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

    /// Insert or replace one record, then render its file.
    ///
    /// # Errors
    /// The write or the render failed. A failed render leaves the row written; `render_all`
    /// repairs the file, and `verify` reports it until then.
    pub async fn put(&self, record: &Record) -> Fallible<()> {
        let tx = self.db.begin().await?;
        tx.execute(
            &format!(
                "INSERT INTO projects_records
                   (path, project, kind, title, done, due, priority, archived, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, {now})
                 ON CONFLICT(path) DO UPDATE SET project = excluded.project, kind = excluded.kind,
                   title = excluded.title, done = excluded.done, due = excluded.due,
                   priority = excluded.priority, archived = excluded.archived,
                   content = excluded.content, updated_at = excluded.updated_at",
                now = sjel_turso::NOW
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
        tx.commit().await?;
        self.render(record)
    }

    /// Delete one record and its rendered file. Returns whether a row was there.
    ///
    /// # Errors
    /// The delete or the file removal failed.
    pub async fn remove(&self, path: &str) -> Fallible<bool> {
        let tx = self.db.begin().await?;
        tx.execute(
            "DELETE FROM projects_inventory_links WHERE record_path = ?1",
            [path],
        )
        .await?;
        let removed = tx
            .execute("DELETE FROM projects_records WHERE path = ?1", [path])
            .await?
            > 0;
        tx.commit().await?;
        match fs::remove_file(self.root.join(path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        Ok(removed)
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
                vec![libsql::Value::from(project)],
            )
            .await?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await? {
            let record = record_from_row(&row)?;
            records.push((record, row.get(2)?));
        }
        Ok(records)
    }

    async fn query(&self, sql: &str, params: Vec<libsql::Value>) -> Fallible<Vec<Record>> {
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

    /// Re-render every record. Repairs any file `verify` reports as stale.
    pub async fn render_all(&self) -> Fallible<usize> {
        let records = self.all().await?;
        for record in &records {
            self.render(record)?;
        }
        Ok(records.len())
    }

    /// Compare the store with the rendered tree, byte for byte.
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

fn proposal_from_row(row: &libsql::Row) -> Fallible<MailProposal> {
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
fn record_from_row(row: &libsql::Row) -> Fallible<Record> {
    let path: String = row.get(0)?;
    let content: String = row.get(1)?;
    Ok(Record::from_content(&path, content)?)
}

/// The table this capability owns in the shared file. `kind` and `priority` carry the same
/// constraints `record.rs` enforces, so a hand-written row cannot disagree with a rendered one.
async fn migrate(conn: libsql::Connection) -> Fallible<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects_records (
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
        CREATE INDEX IF NOT EXISTS projects_inventory_links_target ON projects_inventory_links (item_ref);
        CREATE TABLE IF NOT EXISTS projects_mail_proposals (
            id INTEGER PRIMARY KEY,
            source_id TEXT NOT NULL, line_id TEXT NOT NULL, event TEXT NOT NULL,
            project TEXT NOT NULL, payload TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'rejected', 'approved_pending_action')),
            UNIQUE (source_id, line_id, event, project)
        );",
    )
    .await?;
    let mut rows = conn.query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'projects_mail_proposals'", ()).await?;
    let sql: String = rows
        .next()
        .await?
        .ok_or("proposal schema missing")?
        .get(0)?;
    drop(rows);
    if !sql.contains("approved_applied") {
        conn.execute_batch("BEGIN IMMEDIATE;
            ALTER TABLE projects_mail_proposals RENAME TO projects_mail_proposals_old;
            CREATE TABLE projects_mail_proposals (
                id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, line_id TEXT NOT NULL,
                event TEXT NOT NULL, project TEXT NOT NULL, payload TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'rejected', 'approved_pending_action', 'approved_applied')),
                UNIQUE (source_id, line_id, event, project)
            );
            INSERT INTO projects_mail_proposals SELECT * FROM projects_mail_proposals_old;
            DROP TABLE projects_mail_proposals_old;
            COMMIT;").await?;
    }
    conn.execute("CREATE TABLE IF NOT EXISTS projects_mail_applications (proposal_id INTEGER PRIMARY KEY REFERENCES projects_mail_proposals(id), target_path TEXT NOT NULL)", ()).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) async fn scratch(name: &str) -> (Projects, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sjel-projects-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
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
        let path = dir.join("axon.db");
        let conn = sjel_turso::open_local(&path).await.unwrap();
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

    /// Rows rebuilt from the rendered tree equal the live rows.
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
    async fn verify_reports_a_changed_file_and_an_orphan() {
        let (projects, dir) = scratch("drift").await;
        let record = task("A/Tasks/One.md", false);
        projects.put(&record).await.unwrap();
        fs::write(projects.root().join(&record.path), "edited").unwrap();
        fs::write(projects.root().join("A/stray.md"), "x").unwrap();
        let drift = projects.verify().await.unwrap();
        assert_eq!(drift.stale, vec!["A/Tasks/One.md"]);
        assert_eq!(drift.orphaned, vec!["A/stray.md"]);
        projects.render_all().await.unwrap();
        assert!(projects.verify().await.unwrap().stale.is_empty());
        assert!(projects.remove("A/Tasks/One.md").await.unwrap());
        assert!(!projects.root().join("A/Tasks/One.md").exists());
        assert!(!projects.remove("A/Tasks/One.md").await.unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}
