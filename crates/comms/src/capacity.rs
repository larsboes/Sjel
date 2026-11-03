//! When a run of capacity aborts stops being weather and becomes a fault.
//!
//! A single `capacity_aborted` is normal and self-healing: the local server had
//! another prefill in flight, this item waits and the next drain pass writes it.
//! Nothing should be raised for one. A *streak* of them is a different fact — a
//! model that no longer fits its ceiling, a backend that came up wedged, a
//! second process holding the GPU and never letting go. Those look identical to
//! the weather from inside any one pass, and on 2026-08-07 ninety of them in a
//! day were logged individually and nobody read the log.
//!
//! ## Why there is no new table and no new notifier
//!
//! `source_state` already owns exactly this: a per-source consecutive-failure
//! counter that a success resets, a last-error class, and timestamps. The inbox
//! sweep already uses it for its own streaks and already exposes it to a reader
//! at `GET /triage/sweep/status`. So local inference becomes one more row in
//! that table under [`LOCAL_INFERENCE_SOURCE`], the drain says it out loud on
//! the pass that crosses the threshold, and `GET /feed/evaluation/status` — the
//! endpoint the dashboard already polls for summarizer health — carries the
//! streak for anyone who was not watching stderr at the time.

use crate::store::Store;

/// The `source_state` row local-inference capacity health lives under. Not a
/// feed source id: the name space is per-source-of-work, and this is the local
/// model server, which every collector's items eventually pass through.
pub const LOCAL_INFERENCE_SOURCE: &str = "local-inference";

/// The class written to `source_state.last_error`. A short stable label like
/// every other one there — never a provider message, which quotes request URLs
/// and, on the mail side, occasionally the subject that failed.
const ERROR_CLASS: &str = "capacity";

/// One capacity abort happened. Returns the streak when it has reached the
/// threshold and this is therefore an alert, `None` otherwise.
///
/// Returns rather than logging so the caller names its own pass — "digest
/// drain" and "enrichment drain" are different things to go looking at, and a
/// line that says only "local inference" sends a reader to both.
///
/// A store error is swallowed to `None` on purpose: failing to *record* a
/// capacity abort must not turn a retryable item into a failed pass. The abort
/// itself is already recorded on the row's own ledger.
pub fn record_failure(store: &Store, threshold: i32) -> Option<i32> {
    let streak = store
        .record_sweep_failure(LOCAL_INFERENCE_SOURCE, ERROR_CLASS)
        .ok()?;
    alerts(streak, threshold).then_some(streak)
}

/// The local server answered. Clears the streak, which is the half that makes
/// "consecutive" mean anything — without it the counter is a lifetime total and
/// crosses any threshold eventually.
pub fn record_success(store: &Store) {
    let _ = store.record_sweep_success(LOCAL_INFERENCE_SOURCE, 0, 0);
}

/// Whether a streak of this length is worth raising.
///
/// At or past the threshold, not exactly at it: these passes run every fifteen
/// minutes, and a machine that is still capacity-broken an hour later is still
/// worth saying so about. A threshold of `0` disables the alert, matching how
/// every other `0` in this config reads.
fn alerts(streak: i32, threshold: i32) -> bool {
    threshold > 0 && streak >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole contract of the threshold, at its edges. Two is weather,
    /// three is the fault, and four is still the fault.
    #[test]
    fn a_streak_alerts_at_the_threshold_and_stays_alerting() {
        assert!(!alerts(1, 3), "one abort is the ordinary busy case");
        assert!(!alerts(2, 3));
        assert!(alerts(3, 3), "the threshold itself must fire");
        assert!(alerts(9, 3), "still broken an hour later is still an alert");
    }

    /// `0` is how every other interval and cap in this config spells "off", so
    /// it has to spell it here too rather than meaning "alert on everything".
    #[test]
    fn a_threshold_of_zero_disables_the_alert() {
        for streak in [0, 1, 3, 100] {
            assert!(
                !alerts(streak, 0),
                "streak {streak} alerted with no threshold"
            );
        }
    }

    /// A threshold of one is a legitimate setting — a machine where any
    /// capacity abort is worth knowing about — and must not be read as off.
    #[test]
    fn a_threshold_of_one_alerts_on_the_first_abort() {
        assert!(alerts(1, 1));
        assert!(!alerts(0, 1), "no abort is not an abort");
    }
}
