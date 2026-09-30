//! When the next run is due, and the loop that acts on it.
//!
//! The interval is stored policy, not a manifest line, so the operator can turn it off or
//! re-time it from the surface. The loop lives here rather than in a launchd job for the
//! same reason `IdlePanelReaper` lives in `sjel-status`: a timer that outlives the process
//! owning the state it acts on is a timer that acts on stale state.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::runner;
use crate::store::BackupStore;
use crate::targets;

/// The shortest interval a policy may declare. One hour is already more frequent than any
/// contract here asks for; below it the loop's own period is the real interval, and a
/// policy that says something other than what happens is worse than one that refuses.
pub const MIN_INTERVAL_HOURS: i64 = 1;

/// How often the loop looks. Not the interval: a policy of 24h is checked every minute and
/// fires when 24h have passed, so changing the policy takes effect without a restart.
pub const TICK: Duration = Duration::from_secs(60);

/// The minimum time to wait after a failed run before retrying.
///
/// Prevents spinning in a tight retry loop (e.g. every minute) when an upload or network
/// failure occurs. The interval measures staleness of a successful backup, but failures
/// must back off to avoid thrashing CPU, disk, or remote rate limits.
pub const FAILURE_BACKOFF_SECS: i64 = 1800; // 30 minutes

/// Is another run due?
///
/// Never run counts as due: a target with a policy and no successful run is exactly the
/// case the policy exists for. A run that failed defers the next attempt by
/// `FAILURE_BACKOFF_SECS` so transient failures do not retry on every 60-second tick.
pub fn is_due(
    last_success_epoch: Option<i64>,
    last_attempt: Option<(i64, i64)>, // (epoch, exit_code)
    now_epoch: i64,
    interval_hours: i64,
) -> bool {
    if let Some((attempt_epoch, exit_code)) = last_attempt {
        if exit_code != 0 && now_epoch.saturating_sub(attempt_epoch) < FAILURE_BACKOFF_SECS {
            return false;
        }
    }
    match last_success_epoch {
        None => true,
        Some(last) => now_epoch.saturating_sub(last) >= interval_hours.saturating_mul(3_600),
    }
}

/// What one pass decided, without doing it. Separated so the decision is testable without a
/// clock, a database or a filesystem.
#[derive(Debug, PartialEq)]
pub struct Due {
    pub capability: String,
    pub target: String,
}

/// The runs one pass would start. Per capability, because a run row is per capability and a
/// target's policy covers every contract that names it.
pub fn due_runs(
    policies: &[(String, i64)],
    contracts: &[(String, String)],
    last_success: impl Fn(&str, &str) -> Option<i64>,
    last_attempt: impl Fn(&str, &str) -> Option<(i64, i64)>,
    now_epoch: i64,
    busy: bool,
) -> Vec<Due> {
    // While a run is in flight the whole pass waits. Two concurrent runs would race for the
    // same staging directory and the same receipt, and `backup.sh` is not built for that.
    if busy {
        return Vec::new();
    }
    let mut due = Vec::new();
    for (target, interval_hours) in policies {
        for (capability, contract_target) in contracts {
            if contract_target != target {
                continue;
            }
            if is_due(
                last_success(capability, target),
                last_attempt(capability, target),
                now_epoch,
                *interval_hours,
            ) {
                due.push(Due {
                    capability: capability.clone(),
                    target: target.clone(),
                });
            }
        }
    }
    due
}

/// The loop the surface spawns. Every tick: read the policy, read the contracts, and start
/// the runs that are due, one at a time.
pub async fn due_loop(store: Arc<BackupStore>, root: PathBuf, overlay: PathBuf) {
    loop {
        tokio::time::sleep(TICK).await;
        match one_pass(&store, &root, &overlay).await {
            Ok(started) if started > 0 => eprintln!("[backup] policy started {started} run(s)"),
            Ok(_) => {}
            Err(error) => eprintln!("[backup] policy pass failed: {error}"),
        }
    }
}

async fn one_pass(
    store: &Arc<BackupStore>,
    root: &Path,
    overlay: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let policies = store.active_policies()?;
    if policies.is_empty() {
        return Ok(0);
    }
    let contracts = targets::contracts(root)?;
    let store = Arc::clone(store);

    // The clock and the database are read in the pass, so the decision itself stays a pure
    // function of numbers that a test can hand made-up values.
    let now = runner::now_epoch();
    let busy = store.running_count()? > 0;
    let mut last_success = Vec::with_capacity(contracts.len());
    let mut last_attempt = Vec::with_capacity(contracts.len());
    for (capability, target) in &contracts {
        last_success.push(store.last_success_epoch(capability, target).ok().flatten());
        last_attempt.push(store.last_attempt(capability, target).ok().flatten());
    }
    let by_pair_success = |capability: &str, target: &str| -> Option<i64> {
        contracts
            .iter()
            .position(|(c, t)| c == capability && t == target)
            .and_then(|index| last_success[index])
    };
    let by_pair_attempt = |capability: &str, target: &str| -> Option<(i64, i64)> {
        contracts
            .iter()
            .position(|(c, t)| c == capability && t == target)
            .and_then(|index| last_attempt[index])
    };
    let due = due_runs(
        &policies,
        &contracts,
        by_pair_success,
        by_pair_attempt,
        now,
        busy,
    );

    let mut started = 0;
    for decision in due {
        let store = Arc::clone(&store);
        let root = root.to_path_buf();
        let overlay = overlay.to_path_buf();
        let log_path = overlay
            .join("backup")
            .join("logs")
            .join(format!("{}-policy.log", runner::now_epoch()));
        let run_id = store.start_run(
            &decision.capability,
            &decision.target,
            &log_path.to_string_lossy(),
        )?;
        let outcome = tokio::task::spawn_blocking(move || {
            runner::run(&root, &overlay, &decision.capability, &decision.target)
        })
        .await?;
        store.finish_run(
            run_id,
            outcome.exit_code,
            outcome.archive.as_ref(),
            &outcome.detail,
            &outcome.log_path.to_string_lossy(),
        )?;
        started += 1;
    }
    Ok(started)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contracts() -> Vec<(String, String)> {
        vec![
            ("store".to_string(), "backup-target".to_string()),
            ("vault".to_string(), "other-target".to_string()),
        ]
    }

    #[test]
    fn never_run_is_due_and_a_recent_success_is_not() {
        let hour = 3_600;
        assert!(is_due(None, None, 1_000_000, 24));
        assert!(!is_due(
            Some(1_000_000),
            Some((1_000_000, 0)),
            1_000_000 + 23 * hour,
            24
        ));
        assert!(is_due(
            Some(1_000_000),
            Some((1_000_000, 0)),
            1_000_000 + 24 * hour,
            24
        ));
        // A run that failed defers by FAILURE_BACKOFF_SECS so transient errors do not thrash.
        assert!(!is_due(None, Some((1_000_000, 1)), 1_000_000 + 60, 24));
        assert!(is_due(
            None,
            Some((1_000_000, 1)),
            1_000_000 + FAILURE_BACKOFF_SECS,
            24
        ));
    }

    #[test]
    fn only_contracts_naming_the_target_are_planned() {
        let policies = vec![("backup-target".to_string(), 24_i64)];
        let due = due_runs(&policies, &contracts(), |_, _| None, |_, _| None, 0, false);
        assert_eq!(
            due,
            vec![Due {
                capability: "store".into(),
                target: "backup-target".into()
            }]
        );
    }

    #[test]
    fn an_in_flight_run_suspends_the_whole_pass() {
        // Two runs at once would race for one staging directory and one receipt.
        let policies = vec![("backup-target".to_string(), 24_i64)];
        assert!(due_runs(&policies, &contracts(), |_, _| None, |_, _| None, 0, true).is_empty());
    }

    #[test]
    fn a_policy_of_off_is_not_a_policy() {
        // `active_policies` never returns a NULL interval, so the loop has nothing to read;
        // this asserts the decision function agrees that an empty policy set plans nothing.
        assert!(due_runs(&[], &contracts(), |_, _| None, |_, _| None, 0, false).is_empty());
    }
}
