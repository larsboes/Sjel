//! Shared config resolution for Axon's Rust capabilities. Public crate, Axon
//! doctrine: no personal value lives here — everything personal comes from the
//! private overlay at runtime via `SJEL_PERSONAL_ROOT`.
//!
//! Why a shared crate now: transit/config.rs once argued "duplicated rather than
//! forcing a shared crate for ~15 lines across two standalone crates" — right at
//! two copies. By five capabilities the repo held six copies of `expand_tilde`
//! and five shared-Postgres DSN builders in two *diverging* forms, one of which
//! (the `postgresql://user:password@…` URL form) comms had already documented as
//! an auth trap: the instance's real password is base64 and can contain `/`,
//! `+`, `=`, which URL userinfo silently mangles. The keyword/value form below
//! has no such escaping trap, and `postgres::Client::connect` accepts both
//! identically. One builder, the safe form, everywhere.

pub mod env;
pub use env::{deployment_value, env_var, env_var_os};

use std::path::PathBuf;

/// `~/foo` → `$HOME/foo`; absolute and relative paths pass through unchanged.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// The private overlay's root (`SJEL_PERSONAL_ROOT`, exported by
/// `tools/lib/paths.sh` / the shell), tilde-expanded. `None` when unset —
/// callers degrade to their dev fallback, never guess a location.
pub fn overlay_root() -> Option<PathBuf> {
    crate::env_var("SJEL_PERSONAL_ROOT")
        .ok()
        .map(|p| expand_tilde(&p))
}

/// `<overlay>/config/<name>` — where a capability's instance config lives.
pub fn overlay_config(name: &str) -> Option<PathBuf> {
    overlay_root().map(|r| r.join("config").join(name))
}

/// `<overlay>/data/<capability>` — where a capability persists its data.
pub fn overlay_data_dir(capability: &str) -> Option<PathBuf> {
    overlay_root().map(|r| r.join("data").join(capability))
}

/// Where the one shared SQLite file lives (PRD Q45, 2026-08-27).
///
/// `SJEL_DB_PATH` > `<overlay>/data/axon/axon.db` > a scratch file. Resolved the
/// same way the Postgres DSN was — env first, then the overlay — so moving a
/// deployment is still one variable.
///
/// There is one file for every capability, not one per capability. Cross-schema
/// joins were the reason the shared Postgres instance existed
/// (`capabilities/store/README.md`), and a file per capability would have
/// dropped them. So this takes no capability argument: there is nothing to vary.
///
/// The last resort is deliberately a scratch path rather than a plausible one.
/// The Postgres fallback named the real database and the demo overlay resolved
/// to it: the only thing standing between a demo seeding run and the live store
/// was that the real password was not the word `axon`. A fallback that looks
/// like production is worse than one that obviously is not.
/// Nothing is created here; `axon_store::pool_for` makes the directory when a
/// caller actually opens the file.
pub fn database_path() -> PathBuf {
    if let Some(explicit) = crate::env_var("SJEL_DB_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return expand_tilde(&explicit);
    }
    overlay_data_dir("axon")
        .unwrap_or_else(|| std::env::temp_dir().join("axon-no-overlay"))
        .join("axon.db")
}

/// The deployment's home timezone, from `<overlay>/config/deployment.env`.
///
/// A timezone is a fact about the deployment, not about one capability: calendar
/// and scouting both need it and both used to carry their own copy, which is two
/// values that agree until one of them is edited. This is the single declaration.
///
/// Plain `KEY=value`, the same shape `postgres.env` uses, because this crate is
/// deliberately dependency-free (see Cargo.toml) and a JSON source would change
/// every consumer's dependency resolution to read one string. `None` when the
/// file or key is absent — callers refuse to guess rather than defaulting.
pub fn deployment_home_timezone() -> Option<String> {
    let body = std::fs::read_to_string(overlay_config("deployment.env")?).ok()?;
    deployment_value(&body, "SJEL_HOME_TIMEZONE")
}

/// Resolution order for the home timezone, one implementation shared by every
/// capability that needs one.
///
/// A capability-level value still wins, so an existing config keeps working. What
/// it may not do is silently disagree: two different values are the drift this
/// contract exists to prevent, so that case is an error naming both sources rather
/// than a precedence rule nobody remembers. `Ok(None)` means neither is set, and
/// the caller emits its own refuse-to-guess error with its own domain reason.
pub fn resolve_home_timezone(
    capability_value: Option<&str>,
    capability_source: &str,
) -> Result<Option<String>, String> {
    let capability = capability_value.map(str::trim).filter(|v| !v.is_empty());
    let deployment = deployment_home_timezone();
    match (capability, deployment.as_deref()) {
        (Some(c), Some(d)) if c != d => Err(format!(
            "home timezone conflict: {capability_source} says {c:?}, \
             <overlay>/config/deployment.env says {d:?}. Remove the capability-level \
             value so the deployment declaration is the only one."
        )),
        (Some(c), _) => Ok(Some(c.to_string())),
        (None, Some(d)) => Ok(Some(d.to_string())),
        (None, None) => Ok(None),
    }
}

/// The runner's port contract, one implementation: `SJEL_PORT` (exported by
/// `tools/service-runner.sh` from the manifest plus any machine-local
/// `[capability.<name>]` override) always wins; a capability-specific escape
/// hatch (e.g. `TRANSIT_PORT`) applies when running outside the runner; then
/// the capability's own config file value; then its shipped default. A server
/// that skips this resolution binds its default while the proxy and health
/// poll target the override — the scouting/vaultwarden 8080 collision class.
pub fn resolve_port(fallback_env: Option<&str>, file_port: Option<u16>, default: u16) -> u16 {
    let parse = |v: String| v.parse::<u16>().ok();
    crate::env_var("SJEL_PORT")
        .ok()
        .and_then(parse)
        .or_else(|| {
            fallback_env
                .and_then(|k| std::env::var(k).ok())
                .and_then(parse)
        })
        .or(file_port)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One lock for every env-touching test: cargo runs tests as parallel threads
    /// of one process, and HOME / SJEL_PERSONAL_ROOT / SJEL_PORT are process-global.
    /// EnvGuard restores values, but only a lock stops two tests interleaving.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so a bare `set_var`/`remove_var` leaks into sibling tests.
    struct EnvGuard(Vec<(String, Option<String>)>);

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self(vec![(key.to_string(), previous)])
        }
        /// Clears the setting under its Sjel name and its pre-rename Axon name, so a value
        /// the operator's shell still exports under the old name cannot stand in for it.
        fn take(key: &'static str) -> Self {
            let names = std::iter::once(key.to_string()).chain(crate::env::legacy_name(key));
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
    fn expand_tilde_uses_home() {
        let _env = env_lock();
        let _home = EnvGuard::set("HOME", "/tmp/fake-home");
        assert_eq!(expand_tilde("~/foo"), PathBuf::from("/tmp/fake-home/foo"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn overlay_paths_derive_from_the_env_root() {
        let _env = env_lock();
        let _o = EnvGuard::set("SJEL_PERSONAL_ROOT", "/tmp/fake-overlay");
        assert_eq!(
            overlay_config("postgres.env").unwrap(),
            PathBuf::from("/tmp/fake-overlay/config/postgres.env")
        );
        assert_eq!(
            overlay_data_dir("scouting").unwrap(),
            PathBuf::from("/tmp/fake-overlay/data/scouting")
        );
    }

    /// One file for every capability, resolved env-first. The last case is the
    /// one that matters: with no overlay the path must be obviously scratch, so
    /// nobody mistakes it for the deployment's database.
    #[test]
    fn the_database_path_is_env_then_overlay_then_scratch() {
        let _env = env_lock();
        let _explicit = EnvGuard::set("SJEL_DB_PATH", "/tmp/somewhere/else.db");
        assert_eq!(database_path(), PathBuf::from("/tmp/somewhere/else.db"));

        let _blank = EnvGuard::set("SJEL_DB_PATH", "  ");
        let _o = EnvGuard::set("SJEL_PERSONAL_ROOT", "/tmp/fake-overlay");
        assert_eq!(
            database_path(),
            PathBuf::from("/tmp/fake-overlay/data/axon/axon.db")
        );

        let _none = EnvGuard::take("SJEL_PERSONAL_ROOT");
        let scratch = database_path();
        assert!(
            scratch.starts_with(std::env::temp_dir()),
            "no overlay must not resolve to a plausible location, got {}",
            scratch.display()
        );
    }

    #[test]
    fn overlay_absent_means_none_not_a_guess() {
        let _env = env_lock();
        let _o = EnvGuard::take("SJEL_PERSONAL_ROOT");
        assert!(overlay_root().is_none());
    }

    #[test]
    fn port_resolution_order_is_axon_port_then_fallback_then_file_then_default() {
        let _env = env_lock();
        let _a = EnvGuard::take("SJEL_PORT");
        let _f = EnvGuard::take("SJEL_CONFIG_TEST_PORT");
        assert_eq!(
            resolve_port(Some("SJEL_CONFIG_TEST_PORT"), Some(9000), 8000),
            9000
        );
        assert_eq!(
            resolve_port(Some("SJEL_CONFIG_TEST_PORT"), None, 8000),
            8000
        );
        let _f2 = EnvGuard::set("SJEL_CONFIG_TEST_PORT", "9100");
        assert_eq!(
            resolve_port(Some("SJEL_CONFIG_TEST_PORT"), Some(9000), 8000),
            9100
        );
        let _a2 = EnvGuard::set("SJEL_PORT", "9200");
        assert_eq!(
            resolve_port(Some("SJEL_CONFIG_TEST_PORT"), Some(9000), 8000),
            9200
        );
    }

    /// Writes a deployment.env into a throwaway overlay and points
    /// SJEL_PERSONAL_ROOT at it. Returns the guard so the caller holds it.
    fn with_deployment_env(body: Option<&str>) -> (EnvGuard, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "axon-config-tz-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let file = config.join("deployment.env");
        match body {
            Some(text) => std::fs::write(&file, text).unwrap(),
            None => {
                let _ = std::fs::remove_file(&file);
            }
        }
        let guard = EnvGuard::set("SJEL_PERSONAL_ROOT", root.to_str().unwrap());
        (guard, root)
    }

    #[test]
    fn the_deployment_declaration_is_used_when_a_capability_has_none() {
        let _l = env_lock();
        let (_g, root) = with_deployment_env(Some("SJEL_HOME_TIMEZONE=Europe/Berlin\n"));
        assert_eq!(deployment_home_timezone().as_deref(), Some("Europe/Berlin"));
        assert_eq!(
            resolve_home_timezone(None, "calendar.json")
                .unwrap()
                .as_deref(),
            Some("Europe/Berlin")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_capability_value_still_wins_so_existing_configs_keep_working() {
        let _l = env_lock();
        let (_g, root) = with_deployment_env(None);
        assert_eq!(deployment_home_timezone(), None);
        assert_eq!(
            resolve_home_timezone(Some("UTC"), "scouting.json")
                .unwrap()
                .as_deref(),
            Some("UTC")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_different_values_are_an_error_naming_both_sources() {
        let _l = env_lock();
        let (_g, root) = with_deployment_env(Some("SJEL_HOME_TIMEZONE=Europe/Berlin\n"));
        let error = resolve_home_timezone(Some("UTC"), "scouting.json").unwrap_err();
        assert!(error.contains("scouting.json"), "got: {error}");
        assert!(error.contains("deployment.env"), "got: {error}");
        // Agreeing is not a conflict — that is the state a migration passes through.
        assert_eq!(
            resolve_home_timezone(Some("Europe/Berlin"), "scouting.json")
                .unwrap()
                .as_deref(),
            Some("Europe/Berlin")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn neither_source_set_resolves_to_none_rather_than_a_default() {
        let _l = env_lock();
        let (_g, root) = with_deployment_env(None);
        assert_eq!(resolve_home_timezone(None, "calendar.json").unwrap(), None);
        // An empty or whitespace value is absent, not a zone named "".
        assert_eq!(
            resolve_home_timezone(Some("  "), "calendar.json").unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
