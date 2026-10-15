//! The capability's HTTP surface. Every endpoint returns JSON or finished SVG/HTML from the
//! model; a frontend that kept its own sizes or rules would drift from it. Bound to loopback
//! through `sjel_server::serve_local`.

use crate::clearance::check_layout;
use crate::model::Model;
use crate::plan;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

struct AppState {
    flat: String,
    /// Resolved once at start, so a test can point the router at a temp file.
    database: PathBuf,
}

/// The model is read fresh per request, not cached at start: the TOML files are edited by hand,
/// and a cached copy would show a plan contradicting its own numbers. Reading takes under a
/// millisecond.
fn load(state: &AppState) -> Result<Model, (StatusCode, String)> {
    Model::load(&state.flat).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn health() -> &'static str {
    "ok"
}

/// What this capability answers. `route_manifest::undeclared_routes` reads the router from this
/// source below and fails when a route is missing here.
const ROUTES: &[route_manifest::Route] = &[
    r(
        "GET",
        "/",
        "Every layout as a page, rendered from the model on each request.",
    ),
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/api/media/{*path}",
        "An image from the private asset dir, only from there and only on request.",
    ),
    r(
        "GET",
        "/api/roomplan/reference",
        "The newest native RoomPlan reference as metadata, without absolute paths.",
    ),
    r(
        "GET",
        "/api/roomplan/asset",
        "The newest native RoomPlan reference as the unchanged USDZ file.",
    ),
    r(
        "GET",
        "/api/roomplan/revisions",
        "Synced semantic RoomPlan revisions for comparison.",
    ),
    r(
        "POST",
        "/api/roomplan/revisions/{revision_id}/review",
        "Accept or reject a RoomPlan revision without changing room.toml.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/allowed",
        "Allowed positions of a piece as runs, the hard edges for dragging. ?ref=&rot=",
    ),
    r(
        "POST",
        "/api/layouts/{name}/preview",
        "Verdict and plan for an arrangement without writing it; needed for rotating.",
    ),
    r(
        "POST",
        "/api/layouts",
        "Create a layout: {id, name?, from?, items?, note?}. Without from and items it is empty.",
    ),
    r(
        "DELETE",
        "/api/layouts/{name}",
        "Take a layout off the list. It moves to layouts/archiv/ and is never deleted.",
    ),
    r(
        "PUT",
        "/api/layouts/{name}",
        "Replace a layout's positions and check again at once. The file header stays.",
    ),
    r(
        "PUT",
        "/api/placements",
        "Where the pieces really stand in the active flat. Body: {items}.",
    ),
    r(
        "POST",
        "/api/placements/preview",
        "The as-is state as plan and verdict, without a layout file. Body: {items}.",
    ),
    r(
        "POST",
        "/api/placements/allowed",
        "Allowed positions of a piece in the as-is state. Body: {items, ref, rot}.",
    ),
    r(
        "POST",
        "/api/items/{id}/impact",
        "What a field would do to the verdicts, without writing it.",
    ),
    r("GET", "/routes", "Dieser Katalog."),
    r(
        "GET",
        "/api/layouts/{name}/tolerance",
        "Up to which measurement error the verdict holds, and what breaks then.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/sun",
        "When in the year which piece is in direct sun. Needs [lage] in room.toml.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/delivery",
        "Whether every piece gets through the door and to its place.",
    ),
    r(
        "GET",
        "/api/fits",
        "Does an imagined piece fit through the door? ?width=&depth=&disassembles=, the question before buying.",
    ),
    r(
        "GET",
        "/api/declarations",
        "Which items are still judged by name, what their declaration would be, and what it changes.",
    ),
    r(
        "GET",
        "/api/purchase-order",
        "Which need first, cumulated, and when the monthly balance affords it.",
    ),
    r(
        "POST",
        "/api/search",
        "Start a search. Answers 202 with a job number, not the result.",
    ),
    r(
        "POST",
        "/api/compose",
        "Furnish a whole flat. Also a job: the beam search takes minutes.",
    ),
    r(
        "GET",
        "/api/jobs/{id}",
        "What became of a job: running, done with a result, or failed.",
    ),
    r(
        "GET",
        "/api/model",
        "The measured room: dimensions, polygon, walls, openings, and what of it is estimated.",
    ),
    r(
        "GET",
        "/api/layouts",
        "Every layout with verdict, violation counts and corridor widths.",
    ),
    r(
        "GET",
        "/api/layouts/{name}",
        "One layout: the full check and the finished plan as SVG.",
    ),
    r(
        "GET",
        "/api/flats",
        "Which flats exist and which one this process computes against.",
    ),
    r(
        "GET",
        "/api/placements/{flat}",
        "Where the pieces in this flat actually stand.",
    ),
    r(
        "PUT",
        "/api/placements/{flat}/{item}",
        "Place a piece in this flat. Body: {x, y, rot}.",
    ),
];

/// Short form, so the table above reads like a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<serde_json::Value> {
    Json(route_manifest::manifest("interior", ROUTES))
}

async fn api_model(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let (b, t) = m.room.dimensions();
    Ok(Json(serde_json::json!({
        "flat": m.room.flat,
        "area_m2": m.room.area_m2(),
        // The outer dimensions, so the dashboard can show them without measuring the polygon itself.
        "dimensions": { "width": b, "depth": t },
        "height": m.room.main_room.height,
        "polygon": m.room.main_room.polygon,
        "bathroom": m.room.bathroom,
        "terrace": m.room.terrace,
        "walls": m.room.walls,
        "openings": m.room.openings,
        "fixed_furniture": m.room.fixed_furniture,
        "todo": m.room.todo.open,
        "catalogue_size": m.catalogue.len(),
        "unmeasured": m.uncertainties().into_iter()
            .map(|(id, label, f)| serde_json::json!({ "id": id, "label": label, "fields": f }))
            .collect::<Vec<_>>(),
    })))
}

async fn api_layouts(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut out = Vec::new();
    for n in names {
        let Ok(l) = m.load_layout(&n) else { continue };
        let Ok(r) = check_layout(&m, &l) else {
            continue;
        };
        out.push(serde_json::json!({
            "id": n, "name": l.name, "pass": r.pass,
            "hard": r.hard.len(), "soft": r.soft.len(),
            "corridors": r.metrics.corridors,
            "occupied_m2": r.metrics.occupied_area_m2,
        }));
    }
    Ok(Json(out))
}

async fn api_layout(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let l = m
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let svg = plan::svg(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

fn store(s: &AppState) -> Result<crate::store::Store, (StatusCode, String)> {
    crate::store::Store::open(&s.database)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn boom<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// The flats under `flats/`, and which one this process serves. Every other endpoint answers for
/// exactly one flat, from `SJEL_INTERIOR_FLAT` or the only one.
async fn api_flats(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let alle =
        crate::model::flats().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "flats": alle, "active": s.flat })))
}

async fn api_placements(
    State(s): State<Arc<AppState>>,
    Path(flat): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(store(&s)?.placements(&flat).map_err(boom)?))
}

/// What a change would do to the verdicts, without writing it. Clearance fields are the ones whose
/// effect is invisible before setting them: `opens` on a wardrobe costs 2 or 4 layouts depending on
/// the direction. Patches the catalogue in memory and recomputes every layout.
async fn api_item_impact(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let before = verdicts(&model)?;

    let alt = model
        .catalogue
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no entry `{id}`")))?;
    let created = sjel_inventory::api::merge_patch(alt, patch)?;

    let mut model = load(&s)?;
    model.catalogue.insert(id.clone(), created);
    let after = verdicts(&model)?;

    let mut changed = Vec::new();
    for (name, (pass, hard, soft)) in &before {
        let Some((p2, h2, s2)) = after.get(name) else {
            continue;
        };
        if pass != p2 || hard != h2 || soft != s2 {
            changed.push(serde_json::json!({
                "layout": name,
                "before": { "pass": pass, "hard": hard, "soft": soft },
                "after": { "pass": p2, "hard": h2, "soft": s2 },
            }));
        }
    }
    Ok(Json(serde_json::json!({
        "item": id,
        "layouts": before.len(),
        "passing_before": before.values().filter(|(p, _, _)| *p).count(),
        "passing_after": after.values().filter(|(p, _, _)| *p).count(),
        "changed": changed,
    })))
}

type Verdict = std::collections::BTreeMap<String, (bool, Vec<String>, Vec<String>)>;

fn verdicts(model: &Model) -> Result<Verdict, (StatusCode, String)> {
    let mut out = Verdict::new();
    for name in model.layout_names().map_err(|e| boom(e.to_string()))? {
        let Ok(l) = model.load_layout(&name) else {
            continue;
        };
        let Ok(r) = check_layout(model, &l) else {
            continue;
        };
        let mut hard: Vec<String> = r.hard.iter().map(|v| v.rule.clone()).collect();
        let mut soft: Vec<String> = r.soft.iter().map(|v| v.rule.clone()).collect();
        hard.sort();
        soft.sort();
        out.insert(name, (r.pass, hard, soft));
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
struct PlacementBody {
    x: i32,
    y: i32,
    #[serde(default)]
    rot: i32,
}

async fn api_put_placement(
    State(s): State<Arc<AppState>>,
    Path((flat, item)): Path<(String, String)>,
    Json(body): Json<PlacementBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    store(&s)?
        .place(&crate::store::Placement {
            item_id: item.clone(),
            flat: flat.clone(),
            x: body.x,
            y: body.y,
            rot: body.rot,
        })
        .map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "flat": flat, "item": item, "ok": true }),
    ))
}

#[derive(serde::Deserialize)]
struct LayoutBody {
    items: Vec<crate::model::PlacedItem>,
}

/// Replace an existing layout's positions and check again at once. Answers with verdict and plan;
/// the file header stays (`layout_io` keeps it or refuses).
async fn api_put_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    crate::layout_io::update(&model, &id, &body.items)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let l = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// An image from the configured private asset dir, on request and only from there.
///
/// The path comes from the client, so it is resolved and checked: `canonicalize` on both sides,
/// then `starts_with`, which catches `..`, absolute paths and symlinks pointing out. Uses
/// `tokio::fs`, because this reads whole images and `std::fs` would block a runtime worker.
async fn latest_roomplan_capture(
    flat: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(flat);
    let mut entries = tokio::fs::read_dir(&root)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "no RoomPlan captures".to_string()))?;
    let mut dates = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(boom)? {
        if entry.file_type().await.map_err(boom)?.is_dir() {
            dates.push(entry.path());
        }
    }
    dates.sort();
    for directory in dates.into_iter().rev() {
        let asset = directory.join("captured-room.usdz");
        let observation = directory.join("observation.json");
        if tokio::fs::try_exists(&asset).await.map_err(boom)?
            && tokio::fs::try_exists(&observation).await.map_err(boom)?
        {
            return Ok((directory, asset, observation));
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        "no complete RoomPlan capture".to_string(),
    ))
}

async fn api_roomplan_reference(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (directory, asset, observation) = latest_roomplan_capture(&s.flat).await?;
    let observation: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&observation).await.map_err(boom)?)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let manifest_path = directory.join("sync-manifest.json");
    let manifest = if tokio::fs::try_exists(&manifest_path).await.map_err(boom)? {
        Some(
            serde_json::from_slice::<serde_json::Value>(
                &tokio::fs::read(manifest_path).await.map_err(boom)?,
            )
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )
    } else {
        None
    };
    let bytes = tokio::fs::read(&asset).await.map_err(boom)?;
    let sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Json(serde_json::json!({
        "flat": s.flat,
        "status": "raw-only",
        "revision": manifest.as_ref().and_then(|m| m.get("revision_id")),
        "asset": {
            "format": "usdz",
            "byte_length": bytes.len(),
            "sha256": sha256,
            "url": "/interior/api/roomplan/asset"
        },
        "observation": observation,
        "manifest": manifest,
    })))
}

async fn api_roomplan_asset(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (_, asset, _) = latest_roomplan_capture(&s.flat).await?;
    let bytes = tokio::fs::read(asset).await.map_err(boom)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "model/vnd.usdz+zip")],
        bytes,
    ))
}

async fn roomplan_revision_drafts(
    flat: &str,
) -> Result<Vec<serde_json::Value>, (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(flat);
    if !tokio::fs::try_exists(&root).await.map_err(boom)? {
        return Ok(Vec::new());
    }
    let mut dates = tokio::fs::read_dir(&root).await.map_err(boom)?;
    let mut drafts = Vec::new();
    while let Some(date) = dates.next_entry().await.map_err(boom)? {
        if !date.file_type().await.map_err(boom)?.is_dir() {
            continue;
        }
        let direct = date.path().join("draft.json");
        if tokio::fs::try_exists(&direct).await.map_err(boom)? {
            drafts.push(read_json_file(&direct).await?);
        }
        let revisions = date.path().join("revisions");
        if !tokio::fs::try_exists(&revisions).await.map_err(boom)? {
            continue;
        }
        let mut entries = tokio::fs::read_dir(revisions).await.map_err(boom)?;
        while let Some(entry) = entries.next_entry().await.map_err(boom)? {
            if !entry.file_type().await.map_err(boom)?.is_dir() {
                continue;
            }
            let draft = entry.path().join("draft.json");
            if tokio::fs::try_exists(&draft).await.map_err(boom)? {
                drafts.push(read_json_file(&draft).await?);
            }
        }
    }
    drafts.sort_by(|left, right| {
        left.get("created_at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .cmp(
                right
                    .get("created_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
    });
    Ok(drafts)
}

async fn read_json_file(path: &std::path::Path) -> Result<serde_json::Value, (StatusCode, String)> {
    serde_json::from_slice(&tokio::fs::read(path).await.map_err(boom)?)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn api_roomplan_revisions(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(serde_json::json!({
        "flat": s.flat,
        "revisions": roomplan_revision_drafts(&s.flat).await?,
    })))
}

#[derive(Debug, Deserialize)]
struct RoomPlanReviewRequest {
    decision: String,
    #[serde(default)]
    #[serde(alias = "notiz")]
    note: Option<String>,
}

async fn api_roomplan_review(
    State(s): State<Arc<AppState>>,
    Path(revision_id): Path<String>,
    Json(request): Json<RoomPlanReviewRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !matches!(request.decision.as_str(), "accept" | "reject") {
        return Err((
            StatusCode::BAD_REQUEST,
            "decision must be accept or reject".to_string(),
        ));
    }
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let mut dates = tokio::fs::read_dir(&root)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "no RoomPlan captures".to_string()))?;
    while let Some(date) = dates.next_entry().await.map_err(boom)? {
        if !date.file_type().await.map_err(boom)?.is_dir() {
            continue;
        }
        let mut candidates = Vec::new();
        let direct = date.path().join("draft.json");
        if tokio::fs::try_exists(&direct).await.map_err(boom)? {
            candidates.push(direct);
        }
        let revisions = date.path().join("revisions");
        if tokio::fs::try_exists(&revisions).await.map_err(boom)? {
            let mut entries = tokio::fs::read_dir(revisions).await.map_err(boom)?;
            while let Some(entry) = entries.next_entry().await.map_err(boom)? {
                if entry.file_type().await.map_err(boom)?.is_dir() {
                    candidates.push(entry.path().join("draft.json"));
                }
            }
        }
        for draft_path in candidates {
            if !tokio::fs::try_exists(&draft_path).await.map_err(boom)? {
                continue;
            }
            let draft = read_json_file(&draft_path).await?;
            if draft.get("draft_id").and_then(serde_json::Value::as_str)
                != Some(revision_id.as_str())
            {
                continue;
            }
            let review = serde_json::json!({
                "decision": request.decision,
                "note": request.note,
            });
            let review_path = draft_path.with_file_name("review.json");
            let bytes = serde_json::to_vec_pretty(&review)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            tokio::fs::write(review_path, bytes).await.map_err(boom)?;
            return Ok(Json(review));
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        "RoomPlan revision not found".to_string(),
    ))
}

async fn api_media(
    Path(media_path): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let wurzel = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("media");
    let wurzel = tokio::fs::canonicalize(&wurzel)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "no media dir".to_string()))?;
    let target = tokio::fs::canonicalize(wurzel.join(&media_path))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("no medium `{media_path}`")))?;
    let is_regular_file = tokio::fs::metadata(&target)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !target.starts_with(&wurzel) || !is_regular_file {
        return Err((
            StatusCode::FORBIDDEN,
            format!("`{media_path}` is not below media/"),
        ));
    }
    let content_type = match target
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        // No default type: what is unknown here is neither guessed nor served.
        _ => {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not an image format".into(),
            ))
        }
    };
    let bytes = tokio::fs::read(&target).await.map_err(boom)?;
    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], bytes))
}

/// Where a piece's top-left corner may go: hard edges for dragging. The dashboard asks once when
/// picking up and snaps to the list; it gets runs, not geometry.
async fn api_allowed(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::Query(q): axum::extract::Query<AllowedQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let base = model
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let a = crate::search::allowed_positions(&model, &base, &q.reference, q.rot)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(a))
}

#[derive(serde::Deserialize)]
struct AllowedQuery {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    rot: i32,
}

/// The as-is state as an arrangement, with positions in the body rather than a file. Plan and
/// verdict are computed by the same function as for a layout; only the input differs.
fn as_is(items: Vec<crate::model::PlacedItem>) -> crate::model::Layout {
    crate::model::Layout {
        name: "as-is".to_string(),
        id: "as-is".to_string(),
        items,
    }
}

async fn api_preview_placements(
    State(s): State<Arc<AppState>>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = as_is(body.items);
    let r = check_layout(&model, &l).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Allowed positions of a piece in the as-is state. The arrangement comes in the body, because
/// placements are rows, not a file to reload.
async fn api_placements_allowed(
    State(s): State<Arc<AppState>>,
    Json(body): Json<PlacementsAllowedBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let base = as_is(body.items);
    let a = crate::search::allowed_positions(&model, &base, &body.reference, body.rot)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(a))
}

#[derive(serde::Deserialize)]
struct PlacementsAllowedBody {
    items: Vec<crate::model::PlacedItem>,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    rot: i32,
}

/// How a layout would look and fare, without writing it. Moving the dashboard can draw itself;
/// rotating it cannot, since sizes swap and `opens`/`expands_dir` turn with the piece.
async fn api_preview_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let template = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let l = crate::model::Layout {
        name: template.name,
        items: body.items,
        id: id.clone(),
    };
    let r = check_layout(&model, &l).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Create a layout; never overwrites one. Empty (neither `from` nor `items`), a copy (`from`), or
/// fully placed (`items`). Answers like `PUT` with `{layout, check, svg}`.
async fn api_post_layout(
    State(s): State<Arc<AppState>>,
    Json(body): Json<NewLayout>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    // An unusable name is a bad request, not a conflict.
    crate::layout_io::check_id(&body.id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let template = match &body.from {
        // The template's name, not the read error, which carries a path into the private overlay.
        Some(v) => Some(
            model
                .load_layout(v)
                .map_err(|_| (StatusCode::NOT_FOUND, format!("no template `{v}`")))?,
        ),
        None => None,
    };
    let items = match (body.items, &template) {
        (Some(own), _) => own,
        (None, Some(v)) => v.items.clone(),
        (None, None) => Vec::new(),
    };
    let note_text = body.note_text.unwrap_or_else(|| match &body.from {
        Some(v) => format!(
            "Created {} through the API, as a copy of `{v}`.",
            crate::layout_io::today()
        ),
        None => format!("Created {} through the API.", crate::layout_io::today()),
    });
    let layout = crate::model::Layout {
        name: body.name.unwrap_or_else(|| body.id.clone()),
        items,
        id: body.id.clone(),
    };
    crate::layout_io::create(&model, &body.id, &layout, &note_text)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    let l = model
        .load_layout(&body.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": body.id, "layout": l, "check": r, "svg": svg })),
    ))
}

#[derive(serde::Deserialize)]
struct NewLayout {
    id: String,
    /// The display name; without it, the id.
    #[serde(default)]
    name: Option<String>,
    /// A layout to copy.
    #[serde(default, alias = "von")]
    from: Option<String>,
    /// Beats `from`. Without both the plan is empty, which is how a hand-placed plan starts.
    #[serde(default)]
    items: Option<Vec<crate::model::PlacedItem>>,
    /// The first line of the file header. Without it the header names the date and the API.
    #[serde(default)]
    note_text: Option<String>,
}

/// Take a layout off the list without losing it: `DELETE` archives to `layouts/archiv/`, where its
/// header keeps the reasoning.
async fn api_delete_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    crate::layout_io::check_id(&id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Through the name list, not `load_layout`: a layout with a TOML typo is one someone wants to
    // clear away.
    if !model.layout_names().map_err(boom)?.iter().any(|n| n == &id) {
        return Err((StatusCode::NOT_FOUND, format!("no layout `{id}`")));
    }
    let nach = crate::layout_io::archive_layout(&model, &id)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "id": id, "archived": nach, "deleted": false }),
    ))
}

/// Where the pieces in this flat really stand, as opposed to what is proposed: a layout is a
/// proposal and a file, a placement is the state and a row.
async fn api_put_placements(
    State(s): State<Arc<AppState>>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store(&s)?;
    for it in &body.items {
        st.place(&crate::store::Placement {
            item_id: it.reference.clone(),
            flat: s.flat.clone(),
            x: it.x,
            y: it.y,
            rot: it.rot,
        })
        .map_err(boom)?;
    }
    Ok(Json(
        serde_json::json!({ "flat": s.flat, "placed": body.items.len() }),
    ))
}

async fn index(State(s): State<Arc<AppState>>) -> Result<Html<String>, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let layouts: Vec<_> = names.iter().filter_map(|n| m.load_layout(n).ok()).collect();
    plan::page(&m, &layouts, plan::Origin::Proposal)
        .map(Html)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn serve(flat: &str, port: u16) {
    let state = Arc::new(AppState {
        flat: flat.to_string(),
        database: sjel_config::database_path(),
    });
    sjel_server::serve_local("interior", port, build_router(state)).await;
}

/// Names the origin allowlist variable, `SJEL_INTERIOR_ALLOWED_ORIGIN_HOSTS`.
const CAPABILITY: &str = "interior";

/// The wired router, so a test can drive the real thing.
///
/// Every handler here serves private data, so a foreign origin is refused before the handler
/// rather than only denied the response. The lock sits below all routes because axum wraps only
/// routes registered before `.layer()`.
fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/model", get(api_model))
        .route("/api/layouts", get(api_layouts).post(api_post_layout))
        .route(
            "/api/layouts/{name}",
            get(api_layout)
                .put(api_put_layout)
                .delete(api_delete_layout),
        )
        .route("/api/layouts/{name}/preview", post(api_preview_layout))
        .route("/api/layouts/{name}/allowed", get(api_allowed))
        .route("/api/placements", put(api_put_placements))
        .route("/api/placements/preview", post(api_preview_placements))
        .route("/api/placements/allowed", post(api_placements_allowed))
        .route("/api/flats", get(api_flats))
        .route("/api/media/{*path}", get(api_media))
        .route("/api/roomplan/reference", get(api_roomplan_reference))
        .route("/api/roomplan/asset", get(api_roomplan_asset))
        .route("/api/roomplan/revisions", get(api_roomplan_revisions))
        .route(
            "/api/roomplan/revisions/{revision_id}/review",
            post(api_roomplan_review),
        )
        .route("/api/placements/{flat}", get(api_placements))
        // The item surface lives in `capabilities/inventory`. One item route stays: the impact preview,
        // which computes against clearance rules and layouts and writes nothing.
        .route("/api/items/{id}/impact", post(api_item_impact))
        .route("/api/placements/{flat}/{item}", put(api_put_placement))
        .route("/api/layouts/{name}/tolerance", get(api_tolerance))
        .route("/api/layouts/{name}/sun", get(api_sun))
        .route("/api/layouts/{name}/delivery", get(api_delivery))
        .route("/api/fits", get(api_fits))
        .route("/api/declarations", get(api_declarations))
        .route("/api/purchase-order", get(api_purchase_order))
        .route("/api/search", post(api_search))
        .route("/api/compose", post(api_compose))
        .route("/api/jobs/{id}", get(api_job))
        // NEW ROUTES ABOVE THIS LINE. Below it they lose the origin lock.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

#[cfg(test)]
mod route_manifest_tests {
    /// Reads the router from its own source, so a `.route()` without a summary fails here.
    #[test]
    fn the_catalogue_names_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("api.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but not described: {missing:?}");
    }
}

/// Router-level proof the predicate tests in `libs/sjel-server` cannot give: a route below
/// `.layer()` passes every `origin_allowed_by` test and still answers a foreign page. Uses
/// `oneshot`, no port. `/routes` is the counter-check, since data handlers open the installation's
/// database, which a test must not.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn answer(http_method: &str, media_path: &str, origin: Option<&str>) -> StatusCode {
        let mut req = Request::builder().method(http_method).uri(media_path);
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        build_router(Arc::new(AppState {
            flat: "flat".to_string(),
            database: sjel_config::database_path(),
        }))
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .expect("the router answers")
        .status()
    }

    /// Every data handler serves private data, so the lock refuses the request outright.
    #[tokio::test]
    async fn a_foreign_page_reaches_no_data_handler() {
        for (http_method, media_path) in [
            ("GET", "/api/model"),
            ("GET", "/api/layouts"),
            ("GET", "/api/declarations"),
            ("POST", "/api/search"),
            ("POST", "/api/items/schrank/impact"),
        ] {
            assert_eq!(
                answer(http_method, media_path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{http_method} {media_path} answered a foreign origin; the route is behind the lock"
            );
        }
    }

    /// The other half: 200 from `/routes` means a handler answered.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                answer("GET", "/routes", origin).await,
                StatusCode::OK,
                "the lock refused a caller it has to admit: {origin:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JobState, Jobs, JOBS_MAX};
    use serde_json::json;

    /// Create ten jobs and mark the younger ones done.
    fn full_map(run: u64) -> Jobs {
        let mut a = Jobs::default();
        for _ in 0..JOBS_MAX {
            a.start().expect("below the limit");
        }
        for id in run..JOBS_MAX as u64 {
            a.by_id.get_mut(&id).expect("just created").1 = JobState::Done {
                outcome: json!(null),
            };
        }
        a
    }

    /// Evicted is the oldest done job, not the oldest: a running job's thread would write into a
    /// number that no longer exists.
    #[test]
    fn a_running_job_is_not_evicted() {
        let mut a = full_map(1);
        let created = a.start().expect("neun fertige machen Platz");
        assert!(a.by_id.contains_key(&0), "the running one is still there");
        assert!(!a.by_id.contains_key(&1), "the oldest done one gave way");
        assert!(a.by_id.contains_key(&created));
        assert_eq!(a.by_id.len(), JOBS_MAX);
    }

    /// With all ten running, refusing is the only honest answer.
    #[test]
    fn a_map_full_of_running_jobs_refuses() {
        let mut a = full_map(JOBS_MAX as u64);
        let reason = a.start().expect_err("there is nothing to evict");
        assert!(reason.contains("already running"), "{reason}");
        assert_eq!(a.by_id.len(), JOBS_MAX, "and none disappeared");
        assert!((0..JOBS_MAX as u64).all(|id| a.by_id.contains_key(&id)));
    }

    /// One panic under the lock used to end every path touching the map. This poisons the real
    /// process-wide map on purpose; every other test in this binary still running afterwards is the
    /// promise.
    #[test]
    fn a_poisoned_job_map_still_creates_and_reads() {
        let poison = std::thread::spawn(|| {
            let _lock = super::jobs();
            panic!("someone crashed holding the job map");
        })
        .join();
        assert!(poison.is_err(), "the helper thread must really crash");

        let id = super::jobs().start().expect("the map still creates");
        assert!(
            super::jobs().by_id.contains_key(&id),
            "a recovered lock must still show the entry"
        );
    }
}

// ---------------------------------------------------------------- long computations

/// What became of a search.
///
/// `search` is exhaustive, 3.5 million candidates in about a hundred seconds, too long for one
/// HTTP request. A job is the smallest thing that works: a number, a state, a result, living in
/// the process and dying with it. Its result is proposals, not facts about the flat; whoever wants
/// one keeps it as a layout.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running { since_ms: u128 },
    Done { outcome: serde_json::Value },
    Failed { reason: String },
}

#[derive(Default)]
struct Jobs {
    next_id: u64,
    by_id: std::collections::BTreeMap<u64, (std::time::Instant, JobState)>,
}

/// How many finished jobs are kept: ten, since the dashboard fetches the latest.
const JOBS_MAX: usize = 10;

impl Jobs {
    /// Make room and take the next number, or refuse. Only finished jobs are evicted.
    fn start(&mut self) -> Result<u64, String> {
        while self.by_id.len() >= JOBS_MAX {
            // Keys ascend, so the first evictable one is also the oldest.
            let Some(alt) = self
                .by_id
                .iter()
                .find(|(_, (_, s))| !matches!(s, JobState::Running { .. }))
                .map(|(id, _)| *id)
            else {
                return Err(format!(
                    "{JOBS_MAX} jobs are already running; wait until one is done"
                ));
            };
            self.by_id.remove(&alt);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.by_id.insert(
            id,
            (std::time::Instant::now(), JobState::Running { since_ms: 0 }),
        );
        Ok(id)
    }
}

/// The job map, locked, recovering from a poisoned lock instead of propagating it.
///
/// One place locks, so one place must be right. Nothing durable lies under the lock: a map of
/// numbers to in-process states, which safe Rust cannot leave half-written. The worst case is an
/// entry saying `Running` whose thread is gone, which `start` already does not evict. Panicking
/// instead would take search, compose and job lookups down for the life of the process. The first
/// panic still reaches stderr.
fn jobs() -> std::sync::MutexGuard<'static, Jobs> {
    static A: std::sync::OnceLock<std::sync::Mutex<Jobs>> = std::sync::OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(Jobs::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start a computation in the background and return its number at once. `spawn_blocking`,
/// because `search` and `compose` use rayon for minutes.
fn in_the_background<F>(f: F) -> Result<u64, (StatusCode, String)>
where
    F: FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
{
    let id = jobs()
        .start()
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    tokio::task::spawn_blocking(move || {
        let outcome = f();
        let mut a = jobs();
        if let Some((_, by_id)) = a.by_id.get_mut(&id) {
            *by_id = match outcome {
                Ok(v) => JobState::Done { outcome: v },
                Err(e) => JobState::Failed { reason: e },
            };
        }
    });
    Ok(id)
}

async fn api_job(Path(id): Path<u64>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let a = jobs();
    let (start, by_id) = a
        .by_id
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no job {id}")))?;
    // Elapsed time is computed on read, not tracked.
    let by_id = match by_id {
        JobState::Running { .. } => JobState::Running {
            since_ms: start.elapsed().as_millis(),
        },
        done => done.clone(),
    };
    Ok(Json(serde_json::json!({ "id": id, "state": by_id })))
}

#[derive(serde::Deserialize)]
struct SearchRequest {
    layout: String,
    #[serde(default)]
    move_refs: Vec<String>,
    #[serde(default = "raster_standard")]
    step: i32,
    #[serde(default = "search_limit")]
    limit: usize,
}

fn raster_standard() -> i32 {
    20
}
/// The same number as `--limit` on the command line (`main.rs`, `cmd_search`); 0 would mean
/// unlimited.
fn search_limit() -> usize {
    6
}

/// Start a search; answers with the job number, not the result.
async fn api_search(
    State(s): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if req.move_refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`move_refs` is empty; without movable pieces there is nothing to search".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = in_the_background(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let base = model.load_layout(&req.layout).map_err(|e| e.to_string())?;
        let spec = crate::search::Spec {
            move_refs: req.move_refs,
            step: req.step,
            bands: Default::default(),
            limit: req.limit,
        };
        let rep = crate::search::search(&model, &base, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(rep).map_err(|e| e.to_string())
    })?;
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({ "job": id }))))
}

#[derive(serde::Deserialize)]
struct ComposeRequest {
    refs: Vec<String>,
    #[serde(default = "compose_raster")]
    step: i32,
    #[serde(default = "compose_beam")]
    beam: usize,
    #[serde(default = "compose_limit")]
    limit: usize,
    #[serde(default = "compose_rotations")]
    rotations: Vec<i32>,
}

fn compose_raster() -> i32 {
    25
}
fn compose_beam() -> usize {
    60
}
/// The same number as `--limit` on the command line (`main.rs`, `cmd_compose`).
fn compose_limit() -> usize {
    5
}
fn compose_rotations() -> Vec<i32> {
    vec![0, 90]
}

/// Furnish a whole flat. Also a job: the beam search takes minutes.
async fn api_compose(
    State(s): State<Arc<AppState>>,
    Json(req): Json<ComposeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if req.refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`refs` is empty; without pieces there is nothing to place".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = in_the_background(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let spec = crate::search::ComposeSpec {
            refs: req.refs,
            step: req.step,
            beam: req.beam,
            rotations: req.rotations,
            limit: req.limit,
        };
        let out = crate::search::compose(&model, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(out).map_err(|e| e.to_string())
    })?;
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({ "job": id }))))
}

// ---------------------------------------------------------------- reports

async fn api_tolerance(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    Ok(Json(
        crate::tolerance::robustness(&model, &l).map_err(boom)?,
    ))
}

async fn api_sun(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    Ok(Json(crate::sun::report(&model, &l).map_err(boom)?))
}

async fn api_delivery(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    let mut out = Vec::new();
    for it in &l.items {
        out.push(crate::delivery::delivery(&model, &l, &it.reference).map_err(boom)?);
    }
    Ok(Json(out))
}

#[derive(serde::Deserialize)]
struct PieceSize {
    #[serde(alias = "b")]
    width: i32,
    #[serde(alias = "t")]
    depth: i32,
    #[serde(default)]
    #[serde(alias = "zerlegbar")]
    disassembles: bool,
}

/// Does an imagined piece fit through the door? The question before buying.
async fn api_fits(
    State(s): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<PieceSize>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::delivery::through_the_door(
        &model,
        q.width,
        q.depth,
        q.disassembles,
    )))
}

async fn api_declarations(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::declarations::overview(&model).map_err(boom)?))
}

async fn api_purchase_order(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let st = store(&s)?;
    let conn = st.borrow_connection().map_err(boom)?;
    let balance = crate::budget::monthly_balance(&conn).map_err(boom)?;
    Ok(Json(
        crate::budget::purchase_order(&model, balance).map_err(boom)?,
    ))
}

fn not_found(e: crate::model::ModelError) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, e.to_string())
}
