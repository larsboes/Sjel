//! `comms-server` — HTTP surface for the general Feed.
//!
//! Feed persistence, TELOS relevance, explicit Vault-link discovery and reader
//! payloads live here. Scouting remains a separate opportunity engine. Network
//! fetches and embedding calls run in spawn_blocking; the server binds only to
//! loopback because ingest is allowed to fetch external URLs.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::{HeaderName, HeaderValue, Method};
use axum::{
    extract::{DefaultBodyLimit, Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use comms::cloud_derivative::{self, CloudDerivativePreview, CloudDocumentInput};
use comms::cloud_dispatch;
use comms::cloud_run;
use comms::config::Config;
use comms::content_item::{self, DataClass};
use comms::digest;
use comms::evaluation::{self, EvaluationFactor, FeedEvaluation};
use comms::google::{self, ThreadAction, ThreadLocation};
use comms::intake;
use comms::mail_evaluation;
use comms::mail_model::{self, Mode};
use comms::media;
use comms::people_registry;
use comms::provenance::StageProvenance;
use comms::quality;
use comms::relevance::{self, RelevanceMatch};
use comms::sources;
use comms::store::{
    ClassWrite, CloudDerivativeApproval, CloudDerivativeState, CloudQueueRequest, FeedItem,
    FeedOrigin, FeedRun, GmailActionJob, ModelVerdict, ModelVerdictSummary, OriginSummary, Store,
    StreamWrite, TriageItem,
};
use comms::travel;
use comms::vault_links;

mod background;
mod cloud;
mod content;
mod contracts;
mod error;
mod feed;
mod source_handlers;
mod triage;
mod vault;

use background::*;
use cloud::*;
use content::*;
use contracts::*;
use error::*;
use feed::*;
use source_handlers::*;
use triage::*;
use vault::*;

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
        "/content/:source/:id",
        "An item as content-item-v2. :source is feed or mail.",
    ),
    r(
        "POST",
        "/content/:source/:id/digest",
        "Generate or refine this item's local digest. Optional depth and focus[].",
    ),
    r(
        "POST",
        "/content/:source/:id/diagram",
        "Draw this item as a validated Mermaid diagram.",
    ),
    r(
        "POST",
        "/content/:source/:id/chart",
        "Extract a chartable table, every value verified against the source.",
    ),
    r(
        "POST",
        "/content/digests/refresh",
        "Bounded automatic pass over one source. Requires source, optional limit.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-preview",
        "Build a bounded, reviewable copy for cloud use.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-approval",
        "Approve the exact previewed copy.",
    ),
    r(
        "POST",
        "/content/:source/:id/cloud-queue",
        "Queue an approved copy for a provider role.",
    ),
    r(
        "POST",
        "/content/cloud-jobs/:job_id/run",
        "Run a queued cloud job.",
    ),
    r(
        "GET",
        "/content/cloud-providers",
        "Provider roles and whether each is available.",
    ),
    r("GET", "/feed", "Feed entries. Optional status filter."),
    r("POST", "/ingest", "Ingest one URL into the feed."),
    r("GET", "/feed/:id", "One feed entry."),
    r(
        "POST",
        "/feed/:id/status",
        "Set a feed entry's status (keeper, dismissed).",
    ),
    r("GET", "/feed/runs", "Recent collector runs. Optional days."),
    r(
        "GET",
        "/feed/origins",
        "Which source run produced each entry.",
    ),
    r(
        "GET",
        "/feed/quality",
        "Entries flagged for quality review.",
    ),
    r("POST", "/feed/quality/refresh", "Recompute quality flags."),
    r(
        "GET",
        "/feed/evaluation/status",
        "Evaluation backlog and coverage.",
    ),
    r(
        "POST",
        "/feed/relevance/refresh",
        "Rescore feed relevance against the current profiles.",
    ),
    r("GET", "/sources", "Declared feed sources."),
    r(
        "GET",
        "/__axon/freshness",
        "When data last arrived, for the freshness contract.",
    ),
    r(
        "POST",
        "/sources/scan",
        "Collect from the declared sources.",
    ),
    r("GET", "/triage", "Mail proposals. Optional status filter."),
    r(
        "POST",
        "/triage/:id/status",
        "Set a mail proposal's status.",
    ),
    r(
        "POST",
        "/triage/:id/stream",
        "Reclassify a mail into a category.",
    ),
    r(
        "POST",
        "/triage/:id/data-class",
        "Set a mail's data classification by hand.",
    ),
    r(
        "POST",
        "/feed/:id/data-class",
        "Set a feed item's data classification by hand. Lowering one needs a rationale.",
    ),
    r(
        "POST",
        "/triage/:id/gmail",
        "Apply a Gmail action (archive, trash, restore).",
    ),
    r(
        "POST",
        "/triage/:id/gmail-job",
        "Queue a Gmail action for retry.",
    ),
    r(
        "POST",
        "/triage/bulk",
        "Apply one action across many mails.",
    ),
    r("POST", "/triage/sweep", "Pull new mail from Gmail."),
    r(
        "GET",
        "/triage/sweep/status",
        "Freshness and failure state of the scheduled inbox sweep.",
    ),
    r(
        "POST",
        "/triage/reconcile",
        "Reconcile Axon's mail state against Gmail.",
    ),
    r(
        "POST",
        "/triage/relevance/refresh",
        "Rescore mail relevance against the current profiles.",
    ),
    // Served since it was written, absent from this list until 2026-08-31 (PRD D19). Its
    // sibling two entries up was declared because it fits on one line; this one does not,
    // and the coverage test could not see a mount the formatter had wrapped.
    r(
        "POST",
        "/triage/data-class/refresh",
        "Reclassify the data class of stored mail against the current folder and frontmatter rules. Optional `limit`.",
    ),
    r(
        "POST",
        "/triage/redact",
        "Redact stored review fields of c2 and c3 mail already persisted.",
    ),
    // mail-llm-rung: the model rung's three. Declared here as well as mounted,
    // because `undeclared_routes` reads the router's own source.
    r(
        "POST",
        "/triage/classify/refresh",
        "Run the local model rung over the mail the deterministic rules did not decide. Optional `mode` (shadow, apply) and `limit`. Shadow by default; apply needs the overlay key.",
    ),
    r(
        "GET",
        "/triage/classify/report",
        "Agreement between the rules and the model rung, as counts. Optional ?mode=shadow|applied|held.",
    ),
    r(
        "POST",
        "/triage/classify/revert",
        "Put the deterministic verdict back on rows the model rung moved. Requires `ids` or `all`.",
    ),
    r(
        "POST",
        "/vault-links/scan",
        "Vault notes that could be linked to feed entries.",
    ),
    r(
        "POST",
        "/vault-links/import",
        "Link scanned vault notes to their entries.",
    ),
    // feed-personalization 2026-09-03. Appended rather than filed beside the
    // other /feed rows: a second stream appends to this table tonight, and one
    // region is one conflict.
    r(
        "POST",
        "/feed/:id/interactions",
        "Record that a feed entry was opened or reopened. Body: {event: opened|reopened, surface}. \
         The decisive verbs (kept, dismissed, unkept) are written by /feed/:id/status and refused here.",
    ),
    r(
        "POST",
        "/feed/model/train",
        "Refit the learned feedback factor from the interaction ledger. Optional `dry_run` \
         reports the cold-start gate without writing the model.",
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
    Json(route_manifest::manifest("comms", ROUTES))
}

/// Assemble the public HTTP surface.
///
/// The authentication boundary is no longer here: `sjel_server::authenticated`
/// wraps this router, and it gates every path except `/health` and `/ready`.
/// Kept separate from `main` so tests can exercise the real middleware stack
/// over an ephemeral loopback listener.
fn build_router(dashboard_origin: &str) -> Router {
    // CORS: allow only the dashboard origin, not permissive.
    let cors = CorsLayer::new()
        .allow_origin(
            dashboard_origin
                .parse::<HeaderValue>()
                .unwrap_or_else(|_| "http://127.0.0.1:47117".parse().unwrap()),
        )
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            HeaderName::from_static("x-axon-token"),
        ]);

    // Read routes. They used to bypass authentication entirely; they no longer
    // do. A feed entry and a mail proposal are personal content, and the split
    // that let them answer unauthenticated only ever made sense while the bind
    // was the whole boundary.
    let read_routes = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/feed", get(feed_handler))
        .route("/feed/origins", get(feed_origins_handler))
        .route("/feed/runs", get(feed_runs_handler))
        .route("/feed/evaluation/status", get(evaluation_status_handler))
        .route("/feed/quality", get(quality_queue_handler))
        .route("/feed/:id", get(feed_item_handler))
        .route("/content/cloud-providers", get(cloud_providers_handler))
        .route("/content/:source/:id", get(content_item_handler))
        .route("/sources", get(sources_handler))
        .route("/__axon/freshness", get(freshness_handler))
        .route("/triage", get(triage_handler))
        .route("/triage/sweep/status", get(triage_sweep_status_handler))
        .route(
            "/triage/classify/report",
            get(triage_classify_report_handler),
        );

    // Mutating routes. Still listed apart from the read routes because the two
    // sets differ in what they cost when they run, not because they differ in
    // what admits a caller — one gate covers both.
    let write_routes = Router::new()
        .route("/content/:source/:id/digest", post(digest_handler))
        .route("/content/:source/:id/diagram", post(diagram_handler))
        .route("/content/:source/:id/chart", post(chart_handler))
        .route("/content/digests/refresh", post(digest_refresh_handler))
        .route(
            "/content/:source/:id/cloud-preview",
            post(cloud_preview_handler),
        )
        .route(
            "/content/:source/:id/cloud-approval",
            post(cloud_approval_handler),
        )
        .route(
            "/content/:source/:id/cloud-queue",
            post(cloud_queue_handler),
        )
        .route("/content/cloud-jobs/:job_id/run", post(cloud_run_handler))
        .route("/feed/relevance/refresh", post(relevance_refresh_handler))
        .route("/feed/quality/refresh", post(quality_refresh_handler))
        .route("/feed/:id/status", post(feed_status_handler))
        .route("/feed/:id/data-class", post(feed_data_class_handler))
        .route("/triage/sweep", post(triage_sweep_handler))
        .route("/triage/relevance/refresh", post(triage_relevance_handler))
        .route("/triage/redact", post(triage_redact_handler))
        .route(
            "/triage/data-class/refresh",
            post(triage_data_class_refresh_handler),
        )
        .route("/triage/bulk", post(triage_bulk_handler))
        .route("/triage/:id/status", post(triage_status_handler))
        .route("/triage/:id/stream", post(triage_stream_handler))
        .route("/triage/:id/data-class", post(triage_data_class_handler))
        .route("/triage/:id/gmail", post(triage_gmail_handler))
        .route("/triage/:id/gmail-job", post(triage_gmail_job_handler))
        .route("/triage/reconcile", post(triage_reconcile_handler))
        .route("/ingest", post(ingest_handler))
        .route("/vault-links/scan", post(vault_scan_handler))
        .route("/vault-links/import", post(vault_import_handler))
        .route("/sources/scan", post(source_scan_handler))
        // Appended at the end of the chain on purpose: three streams edit this
        // router tonight, and a tail append is the hunk git merges.
        .route(
            "/triage/classify/refresh",
            post(triage_classify_refresh_handler),
        )
        .route(
            "/triage/classify/revert",
            post(triage_classify_revert_handler),
        )
        // feed-personalization 2026-09-03, on the EXISTING write set rather
        // than a third router: the projection layer below sits on this block
        // and nowhere else, so a route added here is covered by the same
        // sentence that covers every other mutation.
        .route("/feed/:id/interactions", post(feed_interactions_handler))
        .route("/feed/model/train", post(model_train_handler));

    Router::new()
        .merge(read_routes)
        // The projection layer sits on the write set and nowhere else. Trips tests
        // the method instead (`libs`-side rule: non-GET plus 2xx), and the reason
        // this file can do better is that the mutating routes are already declared
        // as one router above — a route added to that block is covered by the same
        // sentence, and a GET never pays for an export it cannot have caused.
        .merge(write_routes.layer(axum::middleware::from_fn(project_library_after_write)))
        .layer(cors)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // 2 MiB
}

/// Serialises exports. `std::fs::write` is not atomic, and a half-written note is
/// the one state the mechanism exists to prevent; two mutations landing together
/// must not interleave inside one file.
static EXPORT_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

/// "The library has changed since the last export finished."
///
/// One flag rather than a queue: the export is a full re-projection of
/// `feed_library()`, so two pending exports and one pending export produce
/// byte-identical folders. See `project_library_after_write` for the ordering
/// that makes coalescing safe.
static EXPORT_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Re-project the feed library into the vault after any successful write.
///
/// A layer rather than a line inside `feed_status_handler`, for the reason
/// `trips::server` gives: one call site puts the rule "a library change reaches the
/// vault" in one place, and the next handler that moves an item silently stops
/// projecting otherwise — a data-loss bug that looks like nothing at all.
///
/// After the response, never before it: the export reads the store, so running it
/// first would project the state the write is about to replace. And in a spawned
/// task, so a vault on iCloud cannot make a save feel slow.
///
/// A failure is logged and the request still succeeds. Refusing a save because the
/// vault is unreachable would trade a durable row for a missing file, which is the
/// wrong way round. `comms export-sources` is the repair, and it runs with this
/// process down.
///
/// What this does *not* catch is enrichment: a summary that arrives later comes from
/// the background drain, not from a request, so the note carries it only after the
/// next mutation or the next repair run.
async fn project_library_after_write(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let response = next.run(request).await;
    if !response.status().is_success() {
        return response;
    }
    let Some(root) = Config::load().obsidian_root else {
        return response;
    };
    // Mark the library dirty BEFORE queuing, so a writer can never be the one
    // whose change is missed: whoever holds the lock next sees the flag.
    EXPORT_DIRTY.store(true, Ordering::SeqCst);
    tokio::spawn(async move {
        let _held = EXPORT_LOCK.get_or_init(Default::default).lock().await;
        // Coalescing, not throttling. Keyboard triage turns one glance into a
        // burst of presses, and `feed_library()` reads the same whole set every
        // time — so N presses cost one export, and the LAST press is always
        // exported. The flag is cleared before the read and re-checked after
        // it: a write that lands mid-export sets it again and buys one more
        // pass, which is why this drops a redundant export and never the last
        // one. A task that finds the flag already clear exits having done
        // nothing, because someone else has just exported the state it wanted.
        while EXPORT_DIRTY.swap(false, Ordering::SeqCst) {
            let root = root.clone();
            let outcome = tokio::task::spawn_blocking(move || -> Result<_, String> {
                let root = markdown_root::MarkdownRoot::declare(root).map_err(|e| e.to_string())?;
                let cfg = Config::load();
                let store = Store::open(&cfg.database_path).map_err(|e| e.to_string())?;
                let saved = store.feed_library().map_err(|e| e.to_string())?;
                comms::projection::export_all(&root, &saved).map_err(|e| e.to_string())
            })
            .await;
            match outcome {
                Ok(Ok(report)) => {
                    for path in &report.refused {
                        eprintln!("comms: vault projection refused, somebody else owns {path}");
                    }
                }
                Ok(Err(error)) => eprintln!(
                    "comms: vault projection failed ({error}); the rows are safe, the folder is stale — run `comms export-sources`"
                ),
                Err(error) => eprintln!("comms: vault projection task failed ({error})"),
            }
        }
    });
    response
}

/// This capability's inbound gate.
///
/// `api_secret_file` still wins, because the dashboard's Vite proxy, the
/// browser extension and `axon-clip` all already hold that value; a deployment
/// converges on one token by pointing that reference and
/// `SJEL_INBOUND_TOKEN_FILE` at the same file.
///
/// `refuse_without_token` because comms is the capability where the loopback
/// bind was never the boundary: `POST /ingest` fetches an attacker-chosen URL,
/// and a page open in the operator's own browser is already inside loopback.
/// Without this, an unconfigured secret would leave those routes open instead
/// of closed, which is the one direction this must never move.
fn inbound_auth(cfg: &Config) -> sjel_server::InboundAuth {
    sjel_server::InboundAuth::resolve(cfg.api_secret.clone()).refuse_without_token()
}

#[tokio::main]
async fn main() {
    let cfg = Config::load();

    let auth = inbound_auth(&cfg);
    if !auth.is_configured() {
        eprintln!("warning: no inbound token is configured — every route except /health and /ready will reject all requests. Set api_secret_file (comms.config.example.json) or SJEL_INBOUND_TOKEN_FILE (schemas/deployment.env.example).");
    }

    let _background_services = BackgroundServices::start(&cfg);

    let app = build_router(&cfg.dashboard_origin);

    // Bind, the gate and the exit-on-failure behaviour all live in sjel_server
    // now; this file used to hand-roll the first and the third, and owned a
    // second copy of the second.
    sjel_server::serve(
        "comms-server",
        sjel_server::Reach::Loopback,
        cfg.port,
        app,
        auth,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backoff and the stored error class both key off this, and both are
    /// wrong in a way nobody notices if it silently answers "unknown" — an
    /// expired refresh token would then retry at full cadence forever.
    #[test]
    fn sweep_errors_land_in_the_class_that_drives_backoff() {
        assert_eq!(sweep_error_class("HTTP 401 Unauthorized"), "auth");
        assert_eq!(sweep_error_class("token refresh failed: auth"), "auth");
        assert_eq!(sweep_error_class("HTTP 429 Too Many Requests"), "quota");
        assert_eq!(sweep_error_class("userRateLimitExceeded"), "quota");
        assert_eq!(sweep_error_class("connect timeout"), "network");
        assert_eq!(sweep_error_class("dns failure"), "network");
        assert_eq!(sweep_error_class("something else entirely"), "unknown");
    }

    /// Serve the real router on an ephemeral port and return its base URL. The
    /// requests below go over actual HTTP, because the thing under test is the
    /// middleware stack — a handler called directly would skip the layer that
    /// is the entire point.
    async fn serve(api_secret: Option<&str>) -> String {
        // `with_token`, not `resolve`: `resolve` would fall back to the machine's
        // own deployment.env, so a test would pass or fail on whether the
        // operator running it has an overlay token.
        let auth = sjel_server::InboundAuth::with_token(api_secret.map(str::to_string))
            .refuse_without_token();
        let app = sjel_server::authenticated(build_router("http://127.0.0.1:47117"), auth);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// A page loaded in a browser on this host is already inside the loopback
    /// boundary, so `127.0.0.1` is not what contains `/ingest` — the token is.
    const HOSTILE_ORIGIN: &str = "https://attacker.example";

    #[tokio::test]
    async fn unauthenticated_cross_origin_post_to_ingest_is_rejected() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/ingest"))
            .header("Origin", HOSTILE_ORIGIN)
            .json(&json!({ "url": "https://example.com/anything" }))
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            401,
            "no token must not reach the handler"
        );
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap_or_default() != HOSTILE_ORIGIN)
                .unwrap_or(true),
            "the hostile origin must never be echoed back as allowed"
        );
    }

    #[tokio::test]
    async fn computed_quality_refresh_is_a_protected_explicit_write() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/feed/quality/refresh"))
            .json(&json!({ "days": 30 }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn every_sensitive_content_write_requires_authentication() {
        let base = serve(Some("s3cret")).await;
        let client = reqwest::Client::new();
        for (path, body) in [
            ("/content/mail/18f17d0a9bc123ef/cloud-preview", json!({})),
            (
                "/content/mail/18f17d0a9bc123ef/cloud-approval",
                json!({ "preview_hash": "reviewed-hash" }),
            ),
            (
                "/content/mail/18f17d0a9bc123ef/cloud-queue",
                json!({
                    "preview_hash": "reviewed-hash",
                    "provider_role": "cloud_summarization"
                }),
            ),
            ("/content/cloud-jobs/cloud-job-123/run", json!({})),
            (
                "/triage/18f17d0a9bc123ef/status",
                json!({ "status": "dismissed" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/stream",
                json!({ "stream": "feed" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/gmail",
                json!({ "action": "trash" }),
            ),
            (
                "/triage/18f17d0a9bc123ef/gmail-job",
                json!({ "decision": "retry" }),
            ),
            ("/triage/sweep", json!({ "limit": 100 })),
            ("/triage/relevance/refresh", json!({ "limit": 200 })),
            ("/triage/data-class/refresh", json!({ "limit": 500 })),
            ("/triage/reconcile", json!({})),
            (
                "/triage/18f17d0a9bc123ef/data-class",
                json!({ "data_class": "c3" }),
            ),
            (
                "/triage/bulk",
                json!({ "ids": ["18f17d0a9bc123ef"], "action": "dismiss" }),
            ),
            (
                "/feed/0123456789abcdef/data-class",
                json!({ "data_class": "c0" }),
            ),
            ("/triage/classify/refresh", json!({ "mode": "shadow" })),
            ("/triage/classify/revert", json!({ "all": true })),
        ] {
            let response = client
                .post(format!("{base}{path}"))
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{path} must stay behind auth");
        }
    }

    /// The de-escalation door answers 400 — not 404, not 500 — and says what it
    /// refused on. The class is checked before the item is looked up, so this
    /// reaches the refusal without a stored row being involved in the answer.
    #[tokio::test]
    async fn a_reclassification_the_rule_refuses_comes_back_as_a_bad_request() {
        let base = serve(Some("s3cret")).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/feed/0123456789abcdef/data-class"))
            .header("x-axon-token", "s3cret")
            .json(&json!({ "data_class": "confidential" }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("c0, c1, c2, c3"),
            "the refusal names the vocabulary, got: {body}"
        );
    }

    /// Both scoring paths, not just mail.
    ///
    /// The feed's own gate is `content_item::local_prompt_allowed`, which
    /// admits c2 -- and `POST /feed/:id/data-class` accepts any class in the
    /// vocabulary, so a c2 feed item is reachable. PRD §6.2b: C2 may only reach
    /// a cloud model after a ladder pass has reduced it to C1, and an embedding
    /// of `item_document` is the stored title, author and content verbatim. So
    /// `relevance_refresh_handler`, `enrich_many_in_background` and
    /// `evaluation_status_handler` resolve the two roles through this filter,
    /// exactly as `triage_relevance_handler` already did.
    #[test]
    fn every_relevance_path_accepts_only_loopback_model_endpoints() {
        assert!(loopback_inference_url("http://127.0.0.1:8000/v1"));
        assert!(loopback_inference_url("http://localhost:11434"));
        assert!(loopback_inference_url("http://[::1]:8000/v1"));
        assert!(!loopback_inference_url("https://api.example.com/v1"));
        assert!(!loopback_inference_url("https://embeddings.example.com"));

        // The source is the check: every `embedding_role()`/`reranking_role()`
        // resolution in the two feed handlers and the status endpoint carries
        // the filter, so a cloud-hosted role is never handed to `score_items`.
        let feed = include_str!("feed.rs");
        let resolutions =
            feed.matches(".embedding_role()").count() + feed.matches(".reranking_role()").count();
        let filtered = feed
            .matches("loopback_inference_url(&role.backend.base_url)")
            .count();
        assert_eq!(
            resolutions, filtered,
            "every role resolution in server/feed.rs must be filtered to loopback"
        );
        assert!(
            resolutions >= 6,
            "expected six resolutions, got {resolutions}"
        );
    }

    #[test]
    fn reconciled_gmail_location_maps_to_truthful_local_status() {
        assert_eq!(
            reconciled_status("proposed", ThreadLocation::Archive),
            "archived"
        );
        assert_eq!(
            reconciled_status("archived", ThreadLocation::Inbox),
            "proposed"
        );
        assert_eq!(
            reconciled_status("approved", ThreadLocation::Inbox),
            "approved"
        );
        assert_eq!(
            reconciled_status("archived", ThreadLocation::Trash),
            "trashed"
        );
    }

    #[test]
    fn mail_adapts_to_the_versioned_content_reader_contract() {
        let item = TriageItem {
            id: "thread-1".into(),
            from_addr: Some("sender@example.com".into()),
            subject: Some("A useful subject".into()),
            snippet: Some("A bounded Gmail preview.".into()),
            internal_date_ms: None,
            internal_date_text: Some("2026-08-04 09:30:00+02".into()),
            stream: "aktiv".into(),
            rationale: "Safe fallback.".into(),
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: comms::rules::MAIL_RULES_VERSION.into(),
            data_class: "c1".into(),
            data_class_rationale: "Mail metadata is Mine by default.".into(),
            data_classification_method: content_item::METHOD_DETERMINISTIC.into(),
            data_classification_version: content_item::MAIL_CLASSIFIER_VERSION.into(),
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: "2026-08-04 09:31:00+02".into(),
            last_seen: "2026-08-04 09:31:00+02".into(),
        };

        let value =
            serde_json::to_value(ContentItemOut::from_mail(item, Vec::new(), None)).unwrap();
        assert_eq!(value["schema_version"], "content-item-v2");
        assert_eq!(value["source"], "mail");
        assert_eq!(value["kind"], "mail");
        assert_eq!(value["content_label"], "Message preview");
        assert_eq!(value["content_status"], "thin");
        assert_eq!(value["data_class"]["value"], "c1");
        assert_eq!(
            value["processing_policy"]["cloud_handling"],
            "pseudonymization_required"
        );
        assert_eq!(value["cloud_processing"]["status"], "not_prepared");
        assert_eq!(value["cloud_processing"]["provider_calls"], 0);
        assert_eq!(value["mail"]["category"], "aktiv");
        assert!(value["mail"]["gmail_location"].is_null());
        assert!(value["mail"]["gmail_sync_status"].is_null());
        assert!(value["evaluation"].is_null());
    }

    /// A stored mail, built here because the lib's test fixtures are not
    /// visible from a binary target.
    fn triage_fixture(id: &str) -> TriageItem {
        TriageItem {
            id: id.into(),
            from_addr: Some("sender@example.com".into()),
            subject: Some("A useful subject".into()),
            snippet: Some("A bounded Gmail preview.".into()),
            internal_date_ms: None,
            internal_date_text: Some("2026-09-04 09:30:00+02".into()),
            stream: "aktiv".into(),
            rationale: "Safe fallback.".into(),
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: "mail-rules-v1".into(),
            data_class: "c1".into(),
            data_class_rationale: "Mail metadata is Mine by default.".into(),
            data_classification_method: content_item::METHOD_DETERMINISTIC.into(),
            data_classification_version: content_item::MAIL_CLASSIFIER_VERSION.into(),
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: "2026-09-04 09:31:00+02".into(),
            last_seen: "2026-09-04 09:31:00+02".into(),
        }
    }

    /// The list states a class for every item, under the exact key finance reads.
    ///
    /// `item_is_quotable` (`capabilities/finance/src/server.rs`) fails closed on a
    /// missing `data_class`, so a rename or a drop here would not break a test in
    /// finance -- it would silently empty the decision inbox's feed evidence, which
    /// is what happened before this field existed. Asserting the serialized key,
    /// not the struct field, is what makes that renaming visible.
    #[test]
    fn a_listed_feed_item_states_the_class_finance_filters_on() {
        let mut item = FeedItem::new("https://example.com/an-article", "news", "article");
        item.title = Some("A synthetic article".into());
        item.day = "2026-09-06".into();
        item.created_at = "2026-09-06 09:00:00+00:00".into();

        // The undeclared default: an item nobody classified is c1, not absent.
        let listed =
            serde_json::to_value(FeedListItem::from_store(item.clone(), None, None, None)).unwrap();
        assert_eq!(listed["data_class"], "c1");

        item.declare_class(&DataClass::declared_by_source("c0", "A published article."));
        let public =
            serde_json::to_value(FeedListItem::from_store(item, None, None, None)).unwrap();
        assert_eq!(public["data_class"], "c0");
    }

    /// One writer, one meaning, and one shape on the wire.
    #[test]
    fn score_bp_is_basis_points_and_absent_is_null() {
        let item = triage_fixture("thread-bp");

        let unscored =
            serde_json::to_value(TriageOut::from_store(item.clone(), Vec::new(), None, None))
                .unwrap();
        assert!(
            unscored["score_bp"].is_null(),
            "a mail with no evaluation is null, not zero -- 0 is a real score"
        );
        assert!(unscored["evaluated_at"].is_null());

        let scored = serde_json::to_value(TriageOut::from_store(
            item,
            Vec::new(),
            Some((0.6234, "2026-09-05 21:00:00+00:00".into())),
            None,
        ))
        .unwrap();
        assert_eq!(scored["score_bp"], 6234);
        assert_eq!(scored["evaluated_at"], "2026-09-05 21:00:00+00:00");
    }

    /// The mail reader's `evaluation` used to be hardcoded null. It carries the
    /// factor breakdown now, in the same shape the feed uses.
    #[test]
    fn a_mail_carries_its_factor_breakdown() {
        let item = triage_fixture("thread-factors");
        let evaluation = mail_evaluation::evaluate(&item, None, None, "context", false);
        let value = serde_json::to_value(ContentItemOut::from_mail(
            item,
            Vec::new(),
            Some(evaluation),
        ))
        .unwrap();
        assert!(!value["evaluation"].is_null());
        assert_eq!(
            value["evaluation"]["evaluator_revision"],
            mail_evaluation::MAIL_EVALUATOR_REVISION
        );
        let keys = value["evaluation"]["factors"]
            .as_array()
            .expect("factors")
            .iter()
            .map(|factor| factor["key"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["interest", "category", "age", "urgency"]);
    }

    #[tokio::test]
    async fn a_wrong_token_is_rejected_too() {
        let base = serve(Some("s3cret")).await;
        for header in [("Authorization", "Bearer wrong"), ("X-Axon-Token", "wrong")] {
            let response = reqwest::Client::new()
                .post(format!("{base}/ingest"))
                .header(header.0, header.1)
                .header("Origin", HOSTILE_ORIGIN)
                .json(&json!({ "url": "https://example.com/anything" }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{} with a wrong value", header.0);
        }
    }

    #[tokio::test]
    async fn an_unconfigured_secret_disables_mutating_routes_rather_than_opening_them() {
        for secret in [None, Some("")] {
            let base = serve(secret).await;
            let response = reqwest::Client::new()
                .post(format!("{base}/ingest"))
                .json(&json!({ "url": "https://example.com/anything" }))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                403,
                "an absent secret must close the route, never leave it open"
            );
        }
    }

    #[tokio::test]
    async fn the_right_token_gets_past_the_layer_and_liveness_needs_none() {
        let base = serve(Some("s3cret")).await;

        // Past the auth layer the handler fails on its own (no database in a
        // unit test); what matters is that the failure is no longer 401/403.
        let authorized = reqwest::Client::new()
            .post(format!("{base}/ingest"))
            .header("Authorization", "Bearer s3cret")
            .json(&json!({ "url": "https://example.com/anything" }))
            .send()
            .await
            .unwrap();
        assert!(
            authorized.status() != 401 && authorized.status() != 403,
            "a valid token was still refused: {}",
            authorized.status()
        );

        let health = reqwest::get(format!("{base}/health")).await.unwrap();
        assert_eq!(health.status(), 200, "liveness stays pollable");
    }

    /// The migration onto the shared gate, stated as a test: reads used to
    /// answer without a token because the loopback bind was treated as the
    /// boundary. A feed entry is personal content and the port is about to be
    /// reachable from the tailnet, so it is not.
    #[tokio::test]
    async fn read_routes_now_need_the_token_too() {
        let base = serve(Some("s3cret")).await;
        let client = reqwest::Client::new();
        for path in ["/feed", "/triage", "/sources", "/routes"] {
            let refused = client.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(refused.status(), 401, "{path} answered without a token");

            let allowed = client
                .get(format!("{base}{path}"))
                .header("X-Axon-Token", "s3cret")
                .send()
                .await
                .unwrap();
            assert!(
                allowed.status() != 401 && allowed.status() != 403,
                "{path} refused a valid token: {}",
                allowed.status()
            );
        }
    }
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("main.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
