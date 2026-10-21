//! The public surface of the `replica` feature, exercised without a live primary.
//!
//! What these tests cannot reach is the pull itself: LibSQL's sync client needs an HTTP primary,
//! and this repository has no server to point it at. They cover the decisions this crate makes —
//! refuse a misconfiguration before touching the network, refuse to open one file as both a local
//! database and a replica, and refuse to call a local store a replica.
#![cfg(feature = "replica")]

use std::path::PathBuf;
use std::time::Duration;

use sjel_turso::{ReplicaConfig, Source, Store};

fn temp_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sjel-turso-replica-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}.db"));
    let _ = std::fs::remove_file(&path);
    path
}

async fn create_items(conn: sjel_turso::libsql::Connection) -> sjel_turso::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS replica_items (id INTEGER PRIMARY KEY, body TEXT)",
        (),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn a_url_without_a_scheme_fails_before_any_network_call() {
    let path = temp_db("bad_scheme");
    let err = tokio::time::timeout(
        Duration::from_secs(10),
        Store::open_replica(
            &path,
            "replica",
            ReplicaConfig::new("example.turso.io"),
            create_items,
        ),
    )
    .await
    .expect("a bad scheme must be refused without waiting on a network")
    .expect_err("a url without a scheme is not a primary");

    assert!(
        err.to_string().contains("names no remote primary"),
        "unexpected message: {err}"
    );
}

#[tokio::test]
async fn an_unreachable_primary_is_an_error_not_a_local_fallback() {
    let path = temp_db("unreachable");
    // Port 1 is never a LibSQL server, so the probe fails as a refused connection.
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        Store::open_replica(
            &path,
            "replica",
            ReplicaConfig::new("http://127.0.0.1:1"),
            create_items,
        ),
    )
    .await
    .expect("opening a replica of an unreachable primary must not hang");

    let err = outcome.expect_err("an unreachable primary cannot serve a replica");
    assert!(
        err.to_string().contains("could not open the replica"),
        "unexpected message: {err}"
    );
}

#[tokio::test]
async fn a_local_store_reports_itself_as_local() {
    let path = temp_db("local");
    let store = Store::open_with(&path, "replica", Source::Local, create_items)
        .await
        .unwrap();

    assert!(!store.is_replica());
    assert_eq!(store.sync_stats(), None);
    let err = store
        .sync_now()
        .await
        .expect_err("a local store has nothing to pull");
    assert!(err.to_string().contains("this store is local"), "{err}");
}

#[tokio::test]
async fn one_file_cannot_be_a_local_database_and_a_replica() {
    let path = temp_db("both");
    let local = Store::open(&path, "replica", create_items).await.unwrap();

    let err = Store::open_replica(
        &path,
        "replica",
        ReplicaConfig::new("http://127.0.0.1:1"),
        create_items,
    )
    .await
    .expect_err("the file is already the local database in this process");
    assert!(
        err.to_string()
            .contains("already open as the shared local database"),
        "unexpected message: {err}"
    );

    // The refusal left the local store untouched and still usable.
    let conn = local.connect().await.unwrap();
    let mut rows = conn
        .query("SELECT COUNT(*) FROM replica_items", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("count row");
    let count: i64 = row.get(0).unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn the_reverse_order_is_refused_too() {
    let path = temp_db("reverse");
    Store::open_replica(
        &path,
        "replica",
        ReplicaConfig::new("http://127.0.0.1:1"),
        create_items,
    )
    .await
    .expect_err("an unreachable primary cannot open");

    // The failed open must leave no registry entry behind: the same path still opens locally.
    let store = Store::open(&path, "replica", create_items).await.unwrap();
    assert!(!store.is_replica());
}
