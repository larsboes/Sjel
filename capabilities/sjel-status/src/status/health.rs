use super::*;

pub(crate) async fn is_up(client: &reqwest::Client, svc: &Service) -> bool {
    let Some(url) = svc.readiness_url() else {
        return false;
    };
    client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        // A redirect counts as up: a locale-routing frontend answers / with a 307 and
        // is perfectly alive.
        .map(|res| res.status().is_success() || res.status().is_redirection())
        .unwrap_or(false)
}

pub(crate) async fn health_handler() -> Json<Value> {
    Json(json!({ "ok": true, "service": "sjel-status" }))
}

/// Version identity for the spine repo and the overlay, read at request time.
///
/// Shelled out to `tools/repos` rather than reimplemented: git plumbing and the
/// overlay's location both already have exactly one home, and this process learning
/// either would be a second one. Same pattern as the registry.
pub(crate) async fn repos_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let out = tokio::process::Command::new(root.join("tools/repos"))
        .arg("--json")
        .output()
        .await
        .map_err(|e| bad_gateway(format!("could not run tools/repos: {e}")))?;
    if !out.status.success() {
        return Err(bad_gateway(format!(
            "tools/repos failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map(Json)
        .map_err(|e| bad_gateway(format!("tools/repos did not emit JSON: {e}")))
}

// `GET /upstreams` lived here until 2026-08-28. It shelled out to
// `tools/upstream-checker --json --offline` and fed the dashboard's `/upstreams` page a
// per-entry `ok`/`na`/`warn`/`fail` status. PRD Q41 retired that script, and the status
// field was the script's verdict — recomputing it here would rebuild in Rust exactly the
// plumbing the item exists to delete.
//
// The doctrine half of that page did not need this endpoint and still does not: `self.json`
// carries `{name, verdict}` for every entry in `upstreams.toml`, read straight from the
// manifest by `tools/self generate`, and `/self` below already serves it. What is gone is
// the enforcement verdict, which Dependabot's pull requests and alerts now report on GitHub.

/// The committed self-model, fused with live state at read time.
///
/// `model` is `self.json` verbatim — structure, compile-time coupling, provenance and
/// code size, all derived from tracked files by `tools/self generate`. `live` is the
/// per-capability `up` map this process already owns.
///
/// They stay two sibling keys rather than one merged object on purpose. `self.json` is
/// committed, so writing `up` into it would give live state a second home and make the
/// file lie the moment a process stops; keeping the fusion visible at the response
/// boundary means a reader can always tell which half is a fact about the repo and which
/// is a fact about this machine right now. `up` is `null` for a capability that declares
/// nothing to poll — unknown, not down — matching the capabilities endpoint.
pub(crate) async fn self_model_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let root = axon_root().map_err(bad_gateway)?;
    let path = root.join("self.json");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        bad_gateway(format!(
            "cannot read {} ({e}) — run: tools/self generate",
            path.display()
        ))
    })?;
    let model: Value = serde_json::from_str(&text)
        .map_err(|e| bad_gateway(format!("{} is not valid JSON: {e}", path.display())))?;

    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();
    let mut live = serde_json::Map::new();
    for service in &services {
        let up = match service.readiness_url() {
            Some(_) => Some(is_up(&client, service).await),
            None => None,
        };
        live.insert(service.name.clone(), json!(up));
    }

    Ok(Json(json!({ "model": model, "live": live })))
}

/// The dashboard's long-standing contract: name -> {up, url}. Same shape as before,
/// now covering every enabled capability instead of two compiled-in names.
pub(crate) async fn sjel_status_health_handler(
) -> Result<Json<AxonStatusHealth>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();

    let mut capabilities = HashMap::new();
    for svc in &services {
        // The URL reported beside `up` is the one `up` was judged on, so the two cannot
        // disagree about which surface was asked.
        let Some(url) = svc.readiness_url() else {
            continue;
        };
        capabilities.insert(
            svc.name.clone(),
            CapabilityStatus {
                up: is_up(&client, svc).await,
                url,
            },
        );
    }

    // "ok" means what should be running is running — the autostart set. Once
    // capabilities start on demand, "everything is up" stopped being a health signal:
    // a stopped on-demand capability is the normal state, not a fault, and reporting
    // it as one would leave the shell permanently amber.
    //
    // Only over what can actually be polled. A capability with no health surface is
    // never in `capabilities` above, and the old `unwrap_or(false)` read that absence
    // as "down" — so the moment postgres and vaultwarden declared `autostart = "true"`
    // (2026-07-30, replacing the undeclared watchdogs that had been keeping them up),
    // the shell reported "at least one service that should be running is not
    // answering" about two containers that were both running fine. Unknown is not down;
    // that is what every other reader of this registry already says.
    let ok = services
        .iter()
        .filter(|s| s.autostart == "true")
        .filter_map(|s| capabilities.get(&s.name))
        .all(|c| c.up);
    let uptime_seconds = STARTED_AT.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);

    Ok(Json(AxonStatusHealth {
        ok,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds,
        capabilities,
    }))
}

/// Everything the shell needs to render itself: what exists and what is up.
///
/// `panel_port` and `panel_path` are reported as the manifest facts they are; the
/// panel's URL is deliberately NOT assembled here. A panel is loaded by a browser, and
/// that browser has to reach it on the host it is already talking to: composing
/// `127.0.0.1:<port>` server-side breaks the moment the shell is opened as `localhost`
/// (Chrome treats the two as different sites and partitions storage, which is enough to
/// kill a framework's client init) and breaks harder over Tailscale, where 127.0.0.1 is
/// the phone. The shell builds the URL from its own `location`.
pub(crate) async fn capabilities_handler(
) -> Result<Json<Vec<CapabilityView>>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();

    let mut views = Vec::with_capacity(services.len());
    for service in services {
        let up = match service.readiness_url() {
            Some(_) => Some(is_up(&client, &service).await),
            None => None,
        };
        views.push(CapabilityView {
            health_url: service.health_url(),
            up,
            service,
        });
    }
    Ok(Json(views))
}

/// Every enabled capability's route manifest, in one map.
///
/// The single "what can I call" endpoint. Each capability reports its own paths,
/// so this stays correct across the five different URL conventions in use
/// without anyone having to remember which capability follows which — and it
/// keeps working unchanged if those conventions later converge.
///
/// A capability that is down, or too old to serve `/routes`, is reported with
/// its reason rather than omitted. Silently returning a shorter list would read
/// as "that capability has no endpoints", which is the one wrong answer here.
pub(crate) async fn routes_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let services = registry().await.map_err(bad_gateway)?;
    let client = reqwest::Client::new();
    // `/routes` sits behind the inbound gate on every capability that starts
    // through sjel_server (only `/health` and `/ready` are exempt). Without the
    // token this aggregation would report every gated capability as "not
    // running", which is the one wrong answer this endpoint must not give.
    // Resolved per request rather than at startup so rotating the token file
    // does not need a restart of this process too.
    let bearer = sjel_server::InboundAuth::from_deployment().bearer_header();

    let mut capabilities = Vec::with_capacity(services.len());
    for service in services {
        if service.port.is_empty() {
            continue;
        }
        let url = format!("http://127.0.0.1:{}/routes", service.port);
        let mut request = client.get(&url).timeout(Duration::from_secs(2));
        if let Some(bearer) = &bearer {
            request = request.header(reqwest::header::AUTHORIZATION, bearer);
        }
        let manifest = request
            .send()
            .await
            .ok()
            .filter(|response| response.status().is_success());
        let entry = match manifest {
            Some(response) => match response.json::<Value>().await {
                Ok(body) => json!({ "name": service.name, "routes": body["routes"] }),
                Err(error) => json!({
                    "name": service.name,
                    "unavailable": format!("served /routes but the body did not parse: {error}"),
                }),
            },
            None => json!({
                "name": service.name,
                "unavailable": "not running, or does not serve /routes",
            }),
        };
        capabilities.push(entry);
    }
    Ok(Json(json!({ "capabilities": capabilities })))
}
