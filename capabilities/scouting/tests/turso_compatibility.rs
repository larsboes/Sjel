use tokio::time::Instant;

#[tokio::test]
async fn scouting_schema_and_concurrent_writes_on_turso() {
    let dir = std::env::temp_dir().join(format!(
        "sjel-scouting-turso-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("scouting_turso.db");

    let conn = sjel_turso::open_local(&db_path).await.unwrap();

    // 1. Initialize schema
    sjel_turso::migrate_once(&conn, &db_path, "scouting", |conn| async move {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS scouting_opportunities (
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

            CREATE TABLE IF NOT EXISTS scouting_links (
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

    // 2. Multi-writer concurrent write test
    // Spawn 10 async tasks each writing 5 opportunities concurrently.
    let start = Instant::now();
    let mut handles = Vec::new();
    let db = sjel_turso::database_for(&db_path).await.unwrap();

    for task_id in 0..10 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            let conn = db.connect()?;
            for i in 0..5 {
                let id = format!("opp-{task_id}-{i}");
                let title = format!("AI Research Summit {task_id} #{i}");
                let score = 0.5 + (i as f64) * 0.1;
                conn.execute(
                    "INSERT INTO scouting_opportunities (
                        id, opportunity_type, source, source_kind, title, score, status, fetched_at
                    ) VALUES (?1, 'conference', 'luma', 'adapter', ?2, ?3, 'new', strftime('%Y-%m-%d %H:%M:%f+00:00','now'))",
                    [id.as_str(), title.as_str(), &score.to_string()],
                )
                .await?;
            }
            Ok::<(), sjel_turso::libsql::Error>(())
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }
    println!("50 concurrent inserts completed in {:?}", start.elapsed());

    // 3. Query total count
    let mut rows = conn
        .query("SELECT count(*) FROM scouting_opportunities", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("count row");
    let count: i64 = row.get(0).unwrap();
    assert_eq!(count, 50, "all 50 concurrent inserts must be recorded");

    // 4. Query top ranked opportunities
    let mut top_rows = conn
        .query(
            "SELECT id, title, score FROM scouting_opportunities WHERE score >= 0.8 ORDER BY score DESC",
            (),
        )
        .await
        .unwrap();

    let mut ranked_count = 0;
    while let Some(r) = top_rows.next().await.unwrap() {
        let id: String = r.get(0).unwrap();
        let title: String = r.get(1).unwrap();
        let score: f64 = r.get(2).unwrap();
        assert!(score >= 0.8);
        assert!(id.starts_with("opp-"));
        assert!(title.contains("AI Research Summit"));
        ranked_count += 1;
    }
    assert_eq!(ranked_count, 20, "2 items per task with score >= 0.8");
}
