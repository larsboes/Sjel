use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sjel_inventory::api::{build_router, AppState};
use std::sync::Arc;
use tower::ServiceExt;

async fn send(router: &Router, method: &str, path: &str, value: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
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
async fn c2_proposals_are_reviewable_and_matchable_but_cannot_create_items() {
    let path =
        std::env::temp_dir().join(format!("inventory-mail-privacy-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let router = build_router(Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(&path).unwrap(),
        database: path.clone(),
    }));
    let body = json!({"source_id":"message","line_id":"one","event":"delivered","title":"Private title","evidence":"delivered","data_class":"c2"});
    let (_, proposal) = send(&router, "POST", "/api/mail-proposals", body.clone()).await;
    let decision = format!("/api/mail-proposals/{}/decision", proposal["id"]);
    let (status, reason) = send(
        &router,
        "POST",
        &decision,
        json!({"version":1,"action":"approve","item_id":"private","title":"Different title"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(reason.as_str().unwrap().contains("c2"));
    assert_eq!(
        send(&router, "GET", "/api/inventory", Value::Null).await.1,
        json!([])
    );
    let (_, proposals) = send(&router, "GET", "/api/mail-proposals", Value::Null).await;
    assert_eq!(proposals["proposals"][0]["data_class"], "c2");
    assert_eq!(proposals["proposals"][0]["decision"], Value::Null);
    assert_eq!(proposals["proposals"][0]["version"], 1);
    assert_eq!(
        send(
            &router,
            "POST",
            "/api/items",
            json!({"id":"existing","kind":"piece","label":"Existing","state":"owned"})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let before = send(&router, "GET", "/api/inventory", Value::Null).await.1;
    let (_, snap) = send(
        &router,
        "GET",
        "/api/items/existing/mail-match-snapshot",
        Value::Null,
    )
    .await;
    let (_, matched) = send(&router,"POST",&decision,json!({"version":1,"action":"match","item_id":"existing","expected_item_revision":snap["item_revision"],"expected_state_id":snap["state_id"]})).await;
    assert_eq!(matched["decision"], "match");
    assert_eq!(matched["data_class"], "c2");
    assert_eq!(
        send(&router, "GET", "/api/inventory", Value::Null).await.1,
        before
    );
    let mut other = body;
    other["line_id"] = json!("two");
    let (_, other) = send(&router, "POST", "/api/mail-proposals", other).await;
    let reject = format!("/api/mail-proposals/{}/decision", other["id"]);
    assert_eq!(
        send(
            &router,
            "POST",
            &reject,
            json!({"version":1,"action":"reject"})
        )
        .await
        .1["decision"],
        "reject"
    );
    for class in ["c0", "c1"] {
        let proposal = json!({"source_id":class,"line_id":"one","event":"delivered","title":format!("Title {class}"),"evidence":"delivered","data_class":class});
        let (_, proposal) = send(&router, "POST", "/api/mail-proposals", proposal).await;
        let path = format!("/api/mail-proposals/{}/decision", proposal["id"]);
        assert_eq!(send(&router,"POST",&path,json!({"version":1,"action":"approve","item_id":format!("item-{class}"),"title":format!("Title {class}")})).await.0,StatusCode::OK);
    }
    let _ = std::fs::remove_file(path);
}
