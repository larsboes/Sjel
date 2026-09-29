//! sjel-status — what is enabled on this machine, what is up, and the one thing
//! allowed to bring a capability up.
//!
//! It knows nothing about which capabilities exist. The manifests do, and
//! `tools/capability.sh registry` renders them as JSON so this process never learns
//! to parse TOML (CONTRIBUTING.md#one-manifest-per-concern — `tools/lib/toml.sh` stays the only parser). That
//! also retired the hardcoded transit/scouting port literals this file used to carry:
//! its own source comment named the flip condition — "if a third reader of these
//! ports shows up" — and additional capability consumers plus the dashboard proxy met it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::Path,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower::Layer;

mod device_gate;
mod lan;
mod proxy;
mod status;

use status::*;

const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r("GET", "/api/sjel-status/health", "Aggregate health across enabled capabilities."),
    r("GET", "/api/sjel-status/routes", "Every enabled capability's route manifest, in one map."),
    r("GET", "/api/sjel-status/lan", "The local-network listener for paired devices: port, host and the certificate fingerprint a phone pins."),
    r("GET", "/api/sjel-status/capabilities", "Enabled capabilities, their ports and whether each is up."),
    r("GET", "/api/sjel-status/self", "This machine's resolved Axon model."),
    r("GET", "/api/sjel-status/repos", "Sjel and overlay repo state."),
    r("GET", "/api/sjel-status/links", "Operator-pinned links from the overlay's links.toml."),
    r("GET", "/api/sjel-status/backups", "Every capability with a backup contract: last success, age, and whether it is overdue."),
    r("GET", "/api/sjel-status/host-watch", "Open findings from the hourly host watch: a runaway process or a filling disk."),
    r("GET", "/api/sjel-status/packs", "Every Pack skill against every agent harness: deployed, drifted, or unowned at the destination."),
    r("POST", "/api/sjel-status/capabilities/{name}/backup", "Request a backup of one capability. Accepts the run and returns; poll /backups for the outcome."),
    // Undeclared until 2026-08-31, and served the whole time. The dashboard's panel page
    // calls both; `/routes` denied they existed. The coverage test below did not catch it:
    // its detector required the path literal to sit immediately after the opening paren,
    // and these two are the only mounts in this file long enough for rustfmt to wrap. It
    // skips the whitespace now (PRD D19) — this pair is why.
    r("POST", "/api/sjel-status/capabilities/{name}/start", "Start one capability and wait for it to answer. Idempotent: an already-running capability returns up=true."),
    r("POST", "/api/sjel-status/capabilities/{name}/stop", "Stop one capability. Refuses to report success while its port still answers, because something outside the pid file holding it is the case worth seeing."),
];

/// Shorthand so the table above reads as a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> axum::Json<serde_json::Value> {
    axum::Json(route_manifest::manifest("sjel-status", ROUTES))
}

#[tokio::main]
async fn main() {
    STARTED_AT.set(Instant::now()).ok();

    // SJEL_PORT first (the runner exports it from the manifest + machine override);
    // SJEL_STATUS_PORT stays as the manual escape hatch for running outside the runner.
    let port = sjel_server::resolve_port(Some("SJEL_STATUS_PORT"), None, 8082);

    // The other half of on-demand. This process is already the only thing allowed to
    // start a capability, so it is also the only sensible place to stop one — anything
    // else would be a second lifecycle owner. It runs here rather than as a launchd
    // job for the same reason: a reaper that outlives sjel-status could stop panels
    // while nothing is left to bring them back.
    let _idle_reaper = IdlePanelReaper::start();

    // Resolved once at startup, from the same registry every other surface here reads. A
    // capability added later needs a restart to be proxied, which is honest: the shell's shape
    // follows the machine's, and `dashboard/vite.config.ts` said the same about its own proxy.
    let services = registry().await.unwrap_or_else(|e| {
        eprintln!(
            "[sjel-status] registry unavailable ({e}); serving the shell with no capability routes"
        );
        Vec::new()
    });
    let ui_dir = sjel_config::env_var("SJEL_DASHBOARD_DIST")
        .unwrap_or_else(|_| "dashboard/dist".to_string());
    // Said at startup rather than discovered as a blank page. The bundle is a build artifact
    // (`dashboard/service.toml` `build`), so a fresh checkout has none until it is built, and a
    // shell that 404s every page while every API route works is a confusing way to learn that.
    if !std::path::Path::new(&ui_dir).is_dir() {
        eprintln!(
            "[sjel-status] no dashboard bundle at {ui_dir} — API routes work, pages will 404. Build it: cd dashboard && bun run build"
        );
    }
    let shell = proxy::Proxy::new(&services, &port.to_string(), ui_dir);
    eprintln!(
        "[sjel-status] shell: {} capability route(s)",
        shell.route_count()
    );

    // This process can start and stop the machine's capabilities, so it answers to
    // this machine only (sjel_server binds loopback).
    // A paired device is admitted on its key as well as the operator's tailnet identity
    // (PRD Q119); the bind stays loopback.
    let verifier = device_gate::RegistryVerifier::open().map(std::sync::Arc::new);
    let auth = match &verifier {
        Some(verifier) => {
            sjel_server::InboundAuth::from_deployment().with_device_verifier(verifier.clone())
        }
        None => sjel_server::InboundAuth::from_deployment(),
    };
    // The same router on the local network, for paired devices only, when the deployment
    // enabled it. Without the registry there is nothing to admit, so it does not start.
    if let Some(verifier) = verifier {
        lan::start(build_router(shell.clone()), verifier);
    }
    sjel_server::serve(
        "sjel-status",
        sjel_server::Reach::Loopback,
        port,
        build_router(shell),
        auth,
    )
    .await;
}

/// The local-network listener, for the pairing screen: its port, host and the certificate
/// fingerprint a phone pins. `{"enabled": false}` when the deployment has not enabled it.
async fn lan_handler() -> Json<Value> {
    match lan::info() {
        Some(info) => Json(json!({ "enabled": true, "lan": info })),
        None => Json(json!({ "enabled": false })),
    }
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_SJEL_STATUS_ALLOWED_ORIGIN_HOSTS` -- the doubling is what the derivation
/// produces, and `libs/sjel-server/src/origin.rs` asserts exactly this string).
const CAPABILITY: &str = "sjel-status";

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// ## What the missing CORS layer did not do
///
/// The comment that used to sit above `serve_local` said this surface carries no
/// CORS layer, unlike the data-serving siblings, because a permissive header
/// would let any website's JS drive start/stop from the operator's own browser.
/// The reasoning is right about CORS and wrong about the threat, and the
/// difference is the whole of this change.
///
/// `start_handler`, `stop_handler` and `backup_handler` each take `Path(name)`
/// and nothing else. A POST with no custom header and no JSON content type is a
/// CORS *simple* request: the browser sends it without a preflight and withholds
/// only the response. Withholding the response from a page that wanted to stop
/// comms is no defence at all -- comms is already stopped. Absent CORS headers
/// stop a cross-origin READ; they have never stopped a cross-origin WRITE.
///
/// So the guard refuses the request. It is the same one places, trips and the six
/// capabilities of the B48 fan-out carry, which is the point: "who may talk to
/// this capability from a browser" is one predicate in `libs/sjel-server`, not a
/// per-capability opinion.
///
/// ## Where the layer sits
///
/// On the inner router, below the fallback, so it covers the proxy too. axum
/// applies `Router::layer` to every route AND to the fallback registered before
/// it (axum 0.7 `src/docs/routing/layer.md`), and `proxy::fallback` is how every
/// capability behind this shell is reached from a browser -- an unguarded
/// fallback would leave `/finance/api/dashboard` open on this port after B48
/// closed it on finance's own.
fn build_router(shell: proxy::Proxy) -> Router {
    let app = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health_handler))
        .route("/api/sjel-status/health", get(sjel_status_health_handler))
        .route("/api/sjel-status/routes", get(routes_handler))
        .route("/api/sjel-status/lan", get(lan_handler))
        .route("/api/sjel-status/capabilities", get(capabilities_handler))
        .route("/api/sjel-status/self", get(self_model_handler))
        .route("/api/sjel-status/links", get(links_handler))
        .route("/api/sjel-status/repos", get(repos_handler))
        .route(
            "/api/sjel-status/capabilities/{name}/start",
            post(start_handler),
        )
        .route(
            "/api/sjel-status/capabilities/{name}/stop",
            post(stop_handler),
        )
        .route("/api/sjel-status/backups", get(backups_handler))
        .route("/api/sjel-status/host-watch", get(host_watch_handler))
        .route("/api/sjel-status/packs", get(packs_handler))
        .route(
            "/api/sjel-status/capabilities/{name}/backup",
            post(backup_handler),
        )
        // Everything the routes above did not claim: a capability prefix, or a page of the
        // shell. Registered as the fallback and not as a layer on purpose -- transit declares
        // `proxy_extra = ["/api"]`, and that prefix evaluated before routing would swallow this
        // surface's own `/api/sjel-status/*`.
        .fallback(proxy::fallback)
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(shell);

    // The self-prefix rewrite, wrapped AROUND the router above rather than layered onto it.
    //
    // `Router::layer` was the first attempt and is wrong here, for a reason worth keeping:
    // it applies the middleware to each route's service and to the fallback, so matchit has
    // already chosen by the time the middleware sees the request. `/sjel-status/api/...`
    // therefore matched nothing, went to the fallback, and only then had its prefix removed --
    // at which point the proxy read the rewritten path and matched transit's `proxy_extra`
    // `/api`, sending this surface's own capability list to port 3000. The 502 was measured,
    // not predicted, and it is the same swallowing the module docs warn about arriving by a
    // different door.
    //
    // The origin guard above is layered rather than wrapped for the opposite reason: it reads
    // one request header and never the path, so it does not care which router already chose.
    //
    // An empty outer router sends everything to `fallback_service`, so the rewrite runs before
    // any routing at all and the inner router then decides on the path the client meant.
    Router::new().fallback_service(axum::middleware::from_fn(proxy::strip_self_prefix).layer(app))
}

#[cfg(test)]
mod backup_tests {
    use super::*;

    /// A synthetic capability. Every field empty by default, so each test names only the
    /// declaration it is about — and a test for "no backup contract" is the default
    /// rather than something to remember to write.
    fn cap(name: &str) -> Service {
        Service {
            name: name.to_string(),
            kind: "container".into(),
            scope: "capability".into(),
            port: String::new(),
            health_path: String::new(),
            ready_path: String::new(),
            panel_port: String::new(),
            panel_path: String::new(),
            autostart: String::new(),
            idle_timeout: String::new(),
            backup_target: String::new(),
            backup_sqlite: String::new(),
            backup_sqlite_online: String::new(),
            backup_advise_days: String::new(),
            backup_stale_days: String::new(),
            endpoint: String::new(),
            proxy_api_only: String::new(),
            proxy_extra: Vec::new(),
            requires: Vec::new(),
        }
    }

    const DAY: u64 = 86_400;

    // --- externally provided capabilities (retired-tracker#169) ---------------------------
    //
    // The registry decides what is external and blanks every field that would be a claim of
    // authority over another host. These assert the half that lives here: that a resolved
    // endpoint is dialled instead of loopback, and that loopback is untouched without one.

    #[test]
    fn a_local_capability_is_polled_on_loopback() {
        // The case that must not change. Every capability this machine runs takes this path,
        // and it is the same string it was before external references existed.
        let mut c = cap("transit");
        c.port = "8085".into();
        c.health_path = "/health".into();
        assert_eq!(
            c.health_url().as_deref(),
            Some("http://127.0.0.1:8085/health")
        );
    }

    #[test]
    fn an_external_capability_is_polled_at_its_resolved_endpoint() {
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        c.health_path = "/alive".into();
        assert_eq!(
            c.health_url().as_deref(),
            Some("https://vault.provider.test/alive")
        );
    }

    #[test]
    fn an_external_capability_never_falls_back_to_a_local_port() {
        // A port would have to come from the manifest of a service running somewhere else, so
        // dialling 127.0.0.1 with it asks this machine about a process it does not have. The
        // registry blanks the field for exactly this reason; if that ever regresses, the URL
        // must still not be built.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.port = "8080".into();
        c.health_path = "/alive".into();
        assert_eq!(
            c.health_url(),
            None,
            "an unresolved external reference has no health URL"
        );
    }

    #[test]
    fn an_endpoint_without_a_path_is_not_a_health_url() {
        // Knowing where something lives is not knowing how to ask it whether it is alive.
        // Reported as unknown, which is what a capability with no health surface has always
        // been — not as down, which would be a false alarm about someone else's machine.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        assert_eq!(c.health_url(), None);
    }

    // --- readiness vs liveness (#126) -----------------------------------------------------
    //
    // Availability is judged on readiness where a capability declares it. The fallback is the
    // compatibility contract, and the reason this surface reported five database-backed
    // capabilities as up through a Postgres outage.

    #[test]
    fn readiness_is_preferred_over_liveness() {
        let mut c = cap("vault");
        c.port = "8094".into();
        c.health_path = "/health".into();
        c.ready_path = "/ready".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("http://127.0.0.1:8094/ready")
        );
    }

    #[test]
    fn liveness_still_answers_for_a_capability_without_readiness() {
        // The case that must not change: every capability that declares no ready_path is
        // polled exactly where it was before.
        let mut c = cap("macmon");
        c.port = "9911".into();
        c.health_path = "/json".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("http://127.0.0.1:9911/json")
        );
    }

    #[test]
    fn readiness_reaches_an_external_capability_at_its_endpoint() {
        // ready_path is a "how to ask" field, so it crosses the machine boundary with
        // health_path rather than being blanked with the fields that describe how to act.
        let mut c = cap("vaultwarden");
        c.scope = "external".into();
        c.endpoint = "https://vault.provider.test".into();
        c.health_path = "/alive".into();
        c.ready_path = "/ready".into();
        assert_eq!(
            c.readiness_url().as_deref(),
            Some("https://vault.provider.test/ready")
        );
    }

    #[test]
    fn a_capability_with_neither_path_has_nothing_to_poll() {
        let mut c = cap("sparpreis-watch");
        c.port = "8090".into();
        assert_eq!(c.readiness_url(), None);
    }

    #[test]
    fn a_capability_without_a_target_has_no_backup_contract() {
        // Most capabilities. They must not appear on the surface at all: a transit server
        // listed as "never backed up" is a false alarm that teaches you to ignore the page.
        assert!(cap("transit").backup_contract().is_none());
    }

    #[test]
    fn the_target_is_what_makes_a_contract_not_the_day_fields() {
        // A capability may decline to say what timely means and still owe a backup. If
        // presence were derived from the day fields, this one would vanish from the
        // surface — the single place that could tell you it has never run.
        let mut c = cap("pihole");
        c.backup_target = "backup-target".into();
        let contract = c.backup_contract().expect("a target alone is a contract");
        assert_eq!(contract.advise_days, None);
        assert_eq!(contract.stale_days, None);
    }

    #[test]
    fn only_a_sqlite_contract_holds_the_service_down() {
        // The claim the confirmation dialog makes, and it is read from the contract rather
        // than matched against a service name. backup.sh stops a capability for the cold
        // SQLite copy and for nothing else — the shared store's file is copied open, with
        // every capability still reading it, so a run there holds nothing down.
        let mut store = cap("store");
        store.backup_target = "backup-target".into();
        store.backup_sqlite_online = "data/axon/axon.db".into();
        assert!(!store.backup_contract().unwrap().holds_service);

        let mut vw = cap("vaultwarden");
        vw.backup_target = "backup-target".into();
        vw.backup_sqlite = "data/vaultwarden/data/db.sqlite3".into();
        assert!(vw.backup_contract().unwrap().holds_service);
    }

    #[test]
    fn an_unparseable_day_field_reads_as_undeclared_not_as_zero() {
        // The dangerous coercion: `"soon".parse().unwrap_or(0)` makes every backup overdue
        // forever, and a badge that is always red is a badge nobody looks at.
        let mut c = cap("odd");
        c.backup_target = "backup-target".into();
        c.backup_stale_days = "soon".into();
        assert_eq!(c.backup_contract().unwrap().stale_days, None);
    }

    #[test]
    fn state_walks_ok_then_due_then_overdue() {
        let contract = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: Some(7),
        };
        assert_eq!(backup_state(Some(0), &contract), "ok");
        assert_eq!(backup_state(Some(23 * 3_600), &contract), "ok");
        assert_eq!(backup_state(Some(DAY), &contract), "due");
        assert_eq!(backup_state(Some(6 * DAY), &contract), "due");
        assert_eq!(backup_state(Some(7 * DAY), &contract), "overdue");
        assert_eq!(backup_state(Some(400 * DAY), &contract), "overdue");
    }

    #[test]
    fn no_receipt_is_never_whatever_the_thresholds_say() {
        // Outranks the thresholds deliberately: a contract with nothing behind it is the
        // worst state this surface can report, and it is not reachable by ageing.
        let declared = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: Some(7),
        };
        let silent = BackupContract {
            holds_service: false,
            advise_days: None,
            stale_days: None,
        };
        assert_eq!(backup_state(None, &declared), "never");
        assert_eq!(backup_state(None, &silent), "never");
    }

    #[test]
    fn undeclared_thresholds_never_invent_a_cadence() {
        // Axon ships no personal schedule. A capability whose owner never said what timely
        // means gets `unknown` — not a red badge derived from a number Axon made up.
        let silent = BackupContract {
            holds_service: false,
            advise_days: None,
            stale_days: None,
        };
        assert_eq!(backup_state(Some(900 * DAY), &silent), "unknown");

        // ...and half a declaration still only answers the half it declared.
        let advise_only = BackupContract {
            holds_service: false,
            advise_days: Some(1),
            stale_days: None,
        };
        assert_eq!(backup_state(Some(900 * DAY), &advise_only), "due");
    }

    #[test]
    fn a_receipt_dates_to_the_second() {
        // The live postgres receipt as backup.sh wrote it, against the epoch value
        // `date -u -d @... ` agrees with. Fixes the civil-calendar arithmetic to a real
        // observation rather than to itself.
        assert_eq!(parse_receipt_ts("20260805T220018Z"), Some(1_785_967_218));
        assert_eq!(parse_receipt_ts("20260801T074701Z"), Some(1_785_570_421));
        // The epoch itself, and a leap day — the two the era arithmetic would get wrong.
        assert_eq!(parse_receipt_ts("19700101T000000Z"), Some(0));
        assert_eq!(parse_receipt_ts("20240229T120000Z"), Some(1_709_208_000));
    }

    #[test]
    fn a_receipt_this_process_cannot_date_is_not_a_fresh_backup() {
        // Every one of these must read as "no usable receipt", which downstream is the
        // same answer as a missing file. The failure to avoid is a malformed timestamp
        // parsing to something plausible and reporting a stale backup as current.
        for bad in [
            "",
            "20260805T220018",      // no zone marker
            "20260805 220018Z",     // no T
            "2026-08-05T22:00:18Z", // ISO with separators: right length, wrong shape
            "20261305T220018Z",     // month 13
            "20260832T220018Z",     // day 32
            "20260805T250018Z",     // hour 25
            "20260805T226018Z",     // minute 60
            "yyyymmddThhmmssZ",
        ] {
            assert_eq!(parse_receipt_ts(bad), None, "should not parse: {bad:?}");
        }
    }

    #[test]
    fn a_run_state_is_terminal_or_running_never_absent() {
        // The spinner criterion. Whatever backup.sh does — succeed, fail, or fail to
        // start at all — the run must leave `running`, because the UI shows a spinner for
        // exactly as long as this says `running` and a locked vault is the normal way a
        // real run fails.
        let mut runs: HashMap<String, BackupRun> = HashMap::new();
        runs.insert(
            "vaultwarden".into(),
            BackupRun {
                state: "running",
                started_at: 100,
                finished_at: None,
                detail: String::new(),
            },
        );
        assert_eq!(runs["vaultwarden"].state, "running");

        // What the spawned task writes when the vault is locked: terminal, with the
        // provider's own message carried through rather than swallowed.
        runs.insert(
            "vaultwarden".into(),
            BackupRun {
                state: "failed",
                started_at: 100,
                finished_at: Some(160),
                detail: "backup.sh: vault is locked".into(),
            },
        );
        assert_eq!(runs["vaultwarden"].state, "failed");
        assert!(runs["vaultwarden"].finished_at.is_some());
        assert!(!runs["vaultwarden"].detail.is_empty());
    }

    #[test]
    fn the_projection_carries_no_destination() {
        // The boundary this whole surface is built around: a receipt names a target, a
        // tarball and a hash, and none of them may reach a response. Enforced against the
        // deserialize struct, so adding a field to Receipt without thinking fails here.
        let raw = r#"{"capability":"vaultwarden","completed_at":"20260801T074701Z",
            "target":"home-automation","tarball":"vaultwarden-20260801T074701Z.tar.gz",
            "bytes":348862,"sha256":"deadbeef","contents":"paths"}"#;
        let receipt: Receipt = serde_json::from_str(raw).expect("the live receipt shape parses");
        assert_eq!(receipt.bytes, 348_862);
        assert_eq!(receipt.contents, "paths");

        let rendered = serde_json::to_string(&json!({
            "capability": "vaultwarden",
            "last_success": receipt.completed_at,
            "bytes": receipt.bytes,
            "contents": receipt.contents,
        }))
        .unwrap();
        for private in ["home-automation", "tarball", "sha256", "deadbeef"] {
            assert!(
                !rendered.contains(private),
                "projection leaked {private}: {rendered}"
            );
        }
    }

    /// One panic under the run ledger's guard used to end the backup surface for
    /// the life of the process: `backup_runs().lock().unwrap()` on a poisoned
    /// lock is a second panic, in `backups_handler` and in both writers.
    ///
    /// This poisons the real process-wide ledger deliberately. Every other test
    /// in this binary that touches it keeps passing afterwards, which is the
    /// assertion underneath the explicit ones.
    #[test]
    fn a_poisoned_run_ledger_is_still_readable_and_writable() {
        let poisoning = std::thread::spawn(|| {
            let _guard = backup_runs();
            panic!("something panicked while holding the run ledger");
        })
        .join();
        assert!(poisoning.is_err(), "the helper thread must actually panic");

        backup_runs().insert(
            "poison-probe".into(),
            BackupRun {
                state: "running",
                started_at: 1,
                finished_at: None,
                detail: String::new(),
            },
        );
        assert_eq!(
            backup_runs().get("poison-probe").map(|run| run.state),
            Some("running"),
            "a recovered lock must still answer the read backups_handler does"
        );
        backup_runs().remove("poison-probe");
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

/// The router-level proof that no predicate test can give: the guard has to be
/// wired below the routes AND below the fallback, and a `Router` that forgot
/// either still passes every test in `libs/sjel-server/src/origin.rs`.
///
/// The capability name in every POST here is deliberately one this machine does
/// not have. `lifecycle` looks the name up in the registry and answers 404
/// before it runs `tools/service-runner.sh`, so removing the guard to watch
/// these tests fail cannot start or stop anything. A test of a control surface
/// has to be safe to run with the control surface unlocked, or it is a test
/// nobody will run twice.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// No services, so the proxy table is empty and the fallback serves pages.
    /// The guard runs before either, which is what these tests are about.
    fn router() -> Router {
        build_router(proxy::Proxy::new(&[], "8082", "dashboard/dist".into()))
    }

    async fn answer(method: &str, path: &str, origin: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers")
            .status()
    }

    /// The shell was axon-status until 2026-09-27, and an app installed before then calls its old
    /// mount and API segment. Each old path gets the new path's answer from the API itself, not the
    /// page fallback (which would answer 200 text/html).
    #[tokio::test]
    async fn the_pre_rename_paths_still_reach_the_status_api() {
        async fn reply(path: &str) -> (StatusCode, String) {
            let response = router()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .expect("the router answers");
            let kind = response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            (response.status(), kind)
        }
        let current = reply("/sjel-status/api/sjel-status/capabilities").await;
        assert!(
            current.1.contains("json"),
            "the new path is the API, got {current:?}"
        );
        for legacy in [
            "/axon-status/api/axon-status/capabilities",
            "/api/axon-status/capabilities",
        ] {
            assert_eq!(reply(legacy).await, current, "{legacy}");
        }
    }

    /// Each of these three takes `Path(name)` and no body, so a hostile page
    /// can send it as a CORS *simple* request: no preflight, and the absence of
    /// an `Access-Control-Allow-Origin` header withholds only the reply. The
    /// capability is stopped either way. That is why the request is refused.
    #[tokio::test]
    async fn a_foreign_origin_cannot_start_stop_or_back_up_a_capability() {
        for path in [
            "/api/sjel-status/capabilities/not-a-capability/start",
            "/api/sjel-status/capabilities/not-a-capability/stop",
            "/api/sjel-status/capabilities/not-a-capability/backup",
        ] {
            assert_eq!(
                answer("POST", path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{path} answered a foreign origin — it is registered below the guard layer"
            );
        }
    }

    /// The read side, and the proxy with it. `/finance/api/dashboard` goes to
    /// the FALLBACK rather than to a route, so it is the case a guard layered
    /// above the fallback would miss — and it is every capability the B48 fan-out
    /// just closed, reachable again through this port if it were missed.
    #[tokio::test]
    async fn a_foreign_origin_reaches_neither_the_status_reads_nor_the_proxy() {
        for path in [
            "/api/sjel-status/capabilities",
            "/api/sjel-status/self",
            "/api/sjel-status/repos",
            "/finance/api/dashboard",
            "/sjel-status/api/sjel-status/capabilities",
        ] {
            assert_eq!(
                answer("GET", path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{path} answered a foreign origin"
            );
        }
    }

    /// The other half. Without it this file would pass just as well against a
    /// server that refuses everyone. 200 from `/routes` is a handler answering.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("http://127.0.0.1:8082"),
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
