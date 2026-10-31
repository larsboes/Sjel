use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use sjel_inventory::{
    api::{build_router, AppState},
    store::Store,
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn state_insert_failure_rolls_back_item_and_decision() {
    let path =
        std::env::temp_dir().join(format!("inventory-mail-rollback-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let device_store = devices::store::DevicesStore::open(&path).unwrap();
    let store = Store::open(&path).unwrap();
    let conn = store.borrow_connection().unwrap();
    conn.execute_batch("CREATE TRIGGER reject_mail_state BEFORE INSERT ON inventory_item_state BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    drop(conn);
    let router = build_router(Arc::new(AppState {
        device_store,
        database: path.clone(),
    }));
    let request = |uri: &str, body: Value| {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let stage = router.clone().oneshot(request("/api/mail-proposals", json!({"source_id":"m","line_id":"l","event":"delivered","title":"Desk","evidence":"delivered","data_class":"c1"}))).await.unwrap();
    let bytes = axum::body::to_bytes(stage.into_body(), 4096).await.unwrap();
    let proposal: Value = serde_json::from_slice(&bytes).unwrap();
    let response = router
        .oneshot(request(
            &format!("/api/mail-proposals/{}/decision", proposal["id"]),
            json!({"version":1,"action":"approve","title":"Desk","item_id":"desk"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(store.item_count().unwrap(), 0);
    let conn = store.borrow_connection().unwrap();
    let decision: Option<String> = conn
        .query_row("SELECT decision FROM inventory_mail_proposal", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(decision.is_none());
    let _ = std::fs::remove_file(path);
}
