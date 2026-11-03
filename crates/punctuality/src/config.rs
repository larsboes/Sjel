//! Where the database is and where the raw cache lives. Same resolution order as
//! `scouting::config` and `transit::config`, deliberately: a fourth capability inventing
//! a fourth way to find the shared store would be a fourth thing to fix when it moves.

use sjel_config::{database_path, expand_tilde, overlay_data_dir};
use std::path::PathBuf;

pub struct Config {
    /// The one shared SQLite file, under the table prefix `punctuality` (PRD Q45).
    /// Resolved by `sjel_config::database_path`: `SJEL_DB_PATH`, else
    /// `<overlay>/data/axon/axon.db`. Not a per-capability setting any more — a file
    /// per capability would drop the cross-capability joins the shared instance
    /// existed for, so `SJEL_PUNCTUALITY_DATABASE_URL` is now ignored.
    pub database_path: PathBuf,
    /// Where downloaded monthly parquet lands. A cache, not state: everything in it is
    /// re-downloadable, so it is deliberately outside the backup set.
    pub raw_dir: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let raw_dir = sjel_config::env_var("SJEL_PUNCTUALITY_RAW_DIR")
            .ok()
            .map(|p| expand_tilde(&p))
            .or_else(|| overlay_data_dir("punctuality").map(|d| d.join("raw")))
            .unwrap_or_else(|| PathBuf::from("data/punctuality/raw"));

        Self {
            database_path: database_path(),
            raw_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store path is a deployment fact, not a capability one. A capability that
    /// resolved its own would be the one that quietly stopped sharing the file.
    #[test]
    fn the_store_path_comes_from_the_deployment() {
        assert_eq!(Config::load().database_path, database_path());
    }
}
