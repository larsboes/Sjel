//! Connections for owner-only native Turso WAL databases.

use futures_executor::block_on;
use std::{
    collections::HashMap,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use turso::{Builder, Database, IntoParams, Statement};

pub use turso::{params, params_from_iter, transaction::TransactionBehavior, Error, Result};
/// The UTC timestamp expression used in persisted Sjel records.
pub const NOW: &str = "strftime('%Y-%m-%d %H:%M:%f+00:00','now')";

pub struct Row<'row> {
    row: &'row turso::Row,
    columns: &'row [String],
}

pub trait ColumnIndex {
    fn index(self, columns: &[String]) -> Result<usize>;
}

impl ColumnIndex for usize {
    fn index(self, _: &[String]) -> Result<usize> {
        Ok(self)
    }
}

impl ColumnIndex for &str {
    fn index(self, columns: &[String]) -> Result<usize> {
        columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case(self))
            .ok_or_else(|| Error::Misuse(format!("unknown column {self}")))
    }
}

impl Row<'_> {
    pub fn get<I: ColumnIndex, T: turso_sdk_kit::rsapi::FromValue>(&self, index: I) -> Result<T> {
        self.row.get(index.index(self.columns)?)
    }
}
pub type PooledClient = Connection;

#[derive(Clone)]
pub struct Pool(Database);

type Pools = HashMap<(PathBuf, String), Pool>;
pub struct Connection(turso::Connection);

pub fn open_pool(
    path: &Path,
    prefix: &str,
    migrate: impl FnOnce(&Connection) -> std::result::Result<(), Box<dyn std::error::Error>>,
) -> std::result::Result<Pool, Box<dyn std::error::Error>> {
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty());
            let name = path
                .file_name()
                .ok_or("owner database requires a filename")?;
            parent
                .unwrap_or_else(|| Path::new("."))
                .canonicalize()?
                .join(name)
        }
        Err(error) => return Err(error.into()),
    };
    static POOLS: OnceLock<Mutex<Pools>> = OnceLock::new();
    let mut pools = POOLS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| "Turso pool lock poisoned")?;
    let key = (path.clone(), prefix.to_owned());
    if let Some(pool) = pools.get(&key) {
        return Ok(pool.clone());
    }
    let filename = path.to_str().ok_or("Owner database path must be UTF-8")?;
    let pool = Pool(block_on(Builder::new_local(filename).build())?);
    let mut conn = pool.get()?;
    let namespace = format!("{prefix}_");
    let tables =
        block_on(async {
            let mut rows = conn.0.query(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*'",
            (),
        ).await?;
            let mut tables = Vec::new();
            while let Some(row) = rows.next().await? {
                tables.push(row.get::<String>(0)?);
            }
            Ok::<_, Error>(tables)
        })?;
    if tables.iter().any(|table| !table.starts_with(&namespace)) {
        return Err("Native Turso WAL requires an owner-only database".into());
    }
    let mode: String = conn.query_row("PRAGMA journal_mode", (), |row| row.get(0))?;
    if mode != "wal" {
        return Err("Native Turso requires WAL mode".into());
    }
    let tx = write_transaction(&mut conn)?;
    migrate(&tx)?;
    tx.commit()?;
    pools.insert(key, pool.clone());
    Ok(pool)
}

impl Pool {
    pub fn get(&self) -> Result<Connection> {
        block_on(self.connect()).map(Connection)
    }

    /// Open a connection with the owner's WAL settings.
    ///
    /// # Errors
    /// Returns a database error if a connection or its settings cannot be opened.
    pub async fn connect(&self) -> Result<turso::Connection> {
        let conn = self.0.connect()?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update("foreign_keys", "ON").await?;
        Ok(conn)
    }
}

impl Connection {
    pub fn execute(&self, sql: &str, params: impl IntoParams) -> Result<usize> {
        let changed = block_on(self.0.execute(sql, params))?;
        usize::try_from(changed).map_err(|error| Error::ConversionFailure(error.to_string()))
    }

    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        block_on(self.0.execute_batch(sql))
    }

    pub fn prepare(&self, sql: &str) -> Result<Query> {
        block_on(self.0.prepare(sql)).map(Query)
    }

    pub fn query_all<T>(
        &self,
        sql: &str,
        params: impl IntoParams,
        decode: impl FnMut(&Row<'_>) -> Result<T>,
    ) -> Result<Vec<T>> {
        self.prepare(sql)?.query_map(params, decode)?.collect()
    }

    pub fn query_row<T>(
        &self,
        sql: &str,
        params: impl IntoParams,
        decode: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        self.prepare(sql)?.query_row(params, decode)
    }
}

pub struct Query(Statement);

impl Query {
    /// Execute the prepared statement with new parameters.
    ///
    /// # Errors
    /// Returns an error if the statement fails or returns rows.
    pub fn execute(&mut self, params: impl IntoParams) -> Result<usize> {
        let changed = block_on(self.0.execute(params))?;
        usize::try_from(changed).map_err(|error| Error::ConversionFailure(error.to_string()))
    }

    pub fn query_row<T>(
        &mut self,
        params: impl IntoParams,
        decode: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        let (row, columns) = block_on(async {
            let mut rows = self.0.query(params).await?;
            let columns = rows.column_names();
            let row = rows.next().await?.ok_or(Error::QueryReturnedNoRows)?;
            // Finish RETURNING statements so their writes commit before decoding.
            while rows.next().await?.is_some() {}
            Ok::<_, Error>((row, columns))
        })?;
        decode(&Row {
            row: &row,
            columns: &columns,
        })
    }

    /// Visit result rows while the cursor is open.
    ///
    /// # Errors
    /// Returns the first database or visitor error. The visitor must not query this connection.
    pub fn visit_rows(
        &mut self,
        params: impl IntoParams,
        mut visit: impl FnMut(&Row<'_>) -> Result<()>,
    ) -> Result<()> {
        block_on(async {
            let mut cursor = self.0.query(params).await?;
            let columns = cursor.column_names();
            while let Some(row) = cursor.next().await? {
                visit(&Row {
                    row: &row,
                    columns: &columns,
                })?;
            }
            Ok(())
        })
    }

    pub fn query_map<T>(
        &mut self,
        params: impl IntoParams,
        mut decode: impl FnMut(&Row<'_>) -> Result<T>,
    ) -> Result<impl Iterator<Item = Result<T>>> {
        // Row decoders may query this same connection, so finish the cursor first.
        let (rows, columns) = block_on(async {
            let mut cursor = self.0.query(params).await?;
            let columns = cursor.column_names();
            let mut rows = Vec::new();
            while let Some(row) = cursor.next().await? {
                rows.push(row);
            }
            Ok::<_, Error>((rows, columns))
        })?;
        Ok(rows
            .iter()
            .map(|row| {
                decode(&Row {
                    row,
                    columns: &columns,
                })
            })
            .collect::<Vec<_>>()
            .into_iter())
    }
}

pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Err(Error::QueryReturnedNoRows) => Ok(None),
            result => result.map(Some),
        }
    }
}

pub struct Transaction<'a> {
    connection: &'a Connection,
    committed: bool,
}

pub fn read_transaction(connection: &mut Connection) -> Result<Transaction<'_>> {
    begin(connection, "BEGIN")
}

pub fn write_transaction(connection: &mut Connection) -> Result<Transaction<'_>> {
    begin(connection, "BEGIN IMMEDIATE")
}

fn begin<'a>(connection: &'a mut Connection, sql: &str) -> Result<Transaction<'a>> {
    connection.execute(sql, ())?;
    Ok(Transaction {
        connection,
        committed: false,
    })
}

impl Deref for Transaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Transaction<'_> {
    pub fn commit(mut self) -> Result<()> {
        self.execute("COMMIT", ())?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.committed && matches!(self.connection.0.is_autocommit(), Ok(false)) {
            if let Err(error) = self.execute("ROLLBACK", ()) {
                eprintln!("native WAL rollback failed: {error}");
            }
        }
    }
}

pub fn snapshot(
    conn: &Connection,
    database_path: &Path,
    target: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let name = target.file_name().ok_or("snapshot requires a filename")?;
    let directory = target
        .parent()
        .ok_or("snapshot requires a parent directory")?;
    std::fs::create_dir_all(directory)?;
    let target = directory.canonicalize()?.join(name);
    let database_path = database_path.canonicalize()?;
    if target == database_path
        || target
            .canonicalize()
            .is_ok_and(|file| file == database_path)
    {
        return Err("Owner snapshot overlaps the owner database".into());
    }
    let stage = tempfile::tempdir_in(directory)?;
    let file = stage.path().join(name);
    let path = file.to_str().ok_or("Owner snapshot path must be UTF-8")?;
    let delimiter = if !path.contains('\'') {
        '\''
    } else if !path.contains('"') {
        '"'
    } else {
        return Err("Owner snapshot path contains both quote characters".into());
    };
    conn.execute(&format!("VACUUM INTO {delimiter}{path}{delimiter}"), ())?;
    if std::fs::metadata(&file)?.len() == 0
        || std::fs::metadata(stage.path().join(format!("{}-wal", name.to_string_lossy())))
            .is_ok_and(|wal| wal.len() != 0)
    {
        return Err("Turso did not produce a closed snapshot".into());
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&file, target)?;
    Ok(())
}
