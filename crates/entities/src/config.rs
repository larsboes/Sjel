//! Store and service settings for the Entities owner.

use std::path::PathBuf;

use sjel_config::{database_path, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
    /// `crates/places`, for turning a city into a coordinate. Same variable and
    /// default as `crates/trips/src/upstream.rs`.
    pub places_url: String,
    /// `services/foundation-models`, the on-device model that advises on unclear
    /// duplicates. Loopback only: C2 goes to it (PRD §6.1).
    pub model_url: String,
    /// `crates/projects`, where the Journal lives since 2026-10-20 (`journal.rs`). Same
    /// variable and default as `crates/finance`.
    pub projects_url: String,
}

impl Config {
    pub fn load() -> Self {
        Self {
            database_path: sjel_config::env_var("SJEL_ENTITIES_DB_PATH")
                .ok()
                .filter(|path| !path.trim().is_empty())
                .map_or_else(database_path, |path| sjel_config::expand_tilde(&path)),
            port: resolve_port(None, None, 8097),
            places_url: sjel_config::env_var("SJEL_PLACES_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8093".to_string()),
            model_url: sjel_config::env_var("SJEL_LOCAL_MODEL_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8091".to_string()),
            projects_url: sjel_config::env_var("SJEL_PROJECTS_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8103".to_string()),
        }
    }
}
