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
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if auth {
        request = request.header("authorization", "Bearer test-token");
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn router(name: &str) -> (Router, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "inventory-resolution-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let state = Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(&path).unwrap(),
        database: path.clone(),
    });
    (
        sjel_server::authenticated(
            build_router(state),
            sjel_server::InboundAuth::with_token(Some("test-token".into())).require_credential(),
        ),
        path,
    )
}

async fn seed(router: &Router, id: &str, title: &str) {
    assert_eq!(
        request(
            router,
            "POST",
            "/api/items",
            json!({"id":id,"kind":"piece","label":title,"state":"owned"}),
            true
        )
        .await
        .0,
        StatusCode::CREATED
    );
}

async fn resolve(router: &Router, body: Value) -> Value {
    let (status, result) = request(router, "POST", "/api/items/resolve", body, true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result
}

#[tokio::test]
async fn only_a_canonical_reference_establishes_exact_identity() {
    let (router, path) = router("identity");
    seed(&router, "mug", "Blue mug").await;
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/items/resolve",
            json!({"title":"Blue mug"}),
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for input in [
        json!({}),
        json!({"reference":"inv:6A"}),
        json!({"title":"..."}),
        json!({"title":"x\u{0}"}),
        json!({"title":"x".repeat(257)}),
        json!({"reference":encode_item_ref("mug").unwrap(),"title":"Blue mug"}),
    ] {
        assert_eq!(
            request(&router, "POST", "/api/items/resolve", input, true)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let exact = resolve(
        &router,
        json!({"reference":encode_item_ref("mug").unwrap()}),
    )
    .await;
    assert_eq!(exact["status"], "exact");
    assert_eq!(exact["candidates"][0]["reason"], "reference");
    assert_eq!(exact["candidates"][0]["item_revision"], 1);
    assert_eq!(exact["candidates"][0]["state"], "owned");
    assert!(exact["candidates"][0]["state_id"].as_i64().is_some());
    assert!(exact["candidates"][0]["overlap_bp"].is_null());
    let title = resolve(&router, json!({"title":"  BLUE   mug  "})).await;
    assert_eq!(title["status"], "ambiguous");
    assert_eq!(title["candidates"][0]["reason"], "title_equal");
    assert_eq!(title["candidates"][0]["overlap_bp"], 10000);
    assert_eq!(
        resolve(
            &router,
            json!({"reference":encode_item_ref("missing").unwrap()})
        )
        .await["status"],
        "unknown"
    );
    assert_eq!(
        resolve(&router, json!({"title":"unrelated"})).await["status"],
        "unknown"
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn identical_products_and_accessories_never_create_automatic_links() {
    let (router, path) = router("duplicates");
    seed(&router, "one", "Dreame R10").await;
    seed(&router, "two", "Dreame R10").await;
    let same = resolve(&router, json!({"title":"Dreame R10"})).await;
    assert_eq!(same["status"], "ambiguous");
    assert_eq!(same["total_candidates"], 2);
    let query = json!({"title":"Dreame R10 filter"});
    let accessory = resolve(&router, query.clone()).await;
    assert_eq!(accessory["status"], "ambiguous");
    assert_eq!(accessory["candidates"][0]["reason"], "token_overlap");
    assert_eq!(accessory["candidates"][0]["overlap_bp"], 6666);
    assert_eq!(
        accessory["candidates"][0]["shared_tokens"],
        json!(["dreame", "r10"])
    );
    assert_eq!(accessory, resolve(&router, query).await);
    let (_, inventory) = request(&router, "GET", "/api/inventory", Value::Null, true).await;
    assert_eq!(inventory.as_array().unwrap().len(), 2);
    let (_, proposals) = request(&router, "GET", "/api/mail-proposals", Value::Null, true).await;
    assert_eq!(proposals["proposals"], json!([]));
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn candidate_memory_is_bounded_and_ties_are_stable() {
    let (router, path) = router("bounded");
    for n in (0..15).rev() {
        seed(&router, &format!("m{n:02}"), "Blue mug").await;
    }
    let result = resolve(&router, json!({"title":"Blue mug"})).await;
    assert_eq!(result["total_candidates"], 15);
    assert_eq!(result["candidates"].as_array().unwrap().len(), 10);
    assert_eq!(result["candidates"][0]["item_id"], "m00");
    assert_eq!(result["candidates"][9]["item_id"], "m09");
    assert_eq!(result, resolve(&router, json!({"title":"Blue mug"})).await);
    std::fs::remove_file(path).unwrap();
}
