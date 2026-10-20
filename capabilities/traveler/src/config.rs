//! Config resolution. Axon doctrine: this crate is public, so no personal value
//! lives here — everything personal comes from the private overlay at runtime,
//! and in this capability that means the profile rows and nothing else.
//!
//! The store path resolves through `sjel_config::database_path`: `SJEL_DB_PATH`,
//! else `<overlay>/data/axon/axon.db`. It is a deployment fact rather than a
//! capability one (PRD Q45), and this capability reads it for the same reason
//! `capabilities/places` does — a file per capability would put the profile in a
//! second database from the plans it is derived from.

use std::path::PathBuf;

use sjel_config::{database_path, env_var, expand_tilde, overlay_config, overlay_root, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
    pub telos_dir: Option<PathBuf>,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        // 8096 mirrors service.toml here; SJEL_PORT (the runner's contract) wins.
        // Checked against the live registry rather than a remembered list: 8095 is
        // ytalburn and 8097 was free at the time of writing, but the overlay owns
        // ports this repo cannot see (the scouting/vaultwarden 8080 collision
        // class, libs/sjel-config), so the runner's variable is the authority.
        let port = resolve_port(None, None, 8096);
        let telos_dir = resolve_telos_dir();
        Self {
            database_path: database_path(),
            telos_dir,
            port,
        }
    }
}

fn resolve_telos_dir() -> Option<PathBuf> {
    if let Some(explicit) = env_var("SJEL_TELOS_DIR")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return Some(expand_tilde(&explicit));
    }
    let root = overlay_root()?;
    let overlay_vault = root.join("vault/TELOS/Personal");
    if overlay_vault.is_dir() {
        return Some(overlay_vault);
    }
    if let Some(scouting_cfg) = overlay_config("scouting.json") {
        if let Ok(text) = std::fs::read_to_string(scouting_cfg) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                for source in val
                    .get("sources")
                    .and_then(|s| s.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(raw_path) = source.get("path").and_then(|p| p.as_str()) {
                        let vault_root = expand_tilde(raw_path);
                        for rel in ["TELOS/Personal", "Atlas/Personal"] {
                            let candidate = vault_root.join(rel);
                            if candidate.is_dir() {
                                return Some(candidate);
                            }
                        }
                    }
                }
            }
        }
    }
    let lifeos_telos = root.join("config/lifeos/TELOS");
    if lifeos_telos.is_dir() {
        return Some(lifeos_telos);
    }
    Some(overlay_vault)
}
