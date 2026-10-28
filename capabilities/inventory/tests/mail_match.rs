use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sjel_inventory::api::{build_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;

async fn send(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[test]
fn migration_preserves_existing_proposals() {
    let path =
        std::env::temp_dir().join(format!("inventory-mail-migrate-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE inventory_mail_proposal (id INTEGER PRIMARY KEY AUTOINCREMENT, source_id TEXT NOT NULL, line_id TEXT NOT NULL, event TEXT NOT NULL, title TEXT NOT NULL, evidence TEXT NOT NULL, data_class TEXT NOT NULL, order_ref TEXT, version INTEGER NOT NULL DEFAULT 1, decision TEXT, item_id TEXT, approved_title TEXT, UNIQUE(source_id,line_id,event)); INSERT INTO inventory_mail_proposal (source_id,line_id,event,title,evidence,data_class) VALUES ('m','l','delivered','Lamp','delivered','c1');").unwrap();
    drop(conn);
    let store = sjel_inventory::store::Store::open(&path).unwrap();
    let conn = store.borrow_connection().unwrap();
    let (title, revision): (String, Option<i64>) = conn
        .query_row(
            "SELECT title, matched_item_revision FROM inventory_mail_proposal",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(title, "Lamp");
    assert_eq!(revision, None);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn match_checks_item_and_state_snapshot_and_preserves_history() {
    let path = std::env::temp_dir().join(format!("inventory-mail-match-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let router = build_router(Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(&path).unwrap(),
        database: path.clone(),
    }));
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/items",
            json!({"id":"lamp","kind":"piece","label":"Lamp","state":"owned"})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let stage = json!({"source_id":"message-a","line_id":"1","event":"delivered","title":"Lamp","evidence":"delivered","data_class":"c1","order_ref":"order"});
    let (_, proposal) = send(&router, "POST", "/api/mail-proposals", stage.clone()).await;
    let decision_path = format!("/api/mail-proposals/{}/decision", proposal["id"]);
    let (_, snap) = send(
        &router,
        "GET",
        "/api/items/lamp/mail-match-snapshot",
        Value::Null,
    )
    .await;
    let mut decision = json!({"version":1,"action":"match","item_id":"lamp","expected_item_revision":snap["item_revision"],"expected_state_id":snap["state_id"]});
    let mut missing = decision.clone();
    missing["item_id"] = json!("missing");
    assert_eq!(
        send(&router, "POST", &decision_path, missing).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &router,
            "GET",
            "/api/items/missing/mail-match-snapshot",
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &router,
            "PATCH",
            "/api/items/lamp",
            json!({"label":"Updated lamp"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, "POST", &decision_path, decision.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, snap) = send(
        &router,
        "GET",
        "/api/items/lamp/mail-match-snapshot",
        Value::Null,
    )
    .await;
    decision["expected_item_revision"] = snap["item_revision"].clone();
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/items/lamp/state",
            json!({"state":"gone"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, "POST", &decision_path, decision.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, gone) = send(
        &router,
        "GET",
        "/api/items/lamp/mail-match-snapshot",
        Value::Null,
    )
    .await;
    decision["expected_state_id"] = gone["state_id"].clone();
    assert_eq!(
        send(&router, "POST", &decision_path, decision.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/items/lamp/state",
            json!({"state":"owned"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, owned) = send(
        &router,
        "GET",
        "/api/items/lamp/mail-match-snapshot",
        Value::Null,
    )
    .await;
    assert_ne!(owned["state_id"], snap["state_id"]);
    decision["expected_state_id"] = snap["state_id"].clone();
    assert_eq!(
        send(&router, "POST", &decision_path, decision.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    decision["expected_state_id"] = owned["state_id"].clone();
    let before = send(&router, "GET", "/api/inventory", Value::Null).await.1;
    let history = send(&router, "GET", "/api/items/lamp/state", Value::Null)
        .await
        .1;
    let (status, matched) = send(&router, "POST", &decision_path, decision.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(matched["decision"], "match");
    assert_eq!(matched["item_id"], "lamp");
    assert_eq!(matched["matched_state_id"], owned["state_id"]);
    assert_eq!(
        send(&router, "GET", "/api/inventory", Value::Null).await.1,
        before
    );
    assert_eq!(
        send(&router, "GET", "/api/items/lamp/state", Value::Null)
            .await
            .1,
        history
    );
    assert_eq!(
        send(&router, "POST", &decision_path, decision.clone())
            .await
            .1,
        matched
    );
    let mut wrong = decision.clone();
    wrong["expected_state_id"] = json!(999);
    assert_eq!(
        send(&router, "POST", &decision_path, wrong).await.0,
        StatusCode::CONFLICT
    );
    let mut duplicate = stage;
    duplicate["source_id"] = json!("message-b");
    let (_, other) = send(&router, "POST", "/api/mail-proposals", duplicate).await;
    let other_path = format!("/api/mail-proposals/{}/decision", other["id"]);
    assert_eq!(
        send(&router, "POST", &other_path, decision).await.1["decision"],
        "match"
    );
    assert_eq!(
        send(&router, "GET", "/api/items/lamp/state", Value::Null)
            .await
            .1,
        history
    );
    let _ = std::fs::remove_file(path);
}
