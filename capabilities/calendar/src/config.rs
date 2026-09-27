//! Public, zero-personal-data config for the calendar store and its Google
//! sync. Every personal value — the home timezone, which Google calendar,
//! where the credential file lives — comes from the private overlay at
//! runtime, never from this repo.
//!
//! Resolution for the store path:
//!   `sjel_config::database_path` -- `$SJEL_DB_PATH`, else
//!   `$SJEL_PERSONAL_ROOT/data/axon/axon.db`. It is a deployment fact, not a
//!   capability one, so `calendar.json` cannot move this capability off the
//!   shared file on its own.
//!
//! Resolution for everything else (a JSON file, mirroring comms/scouting):
//!   1. `$SJEL_CALENDAR_CONFIG` (explicit override, full path)
//!   2. `$SJEL_PERSONAL_ROOT/config/calendar.json` (the overlay)
//!   3. `capabilities/calendar/calendar.config.json` (local, gitignored)
//!
//! There is no file at all in the common case: Phases A–D need none, and
//! `Config::load` returns working defaults for everything except the two
//! values Phase E refuses to guess (see `GoogleConfig`).

use std::path::PathBuf;

use serde::Deserialize;

use sjel_config::{database_path, expand_tilde, overlay_config, resolve_port};

/// Where the Google credential and the calendar to sync are named.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GoogleConfig {
    /// `KEY=value` file holding GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET /
    /// GOOGLE_REFRESH_TOKEN. Default `$SJEL_PERSONAL_ROOT/config/calendar.env`.
    ///
    /// Pointing this at comms' `comms.env` is supported and is the cheap path:
    /// `capabilities/comms/auth/get-refresh-token.ts` already requests
    /// `calendar.events` alongside its Gmail scope, so that grant covers this
    /// capability too. Its own default is a separate file so the two can be
    /// revoked independently.
    pub env_path: Option<String>,
    /// Which Google calendar to sync. `primary` is the account's own calendar;
    /// a secondary calendar is addressed by its long `…@group.calendar.google.com`
    /// id. No default on purpose — a wrong guess would import a stranger's
    /// calendar or export into one.
    pub calendar_id: Option<String>,
    /// Import window, relative to today. Past events are of little use to an
    /// availability layer, so the default reaches back only far enough to
    /// catch an event that moved earlier.
    pub import_days_back: i64,
    pub import_days_ahead: i64,
    /// Hard bound on how many events one import will page through.
    pub max_events: usize,
}

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            env_path: None,
            calendar_id: None,
            import_days_back: 7,
            import_days_ahead: 120,
            max_events: 1_000,
        }
    }
}

impl GoogleConfig {
    /// The credential file's resolved path. Falls back to the overlay's
    /// `calendar.env`, and to a bare relative name when there is no overlay —
    /// which then fails loudly at read time naming that path.
    pub fn env_path(&self) -> PathBuf {
        self.env_path
            .as_deref()
            .map(expand_tilde)
            .or_else(|| overlay_config("calendar.env"))
            .unwrap_or_else(|| PathBuf::from("calendar.env"))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    port: Option<u16>,
    home_timezone: Option<String>,
    home_city: Option<String>,
    trips_base_url: Option<String>,
    google: Option<GoogleConfig>,
    #[serde(default)]
    markdown_sources: Vec<crate::markdown_import::MarkdownSource>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The one shared SQLite file, under the table prefix `calendar` (PRD Q45).
    /// A file per capability would drop the cross-capability joins the shared
    /// instance existed for, so a `database_url` left in `calendar.json` is ignored.
    pub database_path: PathBuf,
    pub port: u16,
    /// The operator's home timezone. Everything stored is naive wall time in
    /// this zone (README § Time model), so Phase E cannot convert a Google
    /// offset without it.
    ///
    /// `None` by default and never guessed: a wrong zone writes every imported
    /// event an hour or more off, silently and plausibly. Import and export
    /// both refuse until it is set. Phases A–D never read it.
    pub home_timezone: Option<String>,
    /// Where the operator lives, so Phase D can tell a trip from an evening
    /// out. Optional on purpose: absent means every place clusters, including
    /// this one, which is wrong in a way you can see rather than a filter that
    /// quietly eats things.
    pub home_city: Option<String>,
    /// Where trips answers. Calendar posts a plan to its public API and never
    /// reaches into its store, so this is a URL and not a database handle.
    pub trips_base_url: String,
    pub google: GoogleConfig,
    /// Declared markdown event sources. Empty by default and empty in the
    /// public template: a note store is something an operator points calendar
    /// at, never something it goes looking for. `~/` is expanded here so the
    /// importer only ever sees a real path.
    pub markdown_sources: Vec<crate::markdown_import::MarkdownSource>,
}

impl Config {
    /// One declared markdown source by id, enabled ones only. A disabled source
    /// answers the same as an unknown one: the operator turned it off, and a
    /// scan that ran anyway would be ignoring that.
    pub fn markdown_source(&self, id: &str) -> Option<&crate::markdown_import::MarkdownSource> {
        self.markdown_sources
            .iter()
            .find(|source| source.enabled && source.id == id)
    }
}

/// The JSON config file this capability would read. Public so a "you have not
/// configured X" error can name the exact path the operator has to create,
/// rather than describing one.
pub fn config_path() -> PathBuf {
    if let Ok(path) = sjel_config::env_var("SJEL_CALENDAR_CONFIG") {
        return expand_tilde(&path);
    }
    if let Some(path) = overlay_config("calendar.json") {
        return path;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("calendar.config.json")
}

/// Read one config file. Absent, unreadable and unparseable all answer with
/// the defaults — a capability that needs no file is the common case, and a
/// typo in one is reported on stderr rather than by refusing to start.
///
/// Takes the path rather than resolving it, so a test can name a file without
/// writing `$SJEL_CALENDAR_CONFIG`. See `Config::from_file`.
fn read_file_config(path: &std::path::Path) -> FileConfig {
    if !path.is_file() {
        return FileConfig::default();
    }
    match std::fs::read_to_string(path) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_else(|error| {
            eprintln!("warning: could not parse {path:?}: {error} — using defaults");
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

impl Config {
    pub fn load() -> Self {
        Self::from_file(read_file_config(&config_path()))
    }

    /// The resolution rules with the file already in hand.
    ///
    /// Split from `load` so a test can supply the file directly instead of
    /// pointing `$SJEL_CALENDAR_CONFIG` at one. The environment is process-wide
    /// and Rust runs a crate's tests as threads of one process, so two tests
    /// that both resolved through that variable read each other's writes: one
    /// removed it while the other held it set, and whichever was inside
    /// `config_path()` at that moment got the wrong file. That is the second
    /// half of PRD silent failure #3 — "a test run is not isolated either".
    ///
    /// The window is microseconds wide and has not been observed to open on its
    /// own: 400 consecutive runs of these tests before the fix failed 0 times,
    /// and no CI run, register row or commit records it firing. What is
    /// measured here is the mechanism, not a symptom. With a 200 ms sleep in
    /// front of `config_path()` and nothing else changed,
    /// `the_home_timezone_has_no_default` failed on every run, resolving
    /// `Europe/Berlin` out of the other test's file.
    ///
    /// `--test-threads=1` would have hidden it instead: the tests would then
    /// pass while the defect they race on stayed exactly where it was.
    fn from_file(file: FileConfig) -> Self {
        Self {
            database_path: database_path(),
            port: resolve_port(Some("SJEL_CALENDAR_PORT"), file.port, 8087),
            // Deployment declaration first, capability override second — one
            // implementation in sjel_config so calendar and scouting cannot drift.
            // A conflict resolves to None deliberately: the caller's own
            // refuse-to-guess error then fires, which is the fail-closed direction
            // for a value that silently shifts every stored wall time when wrong.
            home_timezone: sjel_config::resolve_home_timezone(
                file.home_timezone.as_deref(),
                "calendar.json",
            )
            .unwrap_or_else(|conflict| {
                eprintln!("warning: {conflict}");
                None
            }),
            home_city: file.home_city.filter(|city| !city.trim().is_empty()),
            trips_base_url: file
                .trips_base_url
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:8086".to_string()),
            google: file.google.unwrap_or_default(),
            markdown_sources: file
                .markdown_sources
                .into_iter()
                .map(|mut source| {
                    source.path = expand_tilde(&source.path).to_string_lossy().into_owned();
                    source
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes every test in this module whose answer depends on the process
    /// environment — the ones that write it AND the ones that only read it.
    ///
    /// Restoring a variable on drop is not enough on its own: `set_var` and
    /// `remove_var` are process-wide and a crate's tests are threads of one
    /// process, so a test resolving a path while another holds a variable set
    /// reads the other test's value. Only one test may be inside the
    /// environment at a time, and this is what says so.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The environment, changed for the length of one test and restored after
    /// it, with [`ENV_LOCK`] held throughout.
    ///
    /// One scope per test — it takes the lock in `new` and holds it until it
    /// drops, so a second scope inside the same test would wait for itself.
    /// Poisoning is stepped over deliberately: a test that panicked while
    /// holding the lock has already been reported, and refusing the lock
    /// afterwards would turn one failure into every later one.
    struct EnvScope {
        restore: Vec<(String, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvScope {
        fn new() -> Self {
            Self {
                restore: Vec::new(),
                _lock: ENV_LOCK.lock().unwrap_or_else(|held| held.into_inner()),
            }
        }

        fn set(mut self, key: &'static str, value: &str) -> Self {
            self.restore
                .push((key.to_string(), std::env::var(key).ok()));
            std::env::set_var(key, value);
            self
        }

        /// Clears the Sjel name and its pre-rename Axon name, so a value the operator's shell
        /// still exports under the old name cannot stand in for the setting under test.
        fn unset(mut self, key: &'static str) -> Self {
            for name in std::iter::once(key.to_string()).chain(sjel_config::env::legacy_name(key)) {
                self.restore.push((name.clone(), std::env::var(&name).ok()));
                std::env::remove_var(&name);
            }
            self
        }
    }

    impl Drop for EnvScope {
        fn drop(&mut self) {
            // Reverse order: a key set twice in one scope is restored to what
            // it held before the scope, not to what it held mid-scope.
            for (key, previous) in self.restore.drain(..).rev() {
                match previous {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    #[test]
    fn google_defaults_name_nothing_personal() {
        let google = GoogleConfig::default();
        assert!(google.calendar_id.is_none(), "no calendar is guessed");
        assert!(google.env_path.is_none());
        assert_eq!(google.import_days_ahead, 120);
        assert_eq!(google.import_days_back, 7);
    }

    #[test]
    fn a_configured_env_path_wins_over_the_overlay_default() {
        let google = GoogleConfig {
            env_path: Some("/etc/axon/creds.env".into()),
            ..Default::default()
        };
        assert_eq!(google.env_path(), PathBuf::from("/etc/axon/creds.env"));
    }

    /// No file: `from_file` is the resolution `load` runs once the file is
    /// read, so the question is asked without `$SJEL_CALENDAR_CONFIG` and
    /// therefore without the race that made this flaky.
    ///
    /// The overlay still has to be moved out of the way, and that is not
    /// cosmetic: `sjel_config::deployment_home_timezone` reads
    /// `<overlay>/config/deployment.env`, and a deployment that declares a zone
    /// there answers this question with it. Writing the environment means
    /// taking the lock.
    #[test]
    fn the_home_timezone_has_no_default() {
        let _env = EnvScope::new().unset("SJEL_PERSONAL_ROOT");
        let config = Config::from_file(FileConfig::default());
        assert!(
            config.home_timezone.is_none(),
            "guessing a zone writes every import silently off by an hour"
        );
        assert_eq!(config.port, 8087);
    }

    #[test]
    fn a_file_supplies_the_personal_values() {
        let dir = std::env::temp_dir().join(format!(
            "calendar-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calendar.json");
        std::fs::write(
            &path,
            r#"{"home_timezone":"Europe/Berlin","google":{"calendar_id":"primary","import_days_ahead":30}}"#,
        )
        .unwrap();

        // Named, not exported. This used to point `$SJEL_CALENDAR_CONFIG` at
        // the file, which is what `the_home_timezone_has_no_default` was
        // reading when it failed. The overlay is still moved aside: a
        // deployment that declares a different zone in `deployment.env` makes
        // the capability value a CONFLICT rather than a winner, and the file's
        // value would vanish for a reason that has nothing to do with parsing.
        let _env = EnvScope::new().unset("SJEL_PERSONAL_ROOT");
        let config = Config::from_file(read_file_config(&path));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(config.home_timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(config.google.calendar_id.as_deref(), Some("primary"));
        assert_eq!(config.google.import_days_ahead, 30);
        assert_eq!(
            config.google.import_days_back, 7,
            "an unspecified field keeps its default"
        );
    }

    /// The resolution order the module doc states, which the other two tests
    /// used to cover as a side effect of pointing at their fixtures. It is now
    /// the only test here that writes the environment, and it holds `ENV_LOCK`
    /// while it does.
    #[test]
    fn the_config_path_prefers_the_explicit_override_to_the_overlay() {
        let named = std::env::temp_dir().join("calendar-explicit.json");
        let scope = EnvScope::new()
            .set("SJEL_CALENDAR_CONFIG", &named.to_string_lossy())
            .set("SJEL_PERSONAL_ROOT", "/nonexistent/overlay");
        assert_eq!(
            config_path(),
            named,
            "an explicitly named file outranks the overlay"
        );

        let scope = scope.unset("SJEL_CALENDAR_CONFIG");
        assert_eq!(
            config_path(),
            PathBuf::from("/nonexistent/overlay/config/calendar.json"),
            "with no override the overlay names the file"
        );
        drop(scope);
    }

    /// A file that is not there, and a file that is not JSON, both answer with
    /// the defaults rather than a panic — the path `read_file_config` takes on
    /// every deployment that has never written one.
    #[test]
    fn an_absent_or_broken_file_reads_as_defaults() {
        let missing = std::env::temp_dir().join("calendar-does-not-exist.json");
        let _ = std::fs::remove_file(&missing);
        assert!(read_file_config(&missing).home_timezone.is_none());

        let broken = std::env::temp_dir().join(format!(
            "calendar-broken-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&broken, "{ not json").unwrap();
        assert!(read_file_config(&broken).home_timezone.is_none());
        let _ = std::fs::remove_file(&broken);
    }
}
