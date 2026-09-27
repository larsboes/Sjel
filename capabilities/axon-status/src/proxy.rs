//! The shell's front door: capability routes, then the built dashboard bundle.
//!
//! # Why this is here rather than in the dashboard
//!
//! The dashboard is an `adapter-static` SPA whose every API call is a *relative* fetch
//! (`dashboard/src/lib/api.ts` `request()`), so something has to map `/comms/...` onto
//! `127.0.0.1:8083`. Until now that something was Vite's dev-server proxy, which exists only
//! when `command === "serve"` (`dashboard/vite.config.ts`) -- `preview` declares none. So the
//! built bundle has never had a working API path in any mode, and "serve the bundle statically"
//! was not a swap: it needed this file first.
//!
//! Supervising a dev server to get a reverse proxy cost 95.5 MB across node, esbuild and bun,
//! against 24.9 MB for all three Rust servers combined. axon-status already had to stay up, it
//! already resolves the registry (`status/registry.rs`), and it already links a reqwest client.
//!
//! # Routing, and why `fallback` rather than a layer
//!
//! Proxy matching runs as the router's fallback, so axon-status' own routes always win. That is
//! not a stylistic choice. `transit` declares `proxy_extra = ["/api"]`, and a bare `/api` prefix
//! evaluated *before* routing would swallow `/api/axon-status/health` and send this surface's own
//! health to transit. As a fallback it cannot: the real route matches first, and only genuinely
//! unrouted paths are offered to the proxy.
//!
//! The rules mirror the Vite proxy exactly, because the shell is the same shell:
//!
//! - `/<name>` reaches the capability with the prefix stripped, so a capability's own contract
//!   never has to know it is proxied. `proxy_api_only` narrows the mount to `/<name>/api`.
//! - `proxy_extra` prefixes pass through *unstripped* -- transit's `/api`, scouting's `/discover`
//!   are paths that predate the uniform rule.
//! - Spine and portless entries are skipped, as they are there.
//!
//! One rule Vite never needed: **this surface must not proxy to itself.** `/axon-status` resolves
//! to the port this process is listening on, and forwarding to it would re-enter this fallback and
//! loop until the connection pool gave out. Excluded by port, not by name, so a rename cannot
//! reintroduce it.
//!
//! That exclusion alone made the mirror a lie, and cost the whole Capabilities page. Vite proxies
//! `/axon-status` -> 8082 *with the prefix stripped*, so the client asks for
//! `/axon-status/api/axon-status/capabilities` and 8082 receives `/api/axon-status/capabilities`.
//! Serving the bundle from this process removed the hop but not the prefix the client still sends,
//! and an unrouted path is a page: all ten `axonStatus.*` calls in `dashboard/src/lib/api.ts`
//! answered **200 with the SPA's own HTML**, `JSON.parse` threw on `<!doctype`, and the page that
//! this process was at that moment serving reported that this process was unavailable. The footer
//! read `0 capabilities` for the same reason, and `start`/`stop` were on the same dead prefix --
//! so on-demand capabilities could not be started from the surface that exists to start them.
//!
//! [`strip_self_prefix`] restores the missing half. Stripping before routing is what Vite does,
//! and it is the reason the client needs no change and no rebuild.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

/// One prefix and where it goes. `strip` is the difference between the uniform rule and the
/// grandfathered `proxy_extra` paths, and it is the only difference.
#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub(crate) prefix: String,
    pub(crate) target: String,
    /// What to remove from the front of the path, which is NOT the same as `prefix`.
    ///
    /// `proxy_api_only` mounts a capability at `/<name>/api` while still stripping only
    /// `/<name>`, so `/finance/api/finance/health` reaches finance as `/api/finance/health`.
    /// Conflating the two sent it `/finance/health` and got a 404 — caught by exercising the
    /// route rather than by reading it, because both readings look right in prose.
    ///
    /// `None` for a `proxy_extra` prefix, which passes through untouched.
    pub(crate) strip: Option<String>,
    /// Comms is the only capability whose proxy adds a credential, and the flag is set from the
    /// registry entry rather than re-derived at request time. An earlier draft of this file
    /// decided it by comparing the target against a helper that returned an empty string, and
    /// `ends_with("")` is true for every string — which would have put the comms bearer token on
    /// every request to every capability. Carried as data on the route so there is one place it
    /// can be true.
    pub(crate) inject_comms_auth: bool,
}

#[derive(Clone)]
pub(crate) struct Proxy {
    routes: Arc<Vec<Route>>,
    /// Comms authenticates every route except `/health` and `/ready`
    /// (`libs/sjel-server/src/auth.rs`). Resolved once at startup with
    /// `sjel_server::auth::token_from_file`, which is the same reader the server itself uses --
    /// so this is not a fourth implementation of the token shape, it is the first Rust consumer
    /// of the one that already existed. `None` stays fail-closed: comms answers 401 and the page
    /// says so, which is the honest outcome of an unconfigured credential.
    comms_authorization: Option<HeaderValue>,
    /// The deployment token, sent upstream for a request the gate admitted on a device key.
    /// A capability behind the shell gates on the tailnet identity or this token, and a device
    /// arriving without the tailnet carries neither (PRD Q119).
    device_authorization: Option<HeaderValue>,
    client: reqwest::Client,
    ui_dir: String,
}

/// Hop-by-hop headers, which belong to one connection and must not be forwarded across another
/// (RFC 9110 §7.6.1). Leaving `connection` or `transfer-encoding` on a proxied response is how a
/// client is told to expect framing the proxy has already undone.
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP
        .iter()
        .any(|h| name.as_str().eq_ignore_ascii_case(h))
}

/// Does `path` sit under `prefix` as a *segment*, rather than merely starting with its letters?
///
/// `/comms` must match `/comms` and `/comms/api/feed` and must not match `/commsomething`. A bare
/// `starts_with` says yes to the third, which would route one capability's traffic to another and
/// report it as a 404 from the wrong server.
fn matches(path: &str, prefix: &str) -> bool {
    path == prefix || (path.starts_with(prefix) && path[prefix.len()..].starts_with('/'))
}

impl Proxy {
    pub(crate) fn new(services: &[crate::status::Service], own_port: &str, ui_dir: String) -> Self {
        let mut routes: Vec<Route> = Vec::new();
        for svc in services {
            // Same two exclusions as the Vite proxy: the spine is the shell itself, and a
            // capability with no port has no HTTP surface to reach.
            if svc.scope == "spine" || svc.port.is_empty() {
                continue;
            }
            // The exclusion Vite did not need. Compared by port so a rename cannot smuggle a
            // self-route back in.
            // Compared on BOTH, because either alone has a hole: a port override would let the
            // name back in, and a rename would let the port back in.
            if svc.port == own_port || svc.name == "axon-status" {
                continue;
            }
            let target = format!("http://127.0.0.1:{}", svc.port);
            let mount = if svc.proxy_api_only == "true" {
                format!("/{}/api", svc.name)
            } else {
                format!("/{}", svc.name)
            };
            let comms = svc.name == "comms";
            routes.push(Route {
                prefix: mount,
                target: target.clone(),
                strip: Some(format!("/{}", svc.name)),
                inject_comms_auth: comms,
            });
            for extra in &svc.proxy_extra {
                routes.push(Route {
                    prefix: extra.clone(),
                    target: target.clone(),
                    strip: None,
                    inject_comms_auth: comms,
                });
            }
        }
        // Longest prefix first, so `/trips/api` is considered before a hypothetical `/trips`.
        // `Reverse` rather than a hand-written comparator: clippy's unnecessary_sort_by is
        // right that the two are the same order, and the key form cannot get the operands
        // backwards — which is the one way this line could break, silently, at the exact
        // moment two prefixes overlap.
        routes.sort_by_key(|route| std::cmp::Reverse(route.prefix.len()));

        Self {
            routes: Arc::new(routes),
            comms_authorization: comms_authorization(),
            device_authorization: sjel_server::InboundAuth::from_deployment()
                .bearer_header()
                .and_then(|value| HeaderValue::from_str(&value).ok()),
            client: reqwest::Client::new(),
            ui_dir,
        }
    }

    fn route_for(&self, path: &str) -> Option<&Route> {
        self.routes.iter().find(|r| matches(path, &r.prefix))
    }

    pub(crate) fn route_count(&self) -> usize {
        self.routes.len()
    }
}

/// The comms credential, read from the same config the comms server reads.
fn comms_authorization() -> Option<HeaderValue> {
    // The same three candidates comms itself resolves, in the same order
    // (`capabilities/comms/src/config.rs`). Re-derived rather than imported because axon-status
    // does not depend on comms and must not start doing so to read one path.
    let path = if let Ok(p) = sjel_config::env_var("SJEL_COMMS_CONFIG") {
        sjel_config::expand_tilde(&p)
    } else if let Some(p) = sjel_config::overlay_config("comms.json") {
        p
    } else {
        std::path::PathBuf::from("capabilities/comms/comms.config.json")
    };
    // Two hops, because `comms.json` NAMES the token file rather than holding the token:
    // `api_secret_file` -> that path -> the value (`capabilities/comms/src/config.rs`
    // `api_key_from_file`, which is the same two hops through the same reader).
    //
    // Handing `comms.json` straight to `token_from_file` was the first version and it failed in
    // silence. That reader accepts either a raw token file or a JSON file carrying
    // `auth.api_key`; `comms.json` parses as JSON and has no such key, so it returned `None`,
    // the fail-closed path below sent no header, and comms answered 401 to every proxied read
    // WHILE RUNNING AND HEALTHY. The dashboard showed "Feed" unavailable, which is
    // indistinguishable from the capability being down and is why it survived the B19 cutover.
    let config = std::fs::read_to_string(&path).ok()?;
    let secret_file = serde_json::from_str::<serde_json::Value>(&config)
        .ok()?
        .get("api_secret_file")?
        .as_str()?
        .to_string();
    let token = sjel_server::token_from_file(&sjel_config::expand_tilde(&secret_file))?;
    HeaderValue::from_str(&format!("Bearer {token}")).ok()
}

/// This surface's own name, as the dashboard addresses it. One constant, because the routing
/// table must never carry it (see the module docs' self-proxy rule) and this layer must always.
pub(crate) const SELF_PREFIX: &str = "/axon-status";

/// Strip a leading `/axon-status` segment before routing, mirroring the rewrite Vite's dev
/// proxy performs (`dashboard/vite.config.ts`, `rewrite: path.replace(^/<name>, "")`).
///
/// Must run before *any* routing, so it is installed as an empty outer router's
/// `fallback_service` (see `main.rs`) rather than with `Router::layer`. `Router::layer` maps
/// each route's service and the fallback, which leaves matchit deciding first -- measured, and
/// it fails in the most confusing available way: the path matches nothing, reaches the proxy
/// fallback, is rewritten there, and `/api/axon-status/capabilities` then matches transit's
/// `proxy_extra = ["/api"]` and is answered by port 3000.
///
/// Ordering it before routing does not reintroduce what the module docs reject. They reject
/// resolving the PROXY table before routing; this resolves no table. It removes one prefix no
/// route and no page can claim: axon-status registers only `/health`, `/routes` and
/// `/api/axon-status/*`, and `dashboard/src/routes/` has no `axon-status` entry. After the
/// rewrite the ordinary router -- real routes first, proxy fallback second -- decides as it
/// always did.
///
/// Strips once, never in a loop. A crafted `/axon-status/axon-status/x` becomes
/// `/axon-status/x`, which then finds no route and is served as a page. That is a dead end,
/// not recursion, because the layer sees each request exactly once.
pub(crate) async fn strip_self_prefix(mut req: Request, next: Next) -> Response {
    if matches(req.uri().path(), SELF_PREFIX) {
        let rest = &req.uri().path()[SELF_PREFIX.len()..];
        let rest = if rest.is_empty() { "/" } else { rest };
        let query = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        // Origin-form request URIs carry no scheme or authority, so the path and query are the
        // whole of it and a parse round-trip is exact. A malformed result is left alone rather
        // than guessed at: the unrewritten path still reaches a handler.
        if let Ok(uri) = format!("{rest}{query}").parse::<Uri>() {
            *req.uri_mut() = uri;
        }
    }
    next.run(req).await
}

/// Router fallback: proxy when a prefix claims the path, otherwise serve the SPA.
pub(crate) async fn fallback(State(proxy): State<Proxy>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    match proxy.route_for(&path) {
        Some(route) => forward(&proxy, route.clone(), req).await,
        // Not a capability path, so it is a page. `adapter-static` emits one entry point and
        // routes client-side, which makes an unknown path a route rather than a 404.
        //
        // The trap, since it cost a misread: an API path for a capability with NO route falls here
        // and answers 200 with `text/html`. `GET /foundation-models/health` looked like a passing
        // health check and was the shell's index page. The routing table is resolved once at
        // startup, so a capability enabled afterwards has no route until this process restarts —
        // which is the documented behaviour above, not a bug, but it fails as a plausible success.
        // When a capability path answers HTML, restart axon-status before believing anything else.
        None => serve_ui(&proxy.ui_dir, req).await,
    }
}

/// SvelteKit's build-output root. Every URL under it is generated and content-hashed by the
/// build; none of it is a page a person can navigate to.
const BUILD_OUTPUT_PREFIX: &str = "/_app/";

/// Whether a miss on this path must be a 404 rather than the SPA shell.
///
/// The SPA fallback is right for a page: `adapter-static` emits one entry point and routes
/// client-side, so an unknown *route* is a route. It is wrong for a build artifact, and the cost
/// of getting that wrong is not a confusing 200 -- it is a silent, undebuggable failure.
///
/// Measured 2026-09-06. `dashboard/vite.config.ts` never emitted MapLibre's worker, because
/// MapLibre asks for it through a template literal Rollup cannot follow. The browser requested
/// `/_app/immutable/chunks/maplibre-gl-worker.mjs`, this function's caller answered **200
/// `text/html`** with the app shell, and `new Worker` was handed HTML and died. MapLibre reports
/// nothing in that case, so every map in the dashboard rendered a blank canvas and sat on
/// "Loading map…" forever -- with no error, and no failed request in the network log to find.
/// It shipped that way for a month. A 404 would have named it in seconds.
///
/// Narrow on purpose. The rule is not "anything with a file extension", which would break a
/// client-side route that happens to contain a dot; it is one directory the build owns outright.
fn is_build_output(path: &str) -> bool {
    path.starts_with(BUILD_OUTPUT_PREFIX)
}

async fn serve_ui(dir: &str, req: Request) -> Response {
    let served = if is_build_output(req.uri().path()) {
        // No fallback: a missing build artifact is a 404, and the caller finds out.
        ServeDir::new(dir).oneshot(req).await
    } else {
        ServeDir::new(dir)
            .fallback(ServeFile::new(format!("{dir}/index.html")))
            .oneshot(req)
            .await
    };
    match served {
        Ok(res) => res.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("static: {e}")).into_response(),
    }
}

async fn forward(proxy: &Proxy, route: Route, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    let rest = match &route.strip {
        Some(p) if path.starts_with(p.as_str()) => {
            let stripped = &path[p.len()..];
            if stripped.is_empty() {
                "/"
            } else {
                stripped
            }
        }
        _ => path,
    };
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("{}{}{}", route.target, rest, query);

    // Buffered rather than streamed. Every request through here is a dashboard API call, the
    // largest of which is a CSV import measured in kilobytes, and buffering keeps the failure
    // mode legible. The RESPONSE is streamed below, because that side has soundscape's
    // `/api/soundscape/stream` on it and a buffered SSE response is a hung page.
    let bytes = match axum::body::to_bytes(body, 8 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };

    let mut headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name) || name == axum::http::header::HOST {
            continue;
        }
        headers.insert(name.clone(), value.clone());
    }
    if route.inject_comms_auth {
        if let Some(auth) = &proxy.comms_authorization {
            headers.insert(axum::http::header::AUTHORIZATION, auth.clone());
        }
    } else if parts
        .extensions
        .get::<sjel_server::AdmittedDevice>()
        .is_some()
        || parts
            .extensions
            .get::<sjel_server::AdmittedPairingClaim>()
            .is_some()
    {
        if let Some(auth) = &proxy.device_authorization {
            headers.insert(axum::http::header::AUTHORIZATION, auth.clone());
        }
    }

    let upstream = proxy
        .client
        .request(parts.method.clone(), &url)
        .headers(headers)
        .body(bytes)
        .send()
        .await;

    let upstream = match upstream {
        Ok(r) => r,
        // A capability that is not running is the normal case on a machine where most things are
        // on demand, so this says which one and stays a 502 rather than a panic.
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("{} is not answering: {e}", route.target),
            )
                .into_response()
        }
    };

    let status = upstream.status();
    let mut out = Response::builder().status(status);
    for (name, value) in upstream.headers().iter() {
        if is_hop_by_hop(name) {
            continue;
        }
        out = out.header(name, value);
    }
    let stream = upstream.bytes_stream();
    out.body(Body::from_stream(stream))
        .unwrap_or_else(|e| (StatusCode::BAD_GATEWAY, format!("upstream: {e}")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this layer exists for. The client addresses this surface the way Vite
    /// taught it to, and the built bundle has to answer the same way the dev proxy did.
    fn strip(path: &str) -> String {
        // The layer's rewrite, isolated from axum's plumbing so the RULE is what is tested.
        if !matches(path_of(path), SELF_PREFIX) {
            return path.to_string();
        }
        let rest = &path_of(path)[SELF_PREFIX.len()..];
        let rest = if rest.is_empty() { "/" } else { rest };
        match path.split_once('?') {
            Some((_, q)) => format!("{rest}?{q}"),
            None => rest.to_string(),
        }
    }

    fn path_of(uri: &str) -> &str {
        uri.split_once('?').map(|(p, _)| p).unwrap_or(uri)
    }

    /// The rule that turns a missing build artifact into a 404 instead of the app shell.
    /// A silent `new Worker(html)` cost a month of dead maps; see `is_build_output`.
    #[test]
    fn a_missing_build_artifact_is_not_answered_with_the_page_shell() {
        assert!(is_build_output(
            "/_app/immutable/chunks/maplibre-gl-worker.mjs"
        ));
        assert!(is_build_output(
            "/_app/immutable/workers/maplibre-gl-worker-B6z8KlkX.js"
        ));
        assert!(is_build_output("/_app/immutable/entry/app.hash.js"));
        assert!(is_build_output("/_app/version.json"));
    }

    /// A page still gets the shell, which is the whole point of an SPA fallback. These are the
    /// dashboard's real routes, including one carrying a dot -- the reason the rule names one
    /// directory rather than testing for a file extension.
    #[test]
    fn a_page_still_reaches_the_client_router() {
        for path in [
            "/",
            "/map",
            "/travel",
            "/feed/abc",
            "/projects/some.name",
            "/basemap/style.json",
        ] {
            assert!(
                !is_build_output(path),
                "{path} must still reach the SPA shell"
            );
        }
    }

    #[test]
    fn self_prefix_is_stripped_so_own_routes_match() {
        // Every axonStatus.* call in dashboard/src/lib/api.ts has this shape.
        assert_eq!(
            strip("/axon-status/api/axon-status/capabilities"),
            "/api/axon-status/capabilities"
        );
        assert_eq!(
            strip("/axon-status/api/axon-status/health"),
            "/api/axon-status/health"
        );
    }

    #[test]
    fn start_and_stop_reach_their_handlers() {
        // These two are why the page could not start an on-demand capability: same dead prefix.
        assert_eq!(
            strip("/axon-status/api/axon-status/capabilities/finance/start"),
            "/api/axon-status/capabilities/finance/start"
        );
        assert_eq!(
            strip("/axon-status/api/axon-status/capabilities/finance/stop"),
            "/api/axon-status/capabilities/finance/stop"
        );
    }

    #[test]
    fn a_query_string_survives_the_rewrite() {
        assert_eq!(
            strip("/axon-status/api/axon-status/repos?dirty=1"),
            "/api/axon-status/repos?dirty=1"
        );
    }

    #[test]
    fn the_bare_prefix_becomes_the_index_rather_than_an_empty_path() {
        assert_eq!(strip("/axon-status"), "/");
    }

    #[test]
    fn another_capability_is_left_alone() {
        // Segment matching, same rule as the proxy table: only OUR name, and only whole.
        assert_eq!(strip("/comms/api/feed"), "/comms/api/feed");
        assert_eq!(strip("/axon-statusish/api"), "/axon-statusish/api");
        assert_eq!(
            strip("/api/axon-status/capabilities"),
            "/api/axon-status/capabilities"
        );
    }

    #[test]
    fn a_doubled_prefix_strips_once_and_stops() {
        // One pass per request. The result finds no route and is served as a page, which is a
        // dead end rather than recursion.
        assert_eq!(strip("/axon-status/axon-status/x"), "/axon-status/x");
    }

    fn svc(name: &str, port: &str, api_only: bool, extra: &[&str]) -> crate::status::Service {
        serde_json::from_value(serde_json::json!({
            "name": name, "kind": "process", "scope": "capability", "port": port,
            "health_path": "/health", "panel_port": "", "panel_path": "", "autostart": "",
            "proxy_api_only": if api_only { "true" } else { "" },
            "proxy_extra": extra,
        }))
        .expect("registry fixture")
    }

    fn table(services: &[crate::status::Service]) -> Vec<Route> {
        Proxy::new(services, "8082", "dist".into())
            .routes
            .as_ref()
            .clone()
    }

    /// The rule that broke in the first draft. `proxy_api_only` moves the MOUNT to
    /// `/<name>/api` while still stripping only `/<name>` — so finance is reached at
    /// `/finance/api/subscriptions` and receives `/api/subscriptions`. Stripping the mount
    /// instead sent it `/subscriptions` and produced a 404 that looked like a finance bug.
    #[test]
    fn api_only_mounts_deeper_than_it_strips() {
        let t = table(&[svc("finance", "8090", true, &[])]);
        let r = &t[0];
        assert_eq!(r.prefix, "/finance/api");
        assert_eq!(r.strip.as_deref(), Some("/finance"));
    }

    /// A `proxy_extra` prefix is a path that predates the uniform rule, so it passes through
    /// with nothing removed.
    #[test]
    fn extras_pass_through_unstripped() {
        let t = table(&[svc("scouting", "8084", false, &["/discover"])]);
        let extra = t
            .iter()
            .find(|r| r.prefix == "/discover")
            .expect("extra route");
        assert!(extra.strip.is_none());
    }

    /// A prefix must match on a segment boundary. `starts_with` alone routes `/punctualityXX`
    /// into punctuality, which answers 404 for a path the shell meant as a page.
    #[test]
    fn prefixes_match_on_segment_boundaries() {
        assert!(matches("/comms", "/comms"));
        assert!(matches("/comms/api/feed", "/comms"));
        assert!(!matches("/commsomething", "/comms"));
        assert!(!matches("/com", "/comms"));
    }

    /// Forwarding to our own port re-enters this fallback and loops. Excluded on name and port
    /// both, because either alone has a hole: a port override lets the name back in, a rename
    /// lets the port back in.
    #[test]
    fn never_proxies_to_itself() {
        let t = table(&[
            svc("axon-status", "8082", false, &[]),
            svc("comms", "8083", false, &[]),
        ]);
        assert!(t.iter().all(|r| r.prefix != "/axon-status"));
        assert!(t.iter().any(|r| r.prefix == "/comms"));
    }

    /// The spine is this process, and a capability with no port has no HTTP surface. Same two
    /// exclusions the Vite proxy made.
    #[test]
    fn skips_the_spine_and_the_portless() {
        let mut spine = svc("dashboard", "47117", false, &[]);
        spine.scope = "spine".into();
        let t = table(&[spine, svc("host-watch", "", false, &[])]);
        assert!(t.is_empty());
    }

    /// Only comms gets a credential. An earlier draft decided this by comparing the target
    /// against a helper returning "", and `ends_with("")` is true for everything — which would
    /// have put the comms bearer token on every request to every capability.
    #[test]
    fn only_comms_carries_the_credential() {
        let t = table(&[
            svc("comms", "8083", false, &[]),
            svc("places", "8093", false, &[]),
        ]);
        for r in &t {
            assert_eq!(
                r.inject_comms_auth,
                r.prefix.starts_with("/comms"),
                "{}",
                r.prefix
            );
        }
    }

    /// Longest first, so a mount is considered before any shorter prefix that also matches.
    #[test]
    fn longest_prefix_wins() {
        let t = table(&[svc("trips", "8086", true, &["/t"])]);
        assert!(t[0].prefix.len() >= t[t.len() - 1].prefix.len());
    }
}
