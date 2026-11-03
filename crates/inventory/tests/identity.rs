use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sjel_inventory::{
    api::{build_router, AppState},
    identity::encode_item_ref,
};
use std::sync::Arc;
use tower::ServiceExt;

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    body: Value,
    auth: bool,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if auth {
        req = req.header("authorization", "Bearer test-token");
    }
    let res = router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 100_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn typed_lookup_is_authenticated_canonical_and_reads_only_item_identity() {
    let path = std::env::temp_dir().join(format!("inventory-ref-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let state = Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(&path).unwrap(),
        database: path.clone(),
    });
    let router = sjel_server::authenticated(
        build_router(state),
        sjel_server::InboundAuth::with_token(Some("test-token".into())).require_credential(),
    );
    let reference = encode_item_ref("a:é").unwrap();
    let uri = format!("/api/item-refs/{reference}");
    assert_eq!(
        request(&router, "GET", &uri, Value::Null, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&router, "GET", &uri, Value::Null, true).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/items",
            json!({"id":"a:é","kind":"piece","label":"Lamp","state":"owned"}),
            true
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(request(&router,"POST","/api/mail-proposals",json!({"source_id":"secret","line_id":"1","event":"delivered","title":"private","evidence":"private excerpt","data_class":"c2"}),true).await.0,StatusCode::OK);
    let (status, body) = request(&router, "GET", &uri, Value::Null, true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"typed_ref":"inv:613ac3a9","raw_id":"a:é","revision":1,"state":"owned"})
    );
    for bad in ["inv:6A", "inv:0", "inv:ff", "inv:lamp", "other:61"] {
        assert_eq!(
            request(
                &router,
                "GET",
                &format!("/api/item-refs/{bad}"),
                Value::Null,
                true
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/api/item-refs/inv:{}", "61".repeat(257)),
            Value::Null,
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/items/a:%C3%A9/state",
            json!({"state":"gone"}),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&router, "GET", &uri, Value::Null, true).await.1["state"],
        "gone"
    );
    let _ = std::fs::remove_file(path);
}
