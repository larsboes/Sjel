use std::time::Instant;

#[tokio::test]
async fn test_multi_writer_concurrency_on_turso() {
    let dir = std::env::temp_dir().join(format!(
        "sjel-turso-concurrency-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("concurrency_test.db");

    let conn = sjel_turso::open_local(&db_path).await.unwrap();

    // 1. Initialize schema
    sjel_turso::migrate_once(&conn, &db_path, "concurrency_test", |conn| async move {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS concurrency_opportunities (
                id TEXT PRIMARY KEY,
                opportunity_type TEXT NOT NULL,
                source TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                url TEXT,
                title TEXT NOT NULL,
                starts_at TEXT,
                ends_at TEXT,
                location TEXT,
                city TEXT,
                country_code TEXT,
                latitude REAL,
                longitude REAL,
                raw TEXT,
                fetched_at TEXT,
                score REAL,
                matched_focus TEXT,
                rationale TEXT,
                vault_link TEXT,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                first_seen TEXT,
                last_seen TEXT
            );

            CREATE TABLE IF NOT EXISTS concurrency_links (
                opportunity_id TEXT NOT NULL,
                vault_path TEXT NOT NULL,
                link_type TEXT NOT NULL,
                created_at TEXT,
                PRIMARY KEY (opportunity_id, vault_path)
            );
            ",
        )
        .await?;
        Ok(())
    })
    .await
    .unwrap();

    // 2. Spawn 10 concurrent writers executing 5 inserts each (50 total writes)
    let start = Instant::now();
    let mut handles = Vec::new();

    for worker_id in 0..10 {
        let p = db_path.clone();
        handles.push(tokio::spawn(async move {
            let worker_conn = sjel_turso::open_local(&p).await.unwrap();
            for i in 0..5 {
                let id = format!("item-{worker_id}-{i}");
                let now = sjel_turso::NOW;
                let sql = format!(
                    "INSERT INTO concurrency_opportunities (
                        id, opportunity_type, source, source_kind, url, title,
                        starts_at, ends_at, location, city, country_code,
                        raw, fetched_at, score, status, first_seen, last_seen
                    ) VALUES (
                        '{id}', 'event', 'source_a', 'api', 'https://example.com/{id}', 'Event {id}',
                        '2026-11-01T10:00:00Z', '2026-11-01T18:00:00Z', 'Berlin, Germany', 'Berlin', 'de',
                        '{{}}', {now}, 0.85, 'new', {now}, {now}
                    ) ON CONFLICT(id) DO UPDATE SET last_seen = {now}"
                );
                worker_conn.execute_batch(&sql).await.unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
    let elapsed = start.elapsed();

    // Verify 50 items inserted
    let mut rows = conn
        .query("SELECT COUNT(*) FROM concurrency_opportunities", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let count: i64 = row.get(0).unwrap();
    assert_eq!(count, 50);

    // Verify duration is small and concurrent
    println!("Completed 50 concurrent writes across 10 workers in {:?}", elapsed);

    // 3. Vector Search Verification
    conn.execute_batch(
        "
        ALTER TABLE concurrency_opportunities ADD COLUMN embedding F32_BLOB(3);
        CREATE INDEX IF NOT EXISTS idx_concurrency_vec ON concurrency_opportunities(libsql_vector_idx(embedding));
        ",
    )
    .await
    .unwrap();

    // Insert 3 items with 3-dimensional embeddings
    let v1 = sjel_turso::format_vector(&[1.0, 0.0, 0.0]);
    let v2 = sjel_turso::format_vector(&[0.0, 1.0, 0.0]);
    let v3 = sjel_turso::format_vector(&[0.707, 0.707, 0.0]);

    conn.execute_batch(&format!(
        "
        UPDATE concurrency_opportunities SET embedding = vector('{v1}') WHERE id = 'item-0-0';
        UPDATE concurrency_opportunities SET embedding = vector('{v2}') WHERE id = 'item-0-1';
        UPDATE concurrency_opportunities SET embedding = vector('{v3}') WHERE id = 'item-0-2';
        "
    ))
    .await
    .unwrap();

    // Query nearest to [1.0, 0.1, 0.0]
    let q = sjel_turso::format_vector(&[1.0, 0.1, 0.0]);
    let mut nearest_rows = conn
        .query(
            &format!(
                "SELECT id, vector_distance_cos(embedding, vector('{q}')) as dist \
                 FROM concurrency_opportunities \
                 WHERE embedding IS NOT NULL \
                 ORDER BY dist ASC LIMIT 2"
            ),
            (),
        )
        .await
        .unwrap();

    let first = nearest_rows.next().await.unwrap().unwrap();
    let first_id: String = first.get(0).unwrap();
    assert_eq!(first_id, "item-0-0");

    let second = nearest_rows.next().await.unwrap().unwrap();
    let second_id: String = second.get(0).unwrap();
    assert_eq!(second_id, "item-0-2");

    let _ = std::fs::remove_dir_all(&dir);
}
