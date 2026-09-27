//! The inbound gate: one shared-secret check for every capability server.
//!
//! ## Why this is here and not in each capability
//!
//! Until this module existed, exactly one of twelve Rust capabilities
//! authenticated an inbound request — comms, whose `src/server/auth.rs` carried
//! a constant-time Bearer / `X-Axon-Token` check on its mutating routes. The
//! other eleven relied entirely on the loopback bind, including sjel-status,
//! which serves `POST /api/sjel-status/capabilities/:name/start|stop`: process
//! control. "Reachable from the phone" and "unauthenticated process control"
//! cannot both be true, so the check moved to the crate all twelve already
//! route their startup through. A second copy of this check is the drift the
//! repo's third principle forbids; comms now calls this one.
//!
//! ## The contract
//!
//! | Configured token | `/health`, `/ready`, `OPTIONS` | Every other route | Reach beyond loopback |
//! |---|---|---|---|
//! | yes | served | `401` without a matching token | permitted |
//! | no | served | served (or `403`, see below) | **refused at bind** |
//!
//! ## The second gate, and why one struct decides
//!
//! A shared secret cannot reach a browser without being in the browser, so the
//! phone gets in on an identity instead: [`crate::tailnet`] reads the login
//! `tailscale serve` proves, and a declared operator satisfies the token
//! requirement. It is resolved into this struct rather than layered separately
//! because "is this request allowed to" must stay one comparison in one place —
//! two middlewares deciding admission is the drift this module's own first
//! paragraph exists to refuse.
//!
//! | Declared operator | `Tailscale-User-Login` | Outcome |
//! |---|---|---|
//! | no | anything | header ignored, token rule alone decides |
//! | yes | absent | token rule alone decides — a direct loopback caller |
//! | yes | the operator | served, without a token |
//! | yes | anyone else | `401` |
//!
//! The one exception is [`InboundAuth::refuse_without_token`], which an identity
//! never satisfies.
//!
//! `Reach::AllInterfaces` without a token has no representation:
//! [`crate::bind_addr_for`] is the only constructor of a non-loopback
//! `SocketAddr` in this crate and it returns `Err` for that pairing, while
//! doctor's "Server bind policy" gate fails any capability source that builds
//! its own listener (README.md, "What actually enforces this").
//!
//! ## Token sourcing: shared, not per-capability
//!
//! One token for the whole deployment, because the thing it gates is one thing:
//! whether an inbound request reached this machine legitimately. Twelve tokens
//! would be twelve secrets for one boundary and twelve injections in every
//! client that fans out across capabilities — the dashboard's Vite proxy and
//! sjel-status' `/routes` aggregation both do exactly that.
//!
//! The value is referenced, never inlined, following the pattern comms
//! established for `api_secret_file`: `<overlay>/config/deployment.env`
//! declares `SJEL_INBOUND_TOKEN_FILE=<path>` and the token is the contents of
//! that private file (`schemas/deployment.env.example`). A path is not a
//! secret, which is why the reference may live in a tracked-shape file while
//! the value may not.
//!
//! A capability may still supply its own token to [`InboundAuth::resolve`] and
//! it wins — comms' pre-existing `api_secret_file` is the one caller that does.
//! A deployment converges the two by pointing both references at one file.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

/// Paths that answer before the gate. Liveness and readiness are what a proxy,
/// the runner and sjel-status poll to find out whether a process is alive at
/// all; behind a token they would report a healthy capability as down, and the
/// answer carries nothing an unauthenticated caller could not learn by
/// observing that the port accepts a connection.
///
/// `/__axon/freshness` joins them on the same argument and no weaker one. It answers *when* this
/// capability last took delivery of data, as one integer — never what the data is, how much of
/// it there is, or where it came from. A caller who can reach the port can already watch it
/// accept connections; learning that a collector last succeeded at T tells them nothing further
/// about the operator. It is exempt because the surface that reads it, sjel-status, polls every
/// capability and must not need each one's credential to ask a liveness-shaped question — the
/// alternative is a status page that reports a healthy capability as unknown, which is exactly
/// the failure the two paths above are exempt to prevent.
const EXEMPT_PATHS: &[&str] = &["/health", "/ready", "/__axon/freshness"];

/// The header whose presence makes a request a device-signed one (`axon-device-auth/v1`,
/// `capabilities/devices/src/auth.rs`). The other three headers are read by the verifier.
pub const DEVICE_SIGNATURE_HEADER: &str = "x-axon-signature";

/// The largest body the gate buffers to check a device signature. Same ceiling as the shell's
/// proxy (`capabilities/sjel-status/src/proxy.rs`, `forward`), which buffers it anyway.
const MAX_SIGNED_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Checks one `axon-device-auth/v1` signed request against the device registry.
///
/// A trait rather than a dependency because the registry is a capability
/// (`capabilities/devices`) and this crate is under every capability. The shell supplies the
/// implementation; a server without one never admits on a device key.
pub trait DeviceVerifier: Send + Sync {
    /// `Ok(device_id)` when the signature, the device and the nonce are all valid, else the
    /// reason. `signed_path` is [`device_signed_path`] of the request target.
    fn verify(
        &self,
        method: &str,
        signed_path: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<String, String>;
}

/// Set on a request the gate admitted on a device key, so a proxy behind it can authenticate
/// the request to its upstream (which sees no tailnet identity on a LAN route).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedDevice(pub String);

/// Set on the one unsigned request the LAN listener admits: a pairing claim, which a device sends
/// before it has a registered key. The one-time code in its body is what protects it
/// (`capabilities/devices`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedPairingClaim;

/// The pairing claim as the shell mounts it (`capabilities/devices/README.md`, Contract).
pub const PAIRING_CLAIM_PATH: &str = "/devices/api/pairing/claims";

/// The target a device signs: the path the capability sees after the shell removes its mount.
/// `/api/...` is not mounted and stays as it is. Mirrors `signed_path` in
/// `dashboard/src-tauri/src/mac_bridge.rs`, which is the signing side.
pub fn device_signed_path(path_and_query: &str) -> &str {
    if path_and_query == "/api" || path_and_query.starts_with("/api/") {
        return path_and_query;
    }
    match path_and_query.get(1..).and_then(|rest| rest.find('/')) {
        Some(index) => &path_and_query[index + 1..],
        None => "/",
    }
}

/// The resolved inbound gate for one server.
///
/// Cloned per request by axum's `State`, so the token is a `String` rather than
/// a borrow. Never `Debug`-printed with its value — see the manual impl below.
#[derive(Clone, Default)]
pub struct InboundAuth {
    token: Option<String>,
    /// `true` when the absence of a token must close the non-exempt routes
    /// rather than leave them open. See [`InboundAuth::refuse_without_token`].
    refuse_without_token: bool,
    /// The login `tailscale serve` must vouch for, when the deployment declared
    /// one. See [`crate::tailnet`] for why a second gate exists and what it may
    /// not do.
    tailnet_operator: Option<String>,
    /// The device registry, when this server admits paired devices by their key (PRD Q119).
    device_verifier: Option<Arc<dyn DeviceVerifier>>,
    /// Set on the LAN listener (`crate::lan`): a device signature is required, and a tailnet
    /// identity header is removed rather than believed, because anyone on the Wi-Fi can write it.
    lan_devices_only: bool,
}

/// Redacts the token. A capability that logs its own config must not turn this
/// value into a line in the runner's captured stderr.
impl std::fmt::Debug for InboundAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundAuth")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("refuse_without_token", &self.refuse_without_token)
            // Printed in full, unlike the token: a login is a public value, and
            // "which operator is this deployment admitting" is the first thing
            // worth seeing when the phone is answered 401.
            .field("tailnet_operator", &self.tailnet_operator)
            .field("device_verifier", &self.device_verifier.is_some())
            .field("lan_devices_only", &self.lan_devices_only)
            .finish()
    }
}

impl InboundAuth {
    /// The deployment-wide token, or none.
    ///
    /// Reads `SJEL_INBOUND_TOKEN_FILE` from `<overlay>/config/deployment.env`
    /// and then that file. Every step is allowed to be absent: an overlay that
    /// has not declared a token yields `None`, which is the loopback-only
    /// deployment that predates this gate.
    pub fn from_deployment() -> Self {
        Self::resolve(None)
    }

    /// A capability-supplied token wins over the deployment-wide one.
    ///
    /// Precedence, not merging, and deliberately not the conflict error
    /// `sjel_config::resolve_home_timezone` raises: two timezones are a
    /// mistake, whereas two tokens are a deployment mid-rotation or a
    /// capability whose clients predate the shared file. Both are legitimate.
    pub fn resolve(capability_token: Option<String>) -> Self {
        let token = capability_token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(deployment_token);
        Self {
            token,
            refuse_without_token: false,
            tailnet_operator: crate::tailnet::deployment_operator(),
            device_verifier: None,
            lan_devices_only: false,
        }
    }

    /// No I/O: the token exactly as given. For tests and for a capability that
    /// has already resolved its own value.
    pub fn with_token(token: Option<String>) -> Self {
        Self {
            token: token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()),
            refuse_without_token: false,
            tailnet_operator: None,
            device_verifier: None,
            lan_devices_only: false,
        }
    }

    /// Admit a request that `tailscale serve` proved came from `operator`.
    ///
    /// No I/O, for tests and for a caller that resolved the login itself.
    /// [`InboundAuth::resolve`] reads the deployment's declaration instead.
    pub fn with_tailnet_operator(mut self, operator: Option<String>) -> Self {
        self.tailnet_operator = operator
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty());
        self
    }

    /// Admit a request signed by a paired device (PRD Q119: the device key is the trust root,
    /// and the network it arrived on is not).
    ///
    /// A request that carries [`DEVICE_SIGNATURE_HEADER`] is decided by its signature alone:
    /// valid admits it, invalid answers `401` and does not fall through to the tailnet or token
    /// rules, because a forged signature is not a request that merely lacks credentials. Like a
    /// tailnet identity, a device key does not satisfy [`Self::refuse_without_token`].
    pub fn with_device_verifier(mut self, verifier: Arc<dyn DeviceVerifier>) -> Self {
        self.device_verifier = Some(verifier);
        self
    }

    /// Whether this gate admits paired devices by key.
    pub fn admits_devices(&self) -> bool {
        self.device_verifier.is_some()
    }

    /// The same gate for the LAN listener: paired devices only, plus the pairing claim.
    pub fn lan_devices_only(mut self) -> Self {
        self.lan_devices_only = true;
        self
    }

    /// Answer `403` on the non-exempt routes when no token is configured,
    /// instead of serving them.
    ///
    /// For a capability whose routes must not run for an unauthenticated caller
    /// even on loopback. comms is the reason this exists: `POST /ingest` fetches
    /// an attacker-chosen URL, and a page open in the operator's own browser is
    /// already inside the loopback boundary, so `127.0.0.1` is not what contains
    /// that route — the token is.
    pub fn refuse_without_token(mut self) -> Self {
        self.refuse_without_token = true;
        self
    }

    /// Whether a token was resolved. `false` is what confines a server to
    /// loopback ([`crate::bind_addr_for`]).
    pub fn is_configured(&self) -> bool {
        self.token.is_some()
    }

    /// `Bearer <token>`, for a process that calls a sibling capability through
    /// this same gate — sjel-status polling `/routes` is the only one today.
    /// `None` when no token is configured, which is also when no sibling
    /// requires one.
    pub fn bearer_header(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    /// Whether this gate rejects anything at all. A gate with no token and no
    /// refusal is not layered onto the router, so an unconfigured deployment
    /// pays nothing per request.
    fn gates_anything(&self) -> bool {
        self.token.is_some()
            || self.lan_devices_only
            || self.refuse_without_token
            || self.tailnet_operator.is_some()
            || self.device_verifier.is_some()
    }

    /// `Some(rejection)` when this request must not reach a handler.
    ///
    /// Split out of the middleware so the policy is testable without a socket.
    fn reject(&self, method: &Method, path: &str, headers: &HeaderMap) -> Option<Response> {
        // CORS preflight carries no Authorization header by construction — the
        // browser strips it — so gating OPTIONS would reject every cross-origin
        // request from the dashboard before the CORS layer inside this one ever
        // answered. Nothing is disclosed: the real request still needs the
        // token, and axum answers an unrouted method with 405, not a handler.
        if method == Method::OPTIONS || EXEMPT_PATHS.contains(&path) {
            return None;
        }

        // The tailnet gate runs first, and only when the deployment declared an
        // operator. Without that declaration the identity header is ignored
        // entirely rather than believed — otherwise declaring nothing would
        // silently start trusting a header any caller can write.
        if let Some(operator) = self.tailnet_operator.as_deref() {
            match crate::tailnet::arrival(headers) {
                // The proxy authenticated somebody who is not the operator.
                // Refuse here rather than falling through to the token: a named
                // stranger holding a valid shared secret is a token to rotate,
                // and answering 401 is how that becomes visible.
                crate::tailnet::Arrival::Tailnet(login)
                    if !crate::tailnet::is_operator(&login, operator) =>
                {
                    return Some(
                        (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({ "error": "this tailnet identity is not the declared operator" })),
                        )
                            .into_response(),
                    );
                }
                // Proven operator. This satisfies the token requirement, which
                // is the whole reason the gate exists: the browser that loads
                // the built SPA cannot carry the deployment's shared secret, and
                // shipping it one would put that secret in a bundle.
                //
                // It does NOT satisfy `refuse_without_token`. A route that opts
                // into that wants the secret specifically — comms' `POST
                // /ingest` fetches an attacker-chosen URL, and its own comment
                // records that being inside the loopback boundary is not what
                // contains it. A name is not what contains it either.
                crate::tailnet::Arrival::Tailnet(_) if !self.refuse_without_token => return None,
                // Either the request came straight to loopback, or the proxy is
                // not injecting identity. Both fall through to the token rule
                // below, which is the behaviour that predates this gate. Doctor's
                // "Tailnet identity gate" check is what distinguishes the two,
                // because a request cannot.
                _ => {}
            }
        }

        let Some(expected) = self.token.as_deref() else {
            if self.refuse_without_token {
                return Some(
                    (
                        StatusCode::FORBIDDEN,
                        Json(json!({
                            "error": "no inbound token is configured — these routes are disabled. Declare SJEL_INBOUND_TOKEN_FILE in <overlay>/config/deployment.env."
                        })),
                    )
                        .into_response(),
                );
            }
            return None;
        };
        match presented_token(headers) {
            Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => None,
            _ => Some(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "invalid or missing authentication token" })),
                )
                    .into_response(),
            ),
        }
    }
}

/// Wraps `router` in the inbound gate.
///
/// Returns the router untouched when the gate would reject nothing, so a
/// deployment with no token keeps exactly the request path it had before this
/// module existed.
///
/// Applied outermost, above whatever CORS layer the capability built: a request
/// that fails the token check must not consume a handler, and the exemption for
/// `OPTIONS` in [`InboundAuth::reject`] is what keeps preflight working from
/// underneath.
pub fn authenticated(router: Router, auth: InboundAuth) -> Router {
    if !auth.gates_anything() {
        return router;
    }
    router.layer(axum::middleware::from_fn_with_state(auth, gate))
}

async fn gate(State(auth): State<InboundAuth>, mut request: Request, next: Next) -> Response {
    if auth.lan_devices_only {
        let forged: Vec<_> = request
            .headers()
            .keys()
            .filter(|name| name.as_str().starts_with("tailscale-"))
            .cloned()
            .collect();
        for name in forged {
            request.headers_mut().remove(name);
        }
    }
    let exempt =
        request.method() == Method::OPTIONS || EXEMPT_PATHS.contains(&request.uri().path());
    if let Some(verifier) = auth.device_verifier.as_ref() {
        if !exempt && request.headers().contains_key(DEVICE_SIGNATURE_HEADER) {
            return admit_device(&auth, verifier.as_ref(), request, next).await;
        }
    }
    if auth.lan_devices_only && !exempt {
        if request.method() == Method::POST && request.uri().path() == PAIRING_CLAIM_PATH {
            request.extensions_mut().insert(AdmittedPairingClaim);
            return next.run(request).await;
        }
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "this listener admits paired devices only; pair this device first" })),
        )
            .into_response();
    }
    match auth.reject(request.method(), request.uri().path(), request.headers()) {
        Some(rejection) => rejection,
        None => next.run(request).await,
    }
}

async fn admit_device(
    auth: &InboundAuth,
    verifier: &dyn DeviceVerifier,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_SIGNED_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({ "error": "signed request body is too large" })),
            )
                .into_response()
        }
    };
    let target = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or_else(|| parts.uri.path());
    let device = match verifier.verify(
        parts.method.as_str(),
        device_signed_path(target),
        &parts.headers,
        &bytes,
    ) {
        Ok(device) => device,
        Err(reason) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": format!("device signature rejected: {reason}") })),
            )
                .into_response()
        }
    };
    if auth.refuse_without_token {
        if let Some(rejection) = auth.reject(&parts.method, parts.uri.path(), &parts.headers) {
            return rejection;
        }
    }
    parts.extensions.insert(AdmittedDevice(device));
    next.run(Request::from_parts(parts, axum::body::Body::from(bytes)))
        .await
}

/// `Authorization: Bearer <token>` first, then `X-Axon-Token: <token>`.
///
/// Two header forms because two kinds of client call these ports: HTTP tooling
/// and proxies that already speak `Authorization`, and the browser extension /
/// `curl` callers for which a dedicated header is one fewer thing to get wrong.
fn presented_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-axon-token").and_then(|v| v.to_str().ok()))
}

/// Compares every byte regardless of where the first difference is.
///
/// A short-circuiting `==` leaks the length of the matching prefix through
/// response time, which turns guessing a token from an exhaustive search into a
/// per-character one. The length check ahead of it leaks only the length, which
/// an attacker who can send a token already knows how to measure another way.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// `SJEL_INBOUND_TOKEN_FILE` (or `SJEL_INBOUND_TOKEN_FILE`) from `<overlay>/config/deployment.env`, then that
/// file's contents.
fn deployment_token() -> Option<String> {
    let body = std::fs::read_to_string(sjel_config::overlay_config("deployment.env")?).ok()?;
    let reference = sjel_config::deployment_value(&body, "SJEL_INBOUND_TOKEN_FILE")?;
    token_from_file(&sjel_config::expand_tilde(&reference))
}

/// Reads a token out of a private file: the trimmed contents, or `auth.api_key`
/// when the file is JSON.
///
/// The JSON form is not decoration — comms' `api_secret_file` has always been
/// allowed to point at an existing settings file rather than a bare token, and
/// this is that reader, moved up so Rust has one implementation of it. The
/// third implementation, `tokenFromBody` in `dashboard/vite/comms-proxy-auth.ts`,
/// cannot share code across the language boundary and says so at its own site.
///
/// `None` for absent, unreadable or empty: a token that failed to load must
/// never be mistaken for a token that matched.
pub fn token_from_file(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return json
            .get("auth")
            .and_then(|auth| auth.get("api_key"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    fn status(auth: &InboundAuth, method: Method, path: &str, sent: &[(&str, &str)]) -> u16 {
        match auth.reject(&method, path, &headers(sent)) {
            Some(response) => response.status().as_u16(),
            None => 200,
        }
    }

    const OPERATOR: &str = "lars@example.com";

    fn tailnet(auth: InboundAuth) -> InboundAuth {
        auth.with_tailnet_operator(Some(OPERATOR.into()))
    }

    /// The reason this gate exists: the phone's browser holds no secret.
    #[test]
    fn the_declared_operator_is_admitted_without_a_token() {
        let auth = tailnet(InboundAuth::with_token(Some("s3cret".into())));
        let proven = &[("tailscale-user-login", OPERATOR)];
        assert_eq!(status(&auth, Method::GET, "/feed", proven), 200);
        assert_eq!(
            status(&auth, Method::GET, "/api/sjel-status/capabilities", proven),
            200
        );
    }

    #[test]
    fn another_tailnet_identity_is_refused_even_holding_the_token() {
        // A named stranger with a valid shared secret is a token to rotate. 401
        // is how that becomes visible instead of being served silently.
        let auth = tailnet(InboundAuth::with_token(Some("s3cret".into())));
        assert_eq!(
            status(
                &auth,
                Method::GET,
                "/feed",
                &[
                    ("tailscale-user-login", "someone.else@example.com"),
                    ("authorization", "Bearer s3cret"),
                ]
            ),
            401
        );
    }

    #[test]
    fn an_undeclared_operator_means_the_header_is_ignored_not_believed() {
        // Declaring nothing must not silently start trusting a header any
        // caller can write. The token rule alone decides here.
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        assert_eq!(
            status(
                &auth,
                Method::GET,
                "/feed",
                &[("tailscale-user-login", OPERATOR)]
            ),
            401
        );
    }

    #[test]
    fn a_direct_loopback_request_still_answers_to_the_token_rule() {
        // No identity header: either a local caller or a proxy that stopped
        // injecting one. A request cannot tell those apart, so the gate does not
        // guess — it falls through to the behaviour that predates it, and
        // doctor's "Tailnet identity gate" check covers the second case.
        let auth = tailnet(InboundAuth::with_token(Some("s3cret".into())));
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 401);
        assert_eq!(
            status(
                &auth,
                Method::GET,
                "/feed",
                &[("authorization", "Bearer s3cret")]
            ),
            200
        );
    }

    #[test]
    fn declaring_only_an_operator_gates_the_tailnet_and_leaves_loopback_alone() {
        // The deployment this change actually ships: no token anywhere, and the
        // tailnet surface stops being open to every node on it.
        let auth = tailnet(InboundAuth::with_token(None));
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 200);
        assert_eq!(
            status(
                &auth,
                Method::GET,
                "/feed",
                &[("tailscale-user-login", OPERATOR)]
            ),
            200
        );
        assert_eq!(
            status(
                &auth,
                Method::POST,
                "/api/sjel-status/capabilities/comms/stop",
                &[("tailscale-user-login", "guest@example.com")]
            ),
            401
        );
    }

    #[test]
    fn an_identity_never_satisfies_refuse_without_token() {
        // comms opts into that because `POST /ingest` fetches an attacker-chosen
        // URL. Being inside the loopback boundary does not contain that route,
        // and neither does being named.
        let auth = tailnet(InboundAuth::with_token(None)).refuse_without_token();
        assert_eq!(
            status(
                &auth,
                Method::POST,
                "/ingest",
                &[("tailscale-user-login", OPERATOR)]
            ),
            403
        );
    }

    #[test]
    fn health_stays_exempt_for_an_unknown_identity() {
        // sjel-status polls every capability's /health. Gating it would report a
        // healthy capability as down, which is what the exemption exists to stop.
        let auth = tailnet(InboundAuth::with_token(None));
        assert_eq!(
            status(
                &auth,
                Method::GET,
                "/health",
                &[("tailscale-user-login", "guest@example.com")]
            ),
            200
        );
    }

    #[test]
    fn a_configured_token_gates_every_route_except_health_and_ready() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        assert_eq!(status(&auth, Method::GET, "/health", &[]), 200);
        assert_eq!(status(&auth, Method::GET, "/ready", &[]), 200);
        assert_eq!(status(&auth, Method::GET, "/routes", &[]), 401);
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 401);
        assert_eq!(
            status(
                &auth,
                Method::POST,
                "/api/sjel-status/capabilities/comms/start",
                &[]
            ),
            401
        );
    }

    #[test]
    fn both_header_forms_are_accepted_and_a_wrong_value_in_either_is_not() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        for (name, good, bad) in [
            ("authorization", "Bearer s3cret", "Bearer wrong"),
            ("x-axon-token", "s3cret", "wrong"),
        ] {
            assert_eq!(status(&auth, Method::GET, "/feed", &[(name, good)]), 200);
            assert_eq!(status(&auth, Method::GET, "/feed", &[(name, bad)]), 401);
        }
    }

    /// The browser strips `Authorization` from a preflight, so gating OPTIONS
    /// would break every cross-origin call the dashboard makes.
    #[test]
    fn cors_preflight_passes_through_to_the_cors_layer_underneath() {
        let auth = InboundAuth::with_token(Some("s3cret".into()));
        assert_eq!(status(&auth, Method::OPTIONS, "/ingest", &[]), 200);
    }

    #[test]
    fn without_a_token_a_server_serves_as_it_did_before_this_gate() {
        let auth = InboundAuth::with_token(None);
        assert_eq!(status(&auth, Method::GET, "/feed", &[]), 200);
        assert!(!auth.is_configured());
    }

    /// comms' contract: an absent secret closes the route rather than opening it.
    #[test]
    fn refuse_without_token_closes_the_non_exempt_routes_instead_of_opening_them() {
        let auth = InboundAuth::with_token(None).refuse_without_token();
        assert_eq!(status(&auth, Method::POST, "/ingest", &[]), 403);
        assert_eq!(status(&auth, Method::GET, "/health", &[]), 200);
    }

    /// An empty string is what an unset `api_secret_file` resolves to, and it
    /// must not become a token that any empty header would match.
    #[test]
    fn an_empty_token_counts_as_no_token() {
        let auth = InboundAuth::with_token(Some("  ".into())).refuse_without_token();
        assert!(!auth.is_configured());
        assert_eq!(status(&auth, Method::POST, "/ingest", &[]), 403);
    }

    #[test]
    fn the_comparison_reads_every_byte_and_rejects_a_matching_prefix() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret", b"s3crev"));
        assert!(!constant_time_eq(b"s3cret", b"s3cre"));
        assert!(!constant_time_eq(b"s3cret", b"s3cretx"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn a_token_file_is_read_raw_or_as_the_api_key_of_a_json_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-server-token-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let raw = dir.join("raw");
        std::fs::write(&raw, "  s3cret\n").unwrap();
        assert_eq!(token_from_file(&raw).as_deref(), Some("s3cret"));

        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"auth":{"api_key":"s3cret"}}"#).unwrap();
        assert_eq!(token_from_file(&settings).as_deref(), Some("s3cret"));

        let empty = dir.join("empty");
        std::fs::write(&empty, "\n \n").unwrap();
        assert_eq!(token_from_file(&empty), None);

        assert_eq!(token_from_file(&dir.join("absent")), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The value is a live credential; a capability that dumps its config must
    /// not put it in the runner's stderr.
    #[test]
    fn debug_never_prints_the_token() {
        let rendered = format!("{:?}", InboundAuth::with_token(Some("s3cret".into())));
        assert!(!rendered.contains("s3cret"), "got: {rendered}");
    }
}
