//! Config. This crate is public, so nothing personal lives here: the rows are in the
//! overlay's database (`axon_config::database_path`), and the port is the runner's.

use std::path::PathBuf;

use axon_config::{database_path, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
    /// `capabilities/places`, for turning a city into a coordinate. Same variable and
    /// default as `capabilities/trips/src/upstream.rs`.
    pub places_url: String,
    /// `capabilities/foundation-models`, the on-device model that advises on unclear
    /// duplicates. Loopback only: C2 goes to it (PRD §6.1).
    pub model_url: String,
}

impl Config {
    pub fn load() -> Self {
        Self {
            database_path: database_path(),
            port: resolve_port(None, None, 8097),
            places_url: axon_config::env_var("SJEL_PLACES_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8093".to_string()),
            model_url: axon_config::env_var("SJEL_LOCAL_MODEL_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8091".to_string()),
        }
    }
}
