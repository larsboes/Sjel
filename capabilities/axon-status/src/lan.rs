//! The shell's local-network listener and its Bonjour advertisement (PRD Q119).
//!
//! Opt-in with `SJEL_LAN_PORT` in `<overlay>/config/deployment.env`. The same router is served on
//! a second port with TLS on every interface, behind the devices-only gate
//! (`libs/axon-server/src/lan.rs`). The certificate fingerprint is advertised in the Bonjour TXT
//! record and served at `/api/axon-status/lan`, so the pairing screen can show it for the person
//! to compare with what the phone found.

use std::sync::{Arc, OnceLock};

use axon_server::lan::{self, LanIdentity};
use axon_server::DeviceVerifier;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(crate) struct LanInfo {
    pub port: u16,
    pub host: String,
    pub fingerprint: String,
    pub service: &'static str,
}

static LAN: OnceLock<LanInfo> = OnceLock::new();

/// What `/api/axon-status/lan` answers: the listener, or `None` when it is not enabled.
pub(crate) fn info() -> Option<LanInfo> {
    LAN.get().cloned()
}

/// Starts the listener and the advertisement when the deployment enabled them. Returns at once.
pub(crate) fn start(router: axum::Router, verifier: Arc<dyn DeviceVerifier>) {
    let Some(port) = lan::deployment_port() else {
        return;
    };
    let Some(dir) = axon_config::overlay_data_dir("lan") else {
        eprintln!("[axon-status] SJEL_LAN_PORT is set but no overlay is; the LAN listener needs one for its key");
        return;
    };
    let host = local_host_name();
    let identity = match LanIdentity::load_or_create(&dir, &host) {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("[axon-status] LAN listener not started: {error}");
            return;
        }
    };
    let _ = LAN.set(LanInfo {
        port,
        host: host.clone(),
        fingerprint: identity.fingerprint.clone(),
        service: lan::SERVICE_TYPE,
    });
    advertise(port, &host, &identity.fingerprint);
    let auth = axon_server::InboundAuth::from_deployment().with_device_verifier(verifier);
    tokio::spawn(async move { lan::serve_lan("axon-status", port, router, auth, &identity).await });
}

/// The name the Mac answers to as `<name>.local`. `scutil` is macOS; elsewhere `hostname -s`.
fn local_host_name() -> String {
    let run = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|name| !name.is_empty())
    };
    run("scutil", &["--get", "LocalHostName"])
        .or_else(|| run("hostname", &["-s"]))
        .unwrap_or_else(|| "axon".to_string())
}

/// Registers `_axon._tcp` with macOS's own `dns-sd`, so no mDNS library enters the tree. The
/// registration lives as long as the child process, which lives as long as this one.
fn advertise(port: u16, host: &str, fingerprint: &str) {
    let spawned = tokio::process::Command::new("dns-sd")
        .args([
            "-R",
            &format!("Axon on {host}"),
            lan::SERVICE_TYPE,
            "local",
            &port.to_string(),
            "v=1",
            // The phone connects to `https://<host>.local:<port>` straight from the record,
            // without resolving the service first.
            &format!("host={host}"),
            &format!("port={port}"),
            &format!("fp={fingerprint}"),
        ])
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    match spawned {
        Ok(mut child) => {
            tokio::spawn(async move {
                let status = child.wait().await;
                eprintln!("[axon-status] Bonjour advertisement ended: {status:?}");
            });
        }
        Err(error) => eprintln!(
            "[axon-status] no Bonjour advertisement ({error}); a phone can still connect by address"
        ),
    }
}
