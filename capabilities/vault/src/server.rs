//! `vault-server` — the vault's read-only HTTP surface.
//!
//! Two questions, both answered live off the files.
//!
//! **What actions are open.** PRD Q48 (2026-08-27) retired the `tasks`
//! capability and gave the Action kind back to `Projects/**/Tasks/`, which left
//! the dashboard's decision ladder with a band and no source. This is that
//! source.
//!
//! **What the Journal knows about each person.** D2's three fields have no
//! producer and cannot get one until D3 rules on machine-owned frontmatter. A
//! computed read needs neither: see `list_people`.
//!
//! ## Read-only, and not by omission
//!
//! There is no POST and no PATCH here, and adding one is not a small change.
//! The whole point of the ruling is that a task is a note a human owns: it gets
//! created, edited and marked done in Obsidian. A write endpoint would make
//! this server a second writer of files a human is editing, which is the
//! conflict the one-way rule (§5.5, "Axon reads the vault and does not write to
//! it") exists to prevent. The ladder links to the note instead.
//!
//! ## Why the read is not cached
//!
//! The vault is edited by hand, continuously, in another application. A cache
//! here would show the operator a task they marked done a minute ago, and the
//! only honest invalidation for "a human edited a file in iCloud" is to read
//! the files. It costs 6 ms for `Projects/` (measured 2026-08-28), which is
//! cheaper than the machinery that would avoid it.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use markdown_root::MarkdownRoot;
use vault::{people, tasks};

/// What this capability answers, served as data beside `/health`.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable vault with a Projects folder.",
    ),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/tasks",
        "Every action note under Projects/, read live. Optional status filter: open, done.",
    ),
    r(
        "GET",
        "/api/people",
        "last_contact, met_at and mention_count per Atlas/People note, computed from Journal/ backlinks, \
         plus the note's home, host and host_note keys as typed. Never stored.",
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
    Json(route_manifest::manifest("vault", ROUTES))
}

/// The vault root is resolved once, at startup, and held.
///
/// Resolution reads the overlay's `config/knowledge.toml`; the notes under it
/// are read per request. A root that moves is a restart, which is the same
/// contract every other capability has with its own config.
#[derive(Clone)]
struct AppState {
    vault_root: Arc<String>,
}

type ApiResponse = (StatusCode, Json<Value>);

/// Resolve the vault and read it, off the async runtime.
///
/// Reading a few hundred files off an iCloud-backed directory is blocking I/O
/// that can stall on an evicted file. Holding an async worker for that is the
/// mistake `tasks` documented in its own `with_store`, and the reason every
/// filesystem touch in this server goes through one place.
async fn with_vault<T, F>(state: &AppState, work: F) -> Result<T, ApiResponse>
where
    T: Send + 'static,
    F: FnOnce(&MarkdownRoot) -> Result<T, String> + Send + 'static,
{
    let root = state.vault_root.clone();
    let joined = tokio::task::spawn_blocking(move || {
        MarkdownRoot::declare(sjel_config::expand_tilde(&root))
            .map_err(|e| format!("vault_root: {e}"))
            .and_then(|vault| work(&vault))
    })
    .await;
    match joined {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(unavailable(error)),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "read failed" })),
        )),
    }
}

/// 503 rather than 400 or 500: an unreachable vault is a dependency that is not
/// there — an iCloud folder not yet materialised, a root that moved — and a
/// caller that retries should be told to come back rather than to fix its
/// request.
fn unavailable(error: impl std::fmt::Display) -> ApiResponse {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": error.to_string() })),
    )
}

fn ok<T: serde::Serialize>(status: StatusCode, value: T) -> ApiResponse {
    match serde_json::to_value(value) {
        Ok(value) => (status, Json(value)),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "service": "vault", "status": "ok", "version": "0.0.0" }))
}

/// Readiness: whether the vault this server exists to read is actually there.
///
/// Liveness cannot answer it — the process is fine while the iCloud folder is
/// not — and `axon-status` judges availability on this endpoint where a
/// manifest declares one (#126).
async fn ready(State(state): State<AppState>) -> ApiResponse {
    match with_vault(&state, |vault| tasks::projects_root(vault).map(|_| ())).await {
        Ok(()) => ok(
            StatusCode::OK,
            json!({ "service": "vault", "status": "ready" }),
        ),
        Err((_, Json(body))) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "service": "vault", "status": "unavailable", "error": body["error"] })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    status: Option<String>,
}

/// `open` and `done`, and no `dropped`.
///
/// The retired capability had three states because a machine-filed row needed a
/// way to be refused without being completed. A note a human wrote has no such
/// state: they delete it, or they mark it done. The vault's own `done` boolean
/// is the whole vocabulary, so the filter has to be too.
const STATUSES: [&str; 2] = ["open", "done"];

async fn list_tasks(State(state): State<AppState>, Query(query): Query<ListQuery>) -> ApiResponse {
    if let Some(status) = query.status.as_deref() {
        if !STATUSES.contains(&status) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid status '{status}'") })),
            );
        }
    }
    let wanted = query.status.clone();
    match with_vault(&state, move |vault| {
        let projects = tasks::projects_root(vault)?;
        let all = tasks::read(&projects, &tasks::vault_name(vault))?;
        Ok(match wanted.as_deref() {
            Some("open") => all.into_iter().filter(|t| !t.done).collect(),
            Some("done") => all.into_iter().filter(|t| t.done).collect(),
            _ => all,
        })
    })
    .await
    {
        Ok(tasks) => ok(StatusCode::OK, json!({ "tasks": tasks })),
        Err(response) => response,
    }
}

/// D2, served rather than written.
///
/// `last_contact`, `met_at` and `mention_count` sit on 70 of the 89 `Atlas/People` notes with
/// no producer. All three come out of `Journal/` backlinks and `vault people` has computed
/// them since 2026-09-07 — and refused to write them, because D3 is unresolved: machine-owned
/// frontmatter has no protection mechanism, so a producer could not tell its own value from a
/// human's correction and would overwrite the correction on its next run.
///
/// **A computed read has that problem and does not need D3 ruled first.** Nothing is stored,
/// so nothing can be overwritten; the answer is recomputed off the notes on every request and
/// is stale for exactly as long as the request takes. The three fields become available to a
/// reader without Axon becoming a second writer of files a human edits (§5.5).
///
/// It serves the drift too — `stored` and `disagrees` per person, the 4 notes whose written
/// value contradicts the Journal — because the disagreement is the row that tells a reader
/// which of the two is wrong, and a computed value served alone would look authoritative.
async fn list_people(State(state): State<AppState>) -> ApiResponse {
    match with_vault(&state, |vault| {
        // Two folders, not the vault: `people::report` reads `Atlas/People/` and `Journal/`
        // and nothing else. See `note::load_under` for the measurement.
        let (mut notes, _) = vault::note::load_under(vault, people::FOLDER)?;
        let (journal, _) = vault::note::load_under(vault, people::JOURNAL)?;
        notes.extend(journal);
        Ok(people::report(&notes))
    })
    .await
    {
        Ok(report) => ok(StatusCode::OK, report),
        Err(response) => response,
    }
}

#[tokio::main]
async fn main() {
    // The same resolution the CLI uses, so both binaries read one declaration
    // of where the vault is: `--root`, else the overlay's config/knowledge.toml.
    let vault = match vault::note::resolve_root(None) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("vault-server: {error}");
            std::process::exit(1);
        }
    };
    let state = AppState {
        vault_root: Arc::new(vault.path().to_string_lossy().into_owned()),
    };
    let port = sjel_server::resolve_port(None, None, 8094);
    sjel_server::serve_local("vault-server", port, build_router(state)).await;
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_VAULT_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "vault";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// `CorsLayer::permissive()` alone said any web page may read this, and the
/// sentence that used to stand here — no control surface, no secret, loopback
/// bind — is all true and still leaves that open. What `/api/tasks` returns is
/// every open action under `Projects/`, each with the note path that names the
/// project it belongs to: what the operator is working on this week, readable
/// by any page open in their browser. The guard decides who may ask; CORS stays
/// underneath it and decides what a permitted answer may say.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`: "you have to first add your routes (and / or
/// fallback) and then call `layer`"), so a route appended under it would
/// silently lose the refusal.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/tasks", get(list_tasks))
        .route("/api/people", get(list_people))
        // Permissive CORS, matching every other capability the dashboard reads
        // directly. This server serves no control surface and no secret, and
        // the bind is loopback.
        //
        // It stopped serving only task titles on 2026-09-08. `/api/people`
        // answers with 89 real people's names and the dates they were last
        // named in the Journal — the same names `names.rs` exists to redact
        // before anything leaves this machine. Loopback is not what holds that
        // in: `InboundAuth::refuse_without_token` (libs/sjel-server/src/auth.rs)
        // states the rule, that a page open in the operator's own browser is
        // already inside the loopback boundary. What holds it in is the token
        // `sjel_server::serve_local` resolves through
        // `InboundAuth::from_deployment()`, and a deployment that declares none
        // leaves this route readable by any origin. Whether this server should
        // refuse without a token is an operator ruling and not a comment's to
        // make, so the comment stops at saying what is true.
        //
        // Since 2026-09-08 the origin guard below is what makes that concrete:
        // a page on an origin this deployment does not name cannot read the
        // route at all, token or no token. The token question above is still
        // open and still the operator's.
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    /// The contract `axon-status` depends on: a vault that is not there is
    /// reported unavailable rather than as a healthy service (#126). The case
    /// is real — the vault lives in iCloud Drive, which can be absent on a
    /// freshly signed-in machine.
    #[tokio::test]
    async fn readiness_fails_when_the_vault_is_unreachable() {
        let missing = std::env::temp_dir().join(format!("vault-absent-{}", std::process::id()));
        let state = AppState {
            vault_root: Arc::new(missing.to_string_lossy().into_owned()),
        };

        let (status, Json(body)) = ready(State(state)).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "an unreachable vault was reported as ready: {body}"
        );
        assert_eq!(body["status"], "unavailable");

        // The control: liveness is deliberately unaffected, because the process
        // is fine.
        let Json(live) = health().await;
        assert_eq!(
            live["status"], "ok",
            "liveness must not depend on the vault"
        );
    }

    /// A vault that IS there answers with the notes on disk, filtered. This is
    /// the whole contract band 620 reads, exercised end to end through the
    /// handler rather than through the reader alone.
    #[tokio::test]
    async fn the_open_filter_serves_what_the_fixture_vault_holds() {
        let root = std::env::temp_dir().join(format!("vault-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("Projects").join("Home-Lab").join("Tasks");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("Buy a drive.md"),
            "---\ntype: task\ndone: false\ndue: 2026-09-01\n---\n\nbody\n",
        )
        .unwrap();
        std::fs::write(
            folder.join("Already bought.md"),
            "---\ntype: task\ndone: true\n---\n\nbody\n",
        )
        .unwrap();

        let state = AppState {
            vault_root: Arc::new(root.to_string_lossy().into_owned()),
        };

        let (status, Json(body)) = list_tasks(
            State(state.clone()),
            Query(ListQuery {
                status: Some("open".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let open = body["tasks"].as_array().expect("a tasks array");
        assert_eq!(open.len(), 1, "the done note leaked into the open list");
        assert_eq!(open[0]["title"], "Buy a drive");
        assert_eq!(open[0]["due"], "2026-09-01");
        assert!(
            open[0]["uri"]
                .as_str()
                .is_some_and(|uri| uri.starts_with("obsidian://open?vault=")),
            "the ladder's only remaining action is the link back to the note"
        );

        // An unknown status is a caller mistake, not a missing dependency, so
        // it is the one 400 this server can produce.
        let (status, _) = list_tasks(
            State(state),
            Query(ListQuery {
                status: Some("dropped".into()),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod people_tests {
    use super::*;

    /// A vault laid out the way the operator's is: one person, two journal entries that name
    /// them, and a `last_contact` on the note that the Journal contradicts.
    fn fixture(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("vault-people-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(people::FOLDER)).unwrap();
        std::fs::create_dir_all(root.join(people::JOURNAL)).unwrap();
        std::fs::write(
            root.join(people::FOLDER).join("Erika.md"),
            "---\nlast_contact: 2020-01-01\nmention_count: 40\n---\n\nbody\n",
        )
        .unwrap();
        std::fs::write(
            root.join(people::JOURNAL).join("2026-01-05.md"),
            "coffee with [[Erika]]\n",
        )
        .unwrap();
        std::fs::write(
            root.join(people::JOURNAL).join("2026-03-09 Monday.md"),
            "[[Erika]] again, and [[Erika]] twice\n",
        )
        .unwrap();
        root
    }

    /// D2, end to end through the handler. The three fields are answered off the notes and the
    /// note on disk is not touched — which is the whole argument for a route instead of a
    /// producer, since D3 has no ruling yet.
    #[tokio::test]
    async fn the_three_computed_fields_are_served_and_the_note_is_not_rewritten() {
        let root = fixture("serves");
        let note = root.join(people::FOLDER).join("Erika.md");
        let before = std::fs::read_to_string(&note).unwrap();

        let state = AppState {
            vault_root: Arc::new(root.to_string_lossy().into_owned()),
        };
        let (status, Json(body)) = list_people(State(state)).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        assert_eq!(body["people"], 1);
        let erika = &body["facts"][0];
        // Two notes, not three mentions: a day on which someone came up is one day.
        assert_eq!(erika["mention_count"], 2);
        assert_eq!(erika["met_at"], "2026-01-05");
        assert_eq!(erika["last_contact"], "2026-03-09");
        // The drift is served beside the computed value, because a computed number alone
        // would read as authoritative and the stored one is what a human typed.
        assert_eq!(erika["stored"]["last_contact"], "2020-01-01");
        assert_eq!(body["disagreeing"], 1);
        assert_eq!(
            erika["disagrees"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or_default(),
            2
        );

        assert_eq!(
            std::fs::read_to_string(&note).unwrap(),
            before,
            "the route wrote to a note a human owns — §5.5 is one-way"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A vault with People and no Journal answers 503, not 89 people with zero mentions.
    /// `people.rs` states the reason: zero is a measurement and absent is a producer that did
    /// not run, and a route that returned zeroes here would serve the second as the first.
    #[tokio::test]
    async fn a_vault_without_a_journal_is_unavailable_rather_than_all_zeroes() {
        let root = fixture("no-journal");
        std::fs::remove_dir_all(root.join(people::JOURNAL)).unwrap();

        let state = AppState {
            vault_root: Arc::new(root.to_string_lossy().into_owned()),
        };
        let (status, Json(body)) = list_people(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|e| e.contains("Journal")),
            "the error must name the folder that is missing: {body}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This
    /// reads the router's own source, so adding a `.route()` without a summary
    /// fails here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("server.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}

/// The router-level proof that `libs/sjel-server`'s own predicate tests cannot
/// give: a route registered BELOW the `.layer()` call passes every test of
/// `origin_allowed_by` and still answers a hostile page.
///
/// Driven with `tower::ServiceExt::oneshot` rather than over a loopback listener
/// (the pattern `capabilities/places` uses), because this crate has no HTTP
/// client and reqwest would pull a whole TLS stack into a test build to send one
/// request to itself. `tower` is already in the graph under axum.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// A vault root that is deliberately absent. The guard runs before the
    /// handler, so the refusal does not need a vault — and the control below
    /// reads its 503 as proof that the request got past the guard to a handler
    /// that then found no vault.
    fn router() -> Router {
        let absent = std::env::temp_dir().join(format!("vault-origin-{}", std::process::id()));
        build_router(AppState {
            vault_root: Arc::new(absent.to_string_lossy().into_owned()),
        })
    }

    async fn tasks_status(origin: Option<&str>) -> StatusCode {
        let mut request = Request::builder().uri("/api/tasks");
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers")
            .status()
    }

    #[tokio::test]
    async fn a_foreign_origin_cannot_read_the_task_list() {
        assert_eq!(
            tasks_status(Some("https://evil.example")).await,
            StatusCode::FORBIDDEN,
            "/api/tasks answered a foreign origin — it is registered below the guard layer"
        );
    }

    /// The other half, and the reason the test above is not just asserting that
    /// this server is broken: the callers that legitimately reach it still do.
    /// 503 is the absent vault, which is a handler answering.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                tasks_status(origin).await,
                StatusCode::SERVICE_UNAVAILABLE,
                "the guard refused a caller it must admit: {origin:?}"
            );
        }
    }
}
