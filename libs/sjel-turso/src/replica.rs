//! Embedded replicas: reads from a local file that is a copy of a remote primary.
//!
//! A replica stores the primary's write-ahead log locally, so reads never leave the machine and
//! a read costs a local page lookup. Writes are not local: LibSQL forwards them to the primary
//! and the next pull brings them back, which is why a replica is a read-latency feature and not
//! an offline-write one. `new_synced_database` is the offline-write shape, and it is not what
//! this module opens.
//!
//! Freshness is a configured promise, not a guarantee. [`ReplicaConfig::sync_interval`] sets how
//! often a background task pulls, and [`SyncStats`] is where a health surface reports how honest
//! that promise has been.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::Result;

/// The setting that names the remote primary, e.g. `libsql://axon-owner.turso.io`.
pub const URL_VAR: &str = "SJEL_TURSO_URL";

/// The setting that carries the primary's auth token.
///
/// A token belongs in the environment or the OS keychain, never in a file this repository
/// tracks (see `Packs/security/skills/secrets-guard`).
pub const AUTH_TOKEN_VAR: &str = "SJEL_TURSO_AUTH_TOKEN";

/// The setting that overrides the background pull interval, in whole seconds.
pub const SYNC_INTERVAL_VAR: &str = "SJEL_TURSO_SYNC_INTERVAL_SECS";

/// The URL schemes LibSQL accepts for a remote primary.
const SCHEMES: [&str; 3] = ["libsql://", "http://", "https://"];

/// How a replica reaches its primary, and how often it pulls.
///
/// Debug redacts the token, so a config can be printed in a log line or a panic message without
/// publishing a credential.
#[derive(Clone)]
pub struct ReplicaConfig {
    url: String,
    auth_token: Option<String>,
    sync_interval: Duration,
    read_your_writes: bool,
}

impl ReplicaConfig {
    /// The pull interval used when nothing else is configured.
    ///
    /// One minute trades freshness for traffic: a reader is at most the interval plus one pull
    /// behind the primary, and a machine that pulls every few seconds for no reason is worse than
    /// one that pulls on demand. A surface that needs to be current calls `Store::sync_now`.
    pub const DEFAULT_SYNC_INTERVAL: Duration = Duration::from_secs(60);

    /// A config for `url` with no token, a one-minute pull interval, and read-your-writes on.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auth_token: None,
            sync_interval: Self::DEFAULT_SYNC_INTERVAL,
            read_your_writes: true,
        }
    }

    /// Set the primary's auth token. An absent token sends an empty bearer, which a token-protected
    /// primary answers with `401`.
    #[must_use]
    pub fn auth_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        self.auth_token = Some(token).filter(|t| !t.trim().is_empty());
        self
    }

    /// Set how often the background task pulls. Must be greater than zero; `open_replica` refuses
    /// a zero interval because the timer would fire without pause.
    #[must_use]
    pub fn sync_interval(mut self, interval: Duration) -> Self {
        self.sync_interval = interval;
        self
    }

    /// Set whether a write is visible to a read on this process before the write returns.
    ///
    /// Defaults to `true`. Turning it off lets a write return sooner at the cost of a read that
    /// can miss the row it just wrote.
    #[must_use]
    pub fn read_your_writes(mut self, read_your_writes: bool) -> Self {
        self.read_your_writes = read_your_writes;
        self
    }

    /// The remote primary's URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The URL with any `user:password@` authority removed, for a message or a log line.
    ///
    /// A LibSQL URL needs no credentials — the auth token carries them — but a URL that has them
    /// must not print them into stderr or a health payload, so every message about this primary
    /// goes through here.
    pub fn public_url(&self) -> String {
        let Some((scheme, rest)) = self.url.split_once("://") else {
            return self.url.clone();
        };
        let (authority, tail) = match rest.find('/') {
            Some(slash) => rest.split_at(slash),
            None => (rest, ""),
        };
        match authority.split_once('@') {
            Some((_, host)) => format!("{scheme}://{host}{tail}"),
            None => self.url.clone(),
        }
    }

    /// The configured background pull interval.
    pub fn interval(&self) -> Duration {
        self.sync_interval
    }

    /// Read [`URL_VAR`], [`AUTH_TOKEN_VAR`] and [`SYNC_INTERVAL_VAR`] from the environment.
    ///
    /// `Ok(None)` means no replica is configured, which is the ordinary case for a capability that
    /// reads and writes the local file. `Err` means a replica was asked for and the request is not
    /// usable, and the caller should refuse to start rather than fall back to the local file: a
    /// silent fallback is a device that stops seeing the primary's writes with no signal.
    ///
    /// # Errors
    /// The interval setting is not a positive whole number of seconds.
    pub fn from_env() -> Result<Option<Self>> {
        let Some(url) = std::env::var(URL_VAR).ok().filter(|u| !u.trim().is_empty()) else {
            return Ok(None);
        };
        let mut config = Self::new(url.trim());
        if let Ok(token) = std::env::var(AUTH_TOKEN_VAR) {
            config = config.auth_token(token);
        }
        if let Ok(seconds) = std::env::var(SYNC_INTERVAL_VAR) {
            let seconds = seconds.trim();
            if !seconds.is_empty() {
                let parsed: u64 = seconds.parse().map_err(|_| {
                    format!(
                        "{SYNC_INTERVAL_VAR} must be a whole number of seconds, not {seconds:?}"
                    )
                })?;
                config = config.sync_interval(Duration::from_secs(parsed));
            }
        }
        config.validate()?;
        Ok(Some(config))
    }

    /// Refuse a configuration that would open a replica nothing can talk to.
    fn validate(&self) -> Result<()> {
        if self.url.trim().is_empty() {
            return Err(format!("{URL_VAR} is empty").into());
        }
        if !SCHEMES.iter().any(|scheme| self.url.starts_with(scheme)) {
            return Err(format!(
                "{} names no remote primary: expected a URL starting with one of {}",
                self.url,
                SCHEMES.join(", ")
            )
            .into());
        }
        if self.sync_interval.is_zero() {
            return Err("the replica sync interval must be greater than zero".into());
        }
        Ok(())
    }

    /// Replace a credentialed URL in `message` with its public form.
    ///
    /// LibSQL's own errors can quote the endpoint they failed to reach, sometimes with the path and
    /// sometimes only the authority, so a caller that passes a URL with `user:password@` in it would
    /// otherwise find the credential in stderr or in a health payload.
    pub(crate) fn scrub(&self, message: &str) -> String {
        let mut scrubbed = message.replace(&self.url, &self.public_url());
        if let Some((scheme, rest)) = self.url.split_once("://") {
            let authority = rest.split('/').next().unwrap_or_default();
            if let Some((userinfo, _)) = authority.split_once('@') {
                scrubbed =
                    scrubbed.replace(&format!("{scheme}://{userinfo}@"), &format!("{scheme}://"));
            }
        }
        scrubbed
    }

    /// The first field in which `other` disagrees with `self`, for a message that does not leak the
    /// token.
    pub(crate) fn difference(&self, other: &Self) -> Option<&'static str> {
        if self.url != other.url {
            return Some("url");
        }
        if self.auth_token != other.auth_token {
            return Some("auth token");
        }
        if self.sync_interval != other.sync_interval {
            return Some("sync interval");
        }
        if self.read_your_writes != other.read_your_writes {
            return Some("read-your-writes");
        }
        None
    }
}

impl fmt::Debug for ReplicaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicaConfig")
            .field("url", &self.public_url())
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field("sync_interval", &self.sync_interval)
            .field("read_your_writes", &self.read_your_writes)
            .finish()
    }
}

/// How a replica's background pulls have gone, for a health surface or a log line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Pulls that completed, including the one `open_replica` performs.
    pub syncs: u64,
    /// Pulls that failed, in total. A streak of failures keeps counting here and is reported once.
    pub failures: u64,
    /// Write-ahead-log frames applied across every successful pull.
    pub frames_synced: u64,
    /// The most recent failure, cleared by the next success.
    pub last_error: Option<String>,
    /// Whole seconds since the last successful pull, `None` before the first one.
    pub seconds_since_success: Option<u64>,
}

impl SyncStats {
    /// Whether the last pull that ran succeeded.
    pub fn healthy(&self) -> bool {
        self.last_error.is_none()
    }
}

/// The counters and the last error a replica carries between pulls.
#[derive(Default)]
struct SyncState {
    syncs: AtomicU64,
    failures: AtomicU64,
    frames: AtomicU64,
    failing: AtomicBool,
    last_error: Mutex<Option<String>>,
    last_success: Mutex<Option<Instant>>,
}

impl SyncState {
    fn record_success(&self, frames: usize) {
        self.syncs.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
        self.failing.store(false, Ordering::Relaxed);
        *self.write(&self.last_error) = None;
        *self.write(&self.last_success) = Some(Instant::now());
    }

    /// Record a failure and answer whether it is the first of its streak, so a caller logs once
    /// per streak instead of once per interval.
    fn record_failure(&self, message: &str) -> bool {
        self.failures.fetch_add(1, Ordering::Relaxed);
        *self.write(&self.last_error) = Some(message.to_string());
        !self.failing.swap(true, Ordering::Relaxed)
    }

    fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    fn snapshot(&self) -> SyncStats {
        SyncStats {
            syncs: self.syncs.load(Ordering::Relaxed),
            failures: self.failures(),
            frames_synced: self.frames.load(Ordering::Relaxed),
            last_error: self.read(&self.last_error).clone(),
            seconds_since_success: self
                .read(&self.last_success)
                .map(|at: Instant| at.elapsed().as_secs()),
        }
    }

    /// A poisoned lock still holds a usable value: these guards protect two plain fields that no
    /// panic can leave half-written, so recovering beats refusing to report sync health forever.
    fn read<'a, T>(&self, lock: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write<'a, T>(&self, lock: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        self.read(lock)
    }
}

/// One opened replica database, shared by every capability in the process that asks for it.
pub(crate) struct Replica {
    config: ReplicaConfig,
    db: Arc<crate::libsql::Database>,
    state: SyncState,
}

impl Replica {
    /// Open the replica at `path` and pull once, so the caller's DDL and first read see the
    /// primary's current schema instead of an empty file.
    pub(crate) async fn open(path: &Path, config: ReplicaConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let path_str = path
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 database path: {}", path.display()))?;
        let db = crate::libsql::Builder::new_remote_replica(
            path_str,
            config.url.clone(),
            config.auth_token.clone().unwrap_or_default(),
        )
        .read_your_writes(config.read_your_writes)
        .build()
        .await
        .map_err(|err| {
            format!(
                "could not open the replica of {} at {}: {}. LibSQL refuses to turn a file that \
                 holds its own writes into a replica, so remove the file to start a fresh copy",
                config.public_url(),
                path.display(),
                config.scrub(&err.to_string())
            )
        })?;

        let replica = Arc::new(Self {
            config,
            db: Arc::new(db),
            state: SyncState::default(),
        });
        replica.sync_now().await.map_err(|err| {
            format!(
                "the first sync of the replica of {} failed: {err}",
                replica.config.public_url()
            )
        })?;
        Ok(replica)
    }

    pub(crate) fn database(&self) -> Arc<crate::libsql::Database> {
        Arc::clone(&self.db)
    }

    pub(crate) fn config(&self) -> &ReplicaConfig {
        &self.config
    }

    /// Pull the primary's log once, then apply it. Counts frames, so a pull that finds nothing new
    /// still counts as a sync.
    pub(crate) async fn sync_now(&self) -> Result<Option<u64>> {
        match self.db.sync().await {
            Ok(replicated) => {
                let frames = replicated.frames_synced();
                self.state.record_success(frames);
                Ok(replicated.frame_no())
            }
            Err(err) => {
                let message = self.config.scrub(&err.to_string());
                if self.state.record_failure(&message) {
                    eprintln!(
                        "sjel-turso: sync from {} failed: {message}",
                        self.config.public_url()
                    );
                }
                Err(message.into())
            }
        }
    }

    pub(crate) fn stats(&self) -> SyncStats {
        self.state.snapshot()
    }

    /// Start the pull loop. It holds a weak reference, so the loop ends when the replica it serves
    /// is gone instead of keeping a dropped database alive.
    pub(crate) fn start_background(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let interval = self.config.sync_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // A pull slower than the interval must not queue a burst of catch-up pulls.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // `open` already pulled, and the first tick fires immediately.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(replica) = Weak::upgrade(&weak) else {
                    return;
                };
                let failures_before = replica.state.failures();
                let synced = replica.sync_now().await.is_ok();
                if synced && failures_before > 0 {
                    eprintln!(
                        "sjel-turso: sync from {} recovered after {failures_before} failed attempt(s)",
                        replica.config.public_url()
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_without_a_scheme_is_refused() {
        let err = ReplicaConfig::new("axon-owner.turso.io")
            .validate()
            .unwrap_err();
        assert!(
            err.to_string().contains("names no remote primary"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn an_empty_interval_is_refused() {
        let err = ReplicaConfig::new("libsql://example.turso.io")
            .sync_interval(Duration::ZERO)
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("greater than zero"), "{err}");
    }

    #[test]
    fn a_token_survives_trimming_but_not_emptiness() {
        let token = ReplicaConfig::new("libsql://example.turso.io")
            .auth_token("  ")
            .auth_token("  secret  ");
        assert_eq!(token.auth_token.as_deref(), Some("  secret  "));
        assert_eq!(
            ReplicaConfig::new("libsql://example.turso.io")
                .auth_token("   ")
                .auth_token,
            None
        );
    }

    #[test]
    fn debug_does_not_print_the_token() {
        let printed = format!(
            "{:?}",
            ReplicaConfig::new("libsql://example.turso.io").auth_token("s3cr3t-token")
        );
        assert!(!printed.contains("s3cr3t-token"), "{printed}");
        assert!(printed.contains("redacted"), "{printed}");
    }

    #[test]
    fn a_url_prints_without_its_credentials() {
        let plain = ReplicaConfig::new("libsql://axon-owner.turso.io");
        assert_eq!(plain.public_url(), "libsql://axon-owner.turso.io");

        let credentialed = ReplicaConfig::new("libsql://owner:s3cr3t@axon-owner.turso.io/axon");
        assert_eq!(
            credentialed.public_url(),
            "libsql://axon-owner.turso.io/axon"
        );
        assert!(!format!("{credentialed:?}").contains("s3cr3t"));

        // An `@` after the authority is part of the path, not a credential.
        let path_at = ReplicaConfig::new("libsql://axon-owner.turso.io/a@b");
        assert_eq!(path_at.public_url(), "libsql://axon-owner.turso.io/a@b");
    }

    #[test]
    fn an_error_message_loses_the_credential_it_quoted() {
        let config = ReplicaConfig::new("libsql://owner:s3cr3t@axon-owner.turso.io/axon");

        // The endpoint as a whole, and the bare authority LibSQL also prints.
        let full = config.scrub("failed to open libsql://owner:s3cr3t@axon-owner.turso.io/axon");
        assert!(!full.contains("s3cr3t"), "{full}");
        assert!(full.contains("libsql://axon-owner.turso.io/axon"), "{full}");

        let authority =
            config.scrub("failed to connect to libsql://owner:s3cr3t@axon-owner.turso.io");
        assert!(!authority.contains("s3cr3t"), "{authority}");
        assert!(
            authority.contains("libsql://axon-owner.turso.io"),
            "{authority}"
        );

        let plain = ReplicaConfig::new("libsql://axon-owner.turso.io");
        assert_eq!(plain.scrub("connection refused"), "connection refused");
    }

    #[test]
    fn a_difference_names_the_field_that_differs() {
        let base = ReplicaConfig::new("libsql://a.turso.io");
        assert_eq!(base.difference(&base.clone()), None);
        assert_eq!(
            base.difference(&ReplicaConfig::new("libsql://b.turso.io")),
            Some("url")
        );
        assert_eq!(
            base.difference(&base.clone().auth_token("t")),
            Some("auth token")
        );
        assert_eq!(
            base.difference(&base.clone().sync_interval(Duration::from_secs(5))),
            Some("sync interval")
        );
        assert_eq!(
            base.difference(&base.clone().read_your_writes(false)),
            Some("read-your-writes")
        );
    }

    #[test]
    fn a_failure_streak_is_reported_once() {
        let state = SyncState::default();
        assert!(
            state.record_failure("connection refused"),
            "first of a streak"
        );
        assert!(!state.record_failure("connection refused"), "same streak");
        state.record_success(7);
        assert!(state.record_failure("timeout"), "new streak");
    }

    #[test]
    fn a_success_clears_the_error_and_counts_frames() {
        let state = SyncState::default();
        state.record_failure("boom");
        state.record_success(12);

        let stats = state.snapshot();
        assert_eq!(stats.syncs, 1);
        assert_eq!(stats.failures, 1);
        assert_eq!(stats.frames_synced, 12);
        assert_eq!(stats.last_error, None);
        assert!(stats.healthy());
        assert!(stats.seconds_since_success.is_some());
    }
}
