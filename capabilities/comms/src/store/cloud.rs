//! Reviewed cloud derivatives, dispatch jobs, and content digests.

use super::*;

/// The eleven columns the queue CTE returns, named so the mapper closure does not
/// have to carry eleven positional `get`s into the caller's scope.
struct QueuedJobRow {
    job_id: String,
    provider_role: String,
    queued_at: String,
    approved_at: String,
    dispatch_status: String,
    provider_calls: i32,
    task: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    last_error: Option<String>,
    result_json: Option<String>,
}

impl Store {
    pub fn stage_cloud_derivative(
        &self,
        approval: &CloudDerivativeApproval,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        if !matches!(approval.source.as_str(), "feed" | "mail") {
            return Err("cloud derivative source must be 'feed' or 'mail'".into());
        }
        if !crate::content_item::valid(&approval.original_data_class) {
            return Err("cloud derivative has an invalid original data class".into());
        }
        if !matches!(approval.derivative_data_class.as_str(), "c0" | "c1") {
            return Err("derivative data class must be one of: c0, c1".into());
        }

        let conn = self.conn()?;
        let (preview_hash, approved_at) = conn.query_row(
            &format!(
                "INSERT INTO {prefix}_content_cloud_derivatives
                    (source, item_id, source_revision, preview_hash,
                     original_data_class, derivative_data_class, transformation,
                     document, redaction_count, approved_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,{now})
                 ON CONFLICT (source, item_id) DO UPDATE SET
                    source_revision = excluded.source_revision,
                    preview_hash = excluded.preview_hash,
                    original_data_class = excluded.original_data_class,
                    derivative_data_class = excluded.derivative_data_class,
                    transformation = excluded.transformation,
                    document = excluded.document,
                    redaction_count = excluded.redaction_count,
                    approved_at = {now}
                 RETURNING preview_hash, approved_at",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            params![
                &approval.source,
                &approval.item_id,
                &approval.source_revision,
                &approval.preview_hash,
                &approval.original_data_class,
                &approval.derivative_data_class,
                &approval.transformation,
                &approval.document,
                approval.redaction_count,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        Ok(CloudDerivativeState {
            status: "staged".into(),
            preview_hash: Some(preview_hash),
            approved_at: Some(approved_at),
            dispatch_status: "not_queued".into(),
            job_id: None,
            provider_role: None,
            queued_at: None,
            provider_calls: 0,
            task: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            result: None,
        })
    }

    pub fn queue_cloud_derivative(
        &self,
        request: &CloudQueueRequest,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        if !matches!(request.source.as_str(), "feed" | "mail") {
            return Err("cloud queue source must be 'feed' or 'mail'".into());
        }
        if !request.provider_role.starts_with("cloud_") {
            return Err("cloud queue provider role must start with 'cloud_'".into());
        }
        if !matches!(
            request.task.as_str(),
            crate::cloud_dispatch::TASK_VERSION | crate::cloud_dispatch::DIGEST_TASK_VERSION
        ) {
            return Err("cloud queue task is unsupported".into());
        }

        let job_id = cloud_job_id(request);
        let mut conn = self.conn()?;
        // Two statements inside one transaction, where Postgres had one
        // data-modifying CTE. SQLite does not allow INSERT inside a WITH clause,
        // and the CTE was buying exactly one thing here: queue only what a review
        // approved, with no window in which the approval could vanish between the
        // check and the insert. A transaction buys that, and the read still has to
        // match all four columns, so a stale hash is refused the way it was.
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let approved_at = transaction
            .query_row(
                &format!(
                    "SELECT approved_at
                     FROM {prefix}_content_cloud_derivatives
                     WHERE source = ?1 AND item_id = ?2
                       AND source_revision = ?3 AND preview_hash = ?4",
                    prefix = self.prefix
                ),
                params![
                    &request.source,
                    &request.item_id,
                    &request.source_revision,
                    &request.preview_hash,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let row = match approved_at {
            None => None,
            Some(approved_at) => Some(transaction.query_row(
                &format!(
                    "INSERT INTO {prefix}_content_cloud_jobs
                            (job_id, source, item_id, source_revision, preview_hash,
                             provider_role, task)
                         VALUES (?5, ?1, ?2, ?3, ?4, ?6, ?7)
                         ON CONFLICT (source, item_id, preview_hash, provider_role, task)
                         DO UPDATE SET provider_role = excluded.provider_role
                         RETURNING job_id, provider_role, queued_at, status,
                                   provider_calls, task, started_at, completed_at,
                                   last_error, result_json",
                    prefix = self.prefix
                ),
                params![
                    &request.source,
                    &request.item_id,
                    &request.source_revision,
                    &request.preview_hash,
                    &job_id,
                    &request.provider_role,
                    &request.task,
                ],
                |row| {
                    Ok(QueuedJobRow {
                        job_id: row.get(0)?,
                        provider_role: row.get(1)?,
                        queued_at: row.get(2)?,
                        approved_at: approved_at.clone(),
                        dispatch_status: row.get(3)?,
                        provider_calls: row.get(4)?,
                        task: row.get(5)?,
                        started_at: row.get(6)?,
                        completed_at: row.get(7)?,
                        last_error: row.get(8)?,
                        result_json: row.get(9)?,
                    })
                },
            )?),
        };
        transaction.commit()?;
        let Some(row) = row else {
            return Err("approved derivative is missing or stale; review it again".into());
        };
        Ok(CloudDerivativeState {
            status: "staged".into(),
            preview_hash: Some(request.preview_hash.clone()),
            approved_at: Some(row.approved_at),
            dispatch_status: row.dispatch_status,
            job_id: Some(row.job_id),
            provider_role: Some(row.provider_role),
            queued_at: Some(row.queued_at),
            provider_calls: row.provider_calls.try_into()?,
            task: Some(row.task),
            started_at: row.started_at,
            completed_at: row.completed_at,
            last_error: row.last_error,
            result: row
                .result_json
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
        })
    }

    /// Every digest that actually produced text, oldest producer grouping first.
    ///
    /// For the quality corpus (PRD D16) and for nothing else so far. `state = 'generated'`
    /// only: a skipped, refused or errored row has no text to judge, and counting it as a bad
    /// digest would measure the ladder's routing rather than its output.
    ///
    /// Ordered by producer then item, so a corpus exported twice from an unchanged database
    /// is byte-identical and a re-export is a readable diff.
    pub fn generated_digests(&self) -> Result<Vec<StoredDigest>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT source, item_id, text, state, shape, depth, focus, producer,
                        source_chars, redactions, attempts, last_error,
                        diagram, diagram_state, diagram_error,
                        chart, chart_state, chart_error, generated_at
                 FROM {}_content_digests
                 WHERE state = 'generated' AND text IS NOT NULL AND TRIM(text) <> ''
                 ORDER BY producer, source, item_id",
                self.prefix
            ),
            [],
            |row| {
                Ok(StoredDigest {
                    source: row.get(0)?,
                    item_id: row.get(1)?,
                    text: row.get(2)?,
                    state: row.get(3)?,
                    shape: row.get(4)?,
                    depth: row.get(5)?,
                    focus: row.get(6)?,
                    producer: row.get(7)?,
                    source_chars: row.get(8)?,
                    redactions: row.get(9)?,
                    attempts: row.get(10)?,
                    last_error: row.get(11)?,
                    diagram: row.get(12)?,
                    diagram_state: row.get(13)?,
                    diagram_error: row.get(14)?,
                    chart: row.get(15)?,
                    chart_state: row.get(16)?,
                    chart_error: row.get(17)?,
                    generated_at: row.get(18)?,
                })
            },
        )?)
    }

    /// The stored digest for one item, if it has one.
    pub fn content_digest(
        &self,
        source: &str,
        item_id: &str,
    ) -> Result<Option<StoredDigest>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT source, item_id, text, state, shape, depth, focus, producer,
                            source_chars, redactions, attempts, last_error,
                            diagram, diagram_state, diagram_error,
                            chart, chart_state, chart_error, generated_at
                     FROM {}_content_digests
                     WHERE source = ?1 AND item_id = ?2",
                    self.prefix
                ),
                params![&source, &item_id],
                |row| {
                    Ok(StoredDigest {
                        source: row.get(0)?,
                        item_id: row.get(1)?,
                        text: row.get(2)?,
                        state: row.get(3)?,
                        shape: row.get(4)?,
                        depth: row.get(5)?,
                        focus: row.get(6)?,
                        producer: row.get(7)?,
                        source_chars: row.get(8)?,
                        redactions: row.get(9)?,
                        attempts: row.get(10)?,
                        last_error: row.get(11)?,
                        diagram: row.get(12)?,
                        diagram_state: row.get(13)?,
                        diagram_error: row.get(14)?,
                        chart: row.get(15)?,
                        chart_state: row.get(16)?,
                        chart_error: row.get(17)?,
                        generated_at: row.get(18)?,
                    })
                },
            )
            .optional()?)
    }

    /// Replace an item's digest.
    ///
    /// Replace rather than append: a digest is the current best answer, not a
    /// history. `attempts` is carried by the caller so a failing row accumulates
    /// its own count instead of resetting every pass.
    pub fn upsert_content_digest(
        &self,
        digest: &StoredDigest,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                // The backoff is computed here, from `{now}` and the attempt
                // count already being written, rather than passed in: the
                // deadline must be on the database's clock, and deriving it
                // from the row's own state is what keeps it from disagreeing
                // with `attempts`. A non-retryable state clears it — a success
                // or a verdict has no next attempt to schedule.
                "INSERT INTO {prefix}_content_digests
                     (source, item_id, text, state, shape, depth, focus, producer,
                      source_chars, redactions, attempts, last_error, generated_at, next_attempt)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12, {now},
                     CASE WHEN ?4 IN ({retryable})
                          THEN {backoff}
                          ELSE NULL END)
                 ON CONFLICT (source, item_id) DO UPDATE SET
                     text = EXCLUDED.text,
                     state = EXCLUDED.state,
                     shape = EXCLUDED.shape,
                     depth = EXCLUDED.depth,
                     focus = EXCLUDED.focus,
                     producer = EXCLUDED.producer,
                     source_chars = EXCLUDED.source_chars,
                     redactions = EXCLUDED.redactions,
                     attempts = EXCLUDED.attempts,
                     last_error = EXCLUDED.last_error,
                     generated_at = {now},
                     next_attempt = EXCLUDED.next_attempt",
                prefix = self.prefix,
                retryable = retryable_digest_states_sql(),
                now = sjel_store::NOW,
                backoff =
                    sjel_store::now_offset("'+' || (5 * (1 << MAX(?11 - 1, 0))) || ' minutes'")
            ),
            params![
                &digest.source,
                &digest.item_id,
                &digest.text,
                &digest.state,
                &digest.shape,
                &digest.depth,
                &digest.focus,
                &digest.producer,
                &digest.source_chars,
                &digest.redactions,
                &digest.attempts,
                &digest.last_error,
            ],
        )?;
        Ok(())
    }

    /// Attach or clear the Mermaid diagram beside an existing digest.
    ///
    /// Separate from [`Store::upsert_content_digest`] because the two are
    /// separate presses: regenerating a digest must not silently discard a
    /// diagram the operator already asked for, and vice versa.
    pub fn update_content_diagram(
        &self,
        source: &str,
        item_id: &str,
        diagram: Option<&str>,
        state: &str,
        error: Option<&str>,
        producer: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let updated = conn.execute(
            &format!(
                "UPDATE {}_content_digests
                    SET diagram = ?3, diagram_state = ?4, diagram_error = ?5, diagram_producer = ?6
                  WHERE source = ?1 AND item_id = ?2",
                self.prefix
            ),
            params![&source, &item_id, &diagram, &state, &error, &producer],
        )?;
        Ok(updated as u64)
    }

    /// Attach or clear the extracted chart table beside an existing digest.
    ///
    /// Its own press, like the diagram: regenerating a digest must not discard
    /// a figure the operator already asked for.
    pub fn update_content_chart(
        &self,
        source: &str,
        item_id: &str,
        chart: Option<&str>,
        state: &str,
        error: Option<&str>,
        producer: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let updated = conn.execute(
            &format!(
                "UPDATE {}_content_digests
                    SET chart = ?3, chart_state = ?4, chart_error = ?5, chart_producer = ?6
                  WHERE source = ?1 AND item_id = ?2",
                self.prefix
            ),
            params![&source, &item_id, &chart, &state, &error, &producer],
        )?;
        Ok(updated as u64)
    }

    /// Items of one source that the automatic pass should still digest.
    ///
    /// Three reasons a row qualifies: it has none, its producer is stale, or it
    /// failed retryably, has attempts left, and its backoff window has elapsed.
    /// The `depth = 'standard'` guard on the stale case is load-bearing — an
    /// operator who pressed *detailed* has made a decision, and a model upgrade
    /// must not quietly overwrite it with the automatic rung.
    ///
    /// The backoff clause is what makes a *timed* drain safe. With the attempt
    /// cap alone, a drain every 15 minutes spends all three attempts inside
    /// three-quarters of an hour, so an outage lasting an hour leaves the row
    /// permanently dead — which is the failure this whole path exists to end.
    ///
    /// `producers` is every producer string this machine could currently write,
    /// not one: the role is chosen per item, so a short source may be digested
    /// by a light model and a long one by the strong model on the same pass and
    /// both are current. A row is stale only when its producer is in none of
    /// them.
    ///
    /// `unattended_producers` is the subset an *automatic* pass can produce —
    /// the light local role and the cloud roles, never the strong local one.
    /// The attempt cap only applies against those. A row that spent its three
    /// attempts on a model this pass will not use has not spent this pass's
    /// budget, and treating it as though it had is how six long public items on
    /// this machine ended up parked at `http_error`, attempt 4, against an oMLX
    /// that had been stopped for good — permanently invisible to the cloud rung
    /// that could have digested every one of them. The row is rewritten with an
    /// unattended producer the moment this pass touches it, so the ordinary cap
    /// governs from then on and this cannot loop.
    pub fn items_needing_digest(
        &self,
        source: &str,
        producers: &[String],
        unattended_producers: &[String],
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let (table, order) = match source {
            "mail" => ("triage_items", "internal_date DESC NULLS LAST"),
            "feed" => ("feed_items", "created_at DESC"),
            other => return Err(format!("no digest queue for source {other:?}").into()),
        };
        let conn = self.conn()?;
        // `<> ALL($2)` bound a Postgres array. SQLite binds one value per
        // placeholder, so the producer lists cross as JSON arrays and the
        // predicate becomes a NOT IN over `json_each` -- one statement with a
        // fixed parameter count, where an assembled `NOT IN (?,?,...)` would make
        // the placeholder count depend on how many roles this machine has.
        Ok(conn.query_all(
            &format!(
                "SELECT i.id
                   FROM {prefix}_{table} i
                   LEFT JOIN {prefix}_content_digests d
                          ON d.source = ?1 AND d.item_id = i.id
                  WHERE d.item_id IS NULL
                     OR (d.producer NOT IN (SELECT value FROM json_each(?2))
                         AND d.depth = 'standard')
                     OR (d.state IN ({retryable})
                         AND (d.producer NOT IN (SELECT value FROM json_each(?3))
                              OR d.attempts < ?4)
                         AND (d.next_attempt IS NULL OR d.next_attempt <= {now}))
                  ORDER BY i.{order}
                  LIMIT ?5",
                prefix = self.prefix,
                table = table,
                order = order,
                retryable = retryable_digest_states_sql(),
                now = sjel_store::NOW
            ),
            params![
                &source,
                &serde_json::to_string(producers)?,
                &serde_json::to_string(unattended_producers)?,
                max_attempts,
                limit,
            ],
            |row| row.get::<_, String>(0),
        )?)
    }

    pub fn cloud_derivative_state(
        &self,
        source: &str,
        item_id: &str,
        current_source_revision: &str,
        current_preview_hash: &str,
    ) -> Result<CloudDerivativeState, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let staged = conn
            .query_row(
                &format!(
                    "SELECT source_revision, preview_hash, approved_at
                     FROM {}_content_cloud_derivatives
                     WHERE source = ?1 AND item_id = ?2",
                    self.prefix
                ),
                params![&source, &item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(match staged {
            None => CloudDerivativeState::not_prepared(),
            Some((source_revision, preview_hash, approved_at)) => {
                let current = source_revision == current_source_revision
                    && preview_hash == current_preview_hash;
                let job = if current {
                    conn.query_row(
                        &format!(
                            "SELECT job_id, provider_role, queued_at, status,
                                    provider_calls, task, started_at, completed_at,
                                    last_error, result_json
                             FROM {}_content_cloud_jobs
                             WHERE source = ?1 AND item_id = ?2
                               AND source_revision = ?3 AND preview_hash = ?4
                             ORDER BY queued_at DESC LIMIT 1",
                            self.prefix
                        ),
                        params![&source, &item_id, &source_revision, &preview_hash],
                        |job| {
                            Ok(QueuedJobRow {
                                job_id: job.get(0)?,
                                provider_role: job.get(1)?,
                                queued_at: job.get(2)?,
                                approved_at: String::new(),
                                dispatch_status: job.get(3)?,
                                provider_calls: job.get(4)?,
                                task: job.get(5)?,
                                started_at: job.get(6)?,
                                completed_at: job.get(7)?,
                                last_error: job.get(8)?,
                                result_json: job.get(9)?,
                            })
                        },
                    )
                    .optional()?
                } else {
                    None
                };
                CloudDerivativeState {
                    status: if current {
                        "staged".into()
                    } else {
                        "stale".into()
                    },
                    preview_hash: Some(preview_hash),
                    approved_at: Some(approved_at),
                    dispatch_status: job
                        .as_ref()
                        .map(|job| job.dispatch_status.clone())
                        .unwrap_or_else(|| "not_queued".to_string()),
                    job_id: job.as_ref().map(|job| job.job_id.clone()),
                    provider_role: job.as_ref().map(|job| job.provider_role.clone()),
                    queued_at: job.as_ref().map(|job| job.queued_at.clone()),
                    provider_calls: job
                        .as_ref()
                        .map(|job| job.provider_calls.try_into())
                        .transpose()?
                        .unwrap_or(0),
                    task: job.as_ref().map(|job| job.task.clone()),
                    started_at: job.as_ref().and_then(|job| job.started_at.clone()),
                    completed_at: job.as_ref().and_then(|job| job.completed_at.clone()),
                    last_error: job.as_ref().and_then(|job| job.last_error.clone()),
                    result: job
                        .as_ref()
                        .and_then(|job| job.result_json.clone())
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                }
            }
        })
    }

    /// The job a dispatch is about to run, with **both** classes: the one frozen
    /// into the staged derivative and the one its source row carries now.
    ///
    /// The source row is joined for that second value alone (T3, review finding
    /// R3). Nothing invalidates a staged derivative or its queued job when the
    /// row behind it is reclassified — no reclassification path touches
    /// `_content_cloud_derivatives` or `_cloud_jobs` — so before this join a
    /// derivative approved while a mail was `c1` stayed dispatchable after the
    /// mail became `c2`. `LEFT JOIN` per source rather than a join that could
    /// drop the row: a job whose item was purged must be *refused* by the class
    /// check, not silently reported as "no such job", which reads as done.
    pub fn cloud_job_for_dispatch(
        &self,
        job_id: &str,
    ) -> Result<Option<CloudDispatchJob>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT j.job_id, j.source, j.item_id, j.source_revision,
                        j.preview_hash, j.provider_role, j.task,
                        d.original_data_class, d.derivative_data_class,
                        d.transformation, d.document, j.provider_calls,
                        COALESCE(f.data_class, t.data_class)
                 FROM {prefix}_content_cloud_jobs j
                 JOIN {prefix}_content_cloud_derivatives d
                   ON d.source = j.source AND d.item_id = j.item_id
                  AND d.source_revision = j.source_revision
                  AND d.preview_hash = j.preview_hash
                 LEFT JOIN {prefix}_feed_items f
                   ON j.source = 'feed' AND f.id = j.item_id
                 LEFT JOIN {prefix}_triage_items t
                   ON j.source = 'mail' AND t.id = j.item_id
                 WHERE j.job_id = ?1
                   AND (j.status IN ('queued','failed')
                     OR (j.status = 'running' AND j.started_at < {stale}))
                   AND j.provider_calls < 5",
                    prefix = self.prefix,
                    stale = sjel_store::now_offset("'-5 minutes'")
                ),
                params![&job_id],
                |row| {
                    Ok(CloudDispatchJob {
                        job_id: row.get(0)?,
                        source: row.get(1)?,
                        item_id: row.get(2)?,
                        source_revision: row.get(3)?,
                        preview_hash: row.get(4)?,
                        provider_role: row.get(5)?,
                        task: row.get(6)?,
                        original_data_class: row.get(7)?,
                        derivative_data_class: row.get(8)?,
                        transformation: row.get(9)?,
                        document: row.get(10)?,
                        provider_calls: row.get(11)?,
                        current_source_class: row.get(12)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn cloud_provider_calls_today(
        &self,
        provider_role: &str,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `(started_at AT TIME ZONE 'UTC')::date` was Postgres normalising a
        // timestamptz before taking its day. The stored stamp is already UTC and
        // already text, so the day is its first ten characters -- and `date('now')`
        // is UTC too, which is what the old expression resolved to.
        let calls: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*)
                 FROM {}_content_cloud_attempts
                 WHERE provider_role = ?1
                   AND substr(started_at, 1, 10) = date('now')",
                self.prefix
            ),
            params![&provider_role],
            |row| row.get(0),
        )?;
        Ok(calls.try_into()?)
    }

    /// Failures and successes this role has recorded in the last `window_minutes`.
    ///
    /// The evidence a caller needs to stop leading with a provider that is refusing. The daily
    /// count above is a budget question -- "may I spend another" -- and answers nothing about
    /// whether spending it would work: on 2026-08-30 Cloudflare answered 429 to 119 consecutive
    /// requests while its budget said 81 remaining, because every new job re-selected it from
    /// the top of a roster that had no memory of the last one.
    ///
    /// Read from the attempts ledger rather than stored as provider state, so there is one
    /// record of what happened and nothing to keep in step with it.
    pub fn cloud_provider_recent_outcomes(
        &self,
        provider_role: &str,
        window_minutes: i64,
    ) -> Result<(u32, u32), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let (failed, succeeded): (i64, i64) = conn.query_row(
            &format!(
                "SELECT
                   COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
                   COALESCE(SUM(CASE WHEN status = 'succeeded' THEN 1 ELSE 0 END), 0)
                 FROM {}_content_cloud_attempts
                 WHERE provider_role = ?1
                   AND started_at >= datetime('now', ?2)",
                self.prefix
            ),
            params![&provider_role, &format!("-{window_minutes} minutes")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((failed.try_into()?, succeeded.try_into()?))
    }

    pub fn utc_date(&self) -> Result<String, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT date('now')", [], |row| row.get(0))?)
    }

    pub fn claim_cloud_job_attempt(
        &self,
        job_id: &str,
        provider_role: &str,
        model: &str,
        max_requests_per_day: u32,
    ) -> Result<CloudAttemptClaim, Box<dyn std::error::Error>> {
        if !provider_role.starts_with("cloud_") || model.trim().is_empty() {
            return Err("cloud attempt requires a reviewed role and model".into());
        }
        let mut conn = self.conn()?;
        // `pg_advisory_xact_lock` served one purpose here: serialize budget
        // decisions so two jobs cannot both observe the final free slot and
        // exceed the local hard ceiling. SQLite admits one writer, so the lock
        // it needs is the write lock -- taken up front with BEGIN IMMEDIATE
        // rather than upgraded into after the count, because a deferred
        // transaction that reads then writes answers a failed upgrade with
        // SQLITE_BUSY and `busy_timeout` deliberately does not retry that.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let calls: i64 = transaction.query_row(
            &format!(
                "SELECT COUNT(*)
                 FROM {}_content_cloud_attempts
                 WHERE provider_role = ?1
                   AND substr(started_at, 1, 10) = date('now')",
                self.prefix
            ),
            params![&provider_role],
            |row| row.get(0),
        )?;
        if calls >= i64::from(max_requests_per_day) {
            return Ok(CloudAttemptClaim::DailyLimitReached);
        }

        let claimed = transaction
            .query_row(
                &format!(
                    "UPDATE {}_content_cloud_jobs
                     SET status = 'running', provider_calls = provider_calls + 1,
                         started_at = {now}, completed_at = NULL,
                         last_error = NULL, result_json = NULL
                     WHERE job_id = ?1
                       AND (status IN ('queued','failed')
                         OR (status = 'running' AND started_at < {stale}))
                       AND provider_calls < 5
                     RETURNING provider_calls, preview_hash",
                    self.prefix,
                    now = sjel_store::NOW,
                    stale = sjel_store::now_offset("'-5 minutes'")
                ),
                params![&job_id],
                |row| Ok((row.get::<_, i32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((sequence, preview_hash)) = claimed else {
            return Ok(CloudAttemptClaim::JobUnavailable);
        };
        let attempt_id = transaction.query_row(
            &format!(
                "INSERT INTO {}_content_cloud_attempts
                    (job_id, sequence, provider_role, model, preview_hash)
                 VALUES (?1,?2,?3,?4,?5)
                 RETURNING attempt_id",
                self.prefix
            ),
            params![&job_id, sequence, &provider_role, &model, &preview_hash],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok(CloudAttemptClaim::Started(attempt_id))
    }

    pub fn complete_cloud_job_attempt(
        &self,
        job_id: &str,
        attempt_id: i64,
        result: &serde_json::Value,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let result = serde_json::to_string(result)?;
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let attempt_updated = transaction.execute(
            &format!(
                "UPDATE {}_content_cloud_attempts
                 SET status = 'succeeded', result_json = ?3,
                     completed_at = {now}, last_error = NULL
                 WHERE attempt_id = ?2 AND job_id = ?1 AND status = 'running'",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id, &attempt_id, &result],
        )?;
        let job_updated = transaction.execute(
            &format!(
                "UPDATE {}_content_cloud_jobs
                 SET status = 'succeeded', result_json = ?2,
                     completed_at = {now}, last_error = NULL
                 WHERE job_id = ?1 AND status = 'running'",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id, &result],
        )?;
        if attempt_updated == 1 && job_updated == 1 {
            transaction.commit()?;
            Ok(true)
        } else {
            transaction.rollback()?;
            Ok(false)
        }
    }

    pub fn fail_cloud_job_attempt(
        &self,
        job_id: &str,
        attempt_id: i64,
        error: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let error: String = error.chars().take(500).collect();
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let attempt_updated = transaction.execute(
            &format!(
                "UPDATE {}_content_cloud_attempts
                 SET status = 'failed', last_error = ?3, completed_at = {now}
                 WHERE attempt_id = ?2 AND job_id = ?1 AND status = 'running'",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id, &attempt_id, &error],
        )?;
        let job_updated = transaction.execute(
            &format!(
                "UPDATE {}_content_cloud_jobs
                 SET status = 'failed', last_error = ?2, completed_at = {now}
                 WHERE job_id = ?1 AND status = 'running'",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id, &error],
        )?;
        if attempt_updated == 1 && job_updated == 1 {
            transaction.commit()?;
            Ok(true)
        } else {
            transaction.rollback()?;
            Ok(false)
        }
    }

    pub fn record_egress(&self, entry: &NewEgressEntry) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let id = conn.query_row(
            &format!(
                "INSERT INTO {prefix}_egress_log
                    (job_id, task, provider, provider_role, model, data_class, preview_hash,
                     document_payload, prompt_tokens, completion_tokens, total_tokens,
                     cost_cents, status, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 RETURNING id",
                prefix = self.prefix
            ),
            params![
                entry.job_id,
                entry.task,
                entry.provider,
                entry.provider_role,
                entry.model,
                entry.data_class,
                entry.preview_hash,
                entry.document_payload,
                entry.prompt_tokens,
                entry.completion_tokens,
                entry.total_tokens,
                entry.cost_cents,
                entry.status,
                entry.error,
            ],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn list_egress_entries(
        &self,
        limit: usize,
    ) -> Result<Vec<EgressEntry>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, timestamp, job_id, task, provider, provider_role, model,
                        data_class, preview_hash, document_payload, prompt_tokens,
                        completion_tokens, total_tokens, cost_cents, status, error
                 FROM {prefix}_egress_log
                 ORDER BY id DESC
                 LIMIT ?1",
            prefix = self.prefix
        ))?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(EgressEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                job_id: row.get(2)?,
                task: row.get(3)?,
                provider: row.get(4)?,
                provider_role: row.get(5)?,
                model: row.get(6)?,
                data_class: row.get(7)?,
                preview_hash: row.get(8)?,
                document_payload: row.get(9)?,
                prompt_tokens: row.get(10)?,
                completion_tokens: row.get(11)?,
                total_tokens: row.get(12)?,
                cost_cents: row.get(13)?,
                status: row.get(14)?,
                error: row.get(15)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn egress_audit(&self) -> Result<EgressAuditReport, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, prompt_tokens, completion_tokens, total_tokens, cost_cents,
                        status, data_class, document_payload
                 FROM {prefix}_egress_log
                 ORDER BY id ASC",
            prefix = self.prefix
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;

        let registry = crate::people_registry::entity_registry();
        let mut report = EgressAuditReport {
            total_calls: 0,
            succeeded_calls: 0,
            failed_calls: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_tokens: 0,
            total_cost_cents: 0.0,
            raw_c2_violations: Vec::new(),
        };

        for row in rows {
            let (id, p_tok, c_tok, tot_tok, cost, status, data_class, doc) = row?;
            report.total_calls += 1;
            if status == "succeeded" {
                report.succeeded_calls += 1;
            } else {
                report.failed_calls += 1;
            }
            report.total_prompt_tokens += p_tok as u64;
            report.total_completion_tokens += c_tok as u64;
            report.total_tokens += tot_tok as u64;
            report.total_cost_cents += cost;

            // Audit for C2 raw entity leakage (PRD §6, Product Rule 4, ISC-22):
            // 1. Check against known people in operator's registry
            let person_matches = registry.find_matches(&doc);
            for m in person_matches {
                report.raw_c2_violations.push(format!(
                    "row {id}: payload leaked raw person name {:?} at offset {}-{} (data class {data_class})",
                    m.original_matched, m.start, m.end
                ));
            }
            // 2. Check for raw unpseudonymized email or IBAN if personal (c1)
            if data_class == "c1" {
                for word in doc.split_whitespace() {
                    if sjel_pseudonymize::pattern::looks_like_email(word) && !word.starts_with('<')
                    {
                        report.raw_c2_violations.push(format!(
                            "row {id}: payload leaked raw email address {word:?} in c1 derivative"
                        ));
                    }
                    if sjel_pseudonymize::pattern::looks_like_iban(word) {
                        report.raw_c2_violations.push(format!(
                            "row {id}: payload leaked raw IBAN {word:?} in c1 derivative"
                        ));
                    }
                }
            }
        }

        Ok(report)
    }
}
