//! The shared store holds every record; each write re-renders that record's file in the same call.
//!
//! The database is canonical. The rendered tree under `<overlay>/data/projects/` is the second
//! copy Principle 8 asks for, written the way finance writes its decisions: whole file,
//! temp-plus-rename, mode 0600 (ISA.md ISC-68).

use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::record::{Kind, Record};

pub const PREFIX: &str = "projects";
type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Projects {
    pool: sjel_store::Pool,
    root: PathBuf,
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
    pub fn open(database: &Path, root: PathBuf) -> Fallible<Self> {
        let pool = sjel_store::open_pool(database, PREFIX, migrate)?;
        Ok(Self { pool, root })
    }

    /// `<overlay>/data/projects`, or nothing when no overlay is configured.
    pub fn default_root() -> Option<PathBuf> {
        sjel_config::overlay_data_dir(PREFIX)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ping(&self) -> Fallible<()> {
        self.pool.get()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    /// Insert or replace one record, then render its file.
    ///
    /// # Errors
    /// The write or the render failed. A failed render leaves the row written; `render_all`
    /// repairs the file, and `verify` reports it until then.
    pub fn put(&self, record: &Record) -> Fallible<()> {
        let mut conn = self.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            &format!(
                "INSERT INTO projects_records
                   (path, project, kind, title, done, due, priority, archived, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, {now})
                 ON CONFLICT(path) DO UPDATE SET project = excluded.project, kind = excluded.kind,
                   title = excluded.title, done = excluded.done, due = excluded.due,
                   priority = excluded.priority, archived = excluded.archived,
                   content = excluded.content, updated_at = excluded.updated_at",
                now = sjel_store::NOW
            ),
            params![
                record.path,
                record.project,
                record.kind.as_str(),
                record.title,
                record.done,
                record.due,
                record.priority,
                record.archived,
                record.content
            ],
        )?;
        tx.commit()?;
        self.render(record)
    }

    pub fn get(&self, path: &str) -> Fallible<Option<Record>> {
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                "SELECT path, content FROM projects_records WHERE path = ?1",
                [path],
                path_and_content,
            )
            .optional()?
            .map(|(p, c)| Record::from_content(&p, c))
            .transpose()?)
    }

    /// Every record, ordered by path.
    pub fn all(&self) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records ORDER BY path",
            [],
        )
    }

    pub fn by_kind(&self, kind: Kind) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records WHERE kind = ?1 ORDER BY path",
            [kind.as_str()],
        )
    }

    pub fn by_project(&self, project: &str) -> Fallible<Vec<Record>> {
        self.query(
            "SELECT path, content FROM projects_records WHERE project = ?1 ORDER BY path",
            [project],
        )
    }

    fn query<P: rusqlite::Params>(&self, sql: &str, params: P) -> Fallible<Vec<Record>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params, path_and_content)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(p, c)| Record::from_content(&p, c).map_err(Into::into))
            .collect()
    }

    fn render(&self, record: &Record) -> Fallible<()> {
        let target = self.root.join(&record.path);
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
        file.write_all(record.content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &target)?;
        Ok(())
    }

    /// Re-render every record. Repairs any file `verify` reports as stale.
    pub fn render_all(&self) -> Fallible<usize> {
        let records = self.all()?;
        for record in &records {
            self.render(record)?;
        }
        Ok(records.len())
    }

    /// Compare the store with the rendered tree, byte for byte (ISA.md ISC-69).
    pub fn verify(&self) -> Fallible<Drift> {
        let records = self.all()?;
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

fn path_and_content(row: &Row<'_>) -> rusqlite::Result<(String, String)> {
    Ok((row.get(0)?, row.get(1)?))
}

fn migrate(conn: &Connection) -> Fallible<()> {
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
        CREATE INDEX IF NOT EXISTS projects_records_kind ON projects_records (kind, done);",
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn scratch(name: &str) -> (Projects, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sjel-projects-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered")).unwrap();
        (projects, dir)
    }

    fn task(path: &str, done: bool) -> Record {
        Record::from_content(
            path,
            format!("---\ntype: task\ndone: {done}\npriority: 1\n---\n\nbody\n"),
        )
        .unwrap()
    }

    /// ISC-68: a write and its file change together.
    #[test]
    fn every_put_renders_the_same_bytes() {
        let (projects, dir) = scratch("render");
        let first = task("Waschmaschine/Tasks/Schlauch.md", false);
        projects.put(&first).unwrap();
        let file = projects.root().join(&first.path);
        assert_eq!(fs::read_to_string(&file).unwrap(), first.content);
        let second = task("Waschmaschine/Tasks/Schlauch.md", true);
        projects.put(&second).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), second.content);
        assert_eq!(projects.verify().unwrap(), Drift::default());
        fs::remove_dir_all(dir).unwrap();
    }

    /// ISC-69: rows rebuilt from the rendered tree equal the live rows.
    #[test]
    fn a_store_rebuilt_from_the_files_is_identical() {
        let (projects, dir) = scratch("roundtrip");
        projects.put(&task("A/Tasks/One.md", false)).unwrap();
        projects.put(&task("Archive/B/Tasks/Two.md", true)).unwrap();
        let rebuilt = Projects::open(&dir.join("rebuilt.db"), dir.join("unused")).unwrap();
        let tree = markdown_root::MarkdownRoot::declare(projects.root()).unwrap();
        for file in tree.markdown_files_recursive().unwrap() {
            let id = tree.relative_id(&file).unwrap();
            rebuilt
                .put(&Record::from_content(&id, fs::read_to_string(file).unwrap()).unwrap())
                .unwrap();
        }
        assert_eq!(rebuilt.all().unwrap(), projects.all().unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verify_reports_a_changed_file_and_an_orphan() {
        let (projects, dir) = scratch("drift");
        let record = task("A/Tasks/One.md", false);
        projects.put(&record).unwrap();
        fs::write(projects.root().join(&record.path), "edited").unwrap();
        fs::write(projects.root().join("A/stray.md"), "x").unwrap();
        let drift = projects.verify().unwrap();
        assert_eq!(drift.stale, vec!["A/Tasks/One.md"]);
        assert_eq!(drift.orphaned, vec!["A/stray.md"]);
        projects.render_all().unwrap();
        assert!(projects.verify().unwrap().stale.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
