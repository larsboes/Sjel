//! Create disposable libSQL databases with and without a native vector index.

use std::path::PathBuf;

const FIXTURE: &str = "
    CREATE TABLE nodes (
        id INTEGER PRIMARY KEY,
        parent INTEGER REFERENCES nodes(id),
        tags TEXT NOT NULL,
        embedding F32_BLOB(3) NOT NULL
    );
    INSERT INTO nodes VALUES
        (1, NULL, '[\"root\"]', vector('[1,0,0]')),
        (2, 1, '[\"child\",\"test\"]', vector('[0,1,0]')),
        (3, 2, '[\"leaf\"]', vector('[0,0,1]'));
";

const QUERY: &str = "
    WITH RECURSIVE descendants(id) AS (
        SELECT 1 UNION ALL
        SELECT nodes.id FROM nodes JOIN descendants ON nodes.parent = descendants.id
    )
    SELECT (SELECT count(*) FROM descendants),
           (SELECT count(*) FROM nodes, json_each(tags)),
           (SELECT id FROM nodes ORDER BY vector_distance_cos(embedding, vector('[0.95,0.1,0]')) LIMIT 1);
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("provide a new fixture directory")?;
    std::fs::create_dir(&directory)?;
    std::fs::write(directory.join("query.sql"), QUERY)?;
    for (name, indexed, wal) in [
        ("plain", false, false),
        ("indexed", true, false),
        ("plain-wal", false, true),
        ("indexed-wal", true, true),
    ] {
        let conn = sjel_turso::open_local(directory.join(format!("{name}.db"))).await?;
        if wal {
            conn.execute_batch("PRAGMA journal_mode = WAL").await?;
        }
        conn.execute_batch(FIXTURE).await?;
        if indexed {
            conn.execute(
                "CREATE INDEX nodes_vector_idx ON nodes(libsql_vector_idx(embedding))",
                (),
            )
            .await?;
        }
        let mut rows = conn.query(QUERY, ()).await?;
        let row = rows
            .next()
            .await?
            .ok_or("compatibility query returned no result")?;
        let result: (i64, i64, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
        if result != (3, 4, 1) {
            return Err(format!("{name}: unexpected libSQL result {result:?}").into());
        }
        println!("{name}: {result:?}");
    }
    Ok(())
}
