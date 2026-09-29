use std::path::PathBuf;

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use scouting::adapters::cfp_conferences::CfpConferencesAdapter;
use scouting::adapters::euro_hackathons::EuroHackathonsAdapter;
use scouting::adapters::luma::LumaAdapter;
use scouting::adapters::meetup::MeetupAdapter;
use scouting::config::Config;
use scouting::event_route::{classify_opportunity, classify_ranked, EventRoute};
use scouting::pipeline::run;
use scouting::score::{load_opp_embeddings, load_telos_profiles};
use scouting::source::{SearchQuery, SourceAdapter};
use scouting::sources::{self, create_adapter};
use scouting::store::Store;

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/discover",
        "Ranked opportunities for the Discover view.",
    ),
    r(
        "GET",
        "/sources",
        "Declared opportunity sources and their state.",
    ),
    r(
        "GET",
        "/opportunities",
        "Stored opportunities. Optional include_dismissed.",
    ),
    r(
        "POST",
        "/opportunities/{id}/status",
        "Set an opportunity's status (saved, dismissed).",
    ),
    r(
        "POST",
        "/sources/proposed",
        "Record a candidate source. Never runs it. Requires adapter + locator.",
    ),
    r(
        "POST",
        "/sources/proposed/{id}/dismiss",
        "Take a candidate source out of the inbox.",
    ),
];

/// Shorthand so the table above reads as a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("scouting", ROUTES))
}

#[derive(Debug, Deserialize)]
struct DiscoverParams {
    adapter: Option<String>,
    location: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
    // Deliberately no `opp_embeddings`. It used to name a file this handler
    // read, which made `GET /discover` a file-existence oracle reachable from
    // any page in the operator's browser (the bind is loopback but the CORS
    // layer below is permissive) -- CodeQL rust/path-injection reported it
    // against score.rs `load_opp_embeddings`. The path is the CLI flag's and
    // the config file's to state (config.rs `opp_embeddings_path`, README
    // "CLI-arg-only still"), so the sink was removed rather than validated.
}

#[derive(Debug, serde::Serialize)]
struct ScoredResult {
    id: String,
    rank: usize,
    score: f64,
    source: String,
    title: String,
    date: Option<String>,
    location: Option<String>,
    city: Option<String>,
    matched_focus: Option<String>,
    rationale: String,
    url: String,
    opportunity_type: String,
    status: String,
    vault_link: Option<String>,
    event_route: Option<EventRoute>,
}

#[derive(Debug, serde::Serialize)]
struct ClassifiedOpportunity {
    #[serde(flatten)]
    opportunity: scouting::store::RankedRow,
    event_route: Option<EventRoute>,
    /// What this row is worth protecting, from the source that fetched it.
    ///
    /// `GET /opportunities` published no class at all until 2026-09-08 (B50),
    /// so 310 stored rows from four adapters reached the dashboard with nothing
    /// said about them -- and a consumer that fails closed, the way finance's
    /// `item_is_quotable` does, has to drop a row it cannot classify. The value
    /// comes from `sources[].data_class` in `scouting.json` and from nowhere
    /// else; a row whose source declares nothing is `c1`, never `c0`.
    data_class: String,
}

#[derive(Debug, serde::Serialize)]
struct DiscoverResponse {
    adapter: String,
    opportunity_type: String,
    total_scored: usize,
    new_count: usize,
    vault_links: usize,
    store_total: i64,
    results: Vec<ScoredResult>,
}

async fn discover_handler(Query(params): Query<DiscoverParams>) -> Json<Value> {
    let adapter_name = params
        .adapter
        .clone()
        .unwrap_or_else(|| "euro_hackathons".into());
    let location = params.location.clone();
    let query_text = params.query.clone().unwrap_or_default();
    let limit = params.limit.unwrap_or(20);

    let result = tokio::task::spawn_blocking(move || -> Option<DiscoverResponse> {
        // Config::load() and the adapter/store construction below all do
        // sync file/DB I/O (matches capabilities/transit's server.rs — same
        // spawn_blocking discipline, see its handle_health comment for why
        // this matters under #[tokio::main]).
        let cfg = Config::load();

        // A configured source id is authoritative. If its adapter cannot be
        // constructed, fail this scan; silently running a different built-in
        // source would attach the wrong provenance to the user's action.
        let adapter: Box<dyn SourceAdapter> = if let Some(source) = cfg
            .sources
            .iter()
            .find(|source| source.id == adapter_name && source.enabled)
        {
            create_adapter(source).ok()?
        } else {
            match adapter_name.as_str() {
                "luma" => Box::new(LumaAdapter::new()),
                "meetup" => Box::new(MeetupAdapter::new()),
                "cfp" | "cfp_conferences" => Box::new(CfpConferencesAdapter::new()),
                "euro_hackathons" => {
                    let cache_dir = PathBuf::from("infra/data/scouting-cache/euro_hackathons");
                    if cache_dir.exists() {
                        Box::new(EuroHackathonsAdapter::with_cache(cache_dir))
                    } else {
                        Box::new(EuroHackathonsAdapter::new())
                    }
                }
                _ => return None,
            }
        };

        let query = SearchQuery {
            query: query_text,
            location,
            limit,
            ..Default::default()
        };

        let telos = load_telos_profiles(&cfg.interest_profile_dir.to_string_lossy(), &cfg.sources);
        let events_dir = cfg.events_dir.as_deref();

        let opp_embeddings = cfg
            .opp_embeddings_path
            .as_ref()
            .map(|p| load_opp_embeddings(&p.to_string_lossy()));

        let mut store = Store::open(&cfg.database_path).ok();

        let report = run(
            &*adapter,
            &query,
            &telos,
            opp_embeddings.as_ref(),
            store.as_mut(),
            events_dir,
        )
        .ok()?;
        let opportunity_type = adapter.opportunity_type().as_str().to_string();

        // The pipeline owns scoring while the store owns human decisions and
        // Obsidian matches. Re-read those small persisted fields so a discovery
        // response is immediately actionable instead of returning a transient,
        // second shape of the same opportunity.
        let persisted = store
            .as_ref()
            .and_then(|st| st.list_top(report.store_total.max(1) as usize, true).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect::<std::collections::HashMap<_, _>>();

        let results: Vec<ScoredResult> = report
            .scored
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let persisted_row = persisted.get(&s.opportunity.id);
                let event_route = classify_opportunity(&s.opportunity, cfg.geo.as_ref());
                ScoredResult {
                    id: s.opportunity.id,
                    rank: i + 1,
                    score: s.score,
                    source: s.opportunity.source,
                    title: s.opportunity.title,
                    date: s.opportunity.starts_at,
                    location: s.opportunity.location,
                    city: s.opportunity.city,
                    matched_focus: s.matched_focus,
                    rationale: s.rationale,
                    url: s.opportunity.url,
                    opportunity_type: s.opportunity.opportunity_type.as_str().to_string(),
                    status: persisted_row
                        .map(|row| row.status.clone())
                        .unwrap_or_else(|| "new".to_string()),
                    vault_link: persisted_row.and_then(|row| row.vault_link.clone()),
                    event_route,
                }
            })
            .collect();

        Some(DiscoverResponse {
            adapter: adapter_name,
            opportunity_type,
            total_scored: results.len(),
            new_count: report.new_count,
            vault_links: report.vault_links,
            store_total: report.store_total,
            results,
        })
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(response) => Json(json!(response)),
        None => Json(json!({"error": "pipeline execution failed"})),
    }
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "scouting",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness: whether this capability can actually serve, which liveness does not answer.
///
/// `health_handler` is a literal and cannot observe the database, so with the store unreachable
/// this capability reported itself up while every query behind it failed (#126). Availability
/// is judged here instead.
///
/// 503, not 500: the request was fine, the dependency is not, and a caller that retries should
/// be told to come back rather than to fix its input.
async fn ready_handler() -> (StatusCode, Json<Value>) {
    let probe = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        Store::open(&cfg.database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await;
    match probe {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(json!({ "status": "ready", "service": "scouting" })),
        ),
        Ok(Err(error)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable", "service": "scouting", "error": error })),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unavailable",
                "service": "scouting",
                "error": "readiness check failed"
            })),
        ),
    }
}

/// Returns the list of configured sources (from scouting.json's `sources[]`),
/// plus the candidate-source inbox. Useful for dashboards and automation to
/// discover what's available.
///
/// On the blocking pool like every other store-touching handler here. It did
/// not need to be until it read the proposal inbox: `rusqlite` is a blocking
/// client, and calling it straight from an async handler panics the worker
/// with "cannot start a runtime from within a runtime". Config::load reads a
/// file, so it belongs on the same side of that line.
async fn sources_handler() -> Json<Value> {
    tokio::task::spawn_blocking(sources_listing)
        .await
        .unwrap_or_else(|error| Json(json!({ "error": error.to_string() })))
}

fn sources_listing() -> Json<Value> {
    let cfg = Config::load();
    let mut sources: Vec<Value> = cfg
        .sources
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "adapter": s.adapter,
                "enabled": s.enabled,
                "configured": true,
                "root_path": s.root_path.as_ref().map(|p| p.to_string_lossy()),
                "url": s.url,
                "events_glob": s.events_glob,
                "opportunities_glob": s.opportunities_glob,
                "opportunity_type": s.opportunity_type.as_str(),
                "profiles_glob": s.profiles_glob,
                // Null when the profile resolves under root_path, which is the
                // compatible form and most entries.
                "profile_root": s.profile_root.as_ref().map(|p| p.to_string_lossy()),
                "doc_path": s.doc_path.as_ref().map(|p| p.to_string_lossy()),
                // What this source declares its opportunities are worth. The
                // audit surface for the class every row it fetched now carries:
                // a source reading c1 here either said so or said nothing.
                "data_class": s.data_class,
            })
        })
        .collect();
    // The EuroHackathons adapter is the one built-in network source with a
    // live-verified contract. Keep experimental adapters out of the dashboard
    // source picker until they have the same evidence.
    if !sources
        .iter()
        .any(|source| source["id"] == "euro_hackathons")
    {
        sources.push(json!({
            "id": "euro_hackathons",
            "adapter": "euro_hackathons",
            "enabled": true,
            "configured": false,
            "root_path": null,
            "url": "https://eurohackathons.com",
            "events_glob": null,
            "opportunities_glob": null,
            "opportunity_type": "hackathon",
            "profiles_glob": null,
            "profile_root": null,
            "doc_path": null,
            // Not in `sources[]`, so nobody has declared anything about it, so
            // c1 -- the same answer `class_for_source` gives its rows.
            "data_class": content_item::DataClass::undeclared().value,
        }));
    }
    // The inbox, beside what is declared rather than mixed into it. A proposal
    // is not a source: it has no id an operator chose, it never runs, and the
    // only way it becomes one is a human editing the overlay. Putting it in the
    // same array under `enabled: false` would have made those two things look
    // like states of one thing.
    let proposed: Vec<Value> = Store::open(&cfg.database_path)
        .and_then(|store| store.list_proposed_sources("proposed"))
        .map(|rows| {
            rows.iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "adapter": p.adapter,
                        "locator": p.locator,
                        "label": p.label,
                        "found_by": p.found_by,
                        "found_at": p.found_at,
                        "note": p.note,
                        // Derived from what is declared right now, never stored:
                        // the fact lives in the overlay's config file, and a
                        // copy here would only ever be a stale one.
                        "declared": p.is_declared_by(&cfg.sources),
                    })
                })
                .collect()
        })
        // A database that is down must not take the declared list with it. The
        // inbox is the optional half of this endpoint.
        .unwrap_or_default();

    Json(json!({
        "sources": sources,
        "count": sources.len(),
        "proposed": proposed,
        "proposed_count": proposed.len(),
    }))
}

#[derive(Debug, Deserialize)]
struct OpportunityParams {
    limit: Option<usize>,
    include_dismissed: Option<bool>,
}

async fn opportunities_handler(
    Query(params): Query<OpportunityParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let include_dismissed = params.include_dismissed.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(internal_error)?;
        let store_total = store.count().map_err(internal_error)?;
        let opportunities = store
            .list_top(limit, include_dismissed)
            .map_err(internal_error)?
            .into_iter()
            .map(|opportunity| {
                let event_route = classify_ranked(&opportunity, cfg.geo.as_ref());
                let data_class = sources::class_for_source(&cfg.sources, &opportunity.source);
                ClassifiedOpportunity {
                    opportunity,
                    event_route,
                    data_class,
                }
            })
            .collect::<Vec<_>>();
        Ok(Json(json!({
            "count": opportunities.len(),
            "store_total": store_total,
            "opportunities": opportunities,
        })))
    })
    .await
    .map_err(internal_error)?
}

#[derive(Debug, Deserialize)]
struct StatusBody {
    status: String,
}

async fn set_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(internal_error)?;
        match store.set_status(&id, &body.status) {
            Ok(true) => Ok(Json(json!({ "id": id, "status": body.status }))),
            Ok(false) => Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "opportunity not found" })),
            )),
            Err(error) => Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )),
        }
    })
    .await
    .map_err(internal_error)?
}

/// A candidate source somebody wants remembered.
///
/// `found_by` defaults to `manual` because typing a hub id in is the day-one
/// producer, and a proposal with no origin is a fact nobody can check later.
#[derive(Deserialize)]
struct ProposalBody {
    adapter: String,
    locator: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    found_by: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// Records a candidate. Deliberately cannot start anything: the only thing that
/// makes a source run is an entry in the overlay's `sources[]`, which no code
/// path here writes. The response says so rather than leaving the caller to
/// assume a POST means it is live.
async fn propose_source_handler(
    Json(body): Json<ProposalBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(internal_error)?;
        let found_by = body.found_by.as_deref().unwrap_or("manual");
        match store.propose_source(
            &body.adapter,
            &body.locator,
            body.label.as_deref(),
            found_by,
            body.note.as_deref(),
        ) {
            Ok(is_new) => Ok(Json(json!({
                "status": "proposed",
                "new": is_new,
                "enabled": false,
                "next": "promote it by adding it to sources[] in the overlay's scouting.json",
            }))),
            Err(error) => Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )),
        }
    })
    .await
    .map_err(internal_error)?
}

async fn dismiss_proposal_handler(
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(internal_error)?;
        match store.dismiss_proposed_source(&id) {
            Ok(true) => Ok(Json(json!({ "id": id, "status": "dismissed" }))),
            Ok(false) => Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "no such proposed source" })),
            )),
            Err(error) => Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": error.to_string() })),
            )),
        }
    })
    .await
    .map_err(internal_error)?
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
}

// No /pulse or transit-fare proxy route here — same reasoning as
// transit-server (dashboard/README.md). The
// existing transit_fare adapter already pulls transit data in-process via
// the transit crate (see Cargo.toml), so scouting has no HTTP-proxy need
// toward transit in the first place.

#[tokio::main]
async fn main() {
    // Config::load() at this point is plain file/env parsing, no DB
    // connection — safe to call directly in the async context (unlike
    // discover_handler's Store::open, which stays inside spawn_blocking).
    let cfg = Config::load();

    // Was 0.0.0.0, which put an unauthenticated POST /opportunities/{id}/status on
    // the LAN. Nothing documented that bind as a decision; it was the last of the
    // three divergences libs/sjel-server exists to end.
    sjel_server::serve_local("scout-server", cfg.port, build_router()).await;
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_SCOUTING_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "scouting";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// No state: every handler resolves its own `Config` and opens the store inside
/// `spawn_blocking`, which is why this takes no argument.
///
/// The sentence that used to sit on the CORS layer — "defensible only because
/// serve_local binds loopback — the browser calling this is on the same machine"
/// — is where the hole was. A page on any website is also "on the same machine"
/// once the operator opens it, so loopback bounds who can route a packet here
/// and not who can make the operator's browser send one. `GET /opportunities`
/// carries the operator's own accept/dismiss decisions and the rationale that
/// scored them, and `POST /opportunities/{id}/status` writes one.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`), so a route appended under it would silently
/// lose the refusal.
fn build_router() -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/discover", get(discover_handler))
        .route("/sources", get(sources_handler))
        .route("/opportunities", get(opportunities_handler))
        .route("/opportunities/{id}/status", post(set_status_handler))
        .route("/sources/proposed", post(propose_source_handler))
        .route(
            "/sources/proposed/{id}/dismiss",
            post(dismiss_proposal_handler),
        )
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        // CorsLayer stays here rather than in sjel_server: it is a per-capability
        // security decision and belongs where it can be seen. It decides what a
        // permitted answer may say; the guard above decides who may ask.
        .layer(CorsLayer::permissive())
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}

/// The router-level proof that `libs/sjel-server`'s predicate tests cannot give:
/// a route registered BELOW the `.layer()` call passes every test of
/// `origin_allowed_by` and still answers a hostile page.
///
/// `/routes` is the control rather than `/opportunities`, because every data
/// handler here opens the deployment's SQLite file and a test must not. The
/// refusal is asserted on the data routes, where the guard answers before the
/// handler runs and nothing is opened.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn answer(method: &str, path: &str, origin: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        build_router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers")
            .status()
    }

    #[tokio::test]
    async fn a_foreign_origin_can_neither_read_nor_write_the_backlog() {
        for (method, path) in [
            ("GET", "/opportunities"),
            ("GET", "/discover"),
            ("GET", "/sources"),
            ("POST", "/opportunities/evt-1/status"),
            ("POST", "/sources/proposed"),
        ] {
            assert_eq!(
                answer(method, path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{method} {path} answered a foreign origin — it is registered below the guard layer"
            );
        }
    }

    /// The other half. 200 from `/routes` is a handler answering.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                answer("GET", "/routes", origin).await,
                StatusCode::OK,
                "the guard refused a caller it must admit: {origin:?}"
            );
        }
    }
}
