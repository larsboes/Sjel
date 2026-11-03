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
use base64::Engine as _;
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
        "/api/roomplan/revisions/{revision_id}/asset",
        "A saved RoomPlan revision as its unchanged USDZ file.",
    ),
    r(
        "GET",
        "/api/roomplan/revisions",
        "Synced semantic RoomPlan revisions for comparison.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/roomplan/captures",
        summary: "Store an immutable phone capture; exact retries are idempotent and the response \
                  includes comparison against this flat.",
        request_schema: Some(route_manifest::schema_of::<CaptureUpload>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/roomplan/reconcile",
        summary: "Align two immutable scans and return unconfirmed furniture match suggestions.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanReconcileRequest>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/roomplan/propose-placement",
        summary: "Map a confirmed scan object into measured flat coordinates for reviewed placement.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanPlacementProposalRequest>),
    },
    r(
        "GET",
        "/api/roomplan/spaces",
        "Stable named room identities with their registered scan revisions and confirmed object links.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/roomplan/spaces",
        summary: "Create a stable room identity from one stored scan revision.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanSpaceCreateRequest>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/roomplan/spaces/{space_id}/revisions",
        summary: "Register another immutable scan in an existing stable room identity.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanSpaceRevisionRequest>),
    },
    route_manifest::Route {
        method: "PUT",
        path: "/api/roomplan/spaces/{space_id}/links",
        summary: "Confirm that one scan observation belongs to one inventory item.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanObjectLinkRequest>),
    },
    route_manifest::Route {
        method: "PUT",
        path: "/api/roomplan/spaces/{space_id}/placements/{item_id}",
        summary: "Apply a confirmed furniture placement if its expected placement revision is current.",
        request_schema: Some(route_manifest::schema_of::<RoomPlanPlacementRequest>),
    },
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
        "GET",
        "/api/placements/{flat}/{item}/history",
        "Append-only placement snapshots for one inventory item.",
    ),
    r(
        "POST",
        "/api/placements/preview",
        "The as-is state as plan and verdict, with route measurements and sunlight when available; writes no layout. Body: {items}.",
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

async fn api_placement_history(
    State(s): State<Arc<AppState>>,
    Path((flat, item)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(
        store(&s)?.placement_history(&flat, &item).map_err(boom)?,
    ))
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
            revision: 0,
            since: String::new(),
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
    let mut captures = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(boom)? {
        if entry.file_type().await.map_err(boom)?.is_dir()
            && !entry.file_name().to_string_lossy().starts_with('.')
        {
            let directory = entry.path();
            let asset = directory.join("captured-room.usdz");
            let observation = directory.join("observation.json");
            if !tokio::fs::try_exists(&asset).await.map_err(boom)?
                || !tokio::fs::try_exists(&observation).await.map_err(boom)?
            {
                continue;
            }
            let bytes = tokio::fs::read(&observation).await.map_err(boom)?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            let created_at = value
                .get("created_at")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            captures.push((created_at, directory, asset, observation));
        }
    }
    captures.sort_by(|left, right| left.0.cmp(&right.0));
    if let Some((_, directory, asset, observation)) = captures.pop() {
        return Ok((directory, asset, observation));
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

async fn api_roomplan_revision_asset(
    State(s): State<Arc<AppState>>,
    Path(revision_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .join("captures")
        .join(&s.flat);
    let asset = roomplan_capture_asset(&root, &revision_id).await?;
    let bytes = tokio::fs::read(asset).await.map_err(boom)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "model/vnd.usdz+zip")],
        bytes,
    ))
}

/// A capture the phone made, as `libs/roomplan-contract` names it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CaptureUpload {
    /// The `roomplan-capture/v1` draft the native layer built from `CapturedRoom`.
    draft: roomplan_contract::CaptureDraft,
    /// The parametric export (`captured-room.usdz`), base64: the shell proxies JSON and a paired
    /// device posts JSON, so a second content type would need a second path through both.
    usdz_base64: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanReconcileRequest {
    before_revision_id: String,
    after_revision_id: String,
}

async fn api_roomplan_reconcile(
    State(s): State<Arc<AppState>>,
    Json(request): Json<RoomPlanReconcileRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if request.before_revision_id == request.after_revision_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose two different revisions to compare.".into(),
        ));
    }
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let before = roomplan_capture_asset(&root, &request.before_revision_id).await?;
    let after = roomplan_capture_asset(&root, &request.after_revision_id).await?;
    let before_scan = crate::roomplan::read_usdz(&before)
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let after_scan = crate::roomplan::read_usdz(&after)
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    Ok(Json(serde_json::json!({
        "before_revision_id": request.before_revision_id,
        "after_revision_id": request.after_revision_id,
        "reconciliation": crate::roomplan::reconcile(&before_scan, &after_scan)
    })))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanPlacementProposalRequest {
    space_id: String,
    revision_id: String,
    observation_id: String,
    item_id: String,
}

async fn api_roomplan_propose_placement(
    State(s): State<Arc<AppState>>,
    Json(request): Json<RoomPlanPlacementProposalRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    validate_roomplan_id(&request.space_id)?;
    validate_roomplan_id(&request.revision_id)?;
    validate_roomplan_id(&request.observation_id)?;

    let store = store(&s)?;
    let space = store
        .roomplan_spaces(&s.flat)
        .map_err(boom)?
        .into_iter()
        .find(|space| space.space_id == request.space_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "RoomPlan space was not found.".into(),
            )
        })?;
    if !space.revision_ids.contains(&request.revision_id) {
        return Err((
            StatusCode::CONFLICT,
            "Scan revision is not registered in this room.".into(),
        ));
    }
    let linked = store
        .roomplan_object_links(&request.space_id)
        .map_err(boom)?
        .iter()
        .any(|link| {
            link.revision_id == request.revision_id
                && link.observation_id == request.observation_id
                && link.item_id == request.item_id
        });
    if !linked {
        return Err((
            StatusCode::CONFLICT,
            "Confirm this scan observation against the inventory item before proposing its position."
                .into(),
        ));
    }

    let model = load(&s)?;
    if model.states.get(&request.item_id) != Some(&crate::store::State::Owned) {
        return Err((
            StatusCode::CONFLICT,
            "Only currently owned inventory items can receive a scan-based position proposal."
                .into(),
        ));
    }
    let item = model.catalogue.get(&request.item_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "The linked inventory item has no catalogue entry.".into(),
        )
    })?;
    let (width_cm, depth_cm) = item.width.zip(item.depth).ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "The linked item needs measured width and depth before its scan pose can be placed."
                .into(),
        )
    })?;
    if width_cm <= 0 || depth_cm <= 0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "The linked item's measured width and depth must be positive.".into(),
        ));
    }

    let root = crate::model::assets_dir()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .join("captures")
        .join(&s.flat);
    let asset = roomplan_capture_asset(&root, &request.revision_id).await?;
    let scan = crate::roomplan::read_usdz(&asset)
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let alignment = crate::roomplan::flat_alignment(&scan, &model.room);
    let proposal = crate::roomplan::flat_position_proposal(
        &scan,
        &model.room,
        &request.observation_id,
        width_cm,
        depth_cm,
    );
    let Some(proposal) = proposal else {
        let reason = alignment
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .filter(|reason| !reason.is_empty())
            .unwrap_or("The scan pose could not be mapped to this flat.");
        return Err((StatusCode::UNPROCESSABLE_ENTITY, reason.to_owned()));
    };
    Ok(Json(serde_json::json!({
        "space_id": request.space_id,
        "revision_id": request.revision_id,
        "observation_id": request.observation_id,
        "item_id": request.item_id,
        "flat_alignment": alignment,
        "proposal": proposal,
        "requires_review": true
    })))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanSpaceCreateRequest {
    revision_id: String,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanSpaceRevisionRequest {
    revision_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanObjectLinkRequest {
    revision_id: String,
    observation_id: String,
    item_id: String,
}

async fn api_roomplan_spaces(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let store = store(&s)?;
    let spaces = store.roomplan_spaces(&s.flat).map_err(boom)?;
    let mut result = Vec::with_capacity(spaces.len());
    for space in spaces {
        let links = store.roomplan_object_links(&space.space_id).map_err(boom)?;
        result.push(serde_json::json!({ "space": space, "object_links": links }));
    }
    Ok(Json(
        serde_json::json!({ "flat": s.flat, "spaces": result }),
    ))
}

async fn api_roomplan_space_create(
    State(s): State<Arc<AppState>>,
    Json(request): Json<RoomPlanSpaceCreateRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let asset = roomplan_capture_asset(&root, &request.revision_id).await?;
    let directory = asset.parent().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Capture path has no parent.".into(),
        )
    })?;
    let draft: serde_json::Value = read_json_file(&directory.join("draft.json")).await?;
    let store = store(&s)?;
    let spaces = store.roomplan_spaces(&s.flat).map_err(boom)?;
    let suggested = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Room {}", spaces.len() + 1));
    if suggested.chars().count() > 80 || suggested.chars().any(char::is_control) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Room labels must be at most 80 characters and contain no control characters.".into(),
        ));
    }
    let space_id = format!("space-{}", request.revision_id);
    store
        .create_roomplan_space(&space_id, &s.flat, &suggested, &request.revision_id)
        .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
    Ok(Json(serde_json::json!({
        "space_id": space_id,
        "label": suggested,
        "revision_id": request.revision_id,
        "room_id": draft.pointer("/room/id").and_then(serde_json::Value::as_str).unwrap_or("")
    })))
}

async fn api_roomplan_space_add_revision(
    State(s): State<Arc<AppState>>,
    Path(space_id): Path<String>,
    Json(request): Json<RoomPlanSpaceRevisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    validate_roomplan_id(&space_id)?;
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let store = store(&s)?;
    let current_asset = roomplan_capture_asset(&root, &request.revision_id).await?;
    let spaces = store.roomplan_spaces(&s.flat).map_err(boom)?;
    let space = spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "RoomPlan space was not found.".into(),
            )
        })?;
    if space.revision_ids.contains(&request.revision_id) {
        return Ok(Json(serde_json::json!({
            "space_id": space_id,
            "revision_id": request.revision_id,
            "reference_revision_id": space.revision_ids.iter().rev().nth(1),
            "alignment_saved": space.alignments.iter().any(|entry| entry.revision_id == request.revision_id),
            "already_registered": true
        })));
    }
    let reference_revision_id = space.revision_ids.last().cloned();
    let alignment_report = if let Some(reference_id) = reference_revision_id.as_deref() {
        let reference_asset = roomplan_capture_asset(&root, reference_id).await?;
        let reference_scan = crate::roomplan::read_usdz(&reference_asset)
            .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
        let current_scan = crate::roomplan::read_usdz(&current_asset)
            .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
        Some(serde_json::json!({
            "before_revision_id": reference_id,
            "after_revision_id": request.revision_id,
            "reconciliation": crate::roomplan::reconcile(&reference_scan, &current_scan)
        }))
    } else {
        None
    };
    store
        .add_roomplan_revision(
            &space_id,
            &s.flat,
            &request.revision_id,
            reference_revision_id.as_deref(),
            alignment_report.as_ref(),
        )
        .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
    Ok(Json(serde_json::json!({
        "space_id": space_id,
        "revision_id": request.revision_id,
        "reference_revision_id": reference_revision_id,
        "alignment_saved": alignment_report.is_some()
    })))
}

async fn api_roomplan_object_link(
    State(s): State<Arc<AppState>>,
    Path(space_id): Path<String>,
    Json(request): Json<RoomPlanObjectLinkRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    validate_roomplan_id(&space_id)?;
    let store = store(&s)?;
    let spaces = store.roomplan_spaces(&s.flat).map_err(boom)?;
    let space = spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "RoomPlan space was not found.".into(),
            )
        })?;
    if !space.revision_ids.contains(&request.revision_id) {
        return Err((
            StatusCode::CONFLICT,
            "Scan revision is not registered in this room.".into(),
        ));
    }
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let asset = roomplan_capture_asset(&root, &request.revision_id).await?;
    let directory = asset.parent().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Capture path has no parent.".into(),
        )
    })?;
    let draft: serde_json::Value = read_json_file(&directory.join("draft.json")).await?;
    let is_object = draft
        .pointer("/room/objects")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|objects| {
            objects.iter().any(|object| {
                object.get("id").and_then(serde_json::Value::as_str)
                    == Some(request.observation_id.as_str())
            })
        });
    if !is_object {
        return Err((
            StatusCode::BAD_REQUEST,
            "Observation ID is not an object in this scan revision.".into(),
        ));
    }
    store
        .link_roomplan_object(
            &space_id,
            &request.revision_id,
            &request.observation_id,
            &request.item_id,
        )
        .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
    Ok(Json(serde_json::json!({
        "space_id": space_id,
        "revision_id": request.revision_id,
        "observation_id": request.observation_id,
        "item_id": request.item_id
    })))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RoomPlanPlacementRequest {
    revision_id: String,
    observation_id: String,
    expected_revision: i64,
    x: i32,
    y: i32,
    #[serde(default)]
    rot: i32,
}

async fn api_roomplan_apply_placement(
    State(s): State<Arc<AppState>>,
    Path((space_id, item_id)): Path<(String, String)>,
    Json(request): Json<RoomPlanPlacementRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    validate_roomplan_id(&space_id)?;
    validate_roomplan_id(&request.revision_id)?;
    if request.expected_revision < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "expected_revision must be zero or greater".into(),
        ));
    }
    let store = store(&s)?;
    let spaces = store.roomplan_spaces(&s.flat).map_err(boom)?;
    let space = spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "RoomPlan space was not found.".into(),
            )
        })?;
    if !space.revision_ids.contains(&request.revision_id) {
        return Err((
            StatusCode::CONFLICT,
            "Scan revision is not registered in this room.".into(),
        ));
    }
    let linked = store
        .roomplan_object_links(&space_id)
        .map_err(boom)?
        .iter()
        .any(|link| {
            link.revision_id == request.revision_id
                && link.observation_id == request.observation_id
                && link.item_id == item_id
        });
    if !linked {
        return Err((
            StatusCode::CONFLICT,
            "Confirm this scan observation against the inventory item before changing its placement."
                .into(),
        ));
    }
    let model = load(&s)?;
    if model.states.get(&item_id) != Some(&crate::store::State::Owned) {
        return Err((
            StatusCode::CONFLICT,
            "Only currently owned inventory items can be placed from a room scan.".into(),
        ));
    }
    let capture_root = crate::model::assets_dir()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .join("captures")
        .join(&s.flat);
    let capture_asset = roomplan_capture_asset(&capture_root, &request.revision_id).await?;
    let scan = crate::roomplan::read_usdz(&capture_asset)
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let flat_alignment = crate::roomplan::flat_alignment(&scan, &model.room);
    let scan_proposal = model
        .catalogue
        .get(&item_id)
        .and_then(|item| item.width.zip(item.depth))
        .and_then(|(width, depth)| {
            crate::roomplan::flat_position_proposal(
                &scan,
                &model.room,
                &request.observation_id,
                width,
                depth,
            )
        });
    let current = store.placements(&s.flat).map_err(boom)?;
    let to_layout = |placements: &[crate::store::Placement]| {
        as_is(
            placements
                .iter()
                .map(|placement| crate::model::PlacedItem {
                    reference: placement.item_id.clone(),
                    x: placement.x,
                    y: placement.y,
                    rot: placement.rot,
                    size: None,
                    kind: None,
                })
                .collect(),
        )
    };
    let before = check_layout(&model, &to_layout(&current))
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let mut proposed = current.clone();
    if let Some(placement) = proposed
        .iter_mut()
        .find(|placement| placement.item_id == item_id)
    {
        placement.x = request.x;
        placement.y = request.y;
        placement.rot = request.rot;
    } else {
        proposed.push(crate::store::Placement {
            item_id: item_id.clone(),
            flat: s.flat.clone(),
            x: request.x,
            y: request.y,
            rot: request.rot,
            revision: 0,
            since: String::new(),
        });
    }
    let after = check_layout(&model, &to_layout(&proposed))
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let existing_hard = before
        .hard
        .iter()
        .map(|finding| (finding.rule.as_str(), finding.item.as_deref()))
        .collect::<std::collections::BTreeSet<_>>();
    let introduced = after
        .hard
        .iter()
        .filter(|finding| {
            !existing_hard.contains(&(finding.rule.as_str(), finding.item.as_deref()))
        })
        .collect::<Vec<_>>();
    if !introduced.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "Placement would add hard interior conflicts: {}",
                introduced
                    .iter()
                    .map(|finding| finding.rule.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    let validation = serde_json::json!({
        "before": before,
        "after": after,
        "flat_alignment": flat_alignment,
        "scan_proposal": scan_proposal,
        "accepted_position": { "x": request.x, "y": request.y, "rot": request.rot }
    });
    let placement = store
        .place_if_revision(
            &crate::store::Placement {
                item_id: item_id.clone(),
                flat: s.flat.clone(),
                x: request.x,
                y: request.y,
                rot: request.rot,
                revision: 0,
                since: String::new(),
            },
            request.expected_revision,
            &request.revision_id,
            &request.observation_id,
            Some(&validation),
        )
        .map_err(|error| {
            let message = error.to_string();
            let status = if message.contains("placement revision conflict") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, message)
        })?;
    Ok(Json(serde_json::json!({
        "placement": placement,
        "check": after,
        "source_revision_id": request.revision_id,
        "source_observation_id": request.observation_id
    })))
}

fn validate_roomplan_id(id: &str) -> Result<(), (StatusCode, String)> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err((StatusCode::BAD_REQUEST, "Invalid RoomPlan ID.".into()))
    }
}

async fn roomplan_capture_asset(
    root: &std::path::Path,
    revision_id: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    if revision_id.is_empty()
        || !revision_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid revision id.".into()));
    }
    let direct = root.join(revision_id);
    let direct_draft = direct.join("draft.json");
    if tokio::fs::try_exists(&direct_draft).await.map_err(boom)? {
        return checked_roomplan_asset(&direct, revision_id).await;
    }
    if !tokio::fs::try_exists(root).await.map_err(boom)? {
        return Err((
            StatusCode::NOT_FOUND,
            "RoomPlan revision was not found.".into(),
        ));
    }
    let mut folders = tokio::fs::read_dir(root).await.map_err(boom)?;
    while let Some(folder) = folders.next_entry().await.map_err(boom)? {
        if !folder.file_type().await.map_err(boom)?.is_dir()
            || folder.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let candidates = [
            folder.path().join(revision_id),
            folder.path().join("revisions").join(revision_id),
        ];
        for candidate in candidates {
            if tokio::fs::try_exists(candidate.join("draft.json"))
                .await
                .map_err(boom)?
            {
                return checked_roomplan_asset(&candidate, revision_id).await;
            }
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        "RoomPlan revision was not found.".into(),
    ))
}

async fn checked_roomplan_asset(
    directory: &std::path::Path,
    revision_id: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    let draft: serde_json::Value = read_json_file(&directory.join("draft.json")).await?;
    if draft.get("draft_id").and_then(serde_json::Value::as_str) != Some(revision_id) {
        return Err((
            StatusCode::CONFLICT,
            "Stored revision id does not match its archive entry.".into(),
        ));
    }
    let asset = directory.join("captured-room.usdz");
    if !tokio::fs::try_exists(&asset).await.map_err(boom)? {
        return Err((
            StatusCode::NOT_FOUND,
            "Revision has no parametric USDZ asset.".into(),
        ));
    }
    Ok(asset)
}

/// `POST /api/roomplan/captures`: the phone's capture becomes the Mac's.
///
/// Three things happen in this order, and the order is the design: the draft is validated against the
/// contract the native layer promises, the flat's own model is loaded (a capture for a flat this Mac
/// does not know is refused before anything is written), and only then is the file written and read
/// back through [`crate::roomplan::read_usdz`]. The comparison is computed from the file the Mac now
/// holds, never from the numbers the phone sent: a phone that lies about its walls would otherwise
/// have its numbers reviewed instead of its walls.
async fn api_roomplan_capture(
    State(s): State<Arc<AppState>>,
    Json(upload): Json<CaptureUpload>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    upload
        .draft
        .validate()
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    if !upload
        .draft
        .draft_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "draft_id may contain only ASCII letters, numbers, hyphens and underscores".into(),
        ));
    }
    let model = load(&s)?;
    let usdz = base64::engine::general_purpose::STANDARD
        .decode(upload.usdz_base64.as_bytes())
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                format!("usdz_base64 is not base64: {error}"),
            )
        })?;
    let asset = upload
        .draft
        .assets
        .iter()
        .find(|asset| asset.role == roomplan_contract::CaptureAssetRole::Parametric)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "draft has no parametric USDZ asset".into(),
            )
        })?;
    let digest = Sha256::digest(&usdz);
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if asset.byte_length != usdz.len() as u64 || !asset.sha256.eq_ignore_ascii_case(&digest) {
        return Err((
            StatusCode::BAD_REQUEST,
            "parametric USDZ length or SHA-256 does not match the capture draft".into(),
        ));
    }
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let (directory, scan) = store_capture(&root, &upload.draft, &usdz).await?;
    let diff = crate::roomplan::compare(&scan, &model);
    Ok(Json(serde_json::json!({
        "flat": s.flat,
        "draft_id": upload.draft.draft_id,
        "room_id": upload.draft.room.id,
        // The comparison is the phone's review: the same document the dashboard and the CLI read,
        // named by the draft id rather than by a path inside this Mac.
        "comparison": crate::roomplan::comparison_json(
            &scan,
            &diff,
            &directory
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
    })))
}

/// Writes one capture and reads it back through the same reader the review uses.
///
/// `draft_id` names an immutable directory. An exact retry is a no-op; a different payload under
/// that ID is refused, so delayed uploads cannot replace a newer or reviewed capture.
async fn store_capture(
    root: &std::path::Path,
    draft: &roomplan_contract::CaptureDraft,
    usdz: &[u8],
) -> Result<(PathBuf, crate::roomplan::Scan), (StatusCode, String)> {
    tokio::fs::create_dir_all(root).await.map_err(boom)?;
    let directory = root.join(&draft.draft_id);
    let draft_json = serde_json::to_vec_pretty(draft).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the draft would not serialise: {error}"),
        )
    })?;
    if tokio::fs::try_exists(&directory).await.map_err(boom)? {
        return existing_capture(&directory, draft, usdz).await;
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(boom)?
        .as_nanos();
    let staging = root.join(format!(".{}.{}.tmp", draft.draft_id, nonce));
    tokio::fs::create_dir(&staging).await.map_err(boom)?;
    let asset = staging.join("captured-room.usdz");
    let staged = async {
        tokio::fs::write(&asset, usdz).await.map_err(boom)?;
        tokio::fs::write(staging.join("draft.json"), &draft_json)
            .await
            .map_err(boom)?;
        tokio::fs::write(staging.join("observation.json"), &draft_json)
            .await
            .map_err(boom)?;
        crate::roomplan::read_usdz(&asset)
            .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
    }
    .await;
    let scan = match staged {
        Ok(scan) => scan,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(error);
        }
    };
    match tokio::fs::rename(&staging, &directory).await {
        Ok(()) => Ok((directory, scan)),
        Err(_error) if tokio::fs::try_exists(&directory).await.unwrap_or(false) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            existing_capture(&directory, draft, usdz).await
        }
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            Err(boom(error))
        }
    }
}

async fn existing_capture(
    directory: &std::path::Path,
    draft: &roomplan_contract::CaptureDraft,
    usdz: &[u8],
) -> Result<(PathBuf, crate::roomplan::Scan), (StatusCode, String)> {
    let draft_path = if tokio::fs::try_exists(directory.join("draft.json"))
        .await
        .map_err(boom)?
    {
        directory.join("draft.json")
    } else {
        directory.join("observation.json")
    };
    let stored_draft: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(&draft_path).await.map_err(boom)?,
    )
    .map_err(|error| {
        (
            StatusCode::CONFLICT,
            format!("stored capture is unreadable: {error}"),
        )
    })?;
    let requested_draft = serde_json::to_value(draft)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let asset = directory.join("captured-room.usdz");
    let stored_asset = tokio::fs::read(&asset).await.map_err(boom)?;
    if stored_draft != requested_draft || stored_asset != usdz {
        return Err((
            StatusCode::CONFLICT,
            "draft_id already belongs to a different capture payload".into(),
        ));
    }
    let scan = crate::roomplan::read_usdz(&asset)
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    Ok((directory.to_path_buf(), scan))
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
        if !date.file_type().await.map_err(boom)?.is_dir()
            || date.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let direct = date.path().join("draft.json");
        if tokio::fs::try_exists(&direct).await.map_err(boom)? {
            drafts.push(read_roomplan_draft(&direct).await?);
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
                drafts.push(read_roomplan_draft(&draft).await?);
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

async fn read_roomplan_draft(
    path: &std::path::Path,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut draft = read_json_file(path).await?;
    let review_path = path.with_file_name("review.json");
    if tokio::fs::try_exists(&review_path).await.map_err(boom)? {
        draft["review"] = read_json_file(&review_path).await?;
    }
    Ok(draft)
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
        if !date.file_type().await.map_err(boom)?.is_dir()
            || date.file_name().to_string_lossy().starts_with('.')
        {
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
    let sun = crate::sun::report(&model, &l);
    let (sun, sun_note) = match sun {
        Ok(report) => (Some(serde_json::to_value(report).map_err(boom)?), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg, "sun": sun, "sun_note": sun_note }),
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
            revision: 0,
            since: String::new(),
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
        .route(
            "/api/roomplan/revisions/{revision_id}/asset",
            get(api_roomplan_revision_asset),
        )
        .route("/api/roomplan/revisions", get(api_roomplan_revisions))
        .route("/api/roomplan/captures", post(api_roomplan_capture))
        .route("/api/roomplan/reconcile", post(api_roomplan_reconcile))
        .route(
            "/api/roomplan/propose-placement",
            post(api_roomplan_propose_placement),
        )
        .route(
            "/api/roomplan/spaces",
            get(api_roomplan_spaces).post(api_roomplan_space_create),
        )
        .route(
            "/api/roomplan/spaces/{space_id}/revisions",
            post(api_roomplan_space_add_revision),
        )
        .route(
            "/api/roomplan/spaces/{space_id}/links",
            put(api_roomplan_object_link),
        )
        .route(
            "/api/roomplan/spaces/{space_id}/placements/{item_id}",
            put(api_roomplan_apply_placement),
        )
        .route(
            "/api/roomplan/revisions/{revision_id}/review",
            post(api_roomplan_review),
        )
        .route("/api/placements/{flat}", get(api_placements))
        .route(
            "/api/placements/{flat}/{item}/history",
            get(api_placement_history),
        )
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

/// The capture a phone sends is a file this Mac either holds or does not: nothing half-written becomes
/// the newest capture that `/api/roomplan/reference` and the dashboard would then draw.
#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::roomplan::fixtures::{stored_zip, wall_usda};

    /// A draft that passes the contract: one room, no elements. Enough for the store, which reads the
    /// id and writes the document.
    const DRAFT: &str = r#"{
        "schema_version": "roomplan-capture/v1",
        "draft_id": "2026-10-24T120000",
        "created_at": "2026-10-24T12:00:00Z",
        "source": { "platform": "ios" },
        "coordinate_system": { "units": "meters", "up_axis": "Y" },
        "room": { "id": "wohnzimmer", "surfaces": [], "openings": [], "objects": [] },
        "provenance": {}
    }"#;

    #[tokio::test]
    async fn a_capture_is_stored_and_read_back_or_leaves_nothing() {
        let draft: roomplan_contract::CaptureDraft =
            serde_json::from_str(DRAFT).expect("the fixture is a draft");
        draft.validate().expect("the fixture passes the contract");

        let root = std::env::temp_dir().join(format!("interior-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let wall = wall_usda();
        let usdz = stored_zip(&[
            ("CapturedRoom.usda", "def Xform \"Section_grp\"\n"),
            ("assets/Mesh/wall.usda", wall.as_str()),
        ]);

        let (directory, scan) = store_capture(&root, &draft, &usdz).await.expect("stored");
        assert_eq!(directory, root.join(&draft.draft_id));
        assert!(directory.join("captured-room.usdz").exists());
        assert!(
            directory.join("observation.json").exists(),
            "the draft travels with the capture, so the review has the phone's own document"
        );
        assert_eq!(scan.byte_length, usdz.len());
        assert_eq!(scan.surfaces().count(), 1, "the fixture's single wall");
        assert!(!scan.sha256.is_empty(), "the Mac hashes what it holds");

        // A file the reader cannot parse is not a capture, and the directory goes with it.
        let bad = std::env::temp_dir().join(format!("interior-capture-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bad);
        let error = store_capture(&bad, &draft, b"not a zip")
            .await
            .expect_err("refused");
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !bad.join(&draft.draft_id).exists(),
            "a refused capture must not become the newest capture"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bad);
    }
}
