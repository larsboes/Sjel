//! The capability's HTTP surface. Every endpoint returns JSON; the dashboard renders it.
//!
//! Item images stay on `interior`'s `/api/media` for now: the media path resolves a private asset
//! root that interior also uses for RoomPlan captures.

use axum::body::{to_bytes, Body};
use axum::extract::{Extension, Path, Request, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
#[path = "sync.rs"]
mod sync;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::store::{Item, Store, WriteOutcome, LEGACY_FIELDS};

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SaveWishGroupRequest {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub constraint_text: String,
    #[serde(default)]
    pub item_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SetWishGroupItemsRequest {
    pub item_ids: Vec<String>,
}

pub struct AppState {
    pub device_store: devices::store::DevicesStore,
    /// Resolved once at start, so a test can point the router at a temp file.
    pub database: PathBuf,
}

type HttpError = (StatusCode, String);

fn store(s: &AppState) -> Result<Store, HttpError> {
    Store::open(&s.database).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn boom<E: std::fmt::Display>(e: E) -> HttpError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn health() -> &'static str {
    "ok"
}

/// What this capability answers. `route_manifest::undeclared_routes` reads the router from this
/// source and fails when a route is missing here.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/", "Every entry as a page, rendered from the rows."),
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This catalogue."),
    r("GET", "/api/inventory", "Every entry with its state."),
    r(
        "GET",
        "/api/wishlist",
        "The open wishlist and what it costs in monthly balances.",
    ),
    r(
        "GET",
        "/api/wish-groups",
        "Groups of wishes sharing a constraint, with member items and cost totals.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/wish-groups",
        summary: "Create or update a wish group with its title and constraint.",
        request_schema: Some(route_manifest::schema_of::<SaveWishGroupRequest>),
    },
    route_manifest::Route {
        method: "DELETE",
        path: "/api/wish-groups/{id}",
        summary: "Delete a wish group.",
        request_schema: None,
    },
    route_manifest::Route {
        method: "PUT",
        path: "/api/wish-groups/{id}/items",
        summary: "Set the items assigned to a wish group.",
        request_schema: Some(route_manifest::schema_of::<SetWishGroupItemsRequest>),
    },
    r(
        "POST",
        "/api/items",
        "Create a piece or a need. `state` is required.",
    ),
    r(
        "PUT",
        "/api/items/{id}",
        "Replace an entry. Takes the item shape /api/inventory returns.",
    ),
    r(
        "PATCH",
        "/api/items/{id}",
        "Change the named fields and leave the rest.",
    ),
    r(
        "GET",
        "/api/items/{id}/state",
        "The state history of an entry.",
    ),
    r("POST", "/api/items/{id}/state", "Append a state change."),
    r(
        "POST",
        "/api/sync",
        "Signed axon-sync/v1 mutations from a paired device.",
    ),
];

const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<serde_json::Value> {
    Json(route_manifest::manifest("inventory", ROUTES))
}

/// A page without the shell, for a look straight at the port.
async fn index(State(s): State<Arc<AppState>>) -> Result<Html<String>, HttpError> {
    let rows = store(&s)?.catalogue().map_err(boom)?;
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>inventory</title>\
         <style>body{font:14px/1.5 -apple-system,sans-serif;margin:2rem;max-width:60rem}\
         td{padding:.2rem .6rem .2rem 0;vertical-align:top}code{color:#555}</style>\
         <h1>inventory</h1><p>What the household owns.</p><table>",
    );
    for (item, state) in rows.values() {
        html.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            escape(&item.id),
            escape(&item.label),
            state.map(|s| s.as_str()).unwrap_or("—"),
        ));
    }
    html.push_str("</table>");
    Ok(Html(html))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn api_inventory(State(s): State<Arc<AppState>>) -> Result<impl IntoResponse, HttpError> {
    let rows = store(&s)?.catalogue().map_err(boom)?;
    let out: Vec<_> = rows
        .into_values()
        .map(
            |(item, state)| serde_json::json!({ "item": item, "state": state.map(|s| s.as_str()) }),
        )
        .collect();
    Ok(Json(out))
}

/// The open wishlist and what it costs in monthly balances.
///
/// Two totals, because a product has a price and a slot has an estimate range; folding them into
/// one number would present a range as a price.
async fn api_wishlist(State(s): State<Arc<AppState>>) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    let rows = st.catalogue().map_err(boom)?;
    let open: Vec<_> = rows
        .values()
        .filter(|(_, s)| *s == Some(crate::store::State::Wanted))
        .map(|(i, _)| i)
        .collect();

    let total_low: i64 = open
        .iter()
        .map(|i| i.price_cents.or(i.cost_min_cents).unwrap_or(0))
        .sum();
    let total_high: i64 = open
        .iter()
        .map(|i| i.price_cents.or(i.cost_max_cents).unwrap_or(0))
        .sum();
    let unpriced = open
        .iter()
        .filter(|i| i.price_cents.is_none() && i.cost_min_cents.is_none())
        .count();

    let conn = st.borrow_connection().map_err(boom)?;
    let balance = crate::budget::monthly_balance(&conn).map_err(boom)?;
    let months = balance
        .as_ref()
        .and_then(|b| crate::budget::months_to_pay(total_low, b));

    Ok(Json(serde_json::json!({
        "items": open,
        "total_low_cents": total_low,
        "total_high_cents": total_high,
        // An unpriced item counts as 0, so the total is a floor; this count says so.
        "unpriced_count": unpriced,
        "monthly_balance": balance,
        "months_to_pay": months,
    })))
}

/// Groups of wishes sharing a constraint, with member items and cost totals.
async fn api_get_wish_groups(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    let groups = st.wish_groups().map_err(boom)?;
    Ok(Json(serde_json::json!({ "wish_groups": groups })))
}

/// Create or update a wish group with its title and constraint.
async fn api_post_wish_group(
    State(s): State<Arc<AppState>>,
    Json(body): Json<SaveWishGroupRequest>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    let group = st
        .create_or_update_wish_group(&body.id, &body.title, &body.constraint_text)
        .map_err(boom)?;
    if let Some(items) = body.item_ids {
        st.set_wish_group_items(&body.id, &items).map_err(boom)?;
    }
    Ok(Json(group))
}

/// Delete a wish group.
async fn api_delete_wish_group(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    let deleted = st.delete_wish_group(&id).map_err(boom)?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "wish group not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Set the items assigned to a wish group.
async fn api_put_wish_group_items(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetWishGroupItemsRequest>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    st.set_wish_group_items(&id, &body.item_ids).map_err(boom)?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct NewItem {
    #[serde(flatten)]
    item: Item,
    /// Required: every list joins on the latest state, so a row without one is invisible.
    state: crate::store::State,
    /// The note on the first state row, not the item's `notes`.
    #[serde(default)]
    note: Option<String>,
}

async fn api_post_item(
    State(s): State<Arc<AppState>>,
    Json(body): Json<NewItem>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    if body.item.id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "`id` is missing".into()));
    }
    if st.item(&body.item.id).map_err(boom)?.is_some() {
        return Err((
            StatusCode::CONFLICT,
            format!("`{}` exists already; PATCH changes it", body.item.id),
        ));
    }
    let revision = st.upsert_item(&body.item).map_err(boom)?;
    st.record_state(
        &body.item.id,
        body.state,
        body.note.as_deref().or(Some("created in the dashboard")),
    )
    .map_err(boom)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": body.item.id, "state": body.state.as_str(), "ok": true, "revision": revision
        })),
    ))
}

/// The expected revision from `If-Match` or the body. Two different numbers are an error.
fn expected_revision(
    headers: &axum::http::HeaderMap,
    body: &mut serde_json::Value,
) -> Result<Option<i64>, HttpError> {
    let from_header = headers
        .get(axum::http::header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<i64>().map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("If-Match is not a number: `{v}`"),
                )
            })
        })
        .transpose()?;
    let from_body = body.get("expected_revision").and_then(|v| v.as_i64());
    if let Some(object) = body.as_object_mut() {
        object.remove("expected_revision");
    }
    match (from_header, from_body) {
        (Some(a), Some(b)) if a != b => Err((
            StatusCode::BAD_REQUEST,
            format!("If-Match says {a}, expected_revision says {b}"),
        )),
        (Some(a), _) => Ok(Some(a)),
        (None, b) => Ok(b),
    }
}

fn not_an_entry(e: serde_json::Error) -> HttpError {
    (
        StatusCode::BAD_REQUEST,
        format!("the body does not fit an entry: {e}"),
    )
}

async fn api_put_item(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(mut body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, HttpError> {
    let expected = expected_revision(&headers, &mut body)?;
    let st = store(&s)?;
    let mut item: Item = serde_json::from_value(body).map_err(not_an_entry)?;
    item.id = id;
    write(&st, &item, expected)
}

/// Change the named fields and leave the rest. `PUT` would blank every field a form does not
/// send. An explicit `null` clears a field; a missing key leaves it.
async fn api_patch_item(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(mut patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, HttpError> {
    let expected = expected_revision(&headers, &mut patch)?;
    let st = store(&s)?;
    let (item, _) = st
        .item(&id)
        .map_err(boom)?
        .ok_or((StatusCode::NOT_FOUND, format!("no entry `{id}`")))?;
    let mut item = merge_patch(&item, patch)?;
    item.id = id;
    write(&st, &item, expected)
}

fn write(st: &Store, item: &Item, expected: Option<i64>) -> Result<Response, HttpError> {
    match expected {
        Some(e) => write_if_revision(st, item, e),
        None => {
            let revision = st.upsert_item(item).map_err(boom)?;
            Ok(written(&item.id, revision))
        }
    }
}

/// The item with `patch` applied: unknown keys are refused, legacy German keys translated.
pub fn merge_patch(current: &Item, patch: serde_json::Value) -> Result<Item, HttpError> {
    let serde_json::Value::Object(patch) = patch else {
        return Err((StatusCode::BAD_REQUEST, "the body is not an object".into()));
    };
    let mut merged = match serde_json::to_value(current).map_err(boom)? {
        serde_json::Value::Object(m) => m,
        _ => unreachable!("an Item serialises as an object"),
    };
    for (k, v) in patch {
        if k == "id" || k == "revision" {
            // The path wins, as with PUT; the revision belongs to the server.
            continue;
        }
        // A phone may still hold an edit queued under a German field name.
        let k = LEGACY_FIELDS
            .iter()
            .find(|(old, _)| *old == k)
            .map_or(k, |(_, new)| (*new).to_string());
        if !merged.contains_key(&k) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("`{k}` is not a field of an entry"),
            ));
        }
        merged.insert(k, v);
    }
    serde_json::from_value(serde_json::Value::Object(merged)).map_err(not_an_entry)
}

/// A successful write: the new revision in the body and as `ETag`, which a client sends back as
/// `If-Match`.
fn written(id: &str, revision: i64) -> Response {
    (
        [(axum::http::header::ETAG, format!("\"{revision}\""))],
        Json(serde_json::json!({ "id": id, "ok": true, "revision": revision })),
    )
        .into_response()
}

/// Write against the revision the client read. Stale is 409 with the current row, so a client
/// that can be offline shows it and lets a person decide instead of retrying.
fn write_if_revision(st: &Store, item: &Item, expected: i64) -> Result<Response, HttpError> {
    match st.update_item_if_revision(item, expected).map_err(boom)? {
        WriteOutcome::Written(revision) => Ok(written(&item.id, revision)),
        WriteOutcome::Missing => Err((StatusCode::NOT_FOUND, format!("no entry `{}`", item.id))),
        // The dashboard shows `error` to the person who decides
        // (dashboard/vite/inventory-conflict.test.ts).
        WriteOutcome::Stale(current, state) => Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!(
                    "`{}` was changed meanwhile: expected revision {expected}, current {}; \
                     reload and apply the change to the current values",
                    item.id, current.revision
                ),
                "current": {
                    "item": current,
                    "state": state.map(|s| s.as_str()),
                },
            })),
        )
            .into_response()),
    }
}

#[derive(Deserialize)]
struct StateBody {
    state: crate::store::State,
    #[serde(default)]
    note: Option<String>,
}

/// Append a state change; a bought wish is a second row, not an overwritten field.
async fn api_post_state(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<StateBody>,
) -> Result<impl IntoResponse, HttpError> {
    let st = store(&s)?;
    if st.item(&id).map_err(boom)?.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no entry `{id}`")));
    }
    let changed = st
        .record_state(&id, body.state, body.note.as_deref())
        .map_err(boom)?;
    Ok(Json(serde_json::json!({
        "id": id, "state": body.state.as_str(), "changed": changed
    })))
}

async fn api_state_history(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    let rows = store(&s)?.state_history(&id).map_err(boom)?;
    let out: Vec<_> = rows
        .into_iter()
        .map(|(s, since, note)| serde_json::json!({ "state": s.as_str(), "since": since, "note": note }))
        .collect();
    Ok(Json(out))
}

// ─── Device sync ─────────────────────────────────────────────────────────────

fn node_id() -> String {
    sjel_config::env_var("SJEL_NODE_ID").unwrap_or_else(|_| "node_mac".to_string())
}

/// Verify the device signature before the handler runs. The signature covers method, path and
/// body, so the body is read here and passed on.
async fn signed_sync_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: middleware::Next,
) -> Response {
    let signed = match devices::auth::SignedRequest::from_headers(request.headers()) {
        Ok(signed) => signed,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response()
        }
    };
    let method = request.method().as_str().to_string();
    let path_and_query = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_string(),
        |value| value.as_str().to_string(),
    );
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, 8 * 1024 * 1024).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };
    let result = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let body = body.to_vec();
        move || {
            state
                .device_store
                .authenticate(&signed, &method, &path_and_query, &body)
        }
    })
    .await;
    let device = match result {
        Ok(Ok(device)) => device,
        Ok(Err(error)) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response()
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let mut request = Request::from_parts(parts, Body::from(body));
    request.extensions_mut().insert(device);
    next.run(request).await
}

/// Apply a batch of mutations from a paired device.
///
/// The entity stays `interior.item`: it is part of the wire format, and a phone with a queued
/// outbox still sends it. A new name would be a new protocol version. A repeated `operation_id`
/// is answered `duplicate` instead of being applied twice.
async fn api_sync(
    State(state): State<Arc<AppState>>,
    Extension(device): Extension<devices::store::Device>,
    Json(envelope): Json<sync::Envelope>,
) -> Result<impl IntoResponse, HttpError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    if envelope.protocol_version != "axon-sync/v1" {
        return Err((
            StatusCode::BAD_REQUEST,
            "unsupported sync protocol version".into(),
        ));
    }
    if envelope.target_node_id != node_id() {
        return Err((
            StatusCode::BAD_REQUEST,
            "sync envelope targets a different node".into(),
        ));
    }
    if envelope.actor_device_id != device.id {
        return Err((
            StatusCode::UNAUTHORIZED,
            "envelope actor does not match the signed device".into(),
        ));
    }
    let st = store(&state)?;
    let mut response = sync::empty_response(&envelope, now);
    for mutation in envelope.mutations {
        let operation_id = mutation.operation_id.clone();
        let processed_at = now;
        let rejected = |error: &str| sync::Acknowledgement {
            operation_id: mutation.operation_id.clone(),
            status: "rejected",
            revision: None,
            processed_at,
            error: Some(error.into()),
        };
        if let Some(revision) = st.sync_operation_revision(&operation_id).map_err(boom)? {
            response.acknowledgements.push(sync::Acknowledgement {
                operation_id,
                status: "duplicate",
                revision: Some(revision.to_string()),
                processed_at,
                error: None,
            });
            continue;
        }
        if mutation.actor_device_id != device.id {
            response
                .acknowledgements
                .push(rejected("mutation actor does not match the signed device"));
            continue;
        }
        if mutation.entity_type != "interior.item"
            || !matches!(mutation.action.as_str(), "patch" | "upsert")
        {
            response.acknowledgements.push(rejected(
                "initial sync supports only interior.item patch or upsert mutations",
            ));
            continue;
        }
        let Some(base_revision) = mutation.base_revision.as_deref() else {
            response.acknowledgements.push(rejected(
                "base_revision is required for an offline mutation",
            ));
            continue;
        };
        let Ok(expected) = base_revision.parse::<i64>() else {
            response
                .acknowledgements
                .push(rejected("base_revision must be a decimal revision"));
            continue;
        };
        let current = st.item(&mutation.entity_id).map_err(boom)?.ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("no item `{}`", mutation.entity_id),
            )
        })?;
        let mut merged = merge_patch(
            &current.0,
            serde_json::Value::Object(mutation.fields.clone()),
        )?;
        merged.id = mutation.entity_id.clone();
        match st
            .update_item_if_revision(&merged, expected)
            .map_err(boom)?
        {
            WriteOutcome::Written(revision) => {
                st.record_sync_operation(&mutation.operation_id, revision)
                    .map_err(boom)?;
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "accepted",
                    revision: Some(revision.to_string()),
                    processed_at,
                    error: None,
                });
            }
            WriteOutcome::Missing => response
                .acknowledgements
                .push(rejected("item does not exist")),
            WriteOutcome::Stale(actual, _) => {
                let canonical = serde_json::to_value(&*actual).unwrap_or(serde_json::Value::Null);
                response.conflicts.push(serde_json::json!({
                    "conflict_id": mutation.operation_id,
                    "operation_id": mutation.operation_id,
                    "entity_type": mutation.entity_type,
                    "entity_id": mutation.entity_id,
                    "base_revision": mutation.base_revision,
                    "canonical_revision": actual.revision.to_string(),
                    "local_fields": mutation.fields,
                    "canonical_fields": canonical,
                    "detected_at": processed_at,
                    "state": "open"
                }));
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "conflict",
                    revision: Some(actual.revision.to_string()),
                    processed_at,
                    error: Some(format!("canonical revision is {}", actual.revision)),
                });
            }
        }
    }
    Ok(Json(response))
}

/// Names the origin allowlist variable, `SJEL_INVENTORY_ALLOWED_ORIGIN_HOSTS`.
const CAPABILITY: &str = "inventory";

pub fn build_router(state: Arc<AppState>) -> Router {
    let signed_sync =
        Router::new()
            .route("/api/sync", post(api_sync))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                signed_sync_auth,
            ));
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/inventory", get(api_inventory))
        .route("/api/wishlist", get(api_wishlist))
        .route(
            "/api/wish-groups",
            get(api_get_wish_groups).post(api_post_wish_group),
        )
        .route("/api/wish-groups/{id}", delete(api_delete_wish_group))
        .route("/api/wish-groups/{id}/items", put(api_put_wish_group_items))
        .route("/api/items", post(api_post_item))
        .route("/api/items/{id}", put(api_put_item).patch(api_patch_item))
        .route(
            "/api/items/{id}/state",
            get(api_state_history).post(api_post_state),
        )
        .merge(signed_sync)
        // Last, because axum wraps only routes registered before `.layer()`. Needed because a
        // POST a browser sends as a simple request reaches the handler with no CORS preflight.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

pub async fn serve() {
    let port: u16 = sjel_config::env_var("SJEL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8101);
    let database = sjel_config::database_path();
    let device_store = devices::store::DevicesStore::open(&database)
        .unwrap_or_else(|error| panic!("inventory: cannot open device registry: {error}"));
    let state = Arc::new(AppState {
        device_store,
        database,
    });
    sjel_server::serve_local("inventory", port, build_router(state)).await;
}

#[cfg(test)]
mod route_manifest_tests {
    #[test]
    fn the_catalogue_names_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("api.rs"), super::ROUTES);
        assert!(
            missing.is_empty(),
            "these routes are not in the catalogue: {missing:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shirt() -> Item {
        Item {
            id: "shirt".into(),
            label: "A shirt".into(),
            category: Some("kleidung".into()),
            price_cents: Some(7990),
            ..Default::default()
        }
    }

    #[test]
    fn a_named_field_changes_and_the_others_stay() {
        let new = merge_patch(&shirt(), json!({ "price_cents": 8990 })).expect("patch fits");
        assert_eq!(new.price_cents, Some(8990));
        assert_eq!(new.label, "A shirt");
        assert_eq!(new.category.as_deref(), Some("kleidung"));
    }

    #[test]
    fn an_unknown_field_is_refused_not_ignored() {
        let e = merge_patch(&shirt(), json!({ "tiefe": 42 })).expect_err("`tiefe` is no field");
        assert!(e.1.contains("tiefe"));
    }

    /// An edit queued on a phone before the English schema still lands on the right field.
    #[test]
    fn a_german_field_name_is_translated() {
        let new = merge_patch(&shirt(), json!({ "preis_cent": 100, "groesse": "M" })).expect("ok");
        assert_eq!(new.price_cents, Some(100));
        assert_eq!(new.size.as_deref(), Some("M"));
    }

    #[test]
    fn null_clears_and_a_missing_key_does_not() {
        let cleared = merge_patch(&shirt(), json!({ "price_cents": null })).expect("null is ok");
        assert_eq!(cleared.price_cents, None);
        let untouched = merge_patch(&shirt(), json!({ "label": "Other" })).expect("ok");
        assert_eq!(untouched.price_cents, Some(7990));
    }
}

/// The revision contract as a client sees it: status, body and header, against a temp file.
/// The race itself is in `tests/revision.rs`.
#[cfg(test)]
mod revision_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, HeaderMap, Request};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn router(name: &str) -> (Router, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "inventory-api-revision-{name}-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let device_store = devices::store::DevicesStore::open(&path).unwrap();
        let r = build_router(Arc::new(AppState {
            device_store,
            database: path.clone(),
        }));
        (r, path)
    }

    async fn send(
        r: &Router,
        method: &str,
        path: &str,
        if_match: Option<&str>,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(v) = if_match {
            request = request.header("if-match", v);
        }
        let response = r
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .expect("the router answers");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, headers, value)
    }

    fn wardrobe(label: &str) -> Value {
        json!({ "id": "wardrobe", "kind": "piece", "label": label, "width": 100 })
    }

    #[tokio::test]
    async fn without_a_condition_every_answer_carries_the_revision() {
        let (r, path) = router("none");
        let (s, h, v) = send(&r, "PUT", "/api/items/wardrobe", None, wardrobe("a")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["revision"], 1);
        assert_eq!(h[header::ETAG], "\"1\"");

        let (s, _, v) = send(
            &r,
            "PATCH",
            "/api/items/wardrobe",
            None,
            json!({"width": 120}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["revision"], 2);

        let (s, _, v) = send(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v[0]["item"]["revision"], 2, "{v}");
        assert_eq!(v[0]["item"]["width"], 120, "{v}");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_matching_revision_writes_and_raises_it_by_one() {
        let (r, path) = router("match");
        send(&r, "PUT", "/api/items/wardrobe", None, wardrobe("a")).await;

        let (s, h, v) = send(
            &r,
            "PUT",
            "/api/items/wardrobe",
            Some("\"1\""),
            wardrobe("b"),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 2);
        assert_eq!(h[header::ETAG], "\"2\"");

        let (s, _, v) = send(
            &r,
            "PATCH",
            "/api/items/wardrobe",
            Some("2"),
            json!({"width": 90}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 3);

        // The body field is the fallback and is not refused as an entry field.
        let (s, _, v) = send(
            &r,
            "PATCH",
            "/api/items/wardrobe",
            None,
            json!({"width": 80, "expected_revision": 3}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 4);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_stale_revision_gets_409_with_the_current_values() {
        let (r, path) = router("stale");
        send(&r, "PUT", "/api/items/wardrobe", None, wardrobe("a")).await;
        send(
            &r,
            "PUT",
            "/api/items/wardrobe",
            Some("1"),
            wardrobe("from the Mac"),
        )
        .await;

        for (method, body) in [
            ("PUT", wardrobe("from the phone")),
            ("PATCH", json!({"label": "from the phone"})),
        ] {
            let (s, _, v) = send(&r, method, "/api/items/wardrobe", Some("\"1\""), body).await;
            assert_eq!(s, StatusCode::CONFLICT, "{method}: {v}");
            assert_eq!(v["current"]["item"]["label"], "from the Mac");
            assert_eq!(v["current"]["item"]["revision"], 2);
            let error = v["error"].as_str().expect("error is text");
            assert!(error.contains("changed meanwhile"), "{error}");
        }
        let (s, _, v) = send(
            &r,
            "PATCH",
            "/api/items/wardrobe",
            None,
            json!({"label": "x", "expected_revision": 1}),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");

        let (_, _, v) = send(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(
            v[0]["item"]["label"], "from the Mac",
            "nothing was overwritten"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_missing_entry_stays_404() {
        let (r, path) = router("missing");
        let (s, _, _) = send(&r, "PATCH", "/api/items/none", None, json!({"width": 1})).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _, _) = send(
            &r,
            "PATCH",
            "/api/items/none",
            Some("1"),
            json!({"width": 1}),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // A conditional PUT creates nothing.
        let (s, _, _) = send(&r, "PUT", "/api/items/wardrobe", Some("1"), wardrobe("a")).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, _, v) = send(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(v, json!([]));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn an_unreadable_or_contradicting_condition_is_refused() {
        let (r, path) = router("unreadable");
        send(&r, "PUT", "/api/items/wardrobe", None, wardrobe("a")).await;
        let (s, _, _) = send(&r, "PUT", "/api/items/wardrobe", Some("*"), wardrobe("b")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let mut body = wardrobe("b");
        body["expected_revision"] = json!(2);
        let (s, _, _) = send(&r, "PUT", "/api/items/wardrobe", Some("1"), body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn wish_groups_roundtrip_with_items_and_costs() {
        let (r, path) = router("wish-groups");
        // Create an item with price
        send(
            &r,
            "POST",
            "/api/items",
            None,
            json!({
                "id": "merino-tee",
                "kind": "piece",
                "label": "Merino Wool T-Shirt",
                "price_cents": 7500,
                "state": "wanted"
            }),
        )
        .await;

        // Create wish group
        let (s, _, v) = send(
            &r,
            "POST",
            "/api/wish-groups",
            None,
            json!({
                "id": "travel-capsule",
                "title": "Travel Capsule",
                "constraint_text": "travel-friendly, fewer but better",
                "item_ids": ["merino-tee"]
            }),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");

        // List wish groups
        let (s, _, v) = send(&r, "GET", "/api/wish-groups", None, Value::Null).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["wish_groups"].as_array().unwrap().len(), 1);
        let group = &v["wish_groups"][0];
        assert_eq!(group["id"], "travel-capsule");
        assert_eq!(group["title"], "Travel Capsule");
        assert_eq!(
            group["constraint_text"],
            "travel-friendly, fewer but better"
        );
        assert_eq!(group["items_count"], 1);
        assert_eq!(group["total_low_cents"], 7500);

        // Delete wish group
        let (s, _, _) = send(
            &r,
            "DELETE",
            "/api/wish-groups/travel-capsule",
            None,
            Value::Null,
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT);

        let (_, _, v) = send(&r, "GET", "/api/wish-groups", None, Value::Null).await;
        assert_eq!(v["wish_groups"].as_array().unwrap().len(), 0);

        let _ = std::fs::remove_file(&path);
    }
}
