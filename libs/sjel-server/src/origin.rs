//! Is this browser origin allowed to talk to this capability at all?
//!
//! Moved here verbatim from `capabilities/places/src/server.rs`, where it was
//! written for the companion register (places README D4, ISA PLC-7). It moved
//! because a second capability needs it: `trips` serves plan-search results
//! that carry the operator's feasible calendar windows and a companion hint,
//! under `CorsLayer::permissive()`. A second copy of a security predicate is
//! the drift this repo's third principle forbids, so there is one home.
//!
//! Refusing the request — not merely omitting CORS headers — is what also stops
//! a hostile page's "simple" cross-site POST to a write route, which a browser
//! sends before it ever reads a response header.
//!
//! A request with no `Origin` header is not a browser cross-origin call (curl,
//! the runner's health probes, same-origin GETs, and every server-to-server
//! caller such as `capabilities/calendar/src/server.rs`'s POST into trips) and
//! passes. With one, the allowed set mirrors how the dashboard itself is
//! reached (`dashboard/vite.config.ts`, `allowedHosts`): the loopback dev
//! origin, or a tailnet name.
//!
//! The tailnet check is a bare `.ts.net` suffix by default, because the
//! machine's MagicDNS name is a house fact and this repo is public (the same
//! trade-off vite.config.ts records). Known gap: the suffix also admits
//! Tailscale Funnel sites — public pages on other people's tailnets, which do
//! NOT authenticate at this tailnet's layer. Set
//! `SJEL_<CAPABILITY>_ALLOWED_ORIGIN_HOSTS` (comma-separated exact hosts, from
//! the overlay) to replace the suffix with the deployment's own names and close
//! that gap without naming the machine in public code.
//!
//! ## axum's layer rule, which this module cannot enforce
//!
//! `Router::layer` wraps only the routes registered **before** it: "Additional
//! routes added after `layer` is called will not have the middleware added"
//! (axum 0.7 `src/docs/routing/layer.md`). A route appended below
//! `.layer(from_fn_with_state("places", refuse_foreign_origins))` silently
//! loses this guard, and a test that exercises [`origin_allowed_by`] alone
//! still passes. Each consumer therefore drives its wired `Router` with a
//! foreign `Origin` in its own test; see `capabilities/places/src/server.rs`
//! and `capabilities/trips/src/server.rs`.

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Json, Response};

/// The env var one capability reads to replace the `.ts.net` suffix.
fn allowed_hosts_var(capability: &str) -> String {
    format!(
        "SJEL_{}_ALLOWED_ORIGIN_HOSTS",
        capability.to_ascii_uppercase().replace('-', "_")
    )
}

/// The predicate with the environment already read. Pure, so tests never touch
/// process env (the explicit-parameter pattern places' geocode db_tests use).
pub fn origin_allowed_by(origin: Option<&str>, allowed_hosts: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    // Obsidian's renderer, and only that exact string. Admitted 2026-09-09 (Q107)
    // because it grants nothing: Obsidian already holds the whole vault open on
    // disk, so refusing it an HTTP route it could answer from a file protects no
    // fact. What the guard exists to stop is a PAGE the operator visited, and a
    // browser sets `Origin` itself — a page cannot claim this one.
    //
    // The literal, not the `app://` scheme: every other Electron app on this Mac
    // also sends an `app://` origin, and none of them has the vault.
    if origin == "app://obsidian.md" {
        return true;
    }
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false; // "null", file://, extensions — nothing a capability serves
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) {
                host
            } else {
                authority // no port; the colon was IPv6's
            }
        });
    if matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
        return true;
    }
    match allowed_hosts.map(str::trim).filter(|list| !list.is_empty()) {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|allowed| !allowed.is_empty())
            .any(|allowed| allowed == host),
        None => host.ends_with(".ts.net"),
    }
}

/// The same predicate, reading `SJEL_<CAPABILITY>_ALLOWED_ORIGIN_HOSTS`.
pub fn origin_allowed(capability: &str, origin: Option<&str>) -> bool {
    let allowed_hosts = sjel_config::env_var(&allowed_hosts_var(capability)).ok();
    origin_allowed_by(origin, allowed_hosts.as_deref())
}

/// The middleware. Wire it as
/// `.layer(axum::middleware::from_fn_with_state("places", refuse_foreign_origins))`
/// **below** every route it must cover — see the module note on axum's rule.
pub async fn refuse_foreign_origins(
    State(capability): State<&'static str>,
    request: Request,
    next: Next,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !origin_allowed(capability, origin) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": format!("cross-origin access to {capability} is not allowed")
            })),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C2 guard: non-browser callers and the origins the dashboard itself
    /// is served from pass; every other web origin is refused, so a hostile
    /// page can neither read a guarded surface nor drive a write route
    /// cross-site.
    #[test]
    fn foreign_browser_origins_are_refused() {
        // No Origin header: curl, the runner, same-origin GETs.
        assert!(origin_allowed_by(None, None));
        // The dashboard's own origins (dashboard/vite.config.ts allowedHosts).
        assert!(origin_allowed_by(Some("http://localhost:47117"), None));
        assert!(origin_allowed_by(Some("http://127.0.0.1:47117"), None));
        assert!(origin_allowed_by(Some("http://[::1]:47117"), None));
        assert!(origin_allowed_by(Some("http://localhost"), None));
        assert!(origin_allowed_by(Some("https://mac.tailnet.ts.net"), None));
        // Everyone else.
        assert!(!origin_allowed_by(Some("https://evil.example"), None));
        assert!(!origin_allowed_by(Some("https://evilts.net"), None));
        assert!(!origin_allowed_by(
            Some("https://mac.ts.net.evil.example"),
            None
        ));
        assert!(!origin_allowed_by(
            Some("http://localhost.evil.example"),
            None
        ));
        // Obsidian's renderer, admitted 2026-09-09 so axon-lens can read at all.
        // Measured before the change: every capability answered 403 to this
        // exact header, which is what a plugin sends on every request.
        assert!(origin_allowed_by(Some("app://obsidian.md"), None));
        // The literal and not the scheme. Another Electron app on this Mac sends
        // an app:// origin too, and none of them already holds the vault.
        assert!(!origin_allowed_by(Some("app://other.app"), None));
        assert!(!origin_allowed_by(
            Some("app://obsidian.md.evil.example"),
            None
        ));
        assert!(!origin_allowed_by(Some("app://"), None));
        // An explicit host list must not silently drop it: the plugin has to keep
        // working on a deployment that has closed the .ts.net suffix gap.
        assert!(origin_allowed_by(
            Some("app://obsidian.md"),
            Some("mac.tailnet.ts.net")
        ));
        assert!(!origin_allowed_by(Some("null"), None));
        assert!(!origin_allowed_by(Some("file:///tmp/page.html"), None));
    }

    /// With the capability's host list set, only the named tailnet hosts pass:
    /// a Funnel page on someone else's tailnet no longer does, which is the gap
    /// the bare `.ts.net` suffix leaves open.
    #[test]
    fn an_explicit_host_list_replaces_the_tailnet_suffix() {
        let list = Some("mac.tailnet.ts.net, phone.tailnet.ts.net");
        assert!(origin_allowed_by(Some("https://mac.tailnet.ts.net"), list));
        assert!(origin_allowed_by(
            Some("https://phone.tailnet.ts.net"),
            list
        ));
        assert!(!origin_allowed_by(
            Some("https://evil.other-tailnet.ts.net"),
            list
        ));
        // Loopback stays allowed whatever the list says.
        assert!(origin_allowed_by(Some("http://localhost:47117"), list));
        // A blank value means unset, not "allow nothing".
        assert!(origin_allowed_by(
            Some("https://mac.tailnet.ts.net"),
            Some("  ")
        ));
    }

    /// The env var name is derived, so a capability whose name carries a hyphen
    /// still reads a legal shell identifier.
    #[test]
    fn the_env_var_is_named_after_the_capability() {
        assert_eq!(
            allowed_hosts_var("places"),
            "SJEL_PLACES_ALLOWED_ORIGIN_HOSTS"
        );
        assert_eq!(
            allowed_hosts_var("trips"),
            "SJEL_TRIPS_ALLOWED_ORIGIN_HOSTS"
        );
        assert_eq!(
            allowed_hosts_var("sjel-status"),
            "SJEL_SJEL_STATUS_ALLOWED_ORIGIN_HOSTS"
        );
    }
}
