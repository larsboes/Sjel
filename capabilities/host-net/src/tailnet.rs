//! The tailnet's posture, as four unprivileged reads: `tailscale status --json`,
//! `tailscale debug prefs`, `tailscale lock status` and `tailscale version --upstream --json`.
//!
//! # Nothing here prints a key
//!
//! `tailscale lock status` writes real key material to stdout — the node's own tailnet-lock
//! key, a KeyID and a WrappingPubkey, all in full. This verb takes one fact from that output,
//! the number of trusted signing keys, and drops the rest before it reaches any surface. The
//! table and `--json` both print the count; neither prints a key. An operator who wants the
//! keys runs the tool that owns them.
//!
//! Node names are treated the same way, one step softer: the table prints counts only, and
//! `--json` names a peer only when this verb has flagged that peer.

use serde::Serialize;
use serde_json::Value;

use crate::sys::capture;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Prefs {
    /// Shields up blocks all incoming tailnet connections. It is NOT in `status --json` —
    /// measured — which is why prefs is a separate read rather than a field lookup.
    pub shields_up: Option<bool>,
    pub route_all: Option<bool>,
    pub run_ssh: Option<bool>,
    pub posture_checking: Option<bool>,
    pub want_running: Option<bool>,
    pub advertised_routes: usize,
    pub uses_exit_node: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Lock {
    pub enabled: bool,
    /// Count only. See this module's header.
    pub trusted_keys: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Version {
    /// The daemon's version from `status --json`. A package upgrade replaces the binary but
    /// not the running process, so the CLI's own version can be newer than this.
    pub running: String,
    /// The newest release on the node's track, from pkgs.tailscale.com. `None` when that read
    /// failed, for example offline.
    pub upstream: Option<String>,
    pub behind: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TailnetReport {
    pub backend_state: String,
    pub tun: Option<bool>,
    pub peers: usize,
    pub peers_online: usize,
    /// Peers whose node key does not expire, by short name. A key with no expiry is a
    /// credential that outlives every rotation policy the tailnet has.
    pub peers_never_expiring: Vec<String>,
    pub self_key_expires: bool,
    pub prefs: Prefs,
    pub lock: Lock,
    pub version: Version,
}

fn as_bool(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// The leading label of a MagicDNS name: `host.tail1234.ts.net` → `host`. The tailnet's own
/// suffix identifies the deployment and is dropped here rather than carried around.
fn short_name(peer: &Value) -> String {
    let dns = peer.get("DNSName").and_then(Value::as_str).unwrap_or("");
    let first = dns.split('.').next().unwrap_or("");
    if !first.is_empty() {
        return first.to_string();
    }
    peer.get("HostName")
        .and_then(Value::as_str)
        .unwrap_or("(unnamed)")
        .to_string()
}

/// Parse `tailscale status --json`.
///
/// `KeyExpiry` is ABSENT, not null, on a node whose key does not expire — tailscale omits the
/// field. Treating "absent" as "unknown, assume fine" is the failure this test pins: on the
/// build host exactly one of three peers omits it, and that peer is the one worth knowing about.
pub fn parse_status(text: &str) -> Result<TailnetReport, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("tailscale status --json: {e}"))?;
    let mut r = TailnetReport {
        backend_state: v
            .get("BackendState")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        tun: as_bool(&v, "TUN"),
        version: Version {
            running: v
                .get("Version")
                .and_then(Value::as_str)
                .and_then(|s| s.split('-').next())
                .unwrap_or("unknown")
                .to_string(),
            ..Default::default()
        },
        self_key_expires: v
            .get("Self")
            .and_then(|s| s.get("KeyExpiry"))
            .is_some_and(|k| !k.is_null()),
        ..Default::default()
    };
    if let Some(peers) = v.get("Peer").and_then(Value::as_object) {
        r.peers = peers.len();
        for peer in peers.values() {
            if peer.get("Online").and_then(Value::as_bool) == Some(true) {
                r.peers_online += 1;
            }
            if peer.get("KeyExpiry").is_none_or(Value::is_null) {
                r.peers_never_expiring.push(short_name(peer));
            }
        }
        r.peers_never_expiring.sort();
    }
    Ok(r)
}

/// Parse `tailscale debug prefs`.
pub fn parse_prefs(text: &str) -> Result<Prefs, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("tailscale debug prefs: {e}"))?;
    Ok(Prefs {
        shields_up: as_bool(&v, "ShieldsUp"),
        route_all: as_bool(&v, "RouteAll"),
        run_ssh: as_bool(&v, "RunSSH"),
        posture_checking: as_bool(&v, "PostureChecking"),
        want_running: as_bool(&v, "WantRunning"),
        advertised_routes: v
            .get("AdvertiseRoutes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        uses_exit_node: v
            .get("ExitNodeID")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
    })
}

/// Parse `tailscale lock status` down to two facts and discard the rest.
///
/// "NOT enabled" contains the word "enabled", so the negative forms are tested first. Getting
/// that backwards reports a tailnet with no lock as locked, which is the one error in this
/// function that would be reassuring rather than noisy.
pub fn parse_lock(text: &str) -> Lock {
    let mut lock = Lock::default();
    let mut in_keys = false;
    for line in text.lines() {
        let l = line.trim();
        let upper = l.to_ascii_uppercase();
        if upper.starts_with("TAILNET LOCK IS") {
            lock.enabled = !upper.contains("NOT ENABLED")
                && !upper.contains("DISABLED")
                && upper.contains("ENABLED");
        }
        if upper.starts_with("TRUSTED SIGNING KEYS") {
            in_keys = true;
            continue;
        }
        if in_keys && l.starts_with("tlpub:") {
            lock.trusted_keys += 1;
        }
    }
    lock
}

/// Parse `tailscale version --upstream --json` down to the `upstream` field.
pub fn parse_upstream(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    v.get("upstream")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `1.104.2` → `(1, 104, 2)`. Anything after a `-` is build metadata and ignored.
fn triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('-').next()?.split('.').map(str::parse::<u64>);
    Some((
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ))
}

/// True only when both versions parse and the running one is lower. An unknown upstream is
/// never "behind": the report shows it as unknown instead.
pub fn is_behind(running: &str, upstream: Option<&str>) -> bool {
    match (triple(running), upstream.and_then(triple)) {
        (Some(r), Some(u)) => r < u,
        _ => false,
    }
}

/// Read the tailnet. Only `status` is required: a node that is not signed in still has one,
/// and a report that refuses to print because an optional read failed is less useful than one
/// that says which part is missing.
///
/// `socket` selects a tailscaled other than the default one, for a host that runs two.
pub fn collect(socket: Option<&str>) -> Result<TailnetReport, String> {
    let flag = socket.map(|s| format!("--socket={s}"));
    let run = |args: &[&str]| {
        let mut all: Vec<&str> = flag.iter().map(String::as_str).collect();
        all.extend_from_slice(args);
        capture("tailscale", &all)
    };
    let status = run(&["status", "--json"])?;
    let mut report = parse_status(&status)?;
    if let Ok(text) = run(&["debug", "prefs"]) {
        report.prefs = parse_prefs(&text).unwrap_or_default();
    }
    if let Ok(text) = run(&["lock", "status"]) {
        report.lock = parse_lock(&text);
    }
    if let Ok(text) = run(&["version", "--upstream", "--json"]) {
        report.version.upstream = parse_upstream(&text);
    }
    report.version.behind = is_behind(&report.version.running, report.version.upstream.as_deref());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = include_str!("../fixtures/tailscale-status.json");
    const PREFS: &str = include_str!("../fixtures/tailscale-prefs.json");
    const LOCK: &str = include_str!("../fixtures/tailscale-lock.txt");

    #[test]
    fn a_peer_with_no_key_expiry_is_flagged() {
        let r = parse_status(STATUS).unwrap();
        assert_eq!(r.peers, 3);
        assert_eq!(r.peers_online, 2);
        assert_eq!(r.peers_never_expiring, vec!["example-node-b".to_string()]);
        assert!(r.self_key_expires);
    }

    /// ShieldsUp is in prefs and NOT in `status --json`. A test that read it from a status mock
    /// would pass here and report the wrong answer against the real tool.
    #[test]
    fn shields_up_comes_from_prefs_not_status() {
        let status: Value = serde_json::from_str(STATUS).unwrap();
        assert!(status.get("ShieldsUp").is_none());
        let p = parse_prefs(PREFS).unwrap();
        assert_eq!(p.shields_up, Some(false));
        assert_eq!(p.route_all, Some(true));
        assert_eq!(p.want_running, Some(false));
        assert_eq!(p.advertised_routes, 0);
        assert!(!p.uses_exit_node);
    }

    /// One trusted signing key is one lost laptop away from a tailnet nobody can re-sign into.
    #[test]
    fn lock_reports_a_count_and_no_key_material() {
        let l = parse_lock(LOCK);
        assert!(l.enabled);
        assert_eq!(l.trusted_keys, 1);
        let rendered = serde_json::to_string(&l).unwrap();
        assert!(
            !rendered.contains("tlpub:"),
            "no key material leaves this parser"
        );
    }

    #[test]
    fn the_running_version_comes_from_status_without_build_metadata() {
        assert_eq!(parse_status(STATUS).unwrap().version.running, "1.0.0");
    }

    #[test]
    fn behind_compares_numerically_and_never_guesses() {
        assert!(is_behind("1.104.2", Some("1.104.4")));
        assert!(
            is_behind("1.98.9", Some("1.104.0")),
            "98 < 104, not \"9\" > \"1\""
        );
        assert!(!is_behind("1.104.4", Some("1.104.4")));
        assert!(!is_behind("1.106.0", Some("1.104.4")));
        assert!(!is_behind("1.104.2", None));
        assert!(!is_behind("unknown", Some("1.104.4")));
    }

    #[test]
    fn upstream_is_read_from_version_json() {
        let text = r#"{"majorMinorPatch":"1.104.2","short":"1.104.2","upstream":"1.104.4"}"#;
        assert_eq!(parse_upstream(text).as_deref(), Some("1.104.4"));
        assert_eq!(parse_upstream(r#"{"short":"1.104.2"}"#), None);
        assert_eq!(parse_upstream("not json"), None);
    }

    #[test]
    fn a_tailnet_without_lock_is_not_read_as_locked() {
        assert!(!parse_lock("Tailnet Lock is NOT enabled.\n").enabled);
        assert!(!parse_lock("Tailnet lock is disabled.\n").enabled);
    }
}
