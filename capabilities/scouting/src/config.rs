//! Axon doctrine: this crate is public. No personal value (interest-profile
//! content, events-directory path, DB location) lives here -- those come
//! from the private overlay at runtime. Mirrors
//! `capabilities/printing/printctl.py`'s `_cfg_path()`/`load_cfg()` exactly.
//! Config is resolved in order:
//!   1. `$SJEL_SCOUTING_CONFIG` (explicit override, full path to a JSON file)
//!   2. `$SJEL_PERSONAL_ROOT/config/scouting.json` (the overlay; exported by
//!      `tools/lib/paths.sh` / `~/.zshrc`)
//!   3. `capabilities/scouting/scouting.config.json` next to this crate's
//!      source (local, gitignored -- dev fallback)
//!
//! See `scouting.config.example.json` for the shape.
//!
//! CLI args always win over whatever this module resolves -- `Config::load()`
//! only supplies defaults; `main.rs`/`server_main.rs` override individual
//! fields from `--flag` values where a flag exists for that field.

use axon_config::{database_path, expand_tilde, resolve_port};
use serde::Deserialize;
use std::path::PathBuf;

use crate::sources::SourceEntry;

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    interest_profile_dir: Option<String>,
    events_dir: Option<String>,
    opp_embeddings_path: Option<String>,
    port: Option<u16>,
    calendar_base_url: Option<String>,
    home_timezone: Option<String>,
    #[serde(default)]
    geo: Option<GeoPolicy>,

    /// Declared opportunity sources (supersedes events_dir/interest_profile_dir).
    /// Each entry declares an adapter type, location, and what data it provides.
    #[serde(default)]
    sources: Vec<SourceEntry>,
}

/// Where the operator can actually get to.
///
/// Country knowledge lives here rather than in Rust: the sources disagree on
/// spelling (Luma says `"Germany"`, meetup says `"de"`), so the tokens listed
/// here are matched case-insensitively against whatever the source stored.
/// Normalising to ISO-2 at ingest would be better and is a separate change --
/// the field is called `country_code` and holds names, which is its own bug.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GeoPolicy {
    /// Private home coordinate. A distance is used only when both values and
    /// an explicit local radius are present and valid.
    pub home_latitude: Option<f64>,
    pub home_longitude: Option<f64>,
    pub local_radius_km: Option<f64>,
    /// Country tokens treated as local when either coordinate pair is absent.
    /// Empty means country evidence cannot decide either way.
    #[serde(default)]
    pub allow_countries: Vec<String>,
    /// Explicit compatibility override for unlocated events. The safe default
    /// is false: without evidence the route is `unresolved`, not guessed local.
    #[serde(default)]
    pub allow_unknown: bool,
    /// IANA timezone prefixes to accept when no country was recorded. Luma
    /// leaves `geo_address_info` null for some events but still stamps
    /// `America/New_York`, which answers the only question this policy asks.
    /// Checked before `allow_unknown` gets to be generous.
    #[serde(default)]
    pub allow_timezone_prefixes: Vec<String>,
}

impl GeoPolicy {
    pub fn country_is_local(&self, country: &str) -> Option<bool> {
        (!self.allow_countries.is_empty()).then(|| {
            self.allow_countries
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(country.trim()))
        })
    }

    pub fn timezone_is_local(&self, timezone: &str) -> Option<bool> {
        (!self.allow_timezone_prefixes.is_empty()).then(|| {
            self.allow_timezone_prefixes
                .iter()
                .any(|prefix| timezone.trim().starts_with(prefix.as_str()))
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Directory of markdown interest-profile files (was hardcoded
    /// `{vault_root}/TELOS/Focus`). Created if missing so the tool runs with
    /// zero config against an empty profile (score.rs degrades gracefully to
    /// "uncategorized" when no profiles are found).
    pub interest_profile_dir: PathBuf,
    /// Directory of existing markdown event notes for the vault-linker to
    /// cross-reference against (was hardcoded `{vault_root}/Atlas/Events`).
    /// `None` silently skips vault-linking -- this already works, matching
    /// `pipeline::run`'s `Option<&Path>` parameter.
    ///
    /// Superseded by `sources` entries with `profiles_glob`/`events_glob`
    /// when the `sources` array is non-empty. Kept for backwards compat
    /// and zero-config single-source runs.
    pub events_dir: Option<PathBuf>,
    /// The one shared SQLite file this capability's tables live in, under the
    /// prefix `scouting` (PRD Q45). Resolved by `axon_config::database_path`:
    /// `SJEL_DB_PATH`, else `<overlay>/data/axon/axon.db`. It is a deployment
    /// fact rather than a capability one, so a `database_url` left in
    /// `scouting.json` is ignored -- a file per capability would drop the
    /// cross-capability correlation with `transit` that Phase 2 exists for.
    pub database_path: PathBuf,
    /// Optional pre-computed opportunity-embeddings JSON path. Left CLI-arg-only
    /// (`--opp-embeddings`) per the original design -- hash-fallback embedding
    /// already works with zero config, so this isn't worth a second config knob.
    pub opp_embeddings_path: Option<PathBuf>,
    /// HTTP port for `scout-server` (was the `SCOUTING_PORT` env var).
    pub port: u16,
    /// Base URL of `capabilities/calendar` for the Luma → calendar promotion
    /// (`calendar_promote`). Loopback by default and expected to stay that
    /// way — calendar binds through `libs/axon-server`'s `serve_local`.
    pub calendar_base_url: String,
    /// The operator's home timezone, used to turn Luma's UTC instants into
    /// the naive local wall time calendar stores. Intentionally has **no
    /// default**: a wrong-by-an-hour entry is worse than a refused
    /// promotion, and a timezone is a personal value that belongs in the
    /// overlay, not in this public crate. See `localtime.rs`.
    pub home_timezone: Option<String>,
    /// Inspectable routing policy for physical events. Absent means their
    /// route is unresolved; it never means that they are dropped.
    pub geo: Option<GeoPolicy>,
    /// Declared opportunity sources (resolved manifests from `sources[]` in config).
    /// When non-empty, supersedes `events_dir`/`interest_profile_dir`.
    /// Populated by `Config::load()`; empty = legacy single-source mode.
    pub sources: Vec<crate::sources::SourceManifest>,
}

fn config_path() -> PathBuf {
    if let Ok(p) = axon_config::env_var("SJEL_SCOUTING_CONFIG") {
        return expand_tilde(&p);
    }
    if let Ok(overlay) = axon_config::env_var("SJEL_PERSONAL_ROOT") {
        return expand_tilde(&overlay).join("config").join("scouting.json");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scouting.config.json")
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

        let interest_profile_dir = file
            .interest_profile_dir
            .map(|p| expand_tilde(&p))
            .unwrap_or_else(|| {
                axon_config::overlay_data_dir("scouting")
                    .map(|d| d.join("interest-profile"))
                    .unwrap_or_else(|| PathBuf::from("data/interest-profile"))
            });
        std::fs::create_dir_all(&interest_profile_dir).ok();

        let events_dir = file
            .events_dir
            .map(|p| expand_tilde(&p))
            .filter(|p| p.exists());

        let opp_embeddings_path = file.opp_embeddings_path.map(|p| expand_tilde(&p));
        // Port contract lives in axon_config::resolve_port. The default is 8084,
        // not 8080: vaultwarden's manifest publishes 8080 on the host, and two
        // capabilities shipping the same default port is a collision waiting on
        // whoever starts both — which is exactly what happened the first time
        // the runner brought the whole enabled set up.
        let port = resolve_port(None, file.port, 8084);

        let sources: Vec<crate::sources::SourceManifest> =
            file.sources.iter().map(|s| s.resolve()).collect();

        Self {
            interest_profile_dir,
            events_dir,
            database_path: database_path(),
            opp_embeddings_path,
            port,
            calendar_base_url: file
                .calendar_base_url
                .unwrap_or_else(|| crate::calendar_promote::DEFAULT_CALENDAR_BASE_URL.to_string()),
            // Deployment declaration first, capability override second — one
            // implementation in axon_config so scouting and calendar cannot drift.
            // A conflict resolves to None deliberately: `--promote-calendar` then
            // refuses for its own reason rather than promoting at a guessed hour.
            home_timezone: axon_config::resolve_home_timezone(
                file.home_timezone.as_deref(),
                "scouting.json",
            )
            .unwrap_or_else(|conflict| {
                eprintln!("warning: {conflict}");
                None
            }),
            geo: file.geo,
            sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GeoPolicy;

    fn policy() -> GeoPolicy {
        GeoPolicy {
            home_latitude: None,
            home_longitude: None,
            local_radius_km: None,
            allow_countries: vec!["Germany".into(), "de".into(), "Netherlands".into()],
            allow_unknown: true,
            allow_timezone_prefixes: vec!["Europe/".into()],
        }
    }

    #[test]
    fn a_country_the_sources_spell_differently_still_matches() {
        // Luma writes "Germany", meetup writes "de", and neither is wrong.
        assert_eq!(policy().country_is_local("Germany"), Some(true));
        assert_eq!(policy().country_is_local("de"), Some(true));
        assert_eq!(policy().country_is_local("DE"), Some(true));
    }

    #[test]
    fn a_configured_non_local_country_is_a_negative_signal() {
        assert_eq!(policy().country_is_local("Canada"), Some(false));
    }

    /// The Atlanta case: Luma left geo_address_info null, so the country is
    /// missing, but the event still carries America/New_York.
    #[test]
    fn a_timezone_answers_when_the_country_is_missing() {
        assert_eq!(policy().timezone_is_local("America/New_York"), Some(false));
        assert_eq!(policy().timezone_is_local("Europe/Berlin"), Some(true));
    }

    #[test]
    fn an_empty_policy_has_no_country_or_timezone_answer() {
        let off = GeoPolicy {
            home_latitude: None,
            home_longitude: None,
            local_radius_km: None,
            allow_countries: vec![],
            allow_unknown: false,
            allow_timezone_prefixes: vec![],
        };
        assert_eq!(off.country_is_local("Canada"), None);
        assert_eq!(off.timezone_is_local("America/New_York"), None);
    }

    #[test]
    fn geo_policy_defaults_unknown_events_to_unresolved() {
        let policy: GeoPolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy.home_latitude, None);
        assert_eq!(policy.home_longitude, None);
        assert_eq!(policy.local_radius_km, None);
        assert!(!policy.allow_unknown);
    }

    use super::*;

    /// Clears env vars for the duration of one test and restores them on drop.
    ///
    /// Rust runs a crate's tests as threads of ONE process, so `remove_var` here
    /// is not local to this test: unrestored, it left every later store test
    /// resolving a different database file from the one they had just written to.
    ///
    /// That was the first half. The second half is that restoring correctly is not
    /// enough while two tests hold guards at the same time — whichever finishes
    /// first puts `SJEL_PERSONAL_ROOT` back under the other, mid-assertion, and the
    /// other fails on a value it never set. Measured 2026-08-31: intermittent, and
    /// it passes eight runs out of eight in isolation, which is what made it a flake
    /// rather than a bug anyone chased.
    ///
    /// So the guard takes a process-wide lock and every key at once. One guard per
    /// test, by construction: there is no way to take a second and deadlock against
    /// yourself, because the only constructor takes a list.
    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    /// A poisoned lock is a test that panicked while holding it. Its env is already
    /// restored by the guard's own drop, so recovering is correct and refusing would
    /// turn one failure into every later test failing for a different reason.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    impl EnvGuard {
        fn take(keys: &[&'static str]) -> Self {
            let lock = env_lock();
            // The pre-rename Axon name is cleared too, so a value the operator's shell still
            // exports under it cannot stand in for the setting under test.
            let saved = keys
                .iter()
                .flat_map(|key| std::iter::once(key.to_string()).chain(axon_config::env::legacy_name(key)))
                .map(|key| {
                    let previous = std::env::var(&key).ok();
                    std::env::remove_var(&key);
                    (key, previous)
                })
                .collect();
            Self { saved, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in self.saved.drain(..) {
                match previous {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    #[test]
    fn load_with_no_env_and_no_file_uses_defaults() {
        // Clear the overlay/override env vars so this test exercises the
        // "zero config" path deterministically regardless of the host's env.
        let _env = EnvGuard::take(&["SJEL_SCOUTING_CONFIG", "SJEL_PERSONAL_ROOT", "SJEL_PORT"]);
        let cfg = Config::load();
        assert_eq!(cfg.port, 8084);
        assert!(cfg.events_dir.is_none());
    }

    /// The store path is a deployment fact, not a capability one. A `database_url`
    /// left in `scouting.json` must not move this capability off the shared file --
    /// the cross-capability correlation with `transit` is why it is shared.
    #[test]
    fn the_store_path_comes_from_the_deployment_not_from_scouting_json() {
        let _env = EnvGuard::take(&["SJEL_SCOUTING_CONFIG", "SJEL_PERSONAL_ROOT"]);
        assert_eq!(Config::load().database_path, axon_config::database_path());
    }
}
