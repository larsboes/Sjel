//! Exercise Turso's native Rust transaction against a restored Inventory file.

use turso::transaction::TransactionBehavior;
use turso::{Builder, Connection, params};

const ITEM: &str = "sjel-turso-rehearsal-item";
const OPERATION: &str = "sjel-turso-native-op";
const BACKUP_OVERLAP: &str = "sjel-turso-backup-overlap";

async fn read_revisions(conn: &Connection) -> turso::Result<(i64, Option<i64>)> {
    let row = conn
        .prepare(
            "SELECT item.revision, receipt.revision FROM inventory_item item
             LEFT JOIN inventory_sync_operation receipt ON receipt.operation_id = ?2
             WHERE item.id = ?1",
        )
        .await?
        .query_row(params![ITEM, OPERATION])
        .await?;
    Ok((row.get(0)?, row.get(1)?))
}

async fn apply(conn: &mut Connection) -> turso::Result<i64> {
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    match stage_edit(&transaction).await {
        Ok(revision) => {
            transaction.commit().await?;
            Ok(revision)
        }
        Err(error) => {
            // A prepared read can run before Turso processes a dropped transaction.
            transaction.rollback().await?;
            Err(error)
        }
    }
}

async fn stage_edit(conn: &Connection) -> turso::Result<i64> {
    let (current, receipt) = read_revisions(conn).await?;
    if let Some(revision) = receipt {
        return Ok(revision);
    }
    let row = conn
        .prepare(
            "UPDATE inventory_item SET label = 'Native Rust edit', revision = revision + 1
             WHERE id = ?1 AND revision = ?2 RETURNING revision",
        )
        .await?
        .query_row(params![ITEM, current])
        .await?;
    let revision: i64 = row.get(0)?;
    conn.execute(
        "INSERT INTO inventory_sync_operation(operation_id, revision, processed_at)
             VALUES (?1, ?2, datetime('now'))",
        params![OPERATION, revision],
    )
    .await?;
    Ok(revision)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("provide an Inventory database copy")?;
    let database = Builder::new_local(&path).build().await?;
    let mut writer = database.connect()?;
    let reader = database.connect()?;
    writer.pragma_update("journal_mode", "wal").await?;
    assert_eq!(read_revisions(&reader).await?, (3, None));

    writer
        .execute_batch(
            "CREATE TEMP TRIGGER refuse_native_receipt BEFORE INSERT ON inventory_sync_operation
             BEGIN SELECT RAISE(ABORT, 'native receipt refused'); END;",
        )
        .await?;
    let error = apply(&mut writer)
        .await
        .expect_err("receipt insert should fail");
    assert!(
        error.to_string().contains("native receipt refused"),
        "{error}"
    );
    assert_eq!(read_revisions(&writer).await?, (3, None));
    assert_eq!(read_revisions(&reader).await?, (3, None));
    writer
        .execute("DROP TRIGGER refuse_native_receipt", ())
        .await?;

    assert_eq!(apply(&mut writer).await?, 4);
    assert_eq!(apply(&mut writer).await?, 4);
    assert_eq!(read_revisions(&reader).await?, (4, Some(4)));
    drop(reader);
    drop(writer);
    drop(database);

    let reopened = Builder::new_local(&path).build().await?;
    let reader = reopened.connect()?;
    assert_eq!(read_revisions(&reader).await?, (4, Some(4)));
    println!(
        "Native Rust WAL rolled back a refused receipt, replayed once, and reopened at revision 4."
    );

    let mut writer = reopened.connect()?;
    let transaction = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    assert_eq!(
        transaction
            .execute(
                "UPDATE inventory_item SET revision = revision + 1 WHERE id = ?1 AND revision = 4",
                params![ITEM],
            )
            .await?,
        1
    );
    transaction
        .execute(
            "INSERT INTO inventory_sync_operation(operation_id, revision, processed_at)
             VALUES (?1, 5, datetime('now'))",
            params![BACKUP_OVERLAP],
        )
        .await?;
    reader.execute("VACUUM INTO 'online.db'", ()).await?;
    transaction.commit().await?;
    assert_eq!(read_revisions(&reader).await?, (5, Some(4)));
    println!("Native Turso completed a WAL snapshot while a second connection held an open write.");
    Ok(())
}
