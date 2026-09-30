//! Axon doctrine: this crate is public. No personal value (a home/default
//! station pair) lives here -- that comes from the private overlay at
//! runtime, if set at all. Mirrors `capabilities/scouting/src/config.rs`'s
//! `Config::load()` resolution exactly:
//!   1. `$SJEL_TRANSIT_CONFIG` (explicit override, full path to a JSON file)
//!   2. `$SJEL_PERSONAL_ROOT/config/transit.json` (the overlay; exported by
//!      `tools/lib/paths.sh` / `~/.zshrc`)
//!   3. `capabilities/transit/transit.config.json` next to this crate's
//!      source (local, gitignored -- dev fallback)
//!
//! See `transit.config.example.json` for the shape.
//!
//! CLI args always win over whatever this module resolves -- `Config::load()`
//! only supplies defaults; `main.rs` overrides individual fields from
//! `--flag` values where a flag exists for that field.
//!
//! Unlike scouting, there is deliberately no baked-in station-pair default
//! anywhere (the original had `8000044`/`8098160` -- real Bonn/Berlin EVA
//! codes -- hardcoded as CLI argument defaults). `default_from_eva`/
//! `default_to_eva`/`default_time` exist purely as an opt-in convenience: set
//! your own home route in the overlay if you want `transit search`/`split`
//! runnable with no flags; leave them unset and the CLI requires `--from`/
//! `--to` explicitly, erroring with a clear message rather than silently
//! defaulting to someone else's stations.

use serde::Deserialize;
use sjel_config::{database_path, expand_tilde};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    default_from_eva: Option<String>,
    default_to_eva: Option<String>,
    default_time: Option<String>,
    document_backend: Option<String>,
    xberg_bin: Option<String>,
    ocr_language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub default_from_eva: Option<String>,
    pub default_to_eva: Option<String>,
    pub default_time: Option<String>,
    /// The one shared SQLite file `store::TransitStore` opens, under the table
    /// prefix `transit` (PRD Q45). Resolved by `sjel_config::database_path`:
    /// `SJEL_DB_PATH`, else `<overlay>/data/axon/axon.db`. Not a per-capability
    /// setting any more -- a file per capability would drop the cross-capability
    /// joins the shared instance existed for, so a `database_url` in
    /// `transit.json` is now ignored.
    pub database_path: PathBuf,
    /// Which reader turns a ticket file into text. `builtin` is the original
    /// pdf_extract/mailparse path; `xberg` shells out to the xberg CLI, which
    /// reads layout and can emit a table as a Markdown table.
    ///
    /// A setting rather than a hardcoded choice because the two are not
    /// interchangeable: builtin needs no external binary and cannot read an
    /// image, xberg reads images and tables and needs to be installed. Which
    /// one is right depends on the machine, which is what the overlay is for.
    pub document_backend: DocumentBackend,
    /// Where the xberg binary lives. `cargo install` puts it on PATH, so the
    /// default is the bare name; an overlay can point at an explicit path.
    pub xberg_bin: String,
    /// Tesseract's ISO 639-3 code. Defaults to German because that is what a DB
    /// confirmation is written in, and the default English model reads its
    /// umlauts as noise.
    pub ocr_language: String,
}

/// The readers a ticket file can go through.
///
/// These names are also what `document::read` reports back as
/// `sjel_extraction::Extraction::producer`, so a reply and this setting cannot
/// disagree about which reader ran. Neither is registered in that crate's
/// class registry: `for_class` answers "who reads a PDF", and this answers
/// "which reader did the operator install", which is a different question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentBackend {
    /// pdf_extract for PDFs, mailparse for .eml, raw bytes otherwise. No
    /// external dependency, no images, no layout.
    #[default]
    Builtin,
    /// The xberg CLI. Reads 100+ formats including images, and can preserve a
    /// table as a Markdown table rather than flattening it into a line.
    Xberg,
}

impl DocumentBackend {
    /// Unknown names fall back to `Builtin` loudly rather than failing to start.
    /// A typo in the overlay should cost layout-aware parsing, not the service.
    fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("builtin") => Self::Builtin,
            Some("xberg") => Self::Xberg,
            Some(other) => {
                eprintln!(
                    "warning: unknown document_backend {other:?} -- using builtin. \
                     Valid values: builtin, xberg"
                );
                Self::Builtin
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Xberg => "xberg",
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(p) = sjel_config::env_var("SJEL_TRANSIT_CONFIG") {
        return expand_tilde(&p);
    }
    if let Ok(overlay) = sjel_config::env_var("SJEL_PERSONAL_ROOT") {
        return expand_tilde(&overlay).join("config").join("transit.json");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transit.config.json")
}

fn load_file_config() -> FileConfig {
    let path = config_path();
    if !path.is_file() {
        return FileConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_else(|e| {
            eprintln!("warning: could not parse {path:?}: {e} -- using defaults");
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

impl Config {
    pub fn load() -> Self {
        let file = load_file_config();
        Self {
            default_from_eva: file.default_from_eva,
            default_to_eva: file.default_to_eva,
            default_time: file.default_time,
            database_path: database_path(),
            document_backend: DocumentBackend::parse(file.document_backend.as_deref()),
            xberg_bin: file.xberg_bin.unwrap_or_else(|| "xberg".into()),
            ocr_language: file.ocr_language.unwrap_or_else(|| "deu".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so `remove_var` here is not local to this test: unrestored, it
    /// left every later store test resolving a different database file from the
    /// one they had just written to.
    struct EnvGuard(Vec<(String, Option<String>)>);

    impl EnvGuard {
        /// Clears the setting, so a value the operator's shell still exports cannot stand in
        /// for it.
        fn take(key: &'static str) -> Self {
            let names = std::iter::once(key.to_string());
            Self(
                names
                    .map(|name| {
                        let previous = std::env::var(&name).ok();
                        std::env::remove_var(&name);
                        (name, previous)
                    })
                    .collect(),
            )
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, previous) in self.0.drain(..).rev() {
                match previous {
                    Some(value) => std::env::set_var(&name, value),
                    None => std::env::remove_var(&name),
                }
            }
        }
    }

    #[test]
    fn load_with_no_env_and_no_file_uses_defaults() {
        let _config = EnvGuard::take("SJEL_TRANSIT_CONFIG");
        let _overlay = EnvGuard::take("SJEL_PERSONAL_ROOT");
        let cfg = Config::load();
        assert!(cfg.default_from_eva.is_none());
        assert!(cfg.default_to_eva.is_none());
    }

    /// A typo in the overlay must cost layout-aware parsing, not the service.
    #[test]
    fn an_unknown_document_backend_falls_back_to_builtin() {
        assert_eq!(DocumentBackend::parse(None), DocumentBackend::Builtin);
        assert_eq!(DocumentBackend::parse(Some("")), DocumentBackend::Builtin);
        assert_eq!(
            DocumentBackend::parse(Some("builtin")),
            DocumentBackend::Builtin
        );
        assert_eq!(
            DocumentBackend::parse(Some(" XBerg ")),
            DocumentBackend::Xberg
        );
        assert_eq!(
            DocumentBackend::parse(Some("dolphin")),
            DocumentBackend::Builtin
        );
    }

    /// The store path is a deployment fact now, not a capability one: a
    /// `database_url` left in an overlay's transit.json must not move this
    /// capability off the shared file on its own.
    #[test]
    fn the_store_path_comes_from_the_deployment_not_from_transit_json() {
        let _config = EnvGuard::take("SJEL_TRANSIT_CONFIG");
        let _overlay = EnvGuard::take("SJEL_PERSONAL_ROOT");
        assert_eq!(Config::load().database_path, sjel_config::database_path());
    }
}
