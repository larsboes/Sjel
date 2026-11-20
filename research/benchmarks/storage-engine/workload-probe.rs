//! Compare graph reads and concurrent updates on disposable database copies.

use std::{
    path::Path,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const NODES: i64 = 37_200;
const READS: i64 = 120;
const WORKERS: i64 = 4;
const WRITES: i64 = 200;
const VECTORS: i64 = 25_000;
const DIMENSIONS: usize = 384;
const VECTOR_READS: i64 = 24;
const CALLERS: &str = "WITH RECURSIVE r(n, d) AS (
    SELECT ?1, 0 UNION SELECT e.src, r.d + 1 FROM edges e JOIN r ON e.dst = r.n
    WHERE e.rel = 'calls' AND r.d < 3
) SELECT DISTINCT n FROM r WHERE n <> ?1 ORDER BY n";
const NEAREST: &str = "SELECT id FROM embeddings
    ORDER BY vector_distance_cos(embedding, (SELECT embedding FROM embeddings WHERE id = ?1)), id
    LIMIT 10";

#[derive(Clone)]
enum Database {
    Libsql(std::sync::Arc<libsql::Database>),
    Turso(turso::Database),
}

enum Connection {
    Libsql(libsql::Connection),
    Turso(turso::Connection),
}

impl Database {
    fn connect(&self) -> Result<Connection> {
        Ok(match self {
            Self::Libsql(db) => Connection::Libsql(db.connect()?),
            Self::Turso(db) => Connection::Turso(db.connect()?),
        })
    }
}

impl Connection {
    async fn sql(&self, sql: &str) -> Result<()> {
        match self {
            Self::Libsql(conn) => {
                conn.execute_batch(sql).await?;
            }
            Self::Turso(conn) => {
                conn.execute_batch(sql).await?;
            }
        }
        Ok(())
    }

    async fn values(&self, sql: &str, key: i64) -> Result<Vec<i64>> {
        let mut values = Vec::new();
        match self {
            Self::Libsql(conn) => {
                let mut rows = conn.query(sql, [key]).await?;
                while let Some(row) = rows.next().await? {
                    values.push(row.get(0)?);
                }
            }
            Self::Turso(conn) => {
                let mut rows = conn.query(sql, [key]).await?;
                while let Some(row) = rows.next().await? {
                    values.push(row.get(0)?);
                }
            }
        }
        Ok(values)
    }

    async fn increment(&self, key: i64, begin: &str) -> Result<()> {
        const UPDATE: &str = "INSERT INTO counters VALUES (?1, 1)
            ON CONFLICT(id) DO UPDATE SET n = n + 1";
        self.sql(begin).await?;
        let update = match self {
            Self::Libsql(conn) => conn
                .execute(UPDATE, [key])
                .await
                .map(|_| ())
                .map_err(|e| Box::new(e) as _),
            Self::Turso(conn) => conn
                .execute(UPDATE, [key])
                .await
                .map(|_| ())
                .map_err(|e| Box::new(e) as _),
        };
        let result = match update {
            Ok(()) => self.sql("COMMIT").await,
            Err(error) => Err(error),
        };
        if result.is_err() {
            let autocommit = match self {
                Self::Libsql(conn) => conn.is_autocommit(),
                Self::Turso(conn) => conn.is_autocommit()?,
            };
            if !autocommit {
                self.sql("ROLLBACK").await?;
            }
        }
        result
    }
}

fn retryable(error: &(dyn std::error::Error + 'static)) -> bool {
    matches!(error.downcast_ref::<libsql::Error>(), Some(libsql::Error::SqliteFailure(code, _)) if code & 0xff == 5)
        || matches!(
            error.downcast_ref::<turso::Error>(),
            Some(turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
        )
        || matches!(error.downcast_ref::<turso::Error>(), Some(turso::Error::Error(message)) if message == "Write-write conflict")
}

async fn seed(path: &Path) -> Result<()> {
    let db = libsql::Builder::new_local(path).build().await?;
    let conn = db.connect()?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
        CREATE TABLE nodes (id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE);
        CREATE TABLE edges (src INTEGER NOT NULL, dst INTEGER NOT NULL, rel TEXT NOT NULL);
        CREATE TABLE counters (id INTEGER PRIMARY KEY, n INTEGER NOT NULL);
        WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM n WHERE i<37199)
            INSERT INTO nodes SELECT i, printf('node-%d', i) FROM n;
        WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM n WHERE i<84231)
            INSERT INTO edges SELECT i%37200, (i*173+97+i/37200)%37200,
                CASE WHEN i<22261 THEN 'calls' ELSE 'related' END FROM n;
        CREATE INDEX edges_dst_rel ON edges (dst, rel);
        CREATE INDEX edges_src ON edges (src);
        CREATE TABLE embeddings (id INTEGER PRIMARY KEY, embedding F32_BLOB(384));
        BEGIN;",
    )
    .await?;
    let insert = conn
        .prepare("INSERT INTO embeddings VALUES (?1, ?2)")
        .await?;
    let mut random = 7_u32;
    for id in 0..VECTORS {
        let mut embedding = Vec::with_capacity(DIMENSIONS * std::mem::size_of::<f32>());
        for _ in 0..DIMENSIONS {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = ((random >> 24) as i32 - 128) as f32;
            embedding.extend_from_slice(&value.to_le_bytes());
        }
        insert.execute(libsql::params![id, embedding]).await?;
        insert.reset();
    }
    drop(insert);
    conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE);")
        .await?;
    Ok(())
}

fn p95(mut samples: Vec<Duration>) -> f64 {
    samples.sort_unstable();
    samples[(samples.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1_000.0
}

async fn run(
    db: Database,
    begin: &'static str,
    expected: Option<&[Vec<i64>]>,
) -> Result<Vec<Vec<i64>>> {
    let conn = db.connect()?;
    let mut answers = Vec::new();
    let mut reads = Vec::new();
    for sample in 0..READS {
        let start = Instant::now();
        let rows = conn.values(CALLERS, (sample * 233 + 97) % NODES).await?;
        reads.push(start.elapsed());
        answers.push(rows);
    }
    println!(
        "  graph read p95 {:.3} ms ({READS} queries, {} rows)",
        p95(reads),
        answers.iter().map(Vec::len).sum::<usize>()
    );
    let mut reads = Vec::new();
    for sample in 0..VECTOR_READS {
        let key = (sample * 937) % VECTORS;
        let start = Instant::now();
        let rows = conn.values(NEAREST, key).await?;
        reads.push(start.elapsed());
        if rows.len() != 10 || rows.first() != Some(&key) {
            return Err("exact cosine search did not return the query vector first".into());
        }
        answers.push(rows);
    }
    if expected.is_some_and(|expected| answers != expected) {
        return Err("graph or vector query results differ from libSQL".into());
    }
    println!(
        "  exact cosine top 10 p95 {:.3} ms ({VECTOR_READS} queries)",
        p95(reads)
    );

    for hot in [false, true] {
        conn.sql("DELETE FROM counters").await?;
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(WORKERS as usize));
        let started = Instant::now();
        let mut workers = tokio::task::JoinSet::new();
        for worker in 0..WORKERS {
            let conn = db.connect()?;
            conn.sql("PRAGMA busy_timeout = 0").await?;
            let barrier = barrier.clone();
            workers.spawn(async move {
                barrier.wait().await;
                let mut timings = Vec::new();
                let mut retries = 0;
                for write in 0..WRITES {
                    let started = Instant::now();
                    let key = if hot { 0 } else { worker * WRITES + write };
                    for attempt in 0..200 {
                        match conn.increment(key, begin).await {
                            Ok(()) => break,
                            Err(error) if attempt < 199 && retryable(&*error) => {
                                retries += 1;
                                tokio::time::sleep(Duration::from_millis(1)).await;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    timings.push(started.elapsed());
                }
                Ok((timings, retries))
            });
        }
        let mut timings = Vec::new();
        let mut retries = 0;
        while let Some(worker) = workers.join_next().await {
            let (samples, count) = worker??;
            timings.extend(samples);
            retries += count;
        }
        let elapsed = started.elapsed().as_secs_f64();
        let counts = conn
            .values("SELECT n FROM counters WHERE id >= ?1 ORDER BY id", 0)
            .await?;
        let expected = if hot {
            vec![WORKERS * WRITES]
        } else {
            vec![1; (WORKERS * WRITES) as usize]
        };
        if counts != expected {
            return Err("committed counters differ from expected writes".into());
        }
        let keys = if hot { "same key" } else { "distinct keys" };
        println!(
            "  {keys}: p95 {:.3} ms, {:.0} tx/s, {retries} retries",
            p95(timings),
            (WORKERS * WRITES) as f64 / elapsed
        );
    }
    Ok(answers)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("provide a new fixture directory")?;
    let path = Path::new(&path);
    std::fs::create_dir(path)?;
    let fixture = path.join("fixture.db");
    seed(&fixture).await?;
    if fixture
        .with_extension("db-wal")
        .metadata()
        .is_ok_and(|m| m.len() > 0)
    {
        return Err("fixture still has WAL frames".into());
    }
    let mut baseline = None;
    for name in ["libsql-wal", "turso-wal", "turso-mvcc"] {
        println!("{name}");
        let copy = path.join(format!("{name}.db"));
        std::fs::copy(&fixture, &copy)?;
        let db = if name == "libsql-wal" {
            Database::Libsql(std::sync::Arc::new(
                libsql::Builder::new_local(&copy).build().await?,
            ))
        } else {
            let db = turso::Builder::new_local(copy.to_str().ok_or("invalid fixture path")?)
                .build()
                .await?;
            if name == "turso-mvcc" {
                db.connect()?.pragma_update("journal_mode", "mvcc").await?;
            }
            Database::Turso(db)
        };
        let begin = if name == "turso-mvcc" {
            "BEGIN CONCURRENT"
        } else {
            "BEGIN IMMEDIATE"
        };
        let answers = run(db, begin, baseline.as_deref()).await?;
        if name != "libsql-wal" {
            let compatible = if name == "turso-mvcc" {
                let converted = path.join("turso-mvcc-converted.db");
                for extension in ["db", "db-log", "db-wal"] {
                    let source = copy.with_extension(extension);
                    if source.try_exists()? {
                        std::fs::copy(source, converted.with_extension(extension))?;
                    }
                }
                let db = turso::Builder::new_local(converted.to_str().ok_or("invalid copy path")?)
                    .build()
                    .await?;
                db.connect()?.pragma_update("journal_mode", "wal").await?;
                converted
            } else {
                copy
            };
            let db = libsql::Builder::new_local(compatible).build().await?;
            let conn = Connection::Libsql(db.connect()?);
            let counts = conn
                .values("SELECT n FROM counters WHERE id >= ?1", 0)
                .await?;
            if counts != [WORKERS * WRITES]
                || conn.values(NEAREST, 0).await? != answers[READS as usize]
            {
                return Err("libSQL could not read back Turso WAL results".into());
            }
            println!("  libSQL reopened the WAL file and matched its data");
        }
        if baseline.is_none() {
            baseline = Some(answers);
        }
    }
    Ok(())
}
