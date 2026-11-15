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

fn alias_key(kind: &str, value: &str) -> Value {
    json!({"namespace":"source:example","value":value,"kind":kind})
}

async fn review_body(router: &Router, id: &str, key: Value) -> Value {
    let (status, snapshot) = request(
        router,
        "GET",
        &format!("/api/items/{id}/mail-match-snapshot"),
        Value::Null,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    json!({"key":key,"reference":encode_item_ref(id).unwrap(),"expected_item_revision":snapshot["item_revision"],"expected_state_id":snapshot["state_id"],"source_ref":"manual:reviewed-label","data_class":"c1"})
}

async fn review(router: &Router, body: Value) -> Value {
    let (status, alias) = request(router, "POST", "/api/item-aliases", body, true).await;
    assert_eq!(status, StatusCode::OK, "{alias}");
    alias
}

#[tokio::test]
async fn aliases_require_owner_review_at_a_current_snapshot_and_keep_exact_scope() {
    let (router, path) = router("alias-review");
    seed(&router, "one", "Blue mug").await;
    let key = alias_key("physical_item", "SERIAL-1");
    let body = review_body(&router, "one", key.clone()).await;
    assert_eq!(
        request(&router, "POST", "/api/item-aliases", body.clone(), false)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for field in ["expected_item_revision", "expected_state_id"] {
        let mut stale = body.clone();
        stale[field] = json!(999);
        assert_eq!(
            request(&router, "POST", "/api/item-aliases", stale, true)
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    for class in ["c2", "c3", "unknown"] {
        let mut invalid = body.clone();
        invalid["data_class"] = json!(class);
        assert_eq!(
            request(&router, "POST", "/api/item-aliases", invalid, true)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let alias = review(&router, body.clone()).await;
    assert_eq!(alias, review(&router, body.clone()).await);
    let exact = resolve(&router, json!({"alias":key})).await;
    assert_eq!(exact["status"], "exact");
    assert_eq!(exact["candidates"][0]["alias_id"], alias["id"]);
    assert_eq!(exact["candidates"][0]["reason"], "reviewed_item_alias");
    let mut other_scope = key.clone();
    other_scope["namespace"] = json!("source:other");
    assert_eq!(
        resolve(&router, json!({"alias":other_scope})).await["status"],
        "unknown"
    );
    assert_eq!(
        resolve(
            &router,
            json!({"alias":alias_key("physical_item","serial-1")})
        )
        .await["status"],
        "unknown"
    );
    assert_eq!(
        resolve(&router, json!({"title":"SERIAL-1"})).await["status"],
        "unknown"
    );
    let mut changed = body;
    changed["source_ref"] = json!("manual:different-review");
    assert_eq!(
        request(&router, "POST", "/api/item-aliases", changed, true)
            .await
            .0,
        StatusCode::CONFLICT
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn product_models_and_duplicate_physical_aliases_remain_ambiguous() {
    let (router, path) = router("alias-ambiguity");
    seed(&router, "one", "Same product").await;
    seed(&router, "two", "Same product").await;
    let product = alias_key("product_model", "MODEL-1");
    review(&router, review_body(&router, "one", product.clone()).await).await;
    assert_eq!(
        resolve(&router, json!({"alias":product})).await["status"],
        "ambiguous"
    );
    review(&router, review_body(&router, "two", product.clone()).await).await;
    assert_eq!(
        resolve(&router, json!({"alias":product})).await["total_candidates"],
        2
    );
    let physical = alias_key("physical_item", "DUPLICATE-SERIAL");
    for id in ["one", "two"] {
        review(&router, review_body(&router, id, physical.clone()).await).await;
    }
    let result = resolve(&router, json!({"alias":physical})).await;
    assert_eq!(result["status"], "ambiguous");
    assert_eq!(result["total_candidates"], 2);
    let request_body = review_body(&router, "one", alias_key("physical_item", "another")).await;
    request(
        &router,
        "POST",
        "/api/items/one/state",
        json!({"state":"gone"}),
        true,
    )
    .await;
    assert_eq!(
        request(&router, "POST", "/api/item-aliases", request_body, true)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let historical = alias_key("physical_item", "historical-item");
    review(
        &router,
        review_body(&router, "one", historical.clone()).await,
    )
    .await;
    let result = resolve(&router, json!({"alias":historical})).await;
    assert_eq!(result["status"], "exact");
    assert_eq!(result["candidates"][0]["state"], "gone");
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn revocations_are_versioned_idempotent_and_retained() {
    let (router, path) = router("alias-revoke");
    seed(&router, "one", "Blue mug").await;
    let key = alias_key("physical_item", "SERIAL-1");
    let body = review_body(&router, "one", key.clone()).await;
    let alias = review(&router, body.clone()).await;
    let uri = format!("/api/item-aliases/{}/revoke", alias["id"]);
    assert_eq!(
        request(&router, "POST", &uri, json!({"version":2}), true)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, revoked) = request(&router, "POST", &uri, json!({"version":1}), true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["version"], 2);
    assert!(revoked["revoked_at"].is_string());
    assert_eq!(
        request(&router, "POST", &uri, json!({"version":1}), true)
            .await
            .1,
        revoked
    );
    assert_eq!(
        resolve(&router, json!({"alias":key})).await["status"],
        "unknown"
    );
    let replacement = review(&router, body).await;
    assert_ne!(replacement["id"], alias["id"]);
    let uri = format!("/api/item-refs/{}/aliases", encode_item_ref("one").unwrap());
    assert_eq!(
        request(&router, "GET", &uri, Value::Null, false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, history) = request(&router, "GET", &uri, Value::Null, true).await;
    assert_eq!(history["total"], 2);
    assert_eq!(history["aliases"][1], revoked);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn item_id_reuse_never_resurrects_an_old_alias() {
    let (router, path) = router("alias-lifetime");
    seed(&router, "one", "Original item").await;
    let key = alias_key("physical_item", "SERIAL-1");
    let body = review_body(&router, "one", key.clone()).await;
    let alias = review(&router, body.clone()).await;
    {
        let store = sjel_inventory::store::Store::open(&path).unwrap();
        let conn = store.borrow_connection().unwrap();
        conn.execute("DELETE FROM inventory_item WHERE id='one'", [])
            .unwrap();
        let original_state: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM inventory_item_state WHERE id=?1",
                [body["expected_state_id"].as_i64().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original_state, 0);
    }
    seed(&router, "one", "Different physical item").await;
    assert_eq!(
        resolve(&router, json!({"alias":key})).await["status"],
        "unknown"
    );
    assert_eq!(review(&router, body).await, alias);
    assert_eq!(
        resolve(&router, json!({"alias":key})).await["status"],
        "unknown"
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn alias_candidates_keep_the_same_bounded_contract() {
    let (router, path) = router("alias-bound");
    let key = alias_key("product_model", "MODEL-1");
    for n in (0..12).rev() {
        let id = format!("item-{n:02}");
        seed(&router, &id, "Same product").await;
        review(&router, review_body(&router, &id, key.clone()).await).await;
    }
    let result = resolve(&router, json!({"alias":key})).await;
    assert_eq!(result["total_candidates"], 12);
    assert_eq!(result["candidates"].as_array().unwrap().len(), 10);
    assert_eq!(result["candidates"][0]["item_id"], "item-00");
    std::fs::remove_file(path).unwrap();
}
