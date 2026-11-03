//! Collector run health, failure streaks, and quiet-hour decisions.

use super::*;

/// Rows in `{prefix}_source_state` that are receipts rather than collectors.
///
/// The freshness contract answers for arrivals, not for passes. `relevance-pass`
/// takes delivery of nothing, and `local-inference` is a model drain writing
/// `last_success_at` through `capacity::record_success` — so either of them
/// could hold `GET /__sjel/freshness` green with every collector dead, which is
/// the nine-days-of-an-empty-Inbox failure the endpoint exists to catch.
/// Two names, one const, one `NOT IN`.
pub(crate) const PSEUDO_SOURCES: [&str; 2] = ["local-inference", "relevance-pass"];

/// The row a relevance pass writes its receipt into.
pub(crate) const RELEVANCE_PASS_SOURCE: &str = "relevance-pass";

impl Store {
    // -- source_state ----------------------------------------------------

    pub fn record_run(
        &self,
        source_name: &str,
        cursor: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state (source_name, last_run_at, cursor)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = COALESCE(excluded.cursor, {prefix}_source_state.cursor)",
                prefix = self.prefix
            ),
            params![&source_name, &now, &cursor],
        )?;
        Ok(())
    }

    pub fn get_source_state(
        &self,
        source_name: &str,
    ) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT source_name, last_run_at, cursor, last_success_at, last_failure_at,
                            last_error, considered_count, new_count, consecutive_failures
                     FROM {}_source_state WHERE source_name = ?1",
                    self.prefix
                ),
                params![&source_name],
                |r| {
                    Ok(SourceState {
                        source_name: r.get(0)?,
                        last_run_at: r.get(1)?,
                        cursor: r.get(2)?,
                        last_success_at: r.get(3)?,
                        last_failure_at: r.get(4)?,
                        last_error: r.get(5)?,
                        considered_count: r.get(6)?,
                        new_count: r.get(7)?,
                        consecutive_failures: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// The most recent moment ANY declared source successfully collected, as epoch seconds.
    ///
    /// `last_success_at`, not `last_run_at`, and the distinction is the whole point: a run that
    /// failed still ran, so the run column stays fresh while nothing arrives. This answers the
    /// question the freshness contract asks — "is data still reaching this capability" — and a
    /// failing collector must not be able to answer it yes.
    ///
    /// The MAX across sources rather than per source: one working source means the capability is
    /// still being fed. A single source that has stopped is a narrower fault and belongs to the
    /// sources page, which already shows per-source state.
    ///
    /// `None` when no source has ever succeeded, which reads downstream as "never" rather than
    /// as "fresh" — the same way a missing backup receipt does.
    ///
    /// [`PSEUDO_SOURCES`] are excluded by name. The freshness contract answers for arrivals, not
    /// for passes: a relevance pass and a local-model drain both write into this table and
    /// neither one takes delivery of anything, so either could answer "yes, data is still
    /// reaching this capability" for a machine whose collectors have all stopped.
    ///
    /// `except` names more sources to leave out. The freshness handler passes the inbox sweep,
    /// which it judges as a producer of its own: mail and feed do not stand in for each other.
    pub fn newest_source_success(
        &self,
        except: &[&str],
    ) -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // Literals from this crate only (`PSEUDO_SOURCES`, `INBOX_SWEEP_SOURCE`), never input.
        let excluded = PSEUDO_SOURCES
            .iter()
            .chain(except)
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(",");
        Ok(conn.query_row(
            &format!(
                "SELECT MAX(CAST(last_success_at AS INTEGER)) FROM {}_source_state
                 WHERE last_success_at IS NOT NULL AND last_success_at <> ''
                   AND source_name NOT IN ({excluded})",
                self.prefix
            ),
            [],
            |r| r.get::<_, Option<i64>>(0),
        )?)
    }

    /// Record what a relevance pass did, WITHOUT `last_success_at`.
    ///
    /// Deliberately not `record_sweep_success`. That method writes
    /// `last_success_at`, which is the whole body of `GET /__sjel/freshness`,
    /// and a pass that re-scores stored rows has collected nothing. What it
    /// writes instead: when it ran, the mode that answered plus the relevance
    /// revision (the cursor), the counters, and — when a chunk fell back — a
    /// failure time, an error class and the streak.
    /// `cursor` is `<mode>|<completed relevance revision>|<in-progress>`, built
    /// by the caller because only the caller knows whether the page it just ran
    /// completed a sweep.
    pub fn record_relevance_pass(
        &self,
        cursor: &str,
        considered: i64,
        written: i64,
        error_class: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state
                     (source_name, last_run_at, cursor, considered_count, new_count,
                      last_failure_at, last_error, consecutive_failures)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = excluded.cursor,
                     considered_count = excluded.considered_count,
                     new_count = excluded.new_count,
                     last_failure_at = COALESCE(excluded.last_failure_at,
                                                {prefix}_source_state.last_failure_at),
                     last_error = excluded.last_error,
                     consecutive_failures = CASE WHEN excluded.last_error IS NULL THEN 0
                         ELSE {prefix}_source_state.consecutive_failures + 1 END",
                prefix = self.prefix
            ),
            params![
                &RELEVANCE_PASS_SOURCE,
                &now,
                &cursor,
                considered,
                written,
                &error_class.map(|_| now.clone()),
                &error_class,
                i32::from(error_class.is_some()),
            ],
        )?;
        Ok(())
    }

    /// The last relevance pass's receipt, for `GET /feed/evaluation/status`.
    pub fn relevance_pass(&self) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        self.get_source_state(RELEVANCE_PASS_SOURCE)
    }

    /// Record a completed pass. Success clears the failure streak; the counts
    /// describe the pass that just ran, not a running total, because "how much
    /// did the last run see" is the question a stale schedule raises.
    pub fn record_sweep_success(
        &self,
        source_name: &str,
        considered: i64,
        new_items: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = epoch_now();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state
                     (source_name, last_run_at, last_success_at, considered_count,
                      new_count, consecutive_failures)
                 VALUES (?1, ?2, ?2, ?3, ?4, 0)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_success_at = excluded.last_success_at,
                     considered_count = excluded.considered_count,
                     new_count = excluded.new_count,
                     consecutive_failures = 0,
                     last_error = NULL",
                prefix = self.prefix
            ),
            params![&source_name, &now, considered, new_items],
        )?;
        Ok(())
    }

    /// `error_class` is a short stable label — `auth`, `quota`, `network`,
    /// `store`. Never a provider message: those quote request URLs and, for
    /// mail, occasionally the subject that failed.
    pub fn record_sweep_failure(
        &self,
        source_name: &str,
        error_class: &str,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let now = epoch_now();
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "INSERT INTO {prefix}_source_state
                     (source_name, last_run_at, last_failure_at, last_error, consecutive_failures)
                 VALUES (?1, ?2, ?2, ?3, 1)
                 ON CONFLICT (source_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     last_failure_at = excluded.last_failure_at,
                     last_error = excluded.last_error,
                     consecutive_failures = {prefix}_source_state.consecutive_failures + 1
                 RETURNING consecutive_failures",
                prefix = self.prefix
            ),
            params![&source_name, &now, &error_class],
            |row| row.get(0),
        )?)
    }

    /// Whether the store's clock currently sits inside a quiet window, given
    /// `[start, end)` in UTC hours. Asked of the store rather than computed in
    /// Rust: its clock is the one every other timestamp here comes from, and
    /// comms carries no date library to disagree with it. UTC, because the
    /// Postgres instance this replaced ran in UTC and `EXTRACT(HOUR FROM now())`
    /// therefore answered the UTC hour -- `strftime('%H','now')` answers the
    /// same one.
    pub fn within_quiet_hours(
        &self,
        start_hour: u32,
        end_hour: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if start_hour == end_hour {
            return Ok(false);
        }
        let conn = self.conn()?;
        let hour: i32 =
            conn.query_row("SELECT CAST(strftime('%H','now') AS INTEGER)", [], |row| {
                row.get(0)
            })?;
        let (start, end) = (start_hour as i32, end_hour as i32);
        // A window that wraps midnight (22→7) is the normal case, so it is the
        // one spelled out rather than the one left to fall through.
        Ok(if start < end {
            hour >= start && hour < end
        } else {
            hour >= start || hour < end
        })
    }
}

#[cfg(test)]
mod db_tests {
    use crate::store::db_tests::open_test_store;

    /// The D13 regression guard.
    ///
    /// `newest_source_success` is the entire body of `GET /__sjel/freshness`, and
    /// doctor compares its answer against comms' `freshness_stale_hours`. A pass
    /// that takes delivery of nothing must not be able to answer it — and neither
    /// must the local-model drain, which writes `last_success_at` today.
    /// The freshness handler judges the inbox sweep apart from the feed, so a mail success must
    /// not answer for a feed that stopped.
    #[test]
    fn an_excepted_source_does_not_answer_for_the_rest() {
        let store = open_test_store("freshness_except");
        store
            .record_sweep_success("gmail-inbox", 25, 1)
            .expect("the sweep succeeds");
        assert!(store.newest_source_success(&[]).unwrap().is_some());
        assert_eq!(store.newest_source_success(&["gmail-inbox"]).unwrap(), None);
    }

    #[test]
    fn a_relevance_pass_does_not_move_freshness() {
        let store = open_test_store("freshness_pseudo_sources");
        store
            .record_sweep_success("github-trending-daily", 10, 3)
            .expect("a collector succeeds");
        let collected = store
            .newest_source_success(&[])
            .expect("a freshness answer");
        assert!(collected.is_some(), "a real collector answers the contract");

        store
            .record_relevance_pass("semantic|relevance-revision|:0", 372, 12, None)
            .expect("the receipt writes");
        assert_eq!(
            store
                .newest_source_success(&[])
                .expect("a freshness answer"),
            collected,
            "a relevance pass takes delivery of nothing and must not answer for arrivals"
        );

        store
            .record_sweep_success(crate::capacity::LOCAL_INFERENCE_SOURCE, 0, 0)
            .expect("the local model drain succeeds");
        assert_eq!(
            store
                .newest_source_success(&[])
                .expect("a freshness answer"),
            collected,
            "a machine talking to itself cannot satisfy its own health contract"
        );

        let receipt = store.relevance_pass().expect("read back").expect("a row");
        assert!(
            receipt.last_success_at.is_none(),
            "the receipt must never write last_success_at"
        );
        assert_eq!(
            receipt.cursor.as_deref(),
            Some("semantic|relevance-revision|:0")
        );
        assert_eq!(receipt.consecutive_failures, 0);

        store
            .record_relevance_pass(
                "lexical|relevance-revision|:0",
                372,
                0,
                Some("embedding-unreachable"),
            )
            .expect("a degraded pass writes too");
        let degraded = store.relevance_pass().expect("read back").expect("a row");
        assert_eq!(
            degraded.last_error.as_deref(),
            Some("embedding-unreachable")
        );
        assert_eq!(degraded.consecutive_failures, 1);
        assert!(degraded.last_success_at.is_none());
    }
}
