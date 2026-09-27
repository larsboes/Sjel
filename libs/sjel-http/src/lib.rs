//! One home for outbound HTTP.
//!
//! Before this crate, 33 call sites across nine capabilities and two libs each built
//! their own `reqwest::blocking::Client`. Three things went wrong there, and all three
//! are the kind a reader only finds by reading all 33:
//!
//! 1. **Seven had no timeout at all.** `capabilities/scouting/src/http.rs`,
//!    `adapters/luma.rs`, `adapters/meetup.rs`, `adapters/splash_hub.rs`,
//!    `src/calendar_promote.rs`, `capabilities/comms/src/google.rs` and
//!    `capabilities/calendar/src/google_sync/auth.rs`. A `reqwest` client with no
//!    timeout waits forever, which is how a scheduled job becomes a stuck process.
//! 2. **The user-agent disagreed with itself.** `axon-places/0.1`, `AxonComms/0.1`,
//!    `Axon-Comms/0.1`, `AxonCalendar/0.1.0`, `Axon-Scouting/0.1-rss`,
//!    `axon-finance/1.0`, and nothing at all in a dozen others. Two of them pointed
//!    at the GitHub Pages site rather than the repository.
//! 3. **Every call rebuilt the client.** A `reqwest::blocking::Client` owns a TLS
//!    configuration, a connection pool, a background thread and a current-thread
//!    runtime. `libs/inference` built one per probe, per rerank and per embedding
//!    call, and threw the pool away each time.
//!
//! [`client`] answers all three: it takes the timeout as an argument so there is no
//! default to forget, it puts one user-agent shape on every request, and it caches
//! the built client per `(purpose, timeout)` so repeated calls share one pool.
//!
//! Where a caller needs an option this crate does not decide — gzip, a cookie store,
//! a redirect policy — [`builder`] hands back the same starting point unbuilt.
//!
//! [`guard`] holds the other half of the same door: what a URL has to prove before a
//! request is made. It was `capabilities/comms/src/media.rs`, the only place in the
//! workspace that checked an outbound destination, and it belongs beside the client
//! that would otherwise fetch it.
//!
//! # Why its own crate
//!
//! `libs/sjel-config` was the alternative, and it is the wrong host: five members
//! (`host-net`, `interior`, `soundscape`, `vault`, `tools/storage`) depend on
//! sjel-config and make no outbound request, and `reqwest` brings a TLS stack with
//! it. A lib in this repository is spine-owned shared code with no domain of its own
//! (ARCHITECTURE.md, "Libs"), and "how Axon talks to the network" is exactly that.

pub mod guard;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The repository every outbound request points a curious server operator at.
///
/// The `+` prefix is the convention RFC 9110 §10.1.5 shows for a URI in a
/// `User-Agent` comment, and it is what the sites that already carried a URL used.
///
/// The URL is the intended one, not a live one: Axon has no public GitHub remote
/// yet (PROJECTS.md). Three scouting adapters each carried that caveat beside their
/// own copy of the string; it is one caveat in one place now.
pub const UPSTREAM: &str = "https://github.com/larsboes/Sjel";

/// What a client is for, in one hyphenated token: `"comms-digest"`, `"scouting-rss"`.
///
/// It is a type rather than a `&str` because it is half of the cache key in
/// [`client`], and because it names the one place the user-agent shape is decided.
/// Capabilities declare their own values — this crate does not enumerate them, so it
/// stays free of any capability's domain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Purpose(&'static str);

impl Purpose {
    /// `const` so a call site can write `Purpose::new("trips-kiwi")` inline at no cost.
    ///
    /// The name is not validated here — a `const fn` cannot return an error — so
    /// [`user_agent`] folds anything outside `[A-Za-z0-9._-]` to `-` before it reaches
    /// a header. A purpose with a newline in it therefore cannot split a request.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The name as given, before the header folding.
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for Purpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// `Axon-<purpose>/<version> (+<upstream>)` — the one user-agent shape.
///
/// The version is this crate's, which is the workspace version, so every capability
/// reports the same number and a server sees one product rather than nine.
pub fn user_agent(purpose: Purpose) -> String {
    format!(
        "Axon-{}/{} (+{UPSTREAM})",
        header_token(purpose.0),
        env!("CARGO_PKG_VERSION")
    )
}

/// Folds a purpose to the characters a `User-Agent` product token allows.
///
/// Not a rejection: a purpose is a compile-time constant written by an Axon author,
/// so the realistic failure is a typo, not an attack. Folding keeps the request
/// working and keeps the header un-splittable either way.
fn header_token(raw: &str) -> String {
    let folded: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if folded.is_empty() {
        "unnamed".to_string()
    } else {
        folded
    }
}

/// A client builder that already carries the user-agent, the timeout and the redirect
/// policy.
///
/// For the four sites that need an option this crate does not decide: gzip
/// (`comms/google.rs`, `comms/media.rs`, `calendar/google_sync/auth.rs`), a cookie
/// store (`finance/price.rs`), a redirect policy (`comms/media.rs`) and the one
/// deliberate user-agent override (`scouting/adapters/meetup.rs`, which spoofs a
/// browser because the site refuses anything else). Calling `.user_agent()` again on
/// the result replaces the default, which is what that override relies on, and
/// `.redirect()` replaces the policy the same way — comms is the one caller that does,
/// because its chain has an allowlist this crate does not know about.
///
/// [`guard::redirect_policy`] is set here rather than left to the caller because a
/// redirect is the one hop a caller cannot see. Thirty-three of the thirty-four call
/// sites in this workspace ran reqwest's default, which follows ten hops and checks
/// nothing (`rg 'sjel_http::(client|builder)\('` against `rg '\.redirect\('`,
/// 2026-09-08).
///
/// The result is not cached — the caller's extra options are invisible to this crate,
/// so two callers with the same purpose could not safely share one client.
pub fn builder(purpose: Purpose, timeout: Duration) -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .user_agent(user_agent(purpose))
        .timeout(timeout)
        .redirect(guard::redirect_policy())
}

type Pool = Mutex<HashMap<(Purpose, Duration), reqwest::blocking::Client>>;

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A blocking client with this purpose's user-agent and this caller's timeout.
///
/// Cached per `(purpose, timeout)`. `reqwest::blocking::Client` is a handle over a
/// shared inner state, so the clone this returns shares the connection pool, the TLS
/// configuration and the background runtime thread with every earlier caller that
/// asked for the same pair. That is the whole efficiency claim: `libs/inference`
/// probes a backend on a 3-second timeout before most calls, and used to pay for a
/// thread and a root-certificate load each time.
///
/// **Not for use from an async runtime worker.** A blocking client driven from inside
/// a Tokio worker panics at run time. Every caller in Axon is either a CLI or inside
/// `tokio::task::spawn_blocking`, whose threads are not runtime workers.
/// `capabilities/finance/src/price.rs` states the same rule at its own call site.
///
/// The error is `reqwest`'s own, because the only way this fails is TLS backend
/// initialisation and the caller already has a shape for reporting that.
pub fn client(
    purpose: Purpose,
    timeout: Duration,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
    // `into_inner` rather than `unwrap`: a poisoned lock here means some other
    // thread panicked while holding a HashMap of clients, which leaves the map
    // perfectly usable. Refusing every later request over it would turn one
    // unrelated panic into an outage.
    let mut guard = pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(&(purpose, timeout)) {
        return Ok(existing.clone());
    }
    let built = builder(purpose, timeout).build()?;
    guard.insert((purpose, timeout), built.clone());
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_agent_names_axon_and_the_upstream_repository() {
        let ua = user_agent(Purpose::new("comms-digest"));
        assert!(ua.starts_with("Axon-comms-digest/"), "{ua}");
        assert!(ua.ends_with(&format!("(+{UPSTREAM})")), "{ua}");
        assert!(ua.contains("github.com/larsboes/Sjel"), "{ua}");
    }

    #[test]
    fn a_purpose_cannot_split_a_header() {
        let ua = user_agent(Purpose::new("bad\r\nX-Injected: 1"));
        assert!(!ua.contains('\r'), "{ua}");
        assert!(!ua.contains('\n'), "{ua}");
        assert_eq!(ua.lines().count(), 1, "{ua}");
    }

    #[test]
    fn an_empty_purpose_still_produces_a_product_token() {
        assert!(user_agent(Purpose::new("")).starts_with("Axon-unnamed/"));
    }

    #[test]
    fn every_purpose_reports_the_same_version() {
        let a = user_agent(Purpose::new("one"));
        let b = user_agent(Purpose::new("two"));
        let version = |ua: &str| {
            ua.split('/')
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap()
                .to_string()
        };
        assert_eq!(version(&a), version(&b));
        assert_eq!(version(&a), env!("CARGO_PKG_VERSION"));
    }

    /// How many distinct clients the cache is holding for one purpose.
    ///
    /// The cache is the efficiency claim, and `reqwest::blocking::Client` exposes
    /// nothing an assertion can compare — its `Debug` prints the configuration, so
    /// two separately built clients with the same settings look identical. Counting
    /// the entries this crate keeps is the observable form of the same question.
    fn cached_for(purpose: Purpose) -> usize {
        pool()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .filter(|(p, _)| *p == purpose)
            .count()
    }

    #[test]
    fn the_same_purpose_and_timeout_are_built_once() {
        const P: Purpose = Purpose::new("test-reuse");
        assert_eq!(cached_for(P), 0, "no other test uses this purpose");
        for _ in 0..5 {
            client(P, Duration::from_secs(7)).expect("client builds");
        }
        assert_eq!(cached_for(P), 1, "five calls, one client");
    }

    #[test]
    fn a_different_timeout_is_a_different_client() {
        // Same purpose, different timeout: the key must not collapse them, or a
        // 3-second probe would silently inherit a 120-second wait.
        const P: Purpose = Purpose::new("test-distinct");
        assert_eq!(cached_for(P), 0, "no other test uses this purpose");
        client(P, Duration::from_secs(1)).expect("client builds");
        client(P, Duration::from_secs(2)).expect("client builds");
        client(P, Duration::from_secs(1)).expect("client builds");
        assert_eq!(cached_for(P), 2);
    }

    #[test]
    fn a_builder_result_carries_the_user_agent_and_the_timeout() {
        // The builder is opaque, so this asserts the only thing it can: that the
        // configured builder still builds, and that the extra option a caller adds
        // does not conflict with what this crate already set.
        let built = builder(Purpose::new("test-builder"), Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .build();
        assert!(built.is_ok());
    }
}
