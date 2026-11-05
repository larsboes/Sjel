use std::path::PathBuf;

pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load() -> Self {
        Self {
            database_path: sjel_config::database_path(),
            port: sjel_config::resolve_port(None, None, operator_profile_api::DEFAULT_OPERATOR_PROFILE_PORT),
        }
    }
}
