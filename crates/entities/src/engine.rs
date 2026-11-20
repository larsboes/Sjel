#[cfg(not(feature = "turso-wal"))]
pub use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Result, Row};
#[cfg(not(feature = "turso-wal"))]
pub use sjel_store::{open_pool, write_transaction, Pool, PooledClient};

#[cfg(not(feature = "turso-wal"))]
pub fn read_transaction(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>> {
    conn.transaction()
}

#[cfg(feature = "turso-wal")]
pub use sjel_native_wal::*;
