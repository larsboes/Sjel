//! Config resolution. Axon doctrine: this crate is public, so no personal value
//! lives here — everything personal comes from the private overlay at runtime.
//!
//! The store path resolves through `sjel_config::database_path`: `SJEL_DB_PATH`,
//! else `<overlay>/data/axon/axon.db`. It is a deployment fact rather than a
//! capability one (PRD Q45) — a file per capability would drop the
//! cross-capability joins `layers.rs` and `backfill.rs` are built on.

use std::path::PathBuf;

use sjel_config::{database_path, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        // 8093 mirrors service.toml here; SJEL_PORT (the runner's contract) wins.
        // Not 8091 (capabilities/foundation-models/service.toml owns it) and not
        // 8092 (the private overlay's interior capability owns it — a repo-only
        // port sweep misses overlay service.tomls) — the scouting/vaultwarden
        // 8080 collision class, libs/sjel-config.
        let port = resolve_port(None, None, 8093);
        Self {
            database_path: database_path(),
            port,
        }
    }
}
