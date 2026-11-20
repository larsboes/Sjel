//! Place owner path and service port.

use std::path::PathBuf;

use sjel_config::{database_path, resolve_port};

pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        // 8093 mirrors service.toml here; SJEL_PORT (the runner's contract) wins.
        // Not 8091 (services/foundation-models/service.toml owns it) and not
        // 8092 (the private overlay's interior capability owns it — a repo-only
        // port sweep misses overlay service.tomls) — the scouting/vaultwarden
        // 8080 collision class, libs/sjel-config.
        let port = resolve_port(None, None, 8093);
        Self {
            database_path: sjel_config::env_var("SJEL_PLACES_DB_PATH")
                .ok()
                .filter(|path| !path.is_empty())
                .map_or_else(owner_database_path, |path| sjel_config::expand_tilde(&path)),
            port,
        }
    }
}

fn owner_database_path() -> PathBuf {
    #[cfg(feature = "turso-wal")]
    if let Some(path) = sjel_config::overlay_data_dir("places-store")
        .map(|directory| directory.join("places.db"))
        .filter(|path| path.is_file())
    {
        return path;
    }
    database_path()
}
