//! The one way a capability server comes up. Extracted after the same ~10
//! lines (resolve port, build a SocketAddr, log, bind, serve) existed in five
//! server binaries with three divergences none of which was a decision:
//! two servers bound 0.0.0.0 while three argued 127.0.0.1 in a comment, one
//! exited cleanly on a bind failure while four panicked, and one had just
//! stopped honouring the runner's port contract.
//!
//! It now also owns the inbound gate ([`auth`]), because the bind and the
//! authentication are one decision: whether a request that arrives is allowed
//! to. Keeping them apart is what let eleven of twelve capabilities serve
//! process control and personal data with no check at all behind a loopback
//! bind nobody was going to keep forever.
//!
//! What is deliberately NOT here: CORS. Whether a server carries
//! `CorsLayer::permissive()` is a per-capability security decision that must
//! stay visible in that capability's own source — sjel-status, which can
//! start and stop the machine's capabilities, correctly carries none, and a
//! helper that silently added it would have widened that surface.

//! This is a normal workspace crate. Consumers declare a Cargo path dependency,
//! which is what enforces the boundary and resolves one `axum` API across all of
//! them.

use std::net::SocketAddr;

/// The agent identity: read-only, every response pseudonymized (ISA F9).
pub mod agent;
mod auth;
/// The TLS listener for paired devices on the local network (PRD Q119).
pub mod lan;
/// The browser-origin refusal two capabilities apply to C2 surfaces.
pub mod origin;
/// The identity `tailscale serve` proves, for the caller that cannot hold a secret.
pub mod tailnet;

pub use auth::{
    authenticated, comms_config_token, device_signed_path, token_from_file, AdmittedDevice,
    AdmittedPairingClaim, DeviceVerifier, InboundAuth, DEVICE_SIGNATURE_HEADER, PAIRING_CLAIM_PATH,
};

// Re-exported so a server binary that depends only on sjel-server still gets the
// port contract.
pub use sjel_config::resolve_port;

/// How far a capability server's listener reaches.
///
/// An enum rather than a `SocketAddr` argument so that the pairing this crate
/// refuses — reach beyond loopback with no token — is one comparison in one
/// place instead of an IP-address predicate each caller could get wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// `127.0.0.1`. Requests arrive from this machine only. Every capability
    /// today, and the only reach an unauthenticated server may have.
    Loopback,
    /// `0.0.0.0` — every interface the host has, including the tailnet one.
    /// Admissible only with a configured inbound token.
    AllInterfaces,
}

/// Loopback-only address for a capability server. 127.0.0.1 is the policy,
/// not a default: these are local services reached through the dashboard's
/// proxy on the same machine. A capability that genuinely needs reach beyond
/// this machine goes through [`bind_addr_for`], which requires a token.
pub fn bind_addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// The only constructor of a non-loopback listening address in this crate.
///
/// Returns `Err` for [`Reach::AllInterfaces`] with no configured token: that
/// pairing is exactly the one this gate exists to prevent, so it has no
/// representation a caller can obtain and then use. The other half of the
/// enforcement is doctor's "Server bind policy" section, which fails any
/// `capabilities/*/src/*.rs` that builds a `Router` and its own listener
/// (README.md, "What actually enforces this") — together they leave no path
/// from a capability to an unauthenticated LAN or tailnet port.
///
/// `Err` carries the operator-facing sentence, not a code: the only caller
/// prints it and exits.
pub fn bind_addr_for(reach: Reach, port: u16, auth: &InboundAuth) -> Result<SocketAddr, String> {
    match reach {
        Reach::Loopback => Ok(bind_addr(port)),
        Reach::AllInterfaces if auth.is_configured() => Ok(SocketAddr::from(([0, 0, 0, 0], port))),
        Reach::AllInterfaces => Err(format!(
            "refusing to bind 0.0.0.0:{port} with no inbound token. Declare \
             SJEL_INBOUND_TOKEN_FILE in <overlay>/config/deployment.env, or keep this \
             server on loopback."
        )),
    }
}

/// Binds loopback, logs, serves, never returns on success.
///
/// Protected routes require a deployment credential. Enrolled agent tokens still take the
/// read-only pseudonymization branch, and valid paired-device signatures remain credentials.
/// Without a token, ordinary direct loopback requests fail closed.
pub async fn serve_local(name: &str, port: u16, router: axum::Router) {
    serve(
        name,
        Reach::Loopback,
        port,
        router,
        InboundAuth::from_deployment().require_credential(),
    )
    .await
}

/// Serves a Unix-domain listener for a trusted reverse proxy. The socket path must be under
/// the operator's protected overlay secrets directory, which the managed agent sandbox denies.
/// The proxy-only gate rejects callers without the identity header Tailscale Serve injects.
#[cfg(unix)]
pub async fn serve_unix(
    name: &str,
    path: &std::path::Path,
    router: axum::Router,
    auth: InboundAuth,
) {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("{name}: cannot create Unix socket directory: {error}");
            std::process::exit(1);
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            if let Err(error) = std::fs::remove_file(path) {
                eprintln!("{name}: cannot remove stale Unix socket: {error}");
                std::process::exit(1);
            }
        }
        Ok(_) => {
            eprintln!("{name}: refusing to replace a non-socket at the Unix listener path");
            std::process::exit(1);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("{name}: cannot inspect Unix listener path: {error}");
            std::process::exit(1);
        }
    }
    let listener = match tokio::net::UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("{name}: cannot bind Unix socket: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("{name}: cannot restrict Unix socket permissions: {error}");
        std::process::exit(1);
    }
    let gated = authenticated(router, auth);
    println!("{name} tailnet listener ready on protected Unix socket");
    if let Err(error) = axum::serve(listener, gated).await {
        eprintln!("{name}: Unix listener failed: {error}");
        std::process::exit(1);
    }
}

/// [`serve_local`] with the gate spelled out. For a capability that resolves
/// its own token or refuses to serve without one — comms is both.
///
/// On failure it exits with a named, single-line error instead of a panic
/// backtrace: the runner captures stderr, and "cannot bind" with the address is
/// the whole diagnosis.
pub async fn serve(name: &str, reach: Reach, port: u16, router: axum::Router, auth: InboundAuth) {
    let addr = match bind_addr_for(reach, port, &auth) {
        Ok(addr) => addr,
        Err(refusal) => {
            eprintln!("{name}: {refusal}");
            std::process::exit(1);
        }
    };
    let gated = authenticated(router, auth);
    println!("{name} starting on {addr}");
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{name}: cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = axum::serve(listener, gated).await {
        eprintln!("{name}: {e}");
        std::process::exit(1);
    }
}

/// Runs a blocking operation (SQLite queries, disk I/O, CPU-heavy work) on Tokio's
/// dedicated blocking pool, flattening the inner Result and JoinError into a unified `Result<T, String>`.
///
/// Keeps blocking calls off cooperative async threads without requiring callers to write
/// verbose nested `match spawn_blocking(...).await { Ok(Ok(v)) => ..., Ok(Err(e)) => ..., Err(e) => ... }`.
pub async fn blocking<F, T, E>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("blocking task panicked or cancelled: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocking_flattens_success_and_error() {
        let ok = blocking(|| Ok::<_, String>("computed")).await;
        assert_eq!(ok, Ok("computed"));

        let err = blocking(|| Err::<(), _>("disk error")).await;
        assert_eq!(err, Err("disk error".to_string()));
    }

    #[test]
    fn bind_addr_is_loopback_only() {
        let addr = bind_addr(8084);
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 8084);
    }

    /// The whole point of `Reach`: an unauthenticated server cannot be handed
    /// an address anything but this machine can reach.
    #[test]
    fn a_non_loopback_bind_without_a_token_is_refused() {
        let open = InboundAuth::with_token(None);
        let refusal = bind_addr_for(Reach::AllInterfaces, 8082, &open).unwrap_err();
        assert!(
            refusal.contains("SJEL_INBOUND_TOKEN_FILE"),
            "the refusal must name the fix, got: {refusal}"
        );
        assert!(
            bind_addr_for(Reach::Loopback, 8082, &open)
                .unwrap()
                .ip()
                .is_loopback(),
            "an unauthenticated server keeps its loopback bind"
        );
    }

    #[test]
    fn a_token_is_what_permits_reach_beyond_this_machine() {
        let gated = InboundAuth::with_token(Some("s3cret".into()));
        let addr = bind_addr_for(Reach::AllInterfaces, 8082, &gated).unwrap();
        assert!(!addr.ip().is_loopback());
        assert_eq!(addr.port(), 8082);
    }
}

/// The gate over real HTTP. A handler called directly would skip the layer that
/// is the entire point, so these go through `authenticated` and a socket.
#[cfg(test)]
mod http_tests {
    use super::*;
    use axum::routing::get;

    const OPERATOR: &str = "operator@example.com";

    fn tailnet(auth: InboundAuth) -> InboundAuth {
        auth.with_tailnet_operator(Some(OPERATOR.into()))
    }

    async fn serve_router(auth: InboundAuth) -> String {
        let router = axum::Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/ready", get(|| async { "ok" }))
            .route("/routes", get(|| async { "{}" }))
            .route("/api/thing", get(|| async { "{}" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = authenticated(router, auth);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    // --- the agent identity (ISA F9, ISC-43) ---------------------------------------------

    fn agent_access() -> agent::AgentAccess {
        use sha2::Digest;
        let hash: [u8; 32] = sha2::Sha256::digest(b"agent-token").into();
        let registry = sjel_pseudonymize::EntityRegistry::builder()
            .add_people(["Katrin"])
            .build();
        agent::AgentAccess::new(hash, vec![9; 32], registry)
    }

    async fn serve_agent_router(auth: InboundAuth) -> String {
        use axum::routing::post;
        let router = axum::Router::new()
            .route(
                "/triage",
                get(|q: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let from_matches = q.get("from").map(|f| f == "Katrin Wissem <katrin@example.com>");
                    axum::Json(serde_json::json!([
                        {"id": "18f3a9c2b7d41e05", "data_class": "c1",
                         "from_addr": "Katrin Wissem <katrin@example.com>",
                         "subject": "Scans from Katrin", "from_matches": from_matches},
                        {"id": "18f3a9c2b7d41e06", "data_class": "c3",
                         "subject": "616685 is your code"}
                    ]))
                }),
            )
            .route("/secret", get(|| async { axum::Json(serde_json::json!({"data_class": "c3"})) }))
            .route("/page", get(|| async { "<html>Katrin</html>" }))
            .route("/triage/{id}/status", post(|| async { "moved" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = authenticated(router, auth);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn an_agent_read_is_pseudonymized_and_secret_rows_are_withheld() {
        let auth = InboundAuth::with_token(None)
            .require_credential()
            .with_agent_access(agent_access());
        let base = serve_agent_router(auth).await;
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/triage"))
            .bearer_auth("agent-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[agent::WITHHELD_HEADER], "1");
        assert!(response.headers().contains_key(agent::RECEIPT_HEADER));
        let body: serde_json::Value = response.json().await.unwrap();
        let rows = body.as_array().unwrap();
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0]["id"], "18f3a9c2b7d41e05");
        let text = body.to_string();
        assert!(!text.contains("Katrin") && !text.contains('@'), "{text}");

        // With no deployment token provisioned, ordinary callers remain closed. The agent
        // credential is not promoted into a raw-data bypass; it only entered the agent branch.
        let raw = client
            .get(format!("{base}/triage"))
            .bearer_auth("some-other-token")
            .send()
            .await
            .unwrap();
        assert_eq!(raw.status().as_u16(), 403);
    }

    #[tokio::test]
    async fn one_value_gets_one_token_per_session_and_the_query_maps_back() {
        let auth = InboundAuth::with_token(Some("full".into())).with_agent_access(agent_access());
        let base = serve_agent_router(auth).await;
        let client = reqwest::Client::new();
        let read = |session: &'static str| {
            let client = client.clone();
            let base = base.clone();
            async move {
                client
                    .get(format!("{base}/triage"))
                    .bearer_auth("agent-token")
                    .header(agent::AGENT_SESSION_HEADER, session)
                    .send()
                    .await
                    .unwrap()
                    .json::<serde_json::Value>()
                    .await
                    .unwrap()
            }
        };
        let a = read("one").await;
        let b = read("one").await;
        let c = read("two").await;
        assert_eq!(a[0]["from_addr"], b[0]["from_addr"]);
        assert_ne!(a[0]["from_addr"], c[0]["from_addr"]);

        // The agent filters by the token; the handler sees the real value.
        let token = a[0]["from_addr"].as_str().unwrap().to_string();
        let echoed: serde_json::Value = client
            .get(format!(
                "{base}/triage?from={}",
                token.replace('<', "%3C").replace('>', "%3E")
            ))
            .bearer_auth("agent-token")
            .header(agent::AGENT_SESSION_HEADER, "one")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(echoed[0]["from_matches"], true, "{echoed}");
    }

    #[tokio::test]
    async fn the_agent_token_cannot_write_or_read_what_it_cannot_rewrite() {
        let auth = InboundAuth::with_token(Some("full".into())).with_agent_access(agent_access());
        let base = serve_agent_router(auth).await;
        let client = reqwest::Client::new();
        let post = client
            .post(format!("{base}/triage/18f3a9c2b7d41e05/status"))
            .bearer_auth("agent-token")
            .send()
            .await
            .unwrap();
        assert_eq!(post.status(), 403);
        let page = client
            .get(format!("{base}/page"))
            .bearer_auth("agent-token")
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), 406);
        assert!(!page.text().await.unwrap().contains("Katrin"));
        let secret = client
            .get(format!("{base}/secret"))
            .bearer_auth("agent-token")
            .send()
            .await
            .unwrap();
        assert_eq!(secret.status(), 403);
    }

    #[tokio::test]
    async fn a_capability_that_did_not_opt_in_refuses_the_agent_token() {
        let base = serve_agent_router(InboundAuth::with_token(Some("full".into()))).await;
        let response = reqwest::Client::new()
            .get(format!("{base}/triage"))
            .bearer_auth("agent-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }

    /// Admits exactly one signature value, and records what it was asked to verify.
    struct FakeRegistry {
        seen: std::sync::Mutex<Vec<(String, String, Vec<u8>)>>,
    }

    impl DeviceVerifier for FakeRegistry {
        fn verify(
            &self,
            method: &str,
            signed_path: &str,
            headers: &axum::http::HeaderMap,
            body: &[u8],
        ) -> Result<String, String> {
            self.seen
                .lock()
                .unwrap()
                .push((method.into(), signed_path.into(), body.to_vec()));
            match headers.get(DEVICE_SIGNATURE_HEADER).map(|v| v.as_bytes()) {
                Some(b"good") => Ok("dev_phone".into()),
                _ => Err("signature is invalid".into()),
            }
        }
    }

    async fn serve_device_router(auth: InboundAuth) -> String {
        use axum::routing::post;
        let router = axum::Router::new()
            .route(
                "/interior/api/items",
                post(
                    |device: Option<axum::Extension<AdmittedDevice>>, body: String| async move {
                        format!("{}:{body}", device.map(|d| d.0 .0).unwrap_or_default())
                    },
                ),
            )
            .route("/health", get(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = authenticated(router, auth);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// PRD Q119: a paired device gets in on its key, with no token and no tailnet identity,
    /// and the handler still receives the body the gate buffered to check it.
    #[tokio::test]
    async fn a_valid_device_signature_admits_without_a_token_and_keeps_the_body() {
        let registry = std::sync::Arc::new(FakeRegistry {
            seen: Default::default(),
        });
        let auth =
            InboundAuth::with_token(Some("s3cret".into())).with_device_verifier(registry.clone());
        let base = serve_device_router(auth).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/interior/api/items?room=k"))
            .header(DEVICE_SIGNATURE_HEADER, "good")
            .body("lamp")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "dev_phone:lamp");
        // The mount is removed, the query kept: what the phone signed (mac_bridge.rs signed_path).
        assert_eq!(
            registry.seen.lock().unwrap()[0],
            ("POST".into(), "/api/items?room=k".into(), b"lamp".to_vec())
        );
    }

    /// A bad signature is refused even when the request also holds the token: a forged key is
    /// not a request that merely lacks credentials.
    #[tokio::test]
    async fn an_invalid_device_signature_is_refused_without_falling_through() {
        let registry = std::sync::Arc::new(FakeRegistry {
            seen: Default::default(),
        });
        let auth = InboundAuth::with_token(Some("s3cret".into())).with_device_verifier(registry);
        let base = serve_device_router(auth).await;
        let client = reqwest::Client::new();
        let forged = client
            .post(format!("{base}/interior/api/items"))
            .header(DEVICE_SIGNATURE_HEADER, "forged")
            .header("Authorization", "Bearer s3cret")
            .send()
            .await
            .unwrap();
        assert_eq!(forged.status(), 401);
        let unsigned = client
            .post(format!("{base}/interior/api/items"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            unsigned.status(),
            401,
            "no key and no token is still refused"
        );
        let health = client
            .get(format!("{base}/health"))
            .header(DEVICE_SIGNATURE_HEADER, "forged")
            .send()
            .await
            .unwrap();
        assert_eq!(
            health.status(),
            200,
            "exempt paths do not check a signature"
        );
    }

    #[test]
    fn the_signed_path_matches_the_phone() {
        assert_eq!(
            device_signed_path("/interior/api/items?x=1"),
            "/api/items?x=1"
        );
        assert_eq!(
            device_signed_path("/sjel-status/api/sjel-status/health"),
            "/api/sjel-status/health"
        );
        assert_eq!(
            device_signed_path("/api/suggest?q=Berlin"),
            "/api/suggest?q=Berlin"
        );
        assert_eq!(device_signed_path("/transit"), "/");
    }

    #[tokio::test]
    async fn a_gated_server_answers_401_without_a_token_and_200_with_either_header() {
        let base = serve_router(InboundAuth::with_token(Some("s3cret".into()))).await;
        let client = reqwest::Client::new();

        assert_eq!(
            client
                .get(format!("{base}/api/thing"))
                .send()
                .await
                .unwrap()
                .status(),
            401,
            "no token must not reach the handler"
        );
        for (name, value) in [
            ("Authorization", "Bearer s3cret"),
            ("X-Axon-Token", "s3cret"),
        ] {
            let response = client
                .get(format!("{base}/api/thing"))
                .header(name, value)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "{name} was refused");
        }
    }

    #[tokio::test]
    async fn health_and_ready_answer_through_the_gate_but_routes_does_not() {
        let base = serve_router(InboundAuth::with_token(Some("s3cret".into()))).await;
        let client = reqwest::Client::new();
        for path in ["/health", "/ready"] {
            let response = client.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(response.status(), 200, "{path} must stay pollable");
        }
        assert_eq!(
            client
                .get(format!("{base}/routes"))
                .send()
                .await
                .unwrap()
                .status(),
            401,
            "the manifest is surface description, not liveness"
        );
    }

    #[tokio::test]
    async fn a_credential_required_server_fails_closed_without_a_token() {
        let base = serve_router(InboundAuth::with_token(None).require_credential()).await;
        let client = reqwest::Client::new();
        for path in ["/health", "/ready"] {
            assert_eq!(
                client
                    .get(format!("{base}{path}"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        for path in ["/routes", "/api/thing"] {
            assert_eq!(
                client
                    .get(format!("{base}{path}"))
                    .header("Tailscale-User-Login", OPERATOR)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                403,
                "loopback callers cannot forge trusted-proxy identity"
            );
        }
    }

    #[tokio::test]
    async fn the_proxy_only_listener_requires_the_declared_operator_identity() {
        let auth = tailnet(InboundAuth::with_token(None)).tailnet_proxy_only();
        let base = serve_router(auth).await;
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(format!("{base}/api/thing"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}/api/thing"))
                .header("Tailscale-User-Login", "stranger@example.com")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}/api/thing"))
                .header("Tailscale-User-Login", OPERATOR)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }

    #[tokio::test]
    async fn an_ungated_server_is_byte_for_byte_the_server_that_predates_this_gate() {
        let base = serve_router(InboundAuth::with_token(None)).await;
        for path in ["/health", "/ready", "/routes", "/api/thing"] {
            let response = reqwest::get(format!("{base}{path}")).await.unwrap();
            assert_eq!(
                response.status(),
                200,
                "{path} changed on a loopback-only deployment"
            );
        }
    }
}
