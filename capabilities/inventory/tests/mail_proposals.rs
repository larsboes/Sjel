use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sjel_inventory::api::{build_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;

async fn send(
    router: &Router,
    method: &str,
    path: &str,
    value: Value,
    auth: bool,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if auth {
        req = req.header("authorization", "Bearer test-token");
    }
    let response = router
        .clone()
        .oneshot(req.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

#[tokio::test]
async fn reviewed_mail_is_atomic_idempotent_and_conflict_guarded() {
    let path = std::env::temp_dir().join(format!("inventory-mail-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let state = Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(&path).unwrap(),
        database: path.clone(),
    });
    let router = sjel_server::authenticated(
        build_router(state),
        sjel_server::InboundAuth::with_token(Some("test-token".into())).require_credential(),
    );
    let proposal = json!({"source_id":"msg-1","line_id":"line-1","event":"delivered","title":"Lamp","evidence":"delivered today","data_class":"c1","order_ref":"order-1"});
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/mail-proposals",
            proposal.clone(),
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, first) = send(
        &router,
        "POST",
        "/api/mail-proposals",
        proposal.clone(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["version"], 1);
    assert_eq!(
        send(&router, "GET", "/api/inventory", Value::Null, true)
            .await
            .1,
        json!([])
    );
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/mail-proposals",
            proposal.clone(),
            true
        )
        .await
        .1,
        first
    );
    let mut changed = proposal.clone();
    changed["evidence"] = json!("different");
    assert_eq!(
        send(&router, "POST", "/api/mail-proposals", changed, true)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut c3 = proposal.clone();
    c3["data_class"] = json!("c3");
    assert_eq!(
        send(&router, "POST", "/api/mail-proposals", c3, true)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut line2 = proposal.clone();
    line2["line_id"] = json!("line-2");
    line2["title"] = json!("Chair");
    let (_, second) = send(&router, "POST", "/api/mail-proposals", line2, true).await;
    assert_ne!(first["id"], second["id"]);
    let mut cancel = proposal.clone();
    cancel["event"] = json!("cancellation");
    let (_, cancellation) = send(&router, "POST", "/api/mail-proposals", cancel, true).await;
    let cancel_path = format!("/api/mail-proposals/{}/decision", cancellation["id"]);
    assert_eq!(
        send(
            &router,
            "POST",
            &cancel_path,
            json!({"version":1,"action":"approve","title":"Lamp","item_id":"lamp"}),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let decision_path = format!("/api/mail-proposals/{}/decision", first["id"]);
    assert_eq!(
        send(
            &router,
            "POST",
            &decision_path,
            json!({"version":2,"action":"approve","title":"Lamp","item_id":"lamp"}),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, approved) = send(
        &router,
        "POST",
        &decision_path,
        json!({"version":1,"action":"approve","title":"Lamp","item_id":"lamp"}),
        true,
    )
    .await;
    assert_eq!(approved["decision"], "approve");
    assert_eq!(approved["version"], 2);
    assert_eq!(
        send(
            &router,
            "POST",
            &decision_path,
            json!({"version":1,"action":"approve","title":"Lamp","item_id":"lamp"}),
            true
        )
        .await
        .1,
        approved
    );
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/mail-proposals",
            proposal.clone(),
            true
        )
        .await
        .1,
        approved
    );
    let (_, items) = send(&router, "GET", "/api/inventory", Value::Null, true).await;
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["state"], "owned");
    let second_path = format!("/api/mail-proposals/{}/decision", second["id"]);
    assert_eq!(
        send(
            &router,
            "POST",
            &second_path,
            json!({"version":1,"action":"approve","title":"Lamp","item_id":"another"}),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(
            &router,
            "POST",
            &second_path,
            json!({"version":1,"action":"reject"}),
            true
        )
        .await
        .1["decision"],
        "reject"
    );
    assert_eq!(
        send(&router, "GET", "/api/mail-proposals", Value::Null, true)
            .await
            .1["proposals"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let _ = std::fs::remove_file(&path);
}
