use super::*;

/// Owns every periodic server task. Dropping the server lifecycle aborts the
/// tasks instead of leaving detached work running beyond its owner.
pub(super) struct BackgroundServices {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl BackgroundServices {
    pub(super) fn start(config: &Config) -> Self {
        let mut tasks = Vec::new();
        tasks.extend(spawn_enrichment_drain(config.enrichment_drain_minutes));
        tasks.extend(spawn_digest_drain(config.digest_drain_minutes));
        tasks.push(spawn_trash_cleanup());
        tasks.extend(spawn_gmail_maintenance(config.gmail_maintenance_minutes));
        tasks.extend(spawn_inbox_sweep(
            config.inbox_sweep_minutes,
            config.inbox_sweep_max_threads,
            config.inbox_sweep_quiet_hours,
        ));
        Self { tasks }
    }
}

impl Drop for BackgroundServices {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// A ticker for a periodic pass, offset by `phase` of its own period.
///
/// `tokio::time::interval` fires immediately and then every period, so two
/// drains spawned in the same breath with the same period fire together
/// forever — which is exactly what `enrichment_drain_minutes` and
/// `digest_drain_minutes` both defaulting to 15 produced. Both passes prefill
/// the same feed transcripts on the same local backend, so firing together is
/// the one schedule guaranteed to make them contend.
///
/// Expressed as a fraction of the period rather than a fixed number of minutes
/// so the offset survives someone setting the drains to 5 minutes or 60: at
/// equal periods a phase of 0 and a phase of 0.5 never coincide, whatever the
/// period is.
fn staggered_ticker(every_minutes: u64, phase: f64) -> tokio::time::Interval {
    let period = drain_period(every_minutes);
    let start = tokio::time::Instant::now() + phase_offset(period, phase);
    let mut ticker = tokio::time::interval_at(start, period);
    // A pass that overran its period must not then run back-to-back trying to
    // catch up; the next tick is a fresh period from when this one finished.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

fn drain_period(every_minutes: u64) -> std::time::Duration {
    std::time::Duration::from_secs(every_minutes * 60)
}

/// How long after spawn this drain's first pass runs. Split out from
/// [`staggered_ticker`] so the property that matters — that two schedules on
/// the same period never land on the same instant — can be checked as
/// arithmetic instead of by waiting out two real fifteen-minute intervals.
fn phase_offset(period: std::time::Duration, phase: f64) -> std::time::Duration {
    period.mul_f64(phase)
}

/// The enrichment drain keeps the immediate first tick: a machine that just
/// booted with a backlog should start on it.
const ENRICHMENT_PHASE: f64 = 0.0;

/// The digest drain waits half a period. At equal intervals — the shipped
/// default and the only configuration anyone runs — this is the offset that
/// makes the two passes maximally far apart and never simultaneous.
const DIGEST_PHASE: f64 = 0.5;

/// Start the bounded enrichment drain. The persistence ledger owns retry and
/// backoff state, so this loop only owns scheduling and task isolation.
fn spawn_enrichment_drain(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("enrichment drain disabled (enrichment_drain_minutes = 0)");
        return None;
    }

    eprintln!("enrichment drain: every {every_minutes} min");
    Some(tokio::spawn(async move {
        let mut ticker = staggered_ticker(every_minutes, ENRICHMENT_PHASE);
        loop {
            ticker.tick().await;
            eprintln!("enrichment drain: pass starting");
            // The join result is inspected, not discarded: a panic inside the
            // blocking half would otherwise take the drain down without a word,
            // which is the same silence this issue exists to remove.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_path) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("enrichment drain: store unavailable: {error}");
                        return;
                    }
                };
                let summary_producer_revision = media::summary_producer_revision(&cfg);
                let before = match store
                    .feed_enrichment_counts(summary_producer_revision.as_deref())
                {
                    Ok(counts) => counts,
                    Err(error) => {
                        eprintln!("enrichment drain: backlog query failed: {error}");
                        return;
                    }
                };

                match media::summarize_pending(&store, &cfg) {
                    Ok(pass) if pass.summarized > 0 || pass.over_window > 0 => eprintln!(
                        "enrichment drain: summarized {}, {} past the on-device window (left for a press)",
                        pass.summarized, pass.over_window
                    ),
                    // A backlog that did not move *and* was not skipped on
                    // purpose is the case this logging exists for. Staying quiet
                    // here would rebuild the silence the ledger was supposed to
                    // end — and naming the light role matters, because that is
                    // the one an unattended pass uses now.
                    Ok(_) if before.pending_summaries > 0 => eprintln!(
                        "enrichment drain: {} pending, {} failed, none summarized — the 'summarization_light' inference role is unreachable or unconfigured",
                        before.pending_summaries, before.failed_summaries
                    ),
                    Ok(_) => {}
                    Err(error) => eprintln!("enrichment drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("enrichment drain: pass did not finish: {error}");
            }
        }
    }))
}

/// Retry feed digests that failed retryably, on an interval.
///
/// `Outcome::EmptyResponse` and its siblings have always been marked retryable
/// and the ledger has always counted attempts, but nothing ever performed the
/// retry: `digest::refresh_pending`'s only caller was an HTTP endpoint no client
/// invoked. A digest lost to a transient failure stayed lost. Two rows sat at
/// `empty_response`, attempt 1 of 3, after oMLX aborted them under memory
/// pressure and the abort arrived shaped like a successful empty answer (#95).
///
/// **Feed only, deliberately.** `refresh_pending` for `mail` reads message
/// bodies, and a background job that quietly pulls every body out of a mailbox
/// is not something a machine should start doing on its own — the same reason
/// that pass is bounded and explicit rather than timer-driven. Mail digests stay
/// a press.
///
/// Bounded by the ledger rather than by anything here: `items_needing_digest`
/// skips rows at the attempt cap or inside their backoff window, so a
/// permanently broken item stops costing model calls and says why.
fn spawn_digest_drain(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("digest drain disabled (digest_drain_minutes = 0)");
        return None;
    }

    eprintln!(
        "digest drain: every {every_minutes} min, feed only, offset half a period from enrichment"
    );
    Some(tokio::spawn(async move {
        let mut ticker = staggered_ticker(every_minutes, DIGEST_PHASE);
        loop {
            ticker.tick().await;
            eprintln!("digest drain: pass starting");
            // Inspected rather than discarded, matching the enrichment drain: a
            // panic in the blocking half would otherwise take the drain down
            // silently, rebuilding the exact silence this exists to end.
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = match Store::open(&cfg.database_path) {
                    Ok(store) => store,
                    Err(error) => {
                        eprintln!("digest drain: store unavailable: {error}");
                        return;
                    }
                };
                match digest::refresh_pending(&store, &cfg, "feed", 25) {
                    Ok(report) if report.unconfigured => eprintln!(
                        "digest drain: no 'summarization_light' inference role on this machine; \
                         unattended digests are off"
                    ),
                    Ok(report)
                        if report.written > 0
                            || report.over_window > 0
                            || report.cloud_digested > 0
                            || report.cloud_failed > 0 =>
                    {
                        eprintln!(
                            "digest drain: wrote {} on-device, {} from the cloud ({} cloud \
                             failure(s)), {} past the window and left for a press",
                            report.written,
                            report.cloud_digested,
                            report.cloud_failed,
                            report.over_window
                        )
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("digest drain: {error}"),
                }
            })
            .await;

            if let Err(error) = joined {
                eprintln!("digest drain: pass did not finish: {error}");
            }
        }
    }))
}

/// Expired Trash rows contain cached Gmail metadata and reviewed derivatives,
/// so cleanup runs independently of inbox sweeps. Gmail's own retention is not
/// modified here.
fn spawn_trash_cleanup() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                let store = Store::open(&cfg.database_path)
                    .map_err(|error| format!("store unavailable: {error}"))?;
                store
                    .purge_expired_trashed()
                    .map_err(|error| error.to_string())
            })
            .await;
            match joined {
                Ok(Ok(count)) if count > 0 => {
                    eprintln!("mail trash cleanup: purged {count} expired item(s)")
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("mail trash cleanup: {error}"),
                Err(error) => eprintln!("mail trash cleanup: pass did not finish: {error}"),
            }
        }
    })
}

/// The stored name this schedule keeps its run state under. Matches the
/// existing `source_state` convention rather than inventing a second one.
pub(super) const INBOX_SWEEP_SOURCE: &str = "gmail-inbox";

/// Unattended inbox collection. Off unless the overlay turns it on, bounded to
/// the newest N threads, and silent during quiet hours.
///
/// No persisted cursor, on purpose. A cursor that advances each pass walks
/// backwards through the entire mailbox over days, which is precisely the
/// unbounded rescan this is supposed to avoid; re-reading the newest page
/// instead is idempotent because proposals upsert on Gmail thread id and
/// preserve human decisions. Paging deeper stays a manual, cursor-carrying
/// call from the board.
fn spawn_inbox_sweep(
    every_minutes: u64,
    max_threads: usize,
    quiet: Option<(u32, u32)>,
) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("Inbox sweep schedule disabled (inbox_sweep_minutes = 0)");
        return None;
    }
    match quiet {
        Some((start, end)) => eprintln!(
            "Inbox sweep: every {every_minutes} min, newest {max_threads}, quiet {start:02}:00-{end:02}:00"
        ),
        None => eprintln!("Inbox sweep: every {every_minutes} min, newest {max_threads}"),
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;

                if let Some((start, end)) = quiet {
                    if store
                        .within_quiet_hours(start, end)
                        .map_err(|error| error.to_string())?
                    {
                        return Ok(None);
                    }
                }

                // Backoff is expressed as skipped ticks rather than a sleep, so
                // a recovered connector is picked up on the next ordinary tick
                // instead of after whatever long sleep was already running.
                let state = store
                    .get_source_state(INBOX_SWEEP_SOURCE)
                    .map_err(|error| error.to_string())?;
                let failures = state.map(|s| s.consecutive_failures).unwrap_or(0);
                if failures > 0 {
                    let skip_ticks = 1i64 << failures.min(5);
                    let elapsed_ticks = TICKS_SINCE_START.fetch_add(1, Ordering::Relaxed) as i64;
                    if elapsed_ticks % skip_ticks != 0 {
                        return Ok(None);
                    }
                }

                match run_inbox_sweep(&cfg, max_threads, None) {
                    Ok(mut outcome) => {
                        outcome.events = mail_events::analyze_batch(&cfg, &outcome.new_ids);
                        outcome.receipts = mail_receipts::analyze_batch(&cfg, &outcome.new_ids);
                        store
                            .record_sweep_success(
                                INBOX_SWEEP_SOURCE,
                                outcome.fetched as i64,
                                outcome.new_count as i64,
                            )
                            .map_err(|error| error.to_string())?;
                        // The registry state rides along because this pass
                        // persists rows unattended: with it absent the
                        // named-person rule escalates nothing, and the counts
                        // alone cannot tell that run from a quiet one.
                        Ok(Some(format!(
                            "{} considered, {} new, {} redacted, {} skipped, {} event proposal(s), \
                             {} paid-receipt candidate(s), {} event failure(s), receipt failures {:?}, people registry {} ({} names)",
                            outcome.fetched,
                            outcome.new_count,
                            outcome.redacted,
                            outcome.skipped,
                            outcome.events.proposals,
                            outcome.receipts.candidates,
                            outcome.events.failed,
                            outcome.receipts.failure_stages,
                            outcome.people_registry,
                            outcome.people_registry_names
                        )))
                    }
                    Err(error) => {
                        let class = sweep_error_class(&error);
                        let streak = store
                            .record_sweep_failure(INBOX_SWEEP_SOURCE, class)
                            .map_err(|error| error.to_string())?;
                        Err(format!("{class} error, {streak} in a row"))
                    }
                }
            })
            .await;
            match joined {
                Ok(Ok(Some(summary))) => eprintln!("Inbox sweep: {summary}"),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => eprintln!("Inbox sweep: {error}"),
                Err(error) => eprintln!("Inbox sweep: pass did not finish: {error}"),
            }
        }
    }))
}

/// Ticks since boot, so the backoff above can skip them without sleeping.
pub(super) static TICKS_SINCE_START: AtomicU64 = AtomicU64::new(0);

fn spawn_gmail_maintenance(every_minutes: u64) -> Option<tokio::task::JoinHandle<()>> {
    if every_minutes == 0 {
        eprintln!("Gmail maintenance disabled (gmail_maintenance_minutes = 0)");
        return None;
    }
    eprintln!("Gmail maintenance: every {every_minutes} min");
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(every_minutes * 60));
        loop {
            ticker.tick().await;
            let joined = tokio::task::spawn_blocking(|| {
                let cfg = Config::load();
                if !cfg.google_env_path.is_file() {
                    return Ok(None);
                }
                run_gmail_maintenance(&cfg, 200).map(Some)
            })
            .await;
            match joined {
                Ok(Ok(Some(counts)))
                    if counts.recovered > 0 || counts.changed > 0 || counts.retry_failures > 0 =>
                {
                    eprintln!(
                        "Gmail maintenance: {} recovered, {} reconciled changes, {} retry failures",
                        counts.recovered, counts.changed, counts.retry_failures
                    )
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("Gmail maintenance: {error}"),
                Err(error) => eprintln!("Gmail maintenance: pass did not finish: {error}"),
            }
        }
    }))
}

// The scheduling maths above is pure and testable; the module sits at the end of the
// file because clippy::items_after_test_module refuses items declared after a test
// module, and the spawn_* functions below it were exactly that.
#[cfg(test)]
mod drain_schedule_tests {
    use super::*;
    use std::time::Duration;

    /// Every instant this schedule fires within `passes` periods of spawn,
    /// measured from the shared spawn instant. Both drains are spawned in the
    /// same breath by [`BackgroundServices::start`], so a shared origin is not
    /// an idealisation — it is what actually happens.
    fn fire_times(every_minutes: u64, phase: f64, passes: u32) -> Vec<Duration> {
        let period = drain_period(every_minutes);
        let offset = phase_offset(period, phase);
        (0..passes).map(|n| offset + period * n).collect()
    }

    /// C16. Both drains prefill the same feed transcripts on the same local
    /// backend. `tokio::time::interval` fires immediately and then every
    /// period, so the two of them — spawned together, both defaulting to 15
    /// minutes — fired together on every single tick, forever.
    #[test]
    fn the_two_drains_never_fire_on_the_same_tick() {
        let enrichment = fire_times(15, ENRICHMENT_PHASE, 4);
        let digest = fire_times(15, DIGEST_PHASE, 4);
        assert!(
            enrichment.len() >= 2 && digest.len() >= 2,
            "the probe needs two intervals of each to say anything"
        );
        for at in &enrichment {
            assert!(
                !digest.contains(at),
                "both drains fire at {at:?}: {enrichment:?} / {digest:?}"
            );
        }
    }

    /// The offset is a fraction of the period, not a fixed number of minutes,
    /// so it survives someone setting the drains to something other than 15 —
    /// including a period short enough that a fixed offset would overrun it and
    /// land back on the other drain's tick.
    #[test]
    fn the_offset_holds_at_any_interval() {
        for minutes in [1_u64, 5, 15, 60, 240] {
            let enrichment = fire_times(minutes, ENRICHMENT_PHASE, 4);
            let digest = fire_times(minutes, DIGEST_PHASE, 4);
            for at in &enrichment {
                assert!(
                    !digest.contains(at),
                    "at {minutes} min both drains fire at {at:?}"
                );
            }
        }
    }

    /// Coinciding is congruence mod the period, so the two phases must differ
    /// by something that is not a whole period. Pinned as the property rather
    /// than as a pair of literals: setting `DIGEST_PHASE` to `1.0` would leave
    /// both constants looking staggered and both drains firing together again.
    #[test]
    fn the_two_phases_are_not_a_whole_period_apart() {
        let gap = (DIGEST_PHASE - ENRICHMENT_PHASE).abs();
        assert!(
            gap.fract() > f64::EPSILON,
            "phases {ENRICHMENT_PHASE} and {DIGEST_PHASE} are a whole period apart"
        );
    }

    /// The enrichment drain keeps its immediate first pass — a machine that
    /// booted with a backlog should start on it — and the digest drain must
    /// not, or the stagger only exists after the first fifteen minutes.
    #[test]
    fn only_one_drain_runs_at_boot() {
        assert_eq!(
            fire_times(15, ENRICHMENT_PHASE, 1),
            vec![Duration::ZERO],
            "the enrichment drain starts on its backlog immediately"
        );
        assert_eq!(
            fire_times(15, DIGEST_PHASE, 1),
            vec![Duration::from_secs(450)],
            "the digest drain waits half a period rather than joining it at boot"
        );
    }
}
