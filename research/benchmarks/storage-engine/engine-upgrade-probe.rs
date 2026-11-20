//! Opens an existing owner file with the Turso version under test, commits one probe write,
//! proves a rolled-back write leaves nothing, and removes the probe table again.

use turso::Builder;

async fn count(conn: &turso::Connection) -> Result<i64, Box<dyn std::error::Error>> {
    let mut rows = conn.query("SELECT count(*) FROM sjel_engine_upgrade_probe", ()).await?;
    let row = rows.next().await?.ok_or("count returned no row")?;
    Ok(*row.get_value(0)?.as_integer().ok_or("count is not an integer")?)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: engine-upgrade-probe <owner.db>")?;
    let db = Builder::new_local(&path).build().await?;
    let conn = db.connect()?;
    conn.execute("CREATE TABLE sjel_engine_upgrade_probe (id INTEGER PRIMARY KEY, note TEXT)", ())
        .await?;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    conn.execute("INSERT INTO sjel_engine_upgrade_probe (note) VALUES ('committed')", ())
        .await?;
    conn.execute("COMMIT", ()).await?;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    conn.execute("INSERT INTO sjel_engine_upgrade_probe (note) VALUES ('rolled back')", ())
        .await?;
    conn.execute("ROLLBACK", ()).await?;
    let rows = count(&conn).await?;
    if rows != 1 {
        return Err(format!("expected one committed probe row, found {rows}").into());
    }
    conn.execute("DROP TABLE sjel_engine_upgrade_probe", ()).await?;
    let mut checkpoint = conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ()).await?;
    while checkpoint.next().await?.is_some() {}
    println!("committed one probe row, rolled back one, removed the probe table");
    Ok(())
}
