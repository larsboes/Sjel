#[cfg(not(feature = "turso-wal"))]
pub use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Result, Row};
#[cfg(not(feature = "turso-wal"))]
pub use sjel_store::{open_pool, write_transaction, Pool, PooledClient};

#[cfg(not(feature = "turso-wal"))]
pub fn read_transaction(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>> {
    conn.transaction()
}

#[cfg(feature = "turso-wal")]
pub use wal::*;

#[cfg(feature = "turso-wal")]
mod wal {
    use futures_executor::block_on;
    use std::{ops::Deref, path::Path, time::Duration};
    use turso::{Builder, Database, Error, IntoParams, Statement};

    pub use turso::{params, params_from_iter, Result};
    pub type Row<'row> = turso::Row;
    pub type PooledClient = Connection;

    pub struct Pool(Database);
    pub struct Connection(turso::Connection);

    pub fn open_pool(
        path: &Path,
        prefix: &str,
        migrate: impl FnOnce(&Connection) -> std::result::Result<(), Box<dyn std::error::Error>>,
    ) -> std::result::Result<Pool, Box<dyn std::error::Error>> {
        let path = path
            .to_str()
            .ok_or("Entities database path must be UTF-8")?;
        let pool = Pool(block_on(Builder::new_local(path).build())?);
        let mut conn = pool.get()?;
        let namespace = format!("{prefix}_");
        let tables = block_on(async {
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
            return Err("Entities Turso WAL requires an owner-only database".into());
        }
        let mode: String = conn.query_row("PRAGMA journal_mode", (), |row| row.get(0))?;
        if mode != "wal" {
            return Err("Entities Turso requires WAL mode".into());
        }
        let tx = write_transaction(&mut conn)?;
        migrate(&tx)?;
        tx.commit()?;
        Ok(pool)
    }

    impl Pool {
        pub fn get(&self) -> Result<Connection> {
            let conn = self.0.connect()?;
            conn.busy_timeout(Duration::from_secs(5))?;
            block_on(conn.pragma_update("foreign_keys", "ON"))?;
            Ok(Connection(conn))
        }
    }

    impl Connection {
        pub fn execute(&self, sql: &str, params: impl IntoParams) -> Result<u64> {
            block_on(self.0.execute(sql, params))
        }

        pub fn execute_batch(&self, sql: &str) -> Result<()> {
            block_on(self.0.execute_batch(sql))
        }

        pub fn prepare(&self, sql: &str) -> Result<Query> {
            block_on(self.0.prepare(sql)).map(Query)
        }

        pub fn query_row<T>(
            &self,
            sql: &str,
            params: impl IntoParams,
            decode: impl FnOnce(&Row<'_>) -> Result<T>,
        ) -> Result<T> {
            let row = block_on(async {
                let mut rows = self.0.query(sql, params).await?;
                rows.next().await?.ok_or(Error::QueryReturnedNoRows)
            })?;
            decode(&row)
        }
    }

    pub struct Query(Statement);

    impl Query {
        pub fn query_map<T>(
            &mut self,
            params: impl IntoParams,
            decode: impl FnMut(&Row<'_>) -> Result<T>,
        ) -> Result<impl Iterator<Item = Result<T>>> {
            // Row decoders may query this same connection, so finish the cursor first.
            let rows = block_on(async {
                let mut cursor = self.0.query(params).await?;
                let mut rows = Vec::new();
                while let Some(row) = cursor.next().await? {
                    rows.push(row);
                }
                Ok::<_, Error>(rows)
            })?;
            Ok(rows.iter().map(decode).collect::<Vec<_>>().into_iter())
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
                    eprintln!("entities: rollback failed: {error}");
                }
            }
        }
    }
}
