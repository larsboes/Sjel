//! The pull path, against a real primary.
//!
//! `replica.rs` covers everything this crate decides before it touches the network. What it could
//! not cover is the pull itself: LibSQL's sync client needs an HTTP primary, and until 2026-10-21
//! this repository had none to point at, so "the pull path is unverified" was the honest note in
//! `PLAN.md`.
//!
//! These tests read the deployment's own settings — `SJEL_TURSO_URL`, and
//! `SJEL_TURSO_AUTH_TOKEN` when the primary wants one, through the same `ReplicaConfig::from_env`
//! the capabilities use. Unset, each test prints why it skipped and returns: a machine with no
//! primary must not fail a suite it cannot satisfy. Set, they are the proof that a replica pulls
//! what the primary holds, that a write through one replica reaches another, and that the stats a
//! freshness surface publishes move.
//!
//! Run them against a scratch server like this:
//!
//! ```console
//! $ sqld --db-path /tmp/probe.db --http-listen-addr 127.0.0.1:8880
//! $ SJEL_TURSO_URL=http://127.0.0.1:8880 cargo test -p sjel-turso --test live_primary
//! ```
#![cfg(feature = "replica")]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sjel_turso::libsql::Value;
use sjel_turso::{ReplicaConfig, Store};

const TABLE: &str = "live_primary_probe";

/// The deployment's replica settings, or `None` when no primary is named.
fn primary() -> Option<ReplicaConfig> {
    match ReplicaConfig::from_env() {
        Ok(Some(config)) => Some(config),
        Ok(None) => {
            eprintln!("skipped: SJEL_TURSO_URL names no primary");
            None
        }
        Err(error) => panic!("the deployment's replica settings are unusable: {error}"),
    }
}

/// A unit of time no other run of this test shares, so two runs against a real primary cannot
/// read each other's rows.
fn stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("live-probe-{}-{nanos}", std::process::id())
}

fn temp_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sjel-turso-live-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}.db"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(dir.join(format!("{tag}.db-shm")));
    let _ = std::fs::remove_file(dir.join(format!("{tag}.db-wal")));
    path
}

async fn create_table(conn: sjel_turso::libsql::Connection) -> sjel_turso::Result<()> {
    let sql = format!("CREATE TABLE IF NOT EXISTS {TABLE} (id TEXT PRIMARY KEY, body TEXT)");
    conn.execute(&sql, ()).await?;
    Ok(())
}

async fn read_bodies(store: &Store, id: &str) -> Vec<String> {
    let sql = format!("SELECT body FROM {TABLE} WHERE id = ?1");
    let conn = store.connect().await.unwrap();
    let mut rows = conn
        .query(&sql, vec![Value::Text(id.to_owned())])
        .await
        .unwrap();
    let mut bodies = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        bodies.push(row.get::<String>(0).unwrap());
    }
    bodies
}

/// The whole path in one test, because each step is what makes the next meaningful: a write
/// forwarded to the primary, a pull that brings it back, and a second replica that had never seen
/// the row finding it after its own first pull.
#[tokio::test(flavor = "multi_thread")]
async fn a_replica_pulls_the_primaries_rows_and_forwards_its_own_writes() {
    let Some(config) = primary() else {
        return;
    };
    let id = stamp();

    // 1. Write through the first replica. LibSQL forwards it to the primary; nothing is in the
    //    local file until a pull brings it back, which is why this is not a local write.
    let first_path = temp_db("writer");
    let first = tokio::time::timeout(
        Duration::from_secs(120),
        Store::open_replica(&first_path, TABLE, config.clone(), create_table),
    )
    .await
    .expect("opening a replica of a live primary must not hang")
    .expect("the primary is reachable and its credentials work");
    assert!(first.is_replica());

    let conn = first.connect().await.unwrap();
    let insert = format!("INSERT INTO {TABLE} (id, body) VALUES (?1, ?2)");
    conn.execute(&insert, (id.as_str(), "written through the first replica"))
        .await
        .expect("a forwarded write is accepted by the primary");

    // 2. Pull it back, so the local file demonstrably holds what the primary holds.
    first
        .sync_now()
        .await
        .expect("the replica pulls from its primary");
    assert_eq!(
        read_bodies(&first, &id).await,
        vec!["written through the first replica".to_owned()]
    );

    // 3. A second replica, opened after the write, must find the row from its first pull — the
    //    pull that runs before the DDL, which is what makes a migration decide against the
    //    primary's current schema instead of an empty file.
    let second_path = temp_db("reader");
    let second = Store::open_replica(&second_path, TABLE, config, create_table)
        .await
        .expect("a second replica of the same primary opens");
    assert_eq!(
        read_bodies(&second, &id).await,
        vec!["written through the first replica".to_owned()],
        "the second replica did not pull the first replica's write"
    );

    // 4. The stats a freshness surface publishes are real: at least one successful sync and no
    //    failures on this streak.
    let stats = second.sync_stats().expect("a replica reports sync stats");
    assert!(stats.syncs >= 1, "no sync recorded: {stats:?}");
    assert_eq!(
        stats.failures, 0,
        "a healthy pull recorded a failure: {stats:?}"
    );

    // 5. Leave the primary as it was found. The table stays: dropping it would race another run,
    //    and an empty probe table costs nothing.
    let delete = format!("DELETE FROM {TABLE} WHERE id = ?1");
    let conn = first.connect().await.unwrap();
    let _ = conn
        .execute(&delete, vec![Value::Text(id.to_owned())])
        .await;
}
