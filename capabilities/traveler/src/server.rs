//! traveler HTTP surface (README "HTTP surface", port 8096). Same shape as the
//! sibling servers: blocking store work in `spawn_blocking`, `/ready` proves the
//! database, and `GET /routes` serves the manifest the coverage test below
//! checks against this file's own source.
//!
//! No permissive CORS, following `capabilities/places` rather than `trips`. This
//! serves personal state — where the traveller lives, when they refuse to
//! travel, what they are interested in — and will serve companion patterns. The
//! shared guard refuses a foreign browser origin outright rather than only
//! withholding a header, which is what stops a hostile page's "simple" cross-site
//! write, and the dashboard reaches this through its own same-origin proxy
//! (`service.toml`, `proxy_api_only`), so nothing needs the header.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use traveler::config::Config;
use traveler::derive::{self, DerivedTravel, TRIPS_PREFIX};
use traveler::model::{ProfileInput, TravelProfile, DEFAULT_PROFILE_ID};
use traveler::store::{PutOutcome, TravelerStore};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/profile",
        "The traveller profile, with a basis entry per field saying whether it was stated, \
         derived, proposed from the vault, or is still the built-in default. Never 404: an \
         unstated profile comes back with stored:false and revision 0, so a consumer has one \
         shape to handle rather than two.",
    ),
    route_manifest::Route {
        method: "PUT",
        path: "/api/profile",
        summary: "Replace the traveller profile. Body: { profile, expected_revision? }. \
                  Omit expected_revision to write unconditionally; send the revision you read \
                  to be refused with 409 stale_profile instead of silently overwriting a \
                  concurrent write. Refused with 400 when the five weights do not sum to 1.0 \
                  (the sum is named), when a clock is not HH:MM, or when basis does not cover \
                  every field exactly once.",
        request_schema: Some(route_manifest::schema_of::<PutProfileRequest>),
    },
    route_manifest::get(
        "GET",
        "/api/profile/derived",
        "What the stored trips actually show: trip length, booking lead time, destinations, \
         repeat destinations, company shape, months and modes, with the plan ids every number \
         was computed from. Computed on read over trips' own tables and stored nowhere. Counts \
         plans, not confirmed trips: only a retrospective recording not_taken excludes one, and \
         lead_time_days covers only plans whose row predates the trip, because the vault import \
         stamps created_at with the import date. Company is counted as a shape, never listed, \
         because travelers holds real names. `notes` carries every limit in the response. \
         Answers with absences rather than 500 when trips has never run here.",
    ),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PutProfileRequest {
    profile: ProfileInput,
    /// The revision the caller believes is current. Absent means "overwrite
    /// whatever is there".
    #[serde(default)]
    expected_revision: Option<u32>,
}

/// A refusal that carries the capability's own reason, in the same shape every
/// other Axon capability uses.
struct ApiError {
    status: StatusCode,
    body: Value,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: json!({ "error": message }),
        }
    }

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: json!({ "error": message }),
        }
    }

    /// A lost update, which is a routine event and not a failure. The current
    /// revision rides along so the caller can re-read and retry in one round
    /// trip rather than discovering it is stale a second time.
    fn stale(current_revision: u32) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            body: json!({
                "error": "the profile changed since you read it",
                "code": "stale_profile",
                "current_revision": current_revision,
            }),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

/// `GET /api/profile`.
async fn get_profile(State(store): State<Arc<TravelerStore>>) -> Result<Json<Value>, ApiError> {
    let stored = read_profile(store).await?;
    // The one place a missing row becomes a body. Every consumer then handles a
    // single shape, and `stored` is what tells it nothing has been established
    // yet — `revision: 0` carries the same fact for a caller that only reads the
    // profile itself.
    let (profile, stored) = match stored {
        Some(profile) => (profile, true),
        None => (TravelProfile::unstated(), false),
    };
    Ok(Json(json!({ "profile": profile, "stored": stored })))
}

/// `PUT /api/profile`.
async fn put_profile(
    State(store): State<Arc<TravelerStore>>,
    Json(request): Json<PutProfileRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // Validated before the store is touched, so a refusal never costs a write
    // transaction and never bumps a revision.
    request
        .profile
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let input = request.profile.clone();
    let expected = request.expected_revision;
    let outcome = tokio::task::spawn_blocking(move || {
        store
            .put(DEFAULT_PROFILE_ID, &input, expected)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::internal(format!("store task failed: {error}")))?
    .map_err(ApiError::internal)?;

    match outcome {
        PutOutcome::Stored { profile, created } => Ok((
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({ "profile": profile, "stored": true, "created": created })),
        )),
        PutOutcome::Stale { current_revision } => Err(ApiError::stale(current_revision)),
    }
}

/// `GET /api/profile/derived`.
///
/// Read-only and computed on demand, so there is no cache to invalidate when a
/// plan changes. It never fails on an empty store: the `Spread`s come back
/// `null` rather than zero, which is the same distinction `punctuality` makes
/// between "no evidence" and "evidence that says zero".
async fn get_derived(
    State(store): State<Arc<TravelerStore>>,
) -> Result<Json<DerivedTravel>, ApiError> {
    let derived = tokio::task::spawn_blocking(move || {
        derive::derive(&store, TRIPS_PREFIX).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::internal(format!("store task failed: {error}")))?
    .map_err(ApiError::internal)?;
    Ok(Json(derived))
}

async fn read_profile(store: Arc<TravelerStore>) -> Result<Option<TravelProfile>, ApiError> {
    tokio::task::spawn_blocking(move || {
        store
            .get(DEFAULT_PROFILE_ID)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::internal(format!("store task failed: {error}")))?
    .map_err(ApiError::internal)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "traveler" }))
}

/// Liveness plus a reachable database, which is what `service.toml`'s
/// `ready_path` promises rather than mere liveness (#126).
async fn ready(State(store): State<Arc<TravelerStore>>) -> Result<Json<Value>, ApiError> {
    tokio::task::spawn_blocking(move || store.ping().map_err(|error| error.to_string()))
        .await
        .map_err(|error| ApiError::internal(format!("store task failed: {error}")))?
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "status": "ready", "service": "traveler" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("traveler", ROUTES))
}

/// The wired router, separated from [`serve`] so tests drive exactly what the
/// process serves — the origin guard included, which is the part a test calling
/// a handler directly would skip.
fn router(store: Arc<TravelerStore>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/profile", get(get_profile).put(put_profile))
        .route("/api/profile/derived", get(get_derived))
        // Below every route, because axum's `layer` wraps only what is
        // registered before it (libs/sjel-server/src/origin.rs).
        .layer(middleware::from_fn_with_state(
            "traveler",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(store)
}

pub async fn serve() {
    let config = Config::load();
    let store = match TravelerStore::open(&config.database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("traveler: cannot open store: {error}");
            std::process::exit(1);
        }
    };
    sjel_server::serve_local("traveler", config.port, router(Arc::new(store))).await;
}

fn main() {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime could not start")
        .block_on(serve());
}

#[cfg(test)]
mod route_manifest_tests {
    use super::*;

    /// The manifest is data a caller reads to learn the surface; a served route
    /// that is not in it is a caller guessing. Adding a mount without a manifest
    /// entry fails here (ISA TRV-3).
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES);
        assert!(missing.is_empty(), "undeclared routes: {missing:?}");
    }

    /// Every write route must publish the body it accepts. The summary for
    /// `PUT /api/profile` names four refusal reasons, and none of them is a
    /// substitute for the schema a caller needs before it can send anything.
    #[test]
    fn every_write_route_declares_its_request_schema() {
        let missing = route_manifest::bodies_without_schemas(ROUTES);
        assert!(missing.is_empty(), "bodies with no schema: {missing:?}");
    }

    #[test]
    fn every_declared_path_is_actually_served() {
        // The one-directional half `undeclared_routes` deliberately skips: a
        // manifest entry for a path nothing mounts sends a caller to a 404.
        let source = include_str!("server.rs");
        for route in ROUTES {
            assert!(
                source.contains(&format!("\"{}\"", route.path)),
                "{} is declared but nothing mounts it",
                route.path
            );
        }
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;

    async fn scratch_server(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("axon-traveler-http-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TravelerStore::open(&dir.join("axon.db")).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(Arc::new(store));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), dir)
    }

    #[tokio::test]
    async fn an_unstated_profile_is_served_rather_than_404d() {
        let (base, dir) = scratch_server("unstated").await;
        let body: Value = reqwest::get(format!("{base}/api/profile"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["stored"], json!(false));
        assert_eq!(body["profile"]["revision"], json!(0));
        assert_eq!(
            body["profile"]["basis"]["soft.budget_fit"],
            json!("default"),
            "an unestablished weight must read as a default, not as a decision"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_profile_round_trips_through_http_and_reports_its_revision() {
        let (base, dir) = scratch_server("round-trip").await;
        let client = reqwest::Client::new();
        let mut input = ProfileInput::unstated();
        input.hard.earliest_departure = Some("07:00".into());
        // Invented, not a real interest list: this repository is public and a
        // person's stated interests are not test data.
        input.interests = vec!["synthetic interest".into(), "second interest".into()];
        input.basis.insert(
            "hard.earliest_departure".into(),
            traveler::Provenance::Stated,
        );

        let created = client
            .put(format!("{base}/api/profile"))
            .json(&json!({ "profile": input }))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), 201);
        let body: Value = created.json().await.unwrap();
        assert_eq!(body["created"], json!(true));
        assert_eq!(body["profile"]["revision"], json!(1));

        let read: Value = reqwest::get(format!("{base}/api/profile"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(read["stored"], json!(true));
        assert_eq!(
            read["profile"]["hard"]["earliest_departure"],
            json!("07:00")
        );
        assert_eq!(
            read["profile"]["basis"]["hard.earliest_departure"],
            json!("stated")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn weights_that_do_not_sum_to_one_are_refused_before_anything_is_written() {
        let (base, dir) = scratch_server("weights").await;
        let client = reqwest::Client::new();
        let mut input = ProfileInput::unstated();
        input.soft.season = 0.9;

        let refused = client
            .put(format!("{base}/api/profile"))
            .json(&json!({ "profile": input }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 400);
        let body: Value = refused.json().await.unwrap();
        let message = body["error"].as_str().unwrap();
        assert!(
            message.contains("must sum to 1.0") && message.contains("1.7"),
            "the refusal must name the rule and the sum, got: {message}"
        );

        // Nothing was written, so the profile is still unstated.
        let read: Value = reqwest::get(format!("{base}/api/profile"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(read["stored"], json!(false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_write_against_a_stale_revision_is_a_409_naming_the_current_one() {
        let (base, dir) = scratch_server("stale").await;
        let client = reqwest::Client::new();
        let input = ProfileInput::unstated();
        client
            .put(format!("{base}/api/profile"))
            .json(&json!({ "profile": input }))
            .send()
            .await
            .unwrap();

        let refused = client
            .put(format!("{base}/api/profile"))
            .json(&json!({ "profile": input, "expected_revision": 7 }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 409);
        let body: Value = refused.json().await.unwrap();
        assert_eq!(body["code"], json!("stale_profile"));
        assert_eq!(body["current_revision"], json!(1));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The guard has to be on the wired router, not merely available. A route
    /// registered after the layer would lose it and every predicate test would
    /// still pass (libs/sjel-server/src/origin.rs states the trap).
    #[tokio::test]
    async fn a_foreign_browser_origin_is_refused_on_the_wired_router() {
        let (base, dir) = scratch_server("origin").await;
        let refused = reqwest::Client::new()
            .get(format!("{base}/api/profile"))
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403);

        // A request with no Origin is not a browser cross-origin call, so the
        // runner's probes and every server-to-server caller still pass.
        let allowed = reqwest::get(format!("{base}/api/profile")).await.unwrap();
        assert_eq!(allowed.status(), 200);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_derived_route_answers_with_no_trips_rather_than_failing() {
        // The scratch store has no trips tables, which is a fresh install. A 500
        // here would read as an outage rather than as "nothing recorded yet".
        let (base, dir) = scratch_server("derived").await;
        let response = reqwest::get(format!("{base}/api/profile/derived"))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["considered"], json!(0));
        assert!(body["length_days"].is_null(), "no evidence is not a zero");
        assert!(body["lead_time_days"].is_null());
        assert_eq!(body["notes"].as_array().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn ready_proves_the_database_and_health_does_not_claim_to() {
        let (base, dir) = scratch_server("ready").await;
        for path in ["/health", "/ready"] {
            assert_eq!(
                reqwest::get(format!("{base}{path}"))
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
