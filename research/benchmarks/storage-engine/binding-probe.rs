//! Deterministic contention probe for two connections from one Turso database handle.

use std::path::Path;

use turso::{Builder, Error};

async fn probe(
    directory: &Path,
    journal: &str,
    begin: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = directory.join(format!("{journal}.db"));
    let db = Builder::new_local(path.to_str().ok_or("database path is not UTF-8")?)
        .build()
        .await?;
    let first = db.connect()?;
    first.pragma_update("journal_mode", journal).await?;
    let second = db.connect()?;
    first
        .execute("CREATE TABLE writes (id INTEGER PRIMARY KEY)", ())
        .await?;
    first.execute(begin, ()).await?;
    first.execute("INSERT INTO writes VALUES (1)", ()).await?;

    let second_result = match second.execute(begin, ()).await {
        Ok(_) => match second.execute("INSERT INTO writes VALUES (2)", ()).await {
            Ok(_) => Ok(()),
            Err(error) => {
                second.execute("ROLLBACK", ()).await?;
                Err(error)
            }
        },
        Err(error) => Err(error),
    };
    first.execute("COMMIT", ()).await?;
    match second_result {
        Ok(()) => {
            second.execute("COMMIT", ()).await?;
            println!("{journal}: independent writes overlapped and both committed");
        }
        Err(error @ (Error::Busy(_) | Error::BusySnapshot(_))) => {
            second.execute(begin, ()).await?;
            second.execute("INSERT INTO writes VALUES (2)", ()).await?;
            second.execute("COMMIT", ()).await?;
            println!("{journal}: second writer retried successfully after {error}");
        }
        Err(error) => return Err(error.into()),
    }

    let mut rows = first.query("SELECT id FROM writes ORDER BY id", ()).await?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await? {
        ids.push(row.get::<i64>(0)?);
    }
    if ids != [1, 2] {
        return Err(format!("{journal}: unexpected committed keys {ids:?}").into());
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("provide a new database directory")?;
    let directory = Path::new(&directory);
    std::fs::create_dir(directory)?;
    probe(directory, "wal", "BEGIN IMMEDIATE").await?;
    probe(directory, "mvcc", "BEGIN CONCURRENT").await?;
    Ok(())
}
