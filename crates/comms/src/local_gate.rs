//! Cross-process admission control for the local inference server.
//!
//! ## Why a lock and not a semaphore
//!
//! On 2026-08-05 four concurrent prefills pushed oMLX past its hard memory
//! watermark and it aborted all four. Two of them were feed digests. Nothing in
//! Sjel knew the others were running, because the competing consumers are
//! separate processes: comms-server, the Pulse DA, anything else that
//! resolves the summarization role. An in-process semaphore bounds one of them
//! and lets the rest collide exactly as before.
//!
//! A SQLite write lock on a small dedicated file is the mechanism. It was a
//! Postgres advisory lock until PRD Q45 retired the server; SQLite has no
//! advisory locks, but a `BEGIN IMMEDIATE` on a file nobody else writes buys the
//! same two properties that earned the advisory lock over a lock *file* with a
//! pid in it. The lock is held by an open connection, so a process that dies
//! mid-request releases it — the OS drops the file lock with the descriptor —
//! and waiting is done by SQLite's `busy_timeout` rather than by a polling loop.
//!
//! A file of its own, not the shared database. Holding the shared file's write
//! lock for the length of a model call would stall every capability's writes for
//! twenty seconds at a time, which is a far worse outcome than the contention
//! this exists to prevent. The lock file carries no rows and is never read.
//!
//! ## Why the cap is one
//!
//! oMLX already batches and schedules; what it does not do is refuse work it
//! cannot fit, which is how a fourth request turns three healthy ones into four
//! aborted ones. One at a time is the setting that cannot produce that. It
//! costs throughput only when two consumers want the GPU at once, and in that
//! situation the alternative was not parallelism — it was a memory abort.
//!
//! ## Why the cap is one *per backend*
//!
//! The first version held one lock for the whole machine, which was right while
//! oMLX was the only local server. It stopped being right the moment Apple
//! Foundation Models joined it: AFM runs on the Neural Engine at zero Metal
//! cost and shares no memory pool with oMLX, so an AFM digest that waits for an
//! oMLX prefill to finish waits ninety seconds for a resource it was never
//! going to touch, and is then told the machine is busy. The resource being
//! protected is a *backend's* memory, not the machine's, so the lock is keyed
//! by backend name: `foundation-models` and `omlx` contend with themselves and
//! with every other process resolving the same backend, and with nobody else.
//!
//! ## What a caller sees when the wait runs out
//!
//! `Outcome::CapacityAborted`, which is retryable and reported to a reader as a
//! machine condition. A background drain picks the item up on its next pass; an
//! operator pressing Regenerate is told the machine is busy rather than left
//! watching a spinner for the length of somebody else's transcript.

use std::sync::Arc;
use std::time::Duration;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::summarize::{Admission, LocalGate};

/// The high half of every key this module takes. It named an advisory-lock
/// namespace; it now names the lock file, so a stray file beside the database is
/// still recognisable as this subsystem's rather than anyone else's.
const LOCAL_INFERENCE_NAMESPACE: i64 = 0x41_58_4F_4E; // "AXON"

/// Which lock a backend takes. Any two processes must derive the same number
/// from the same backend name or they do not contend, and the whole point is
/// that they contend — so the hash is written out here rather than taken from
/// `DefaultHasher`, whose output std explicitly does not promise to keep stable
/// across releases. Two binaries built a month apart must still collide.
///
/// FNV-1a, 32-bit. The low half identifies the backend, the high half is the
/// namespace above, so the whole key stays positive and readable as
/// `AXON:<backend hash>`.
pub(crate) fn lock_key(backend_name: &str) -> i64 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in backend_name.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (LOCAL_INFERENCE_NAMESPACE << 32) | i64::from(hash)
}

/// How long a caller waits before being told the machine is busy.
///
/// Sized against the work, not the impatience: a sectioned digest of a long
/// transcript takes 12-20s on this machine, and a queue two deep is normal
/// during a drain. Much shorter and a drain would spend its passes reporting
/// contention; much longer and an operator's press hangs behind a backlog.
const WAIT_TIMEOUT: Duration = Duration::from_secs(90);

/// The lock file for one backend, beside the shared database.
///
/// Derived from [`lock_key`] rather than from the backend name directly: a
/// backend name is config, and a config value is not a filename. The hash is
/// stable across builds, which is the property that makes two processes contend.
pub(crate) fn lock_path(database_path: &Path, backend_name: &str) -> PathBuf {
    let directory = database_path.parent().unwrap_or(Path::new("."));
    directory.join(format!("axon-local-{:x}.lock", lock_key(backend_name)))
}

/// One SQLite write lock, held for the duration of a local request to one
/// backend.
pub struct AdvisoryGate {
    database_path: PathBuf,
    /// The resolved role's `backend_name`, not the model. Two roles on the same
    /// backend share one memory pool and must contend; the same model served by
    /// two backends does not.
    backend_name: String,
}

impl AdvisoryGate {
    pub fn new(database_path: impl Into<PathBuf>, backend_name: impl Into<String>) -> Self {
        Self {
            database_path: database_path.into(),
            backend_name: backend_name.into(),
        }
    }

    /// Wrap in the `Arc` that `Target::gate` wants.
    pub fn shared(
        database_path: impl Into<PathBuf>,
        backend_name: impl Into<String>,
    ) -> Arc<dyn LocalGate> {
        Arc::new(Self::new(database_path, backend_name))
    }
}

impl LocalGate for AdvisoryGate {
    fn acquire(&self) -> Result<Admission, String> {
        // A dedicated connection to a dedicated file, not the Store's pool. The
        // lock lives on the connection, so it must outlive `acquire` and die with
        // the request; and it must not be the shared database's write lock, which
        // every other capability needs while a model call runs.
        let path = lock_path(&self.database_path, &self.backend_name);
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("local inference gate: {error}"))?;
        }
        let connection = Connection::open(&path)
            .map_err(|error| format!("local inference gate: no lock file ({error})"))?;
        // SQLite does the waiting, with a deadline this side sets. The old code
        // polled `pg_try_advisory_lock` every 250ms because the blocking form had
        // no deadline it could enforce; `busy_timeout` has one.
        connection
            .busy_timeout(WAIT_TIMEOUT)
            .map_err(|error| format!("local inference gate: {error}"))?;

        // IMMEDIATE takes the write lock at BEGIN. A deferred transaction would
        // acquire nothing here and contend only at the first write, which is
        // after the model call this is meant to gate.
        connection.execute_batch("BEGIN IMMEDIATE").map_err(|_| {
            // Names the backend: "the machine is busy" sent a reader looking at
            // the wrong server the first time two of them ran.
            format!(
                "another local inference request held {} for more than {}s",
                self.backend_name,
                WAIT_TIMEOUT.as_secs()
            )
        })?;

        // Dropping the connection closes the file, and the OS drops its lock --
        // so a process killed between here and the rollback releases it too. The
        // explicit rollback is for the ordinary path, so the descriptor is not
        // the only thing standing between a finished request and the next one.
        Ok(Admission::new(move || {
            let _ = connection.execute_batch("ROLLBACK");
            drop(connection);
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The contract `complete` relies on: whatever a gate hands back releases
    /// itself when it goes out of scope, on every path out of the request.
    #[test]
    fn an_admission_releases_itself_on_drop() {
        static RELEASED: AtomicUsize = AtomicUsize::new(0);
        {
            let _held = Admission::new(|| {
                RELEASED.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(RELEASED.load(Ordering::SeqCst), 0, "not while it is held");
        }
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1, "released by drop");
    }

    /// Early returns are the reason drop was chosen over an explicit release.
    #[test]
    fn an_admission_releases_on_an_early_return() {
        static RELEASED: AtomicUsize = AtomicUsize::new(0);
        fn bail() -> Result<(), &'static str> {
            let _held = Admission::new(|| {
                RELEASED.fetch_add(1, Ordering::SeqCst);
            });
            Err("something went wrong before the end of the function")
        }
        assert!(bail().is_err());
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1);
    }

    /// A gate with nothing to clean up must still be a valid permit, so a
    /// counting implementation does not have to invent a no-op closure.
    #[test]
    fn a_free_admission_is_a_permit_too() {
        let _held = Admission::free();
    }

    /// The namespace is what keeps these lock files recognisable beside the
    /// database and clear of whatever else writes there.
    /// Pinned to its derivation rather than to a copy of itself.
    #[test]
    fn every_key_sits_in_the_axon_namespace() {
        assert_eq!(
            LOCAL_INFERENCE_NAMESPACE,
            i64::from(u32::from_be_bytes(*b"AXON")),
            "the namespace must stay byte-identical across every process"
        );
        for backend in ["omlx", "foundation-models", "ollama", ""] {
            let key = lock_key(backend);
            assert_eq!(
                key >> 32,
                LOCAL_INFERENCE_NAMESPACE,
                "{backend} landed outside the namespace"
            );
            assert!(
                key > 0,
                "a negative key would put a minus sign in the lock file's name"
            );
        }
    }

    /// The whole point of C15. Two loopback backends share no memory pool, so
    /// they must not share a lock: an AFM digest queuing ninety seconds behind
    /// an oMLX prefill is a `CapacityAborted` for a resource it never touched.
    #[test]
    fn two_backends_do_not_contend_with_each_other() {
        assert_ne!(
            lock_key("foundation-models"),
            lock_key("omlx"),
            "AFM and oMLX must take different locks"
        );
        // Every pair, so a hash collision between two names actually in
        // inference.json fails here rather than in production as a mystery wait.
        let backends = [
            "omlx",
            "foundation-models",
            "ollama",
            "llama-cpp",
            "lmstudio",
        ];
        for (index, left) in backends.iter().enumerate() {
            for right in &backends[index + 1..] {
                assert_ne!(
                    lock_key(left),
                    lock_key(right),
                    "{left} collides with {right}"
                );
            }
        }
    }

    /// And the other half: the same backend named twice must contend, or the
    /// gate guards nothing while looking exactly like one that works. Two
    /// processes derive this independently, so it has to be a pure function of
    /// the name and nothing else.
    #[test]
    fn one_backend_always_derives_the_same_key() {
        assert_eq!(lock_key("omlx"), lock_key("omlx"));
        assert_eq!(
            AdvisoryGate::new("postgres://a", "omlx").backend_name,
            "omlx"
        );
        // Pinned literals: a refactor of the hash that changes these silently
        // splits a running process from a restarted one, and both would report
        // healthy while sharing no lock at all.
        assert_eq!(lock_key("omlx"), 0x41_58_4F_4E_EA_68_17_05);
        assert_eq!(lock_key("foundation-models"), 0x41_58_4F_4E_54_17_33_81);
    }

    /// Everything above this line is arithmetic and drop semantics, and runs
    /// anywhere. Everything below touches a real lock file, so it lives in its own
    /// module for one reason: CI splits the workspace by module path, `--skip
    /// db_tests::` for the hermetic job and `db_tests::` for the store job. A
    /// database test sitting directly in `local_gate::tests` is invisible to that
    /// split. It was: `comms_test` was red in CI and green locally for exactly
    /// that reason.
    ///
    /// The selector is one string in one workflow, so the module name IS the suite
    /// membership — see `capabilities/scouting/src/store.rs`.
    mod db_tests {
        use super::*;

        /// A directory this process owns, standing in for the shared database's.
        /// The gate only ever uses the path's *parent*, so nothing has to exist
        /// at the path itself.
        fn test_database_path() -> PathBuf {
            let directory =
                std::env::temp_dir().join(format!("comms-gate-test-{}", std::process::id()));
            std::fs::create_dir_all(&directory).expect("a writable temp directory");
            directory.join("axon.db")
        }

        /// C15 against real file locks, not the key arithmetic: two loopback
        /// backends must hold admission **at the same time**, and a second caller
        /// on one of them must not.
        ///
        /// The self-contention half runs on a backend name unique to this test, so
        /// a concurrently running comms-server holding the real `omlx` lock cannot
        /// turn this green or red by accident.
        #[test]
        fn afm_and_omlx_hold_admission_at_the_same_time() {
            let database = test_database_path();
            for backend in ["foundation-models", "omlx-gate-test"] {
                let _ = std::fs::remove_file(lock_path(&database, backend));
            }
            let afm = AdvisoryGate::new(&database, "foundation-models");
            let omlx = AdvisoryGate::new(&database, "omlx-gate-test");

            let held_afm = afm
                .acquire()
                .unwrap_or_else(|error| panic!("could not take the AFM gate: {error}"));
            let held_omlx = omlx
                .acquire()
                .expect("a second backend must not queue behind the first");

            // Same backend, different connection: this is the cross-process case,
            // and it must still be refused. Each gate opens its own connection, so
            // a second `BEGIN IMMEDIATE` on that file is a genuine second holder.
            let contender = Connection::open(lock_path(&database, "omlx-gate-test"))
                .expect("a second connection to the lock file");
            // No busy_timeout, so this answers now rather than after 90 seconds.
            assert!(
                contender.execute_batch("BEGIN IMMEDIATE").is_err(),
                "one backend must still admit only one request at a time"
            );

            drop(held_afm);
            drop(held_omlx);

            // And it is released, not leaked: the next caller gets straight in.
            assert!(
                contender.execute_batch("BEGIN IMMEDIATE").is_ok(),
                "the lock outlived the request that held it"
            );
            let _ = contender.execute_batch("ROLLBACK");
        }

        /// The whole file is one lock, so two backends must not share one file --
        /// which is the same statement `two_backends_do_not_contend_with_each_other`
        /// makes about the key, made about the thing the key now names.
        #[test]
        fn two_backends_get_two_lock_files() {
            let database = test_database_path();
            assert_ne!(
                lock_path(&database, "foundation-models"),
                lock_path(&database, "omlx")
            );
            assert_eq!(
                lock_path(&database, "omlx").parent(),
                database.parent(),
                "the lock lives beside the database, not inside it"
            );
        }
    }
}
