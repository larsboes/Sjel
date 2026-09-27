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

use sjel_config::{database_path, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
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
        Self {
            database_path: database_path(),
            port,
        }
    }
}
