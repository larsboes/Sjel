use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use devices::auth;
use devices::store::DevicesStore;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::store::{Item, State as ItemState, Store};

const ITEM: &str = "signed-wal-fixture";
const OPERATION: &str = "signed-wal-operation";
const TOKEN: &str = "public-wal-sync-fixture";

fn signed_request(
    key: &Ed25519KeyPair,
    device_id: &str,
    operation: &str,
    nonce: u8,
    create: Option<&str>,
) -> Request<Body> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (entity_id, action, base_revision, fields) = match create {
        Some(id) => (
            id,
            "create",
            None,
            json!({"label": "Offline phone addition", "kind": "gear", "state": "wanted"}),
        ),
        None => (
            ITEM,
            "patch",
            Some("1"),
            json!({"label": "Acknowledged phone edit"}),
        ),
    };
    let envelope = json!({
        "protocol_version": "sjel-sync/v1",
        "envelope_id": format!("fixture-{nonce}"),
        "origin_node_id": "node_iphone",
        "target_node_id": super::super::node_id(),
        "actor_device_id": device_id,
        "sent_at": now,
        "mutations": [{
            "operation_id": operation,
            "entity_type": "interior.item",
            "entity_id": entity_id,
            "action": action,
            "base_revision": base_revision,
            "fields": fields,
            "actor_device_id": device_id,
            "created_at": now
        }]
    });
    let body = serde_json::to_vec(&envelope).unwrap();
    let signature = auth::SignedRequest {
        device_id: device_id.into(),
        timestamp: now,
        nonce: format!("{nonce:02x}").repeat(16),
        signature: Vec::new(),
    };
    let message = auth::signing_message(&signature, "POST", "/api/sync", &body);
    Request::post("/api/sync")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header(auth::DEVICE_ID_HEADER, device_id)
        .header(auth::TIMESTAMP_HEADER, now.to_string())
        .header(auth::NONCE_HEADER, signature.nonce)
        .header(
            auth::SIGNATURE_HEADER,
            auth::hex(key.sign(&message).as_ref()),
        )
        .body(Body::from(body))
        .unwrap()
}

fn router(store: Arc<WalPreview>, devices: Arc<DevicesStore>) -> Router {
    sjel_server::authenticated(
        preview_router(store, true, Some(devices), None),
        sjel_server::InboundAuth::with_token(Some(TOKEN.into())).require_credential(),
    )
}

async fn acknowledgement(
    router: &Router,
    request: Request<Body>,
    status: &str,
    revision: Option<&str>,
) -> Value {
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["acknowledgements"][0]["status"], status);
    assert_eq!(result["acknowledgements"][0]["revision"].as_str(), revision);
    result
}

#[tokio::test]
async fn signed_wal_sync_commits_once_and_restores_the_acknowledgement() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("sjel-wal-sync-{unique}"));
    std::fs::create_dir(&directory).unwrap();
    let seed_path = directory.join("seed.db");
    let native_path = directory.join("native.db");
    let restored_path = directory.join("restored.db");
    let seed = Store::open(&seed_path).unwrap();
    seed.upsert_item(&Item {
        id: ITEM.into(),
        label: "Before sync".into(),
        ..Default::default()
    })
    .unwrap();
    seed.record_state(ITEM, ItemState::Owned, None).unwrap();
    rusqlite::Connection::open(&seed_path)
        .unwrap()
        .execute("VACUUM INTO ?1", [native_path.to_str().unwrap()])
        .unwrap();
    rusqlite::Connection::open(&native_path)
        .unwrap()
        .pragma_update(None, "journal_mode", "wal")
        .unwrap();

    let devices = Arc::new(DevicesStore::open(&directory.join("devices.db")).unwrap());
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let pairing = devices
        .request_pairing(
            "Fixture phone".into(),
            "ios".into(),
            "ed25519".into(),
            auth::hex(key.public_key().as_ref()),
        )
        .unwrap();
    let id = devices
        .decide_request(&pairing.id, true)
        .unwrap()
        .device
        .unwrap()
        .id;
    let send = |operation, nonce| signed_request(&key, &id, operation, nonce, None);
    let send_create =
        |operation, nonce| signed_request(&key, &id, operation, nonce, Some("new-item"));

    let store = Arc::new(WalPreview::open(&native_path, true).await.unwrap());
    let api = router(Arc::clone(&store), Arc::clone(&devices));
    store
        .database
        .connect()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_sync_receipt BEFORE INSERT ON inventory_sync_operation
         BEGIN SELECT RAISE(ABORT, 'receipt refused'); END;",
        )
        .await
        .unwrap();
    let response = api.clone().oneshot(send(OPERATION, 1)).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(store.reference(ITEM).await.unwrap().unwrap().0, 1);
    assert_eq!(store.sync_receipt(OPERATION).await.unwrap(), None);
    let response = api
        .clone()
        .oneshot(send_create("create-op", 5))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.reference("new-item").await.unwrap().is_none());
    assert_eq!(store.sync_receipt("create-op").await.unwrap(), None);
    store
        .database
        .connect()
        .unwrap()
        .execute("DROP TRIGGER refuse_sync_receipt", ())
        .await
        .unwrap();

    acknowledgement(&api, send(OPERATION, 2), "accepted", Some("2")).await;
    let replayed_nonce = api.clone().oneshot(send(OPERATION, 2)).await.unwrap();
    assert_eq!(replayed_nonce.status(), StatusCode::UNAUTHORIZED);
    acknowledgement(&api, send_create("create-op", 6), "accepted", Some("1")).await;
    drop(api);
    drop(store);

    let store = Arc::new(WalPreview::open(&native_path, true).await.unwrap());
    let api = router(Arc::clone(&store), devices);
    acknowledgement(&api, send(OPERATION, 3), "duplicate", Some("2")).await;
    let stale = acknowledgement(&api, send("signed-wal-stale", 4), "conflict", Some("2")).await;
    assert_eq!(
        stale["conflicts"][0]["canonical_fields"]["label"],
        "Acknowledged phone edit"
    );
    assert_eq!(store.sync_receipt("signed-wal-stale").await.unwrap(), None);

    acknowledgement(&api, send_create("create-op", 7), "duplicate", Some("1")).await;
    acknowledgement(&api, send_create("different-op", 8), "rejected", None).await;

    let snapshot_path = directory.join("snapshot");
    std::fs::create_dir(&snapshot_path).unwrap();
    let exporter = snapshot::SnapshotDirectory::open(&snapshot_path, &[&native_path]).unwrap();
    let mut connection = store.writer().await.unwrap();
    let pending = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .unwrap();
    pending
        .execute(
            "UPDATE inventory_item SET revision = 3 WHERE id = ?1",
            [ITEM],
        )
        .await
        .unwrap();
    exporter.take(&store).await.unwrap();
    let snapshot = Store::open(&snapshot_path.join("inventory.db")).unwrap();
    assert_eq!(snapshot.item(ITEM).unwrap().unwrap().0.revision, 2);
    assert_eq!(
        snapshot.sync_operation_revision(OPERATION).unwrap(),
        Some(2)
    );
    pending.rollback().await.unwrap();
    drop(connection);
    drop(api);
    drop(store);

    let native_closed = rusqlite::Connection::open(&native_path).unwrap();
    native_closed
        .execute("VACUUM INTO ?1", [restored_path.to_str().unwrap()])
        .unwrap();
    let restored = Store::open(&restored_path).unwrap();
    assert_eq!(
        restored.sync_operation_revision(OPERATION).unwrap(),
        Some(2)
    );
    let (item, state) = restored.item(ITEM).unwrap().unwrap();
    assert_eq!(item.revision, 2);
    assert_eq!(item.label, "Acknowledged phone edit");
    assert_eq!(state, Some(ItemState::Owned));
    assert_eq!(
        restored.sync_operation_revision("create-op").unwrap(),
        Some(1)
    );
    let (item, state) = restored.item("new-item").unwrap().unwrap();
    assert_eq!(item.revision, 1);
    assert_eq!(state, Some(ItemState::Wanted));
    std::fs::remove_dir_all(directory).unwrap();
}
