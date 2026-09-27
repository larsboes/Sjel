//! Feed ingest, normalized content, review signals, and relevance.

use super::*;

impl Store {
    // -- feed ------------------------------------------------------------

    pub const FEED_STATUSES: [&'static str; 3] = ["new", "keeper", "dismissed"];

    /// Upsert a feed item. `status`/`day`/`created_at` are set only on first
    /// INSERT and are absent from the ON CONFLICT update. `summary`/`transcript`
    /// use COALESCE so a re-ingest that lacks them never wipes a previously
    /// stored value. Returns `is_new`.
    pub fn upsert_feed(&self, item: &FeedItem) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let summary_provenance = item.summary.as_ref().map(|_| {
            item.summary_provenance
                .clone()
                .unwrap_or_else(|| StageProvenance::legacy("inline-summary-unknown"))
        });
        let summary_tier = summary_provenance.as_ref().map(|value| value.tier.as_str());
        let summary_revision = summary_provenance
            .as_ref()
            .map(|value| value.revision.as_str());
        let normalization_tier = item.transcript.as_ref().map(|_| "deterministic");
        let normalization_revision = item
            .transcript
            .as_ref()
            .map(|_| provenance::NORMALIZATION_REVISION);
        let existing = conn
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&item.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let is_new = existing.is_none();

        // Escalation only, decided in one place. A re-ingest may raise a class
        // -- a collector reclassifying its own source, or a first declaration
        // landing on a row that was undeclared -- and may never lower one. The
        // rule lives in `content_item::admit_reclassification` rather than in
        // this statement so there is one copy of it: this path proposes a
        // machine decision, so a lowering is refused here whatever the item
        // says, and the row keeps what it had.
        //
        // Who decided the stored class is read alongside it, because the four
        // columns move together: at equal class there is no class to change,
        // only a human's method and rationale to erase, and this path erased
        // them until the stored method became an input to the rule.
        let reclassifies = existing
            .as_ref()
            .is_none_or(|(stored_class, stored_method)| {
                crate::content_item::admit_reclassification(
                    stored_class,
                    stored_method,
                    &item.data_class,
                    &item.data_classification_method,
                    &item.data_class_rationale,
                )
                .is_ok()
            });

        conn.execute(
            &format!(
                "INSERT INTO {prefix}_feed_items
                    (id, stream, kind, title, url, author, summary, transcript, day, created_at,
                     status, content_status, transcript_source, captured_via, normalization_tier,
                     normalization_revision, normalization_completed_at,
                     summary_tier, summary_revision, summary_completed_at,
                     data_class, data_class_rationale,
                     data_classification_method, data_classification_version)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8, date('now'), {now}, 'new',
                         ?9,?10,?11,?12,?13, CASE WHEN ?8 IS NOT NULL THEN {now} END,
                         ?14,?15, CASE WHEN ?7 IS NOT NULL THEN {now} END,
                         ?17,?18,?19,?20)
                 ON CONFLICT (id) DO UPDATE SET
                     stream = excluded.stream,
                     kind = excluded.kind,
                     title = COALESCE(excluded.title, {items}.title),
                     url = excluded.url,
                     author = COALESCE(excluded.author, {items}.author),
                     summary = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary ELSE {items}.summary END,
                     summary_tier = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary_tier ELSE {items}.summary_tier END,
                     summary_revision = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.summary_revision ELSE {items}.summary_revision END,
                     summary_completed_at = CASE WHEN excluded.summary IS NOT NULL AND
                         CASE excluded.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.summary_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN {now} ELSE {items}.summary_completed_at END,
                     transcript = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.transcript ELSE {items}.transcript END,
                     normalization_tier = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.normalization_tier ELSE {items}.normalization_tier END,
                     normalization_revision = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.normalization_revision ELSE {items}.normalization_revision END,
                     normalization_completed_at = CASE WHEN excluded.transcript IS NOT NULL AND
                         CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                         CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN {now} ELSE {items}.normalization_completed_at END,
                     -- Follows the transcript: provenance describes the content
                     -- actually stored, so a re-fetch that yields nothing must
                     -- not relabel a captured body as server-fetched.
                     captured_via = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.captured_via
                         ELSE {items}.captured_via
                     END,
                     content_status = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.content_status
                         ELSE {items}.content_status
                     END,
                     -- Same guard as content_status: both describe the text
                     -- actually stored, so a re-fetch that loses to the
                     -- existing transcript must not relabel it.
                     transcript_source = CASE
                         WHEN excluded.transcript IS NOT NULL AND
                           CASE excluded.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE {items}.normalization_tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END
                         THEN excluded.transcript_source
                         ELSE {items}.transcript_source
                     END,
                     data_class = CASE WHEN ?16 THEN excluded.data_class ELSE {items}.data_class END,
                     data_class_rationale = CASE WHEN ?16
                         THEN excluded.data_class_rationale ELSE {items}.data_class_rationale END,
                     data_classification_method = CASE WHEN ?16
                         THEN excluded.data_classification_method
                         ELSE {items}.data_classification_method END,
                     data_classification_version = CASE WHEN ?16
                         THEN excluded.data_classification_version
                         ELSE {items}.data_classification_version END",
                prefix = self.prefix,
                now = sjel_store::NOW,
                items = format!("{}_feed_items", self.prefix)
            ),
            params![&item.id,
                &item.stream,
                &item.kind,
                &item.title,
                &item.url,
                &item.author,
                &item.summary,
                &item.transcript,
                &item.content_status,
                &item.transcript_source,
                &item.captured_via,
                &normalization_tier,
                &normalization_revision,
                &summary_tier,
                &summary_revision,
                &reclassifies,
                &item.data_class,
                &item.data_class_rationale,
                &item.data_classification_method,
                &item.data_classification_version,
            ],
        )?;

        // Raw extraction output, when this upsert carries one. A re-fetch
        // replaces it; a re-normalize never touches it.
        if let Some(raw) = &item.raw_content {
            conn.execute(
                &format!(
                    "INSERT INTO {prefix}_feed_raw_content (feed_id, raw, tier, revision)
                     VALUES (?1, ?2, 'deterministic', ?3)
                     ON CONFLICT (feed_id) DO UPDATE SET
                         raw = excluded.raw,
                         tier = excluded.tier,
                         revision = excluded.revision,
                         extracted_at = {now}
                     WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                           CASE {prefix}_feed_raw_content.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                    prefix = self.prefix,
                    now = sjel_store::NOW,
                ),
                params![&item.id, raw, &provenance::EXTRACTION_REVISION],
            )?;
        }

        Ok(is_new)
    }

    /// Set a feed item's class by hand.
    ///
    /// The only path that can lower one, and only because a human is on the
    /// other end of it. `rationale` is optional for an escalation and required
    /// for a de-escalation — the refusal comes back as an `Err`, which the
    /// server turns into a 400, so "I made it c0 and cannot say why" never
    /// reaches the database.
    ///
    /// `Ok(false)` is a missing item, never a refusal: a caller has to be able
    /// to tell "no such item" from "not allowed".
    /// `(id, kind, data_class)` for every feed row a given classification method wrote.
    ///
    /// Exists for re-derivation: `legacy` names rows classified before the deterministic
    /// source-declared rule existed, and they are the only rows whose stored class reflects when
    /// they arrived rather than where they came from. Returning `kind` with the id is what lets
    /// the caller match a row back to a declared source without a second query per row.
    ///
    /// Ordered by kind so a dry run reads as a summary of the decision rather than as a list.
    pub fn feed_items_classified_by(
        &self,
        method: &str,
    ) -> Result<Vec<FeedClassRow>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "SELECT id, kind, data_class FROM {}_feed_items
             WHERE data_classification_method = ?1
             ORDER BY kind, id",
            self.prefix
        ))?;
        let rows = statement
            .query_map(params![method], |row| {
                Ok(FeedClassRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    data_class: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_feed_data_class(
        &self,
        id: &str,
        data_class: &str,
        rationale: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Checked before the row is read, so an unknown class is a 400 whatever
        // id it was aimed at -- a caller with a typo learns it from the error
        // rather than from a 404 about the item.
        if !crate::content_item::valid(data_class) {
            return Err(format!(
                "data class must be one of: {}",
                crate::content_item::DATA_CLASSES.join(", ")
            )
            .into());
        }
        let conn = self.conn()?;
        let Some((stored_class, stored_method)) = conn
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let classification =
            human_reclassification(&stored_class, &stored_method, data_class, rationale)?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_feed_items SET
                    data_class = ?1,
                    data_class_rationale = ?2,
                    data_classification_method = ?3,
                    data_classification_version = ?4
                 WHERE id = ?5",
                self.prefix
            ),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        Ok(affected > 0)
    }

    /// The extractor's output for an item, if it was stored with one. Items
    /// ingested before #86 have none — they can only be re-fetched, not
    /// re-normalized, and `renormalize_all` reports them as skipped.
    pub fn get_raw_content(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT raw FROM {}_feed_raw_content WHERE feed_id = ?1",
                    self.prefix
                ),
                params![&id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Every item that has retained raw content, oldest first. The input to a
    /// re-normalization pass.
    pub fn feed_ids_with_raw_content(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT feed_id FROM {}_feed_raw_content ORDER BY extracted_at ASC",
                self.prefix
            ),
            [],
            |r| r.get(0),
        )?)
    }

    /// Replace an item's normalized body and its derived status. Used by the
    /// re-normalization pass; leaves the raw and the summary alone, because a
    /// rule change is not a reason to re-fetch or re-summarize.
    pub fn set_normalized(
        &self,
        id: &str,
        transcript: Option<&str>,
        content_status: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_feed_items SET transcript = ?1, content_status = ?2,
                    normalization_tier = 'deterministic', normalization_revision = ?4,
                    normalization_completed_at = {now}
                 WHERE id = ?3 AND 10 >= CASE normalization_tier
                    WHEN 'human' THEN 30 WHEN 'model' THEN 20
                    WHEN 'deterministic' THEN 10 ELSE 0 END",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![
                &transcript,
                &content_status,
                &id,
                &provenance::NORMALIZATION_REVISION,
            ],
        )?;
        Ok(affected > 0)
    }

    /// Move an item between `new`, `keeper` and `dismissed`, and record that the
    /// operator did so.
    ///
    /// The UPDATE and exactly one row in `{prefix}_feed_interactions` commit
    /// together, and a press that does not move the status writes neither: one
    /// ledger row per status CHANGE, not per POST. This route is the only
    /// writer of the three decisive verbs —
    /// `POST /feed/:id/interactions` refuses them — because two paths writing
    /// one decision double every count the learned factor is gated on, and the
    /// transactional write is the one that cannot be lost.
    ///
    /// `surface` says where the press happened (`inbox`, `reader`, `library`,
    /// `home`, `cli`, `api`). It is a verb and a place, never content.
    pub fn set_feed_status(
        &self,
        id: &str,
        status: &str,
        surface: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::FEED_STATUSES.contains(&status) {
            return Err(format!(
                "invalid feed status '{status}' -- must be one of: {}",
                Self::FEED_STATUSES.join(", ")
            )
            .into());
        }
        if let Some(refusal) = super::feedback::reject_surface(surface) {
            return Err(refusal.into());
        }
        let event = match status {
            "keeper" => "kept",
            "dismissed" => "dismissed",
            // Back to `new` is a retraction, and the trainer reads it as one.
            _ => "unkept",
        };
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        // Read inside the transaction rather than keying the ledger off the
        // UPDATE's row count: an UPDATE that sets a column to the value it
        // already holds still reports one row affected, so a second press of
        // `u` or `d` on the same entry appended a decision that never happened
        // and over-counted the ledger the learned factor is gated on. `None`
        // here is a missing item and the route answers 404; `Some` equal to the
        // incoming status is a no-op and writes nothing.
        let current = transaction
            .query_row(
                &format!(
                    "SELECT status FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(false);
        };
        if current == status {
            return Ok(true);
        }
        transaction.execute(
            &format!(
                "UPDATE {}_feed_items SET status = ?1 WHERE id = ?2",
                self.prefix
            ),
            params![&status, &id],
        )?;
        super::feedback::insert_interaction(&transaction, &self.prefix, id, event, surface)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn get_feed_status(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn update_feed_summary(
        &self,
        id: &str,
        summary: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_feed_items SET summary = ?1, summary_tier = 'model',
                    summary_revision = ?3, summary_completed_at = {now}, summary_attempts = 0,
                    summary_last_error = NULL, summary_next_attempt = NULL,
                    summary_attempt_revision = NULL
                 WHERE id = ?2 AND 20 >= CASE summary_tier
                    WHEN 'human' THEN 30 WHEN 'model' THEN 20
                    WHEN 'deterministic' THEN 10 ELSE 0 END",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&summary, &id, &producer_revision],
        )?;
        Ok(affected > 0)
    }

    /// Read producer provenance from the same rows that own each stage value.
    pub fn feed_stage_results(
        &self,
        id: &str,
    ) -> Result<Vec<StageProvenance>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // Twelve nullable columns, read into a fixed array so the stage loop below
        // can index them positionally the way it always has.
        let Some(columns) = conn
            .query_row(
                &format!(
                    "SELECT r.tier, r.revision, r.extracted_at,
                            f.normalization_tier, f.normalization_revision, f.normalization_completed_at,
                            f.summary_tier, f.summary_revision, f.summary_completed_at,
                            e.tier, e.evaluator_revision, e.evaluated_at
                     FROM {prefix}_feed_items f
                     LEFT JOIN {prefix}_feed_raw_content r ON r.feed_id = f.id
                     LEFT JOIN {prefix}_feed_evaluations e ON e.feed_id = f.id
                     WHERE f.id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                |row| {
                    let mut columns: [Option<String>; 12] = Default::default();
                    for (index, slot) in columns.iter_mut().enumerate() {
                        *slot = row.get(index)?;
                    }
                    Ok(columns)
                },
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };
        let mut stages = Vec::new();
        for (stage, tier_index, revision_index, time_index) in [
            ("extraction", 0, 1, 2),
            ("normalization", 3, 4, 5),
            ("summary", 6, 7, 8),
            ("ranking", 9, 10, 11),
        ] {
            if let Some(tier) = columns[tier_index].clone() {
                stages.push(StageProvenance {
                    stage: stage.to_string(),
                    tier,
                    revision: columns[revision_index]
                        .clone()
                        .unwrap_or_else(|| "legacy-unknown".into()),
                    completed_at: columns[time_index].clone().unwrap_or_default(),
                });
            }
        }
        Ok(stages)
    }

    /// Atomically replace the computed review signals for one item. An empty
    /// set clears flags that no longer fire on the current stored evidence.
    pub fn replace_feed_quality_flags(
        &self,
        feed_id: &str,
        flags: &[QualityFlag],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(
            &format!(
                "DELETE FROM {}_feed_quality_flags WHERE feed_id = ?1",
                self.prefix
            ),
            params![&feed_id],
        )?;
        for flag in flags {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_feed_quality_flags
                        (feed_id, signal, reason, evidence, derived_at)
                     VALUES (?1, ?2, ?3, ?4, {now})",
                    prefix = self.prefix,
                    now = sjel_store::NOW
                ),
                params![&feed_id, &flag.signal, &flag.reason, &flag.evidence],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn feed_quality_review_queue(
        &self,
        limit: usize,
    ) -> Result<Vec<QualityReviewRow>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT q.feed_id, f.title, f.url, f.status, f.content_status,
                        q.signal, q.reason, q.evidence, q.derived_at
                 FROM {prefix}_feed_quality_flags q
                 JOIN {prefix}_feed_items f ON f.id = q.feed_id
                 ORDER BY f.created_at DESC, q.signal ASC
                 LIMIT ?1",
                prefix = self.prefix
            ),
            params![limit as i64],
            |row| {
                Ok(QualityReviewRow {
                    feed_id: row.get(0)?,
                    title: row.get(1)?,
                    url: row.get(2)?,
                    status: row.get(3)?,
                    content_status: row.get(4)?,
                    signal: row.get(5)?,
                    reason: row.get(6)?,
                    evidence: row.get(7)?,
                    derived_at: row.get(8)?,
                })
            },
        )?)
    }

    /// Increment the attempt counter, record the error class, and set the next
    /// retry time with exponential backoff (5 min × 2^attempts, capped at 3
    /// attempts). After 3 failures `feed_pending_summaries` stops returning it.
    pub fn record_summary_attempt(
        &self,
        id: &str,
        error_class: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_feed_items SET
                    summary_attempts = CASE
                        WHEN summary_attempt_revision IS NOT ?3 THEN 1
                        ELSE summary_attempts + 1 END,
                    summary_last_error = ?1,
                    summary_next_attempt = {backoff},
                    summary_attempt_revision = ?3
                 WHERE id = ?2",
                self.prefix,
                // `interval '5 minutes' * power(2, n)` becomes a computed modifier:
                // SQLite has no `power`, and `1 << n` is the same doubling.
                backoff = sjel_store::now_offset(
                    "'+' || (5 * (1 << CASE WHEN summary_attempt_revision IS NOT ?3 \
                                THEN 0 ELSE summary_attempts END)) || ' minutes'"
                )
            ),
            params![&error_class, &id, &producer_revision],
        )?;
        Ok(affected > 0)
    }

    /// Enrichment backlog: items eligible for summarization vs. permanently
    /// failed (≥3 attempts).
    pub fn feed_enrichment_counts(
        &self,
        producer_revision: Option<&str>,
    ) -> Result<EnrichmentCounts, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "SELECT
                    COUNT(*) FILTER (WHERE transcript IS NOT NULL
                        AND (summary IS NULL OR (?1 IS NOT NULL AND
                            ((summary_tier = 'model' AND summary_revision IS NOT ?1)
                             OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                        AND (summary_attempt_revision IS NOT ?1
                            OR summary_attempts < 3)) AS pending,
                    COUNT(*) FILTER (WHERE transcript IS NOT NULL
                        AND (summary IS NULL OR (?1 IS NOT NULL AND
                            ((summary_tier = 'model' AND summary_revision IS NOT ?1)
                             OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                        AND summary_attempt_revision IS ?1
                        AND summary_attempts >= 3) AS failed
                 FROM {}_feed_items",
                self.prefix
            ),
            params![&producer_revision],
            |row| {
                Ok(EnrichmentCounts {
                    pending_summaries: row.get(0)?,
                    failed_summaries: row.get(1)?,
                })
            },
        )?)
    }

    /// Distribution of `content_status` across all feed items.
    pub fn feed_content_status_counts(
        &self,
    ) -> Result<ContentStatusCounts, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "SELECT
                    COUNT(*) FILTER (WHERE content_status = 'full') AS full_count,
                    COUNT(*) FILTER (WHERE content_status = 'thin') AS thin_count,
                    COUNT(*) FILTER (WHERE content_status = 'none') AS none_count,
                    COUNT(*) FILTER (WHERE content_status = 'unknown') AS unknown_count
                 FROM {}_feed_items",
                self.prefix
            ),
            [],
            |row| {
                Ok(ContentStatusCounts {
                    full: row.get(0)?,
                    thin: row.get(1)?,
                    none: row.get(2)?,
                    unknown: row.get(3)?,
                })
            },
        )?)
    }

    /// Single item incl. transcript (server /feed/:id, keeper export).
    pub fn get_feed(&self, id: &str) -> Result<Option<FeedItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, stream, kind, title, url, author, summary, transcript, day, created_at, status,
                           content_status, summary_attempts, summary_last_error, summary_next_attempt, captured_via, transcript_source,
                           data_class, data_class_rationale, data_classification_method, data_classification_version
                     FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                row_to_feed_full,
            )
            .optional()?)
    }

    /// List feed items (no transcript in the payload), newest first. `days`
    /// bounds by `day >= date('now', '-N days')`. Excludes dismissed unless asked.
    /// Optionally filters by `source_id`.
    pub fn list_feed(
        &self,
        stream: Option<&str>,
        source_id: Option<&str>,
        days: i32,
        include_dismissed: bool,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let mut sql = format!(
            "SELECT DISTINCT f.id, f.stream, f.kind, f.title, f.url, f.author, f.summary, NULL, f.day, f.created_at, f.status,
                    f.content_status, f.summary_attempts, f.summary_last_error, f.summary_next_attempt, f.captured_via, f.transcript_source,
                    f.data_class, f.data_class_rationale, f.data_classification_method, f.data_classification_version, f.created_at
             FROM {prefix}_feed_items f",
            prefix = self.prefix
        );

        if source_id.is_some() {
            sql.push_str(&format!(
                " JOIN {prefix}_feed_origins o ON f.id = o.feed_id",
                prefix = self.prefix
            ));
        }

        // `CURRENT_DATE - $1` becomes `date('now', '-N days')`, built from the
        // bound parameter rather than from arithmetic on a date.
        sql.push_str(" WHERE f.day >= date('now', '-' || ?1 || ' days')");
        // The parameter vector is rusqlite's `&dyn ToSql` rather than postgres's
        // `&(dyn ToSql + Sync)`: rusqlite binds on the calling thread, so nothing
        // has to cross one.
        let mut bound: Vec<&dyn ToSql> = vec![&days];

        if let Some(s) = &stream {
            sql.push_str(&format!(" AND f.stream = ?{}", bound.len() + 1));
            bound.push(s);
        }
        if let Some(src) = &source_id {
            sql.push_str(&format!(" AND o.source_id = ?{}", bound.len() + 1));
            bound.push(src);
        }
        if !include_dismissed {
            sql.push_str(" AND f.status != 'dismissed'");
        }
        sql.push_str(" ORDER BY f.created_at DESC");
        Ok(conn.query_all(&sql, bound.as_slice(), row_to_feed_list)?)
    }

    /// Every saved item, oldest first — the feed library, unbounded by time.
    ///
    /// `list_feed` cannot answer this. It bounds by `day >= date('now', -N days)`
    /// because it serves a queue, and the library is the opposite: `/feed/library`
    /// is documented as "the durable collection view over up to ten years of Feed
    /// state" (`dashboard/README.md`). A projection built on a windowed query would
    /// delete every note whose item aged out of the window, which is a sweep that
    /// destroys exactly what the bridge exists to preserve.
    ///
    /// Saved means `status = 'keeper'`. Nothing else in the store records a decision
    /// to keep something: `dismissed` is the explicit no and `new` is the unread
    /// queue, so `keeper` is the whole library and there is no second flag to
    /// consult.
    ///
    /// Oldest first so the collision rule sees a stable order. No transcript: the
    /// projection renders the summary, and pulling every stored transcript to render
    /// none of them would be the largest read this capability makes.
    pub fn feed_library(&self) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let sql = format!(
            "SELECT id, stream, kind, title, url, author, summary, NULL, day, created_at, status,
                    content_status, summary_attempts, summary_last_error, summary_next_attempt,
                    captured_via, transcript_source,
                    data_class, data_class_rationale, data_classification_method,
                    data_classification_version
             FROM {}_feed_items
             WHERE status = 'keeper'
             ORDER BY created_at ASC, id ASC",
            self.prefix
        );
        Ok(conn.query_all(&sql, [], row_to_feed_list)?)
    }

    /// Feed items eligible for summarization: transcript present, no summary,
    /// not past the attempt cap, and backoff window elapsed. Bounded retry
    /// replaces the old unbounded "summary IS NULL" scan.
    pub fn feed_pending_summaries(
        &self,
        producer_revision: Option<&str>,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day, created_at, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt, captured_via, transcript_source,
                        data_class, data_class_rationale, data_classification_method, data_classification_version
                 FROM {}_feed_items
                 WHERE transcript IS NOT NULL
                   AND (summary IS NULL OR (?1 IS NOT NULL AND
                        ((summary_tier = 'model' AND summary_revision IS NOT ?1)
                         OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown'))))
                   AND (summary_attempt_revision IS NOT ?1
                        OR (summary_attempts < 3
                            AND (summary_next_attempt IS NULL OR summary_next_attempt <= {now})))
                 ORDER BY created_at DESC",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&producer_revision],
            row_to_feed_full,
        )?)
    }

    pub fn feed_summary_needs_revision(
        &self,
        id: &str,
        producer_revision: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT summary IS NULL OR (summary_tier = 'model'
                        AND summary_revision IS NOT ?2)
                        OR (summary_tier = 'legacy' AND summary_revision = 'legacy-unknown')
                     FROM {}_feed_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id, &producer_revision],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Full feed items for a bounded relevance refresh. This intentionally
    /// includes dismissed items: a later filter change can make an old item
    /// useful again, while the human status remains untouched.
    pub fn feed_for_relevance(
        &self,
        days: i32,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day, created_at, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt, captured_via, transcript_source,
                        data_class, data_class_rationale, data_classification_method, data_classification_version
                 FROM {}_feed_items
                 WHERE day >= date('now', '-' || ?1 || ' days')
                 ORDER BY created_at DESC
                 LIMIT ?2 OFFSET ?3",
                self.prefix
            ),
            params![days, limit as i64, offset as i64],
            row_to_feed_full,
        )?)
    }

    /// The items a caller named, whatever page they would have fallen on.
    ///
    /// The refresh route used to select the newest 200 rows and THEN retain the
    /// requested ids, so an explicitly named item outside that window was
    /// silently dropped and could never be re-scored. Selecting by id is the
    /// fix; the caller applies no limit to it, because it already named the set.
    pub fn feed_items_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<FeedItem>, Box<dyn std::error::Error>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let placeholders = (1..=ids.len())
            .map(|position| format!("?{position}"))
            .collect::<Vec<_>>()
            .join(",");
        let bound = ids
            .iter()
            .map(|id| id as &dyn ToSql)
            .collect::<Vec<&dyn ToSql>>();
        Ok(conn.query_all(
            &format!(
                "SELECT id, stream, kind, title, url, author, summary, transcript, day, created_at, status,
                        content_status, summary_attempts, summary_last_error, summary_next_attempt, captured_via, transcript_source,
                        data_class, data_class_rationale, data_classification_method, data_classification_version
                 FROM {}_feed_items
                 WHERE id IN ({placeholders})
                 ORDER BY created_at DESC",
                self.prefix
            ),
            bound.as_slice(),
            row_to_feed_full,
        )?)
    }

    /// Which collector source each item arrived from, for the feature vector's
    /// source slot. The newest origin wins when an item has several.
    pub fn feed_origin_sources(
        &self,
    ) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let rows = conn.query_all(
            &format!(
                "SELECT feed_id, source_id FROM {}_feed_origins ORDER BY last_seen ASC",
                self.prefix
            ),
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        Ok(rows.into_iter().collect())
    }

    /// Stored matches for many items in one query.
    ///
    /// The split currency check re-evaluates from stored matches instead of
    /// re-embedding, and doing that per item would be one `feed_relevance` call
    /// per row — 372 round trips to answer a question one grouped read answers.
    pub fn feed_relevance_map(
        &self,
        ids: &[String],
    ) -> Result<BTreeMap<String, Vec<RelevanceMatch>>, Box<dyn std::error::Error>> {
        let mut map: BTreeMap<String, Vec<RelevanceMatch>> = BTreeMap::new();
        if ids.is_empty() {
            return Ok(map);
        }
        let conn = self.conn()?;
        let placeholders = (1..=ids.len())
            .map(|position| format!("?{position}"))
            .collect::<Vec<_>>()
            .join(",");
        let bound = ids
            .iter()
            .map(|id| id as &dyn ToSql)
            .collect::<Vec<&dyn ToSql>>();
        let rows = conn.query_all(
            &format!(
                "SELECT feed_id, profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}_feed_relevance
                 WHERE feed_id IN ({placeholders})
                 ORDER BY feed_id, score DESC",
                self.prefix
            ),
            bound.as_slice(),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    RelevanceMatch {
                        profile_key: row.get(1)?,
                        profile_label: row.get(2)?,
                        score: row.get(3)?,
                        rationale: row.get(4)?,
                        mode: row.get(5)?,
                        profile_revision: row.get(6)?,
                    },
                ))
            },
        )?;
        for (feed_id, matched) in rows {
            map.entry(feed_id).or_default().push(matched);
        }
        Ok(map)
    }

    /// Delete every stored match for one item, past the tier gate.
    ///
    /// The withdrawal half of [`Store::replace_feed_relevance`], and separate
    /// from it on purpose. `replace_feed_relevance(id, &[])` computes an
    /// incoming tier of `deterministic` from an empty match set, so on an item
    /// that already carried a `model`-tier evaluation it returned `Ok(false)`
    /// and deleted nothing -- which is how a c3 escalation left three stored
    /// matches in place while the pass reported `refused_class: 1`. An empty set
    /// cannot simply be made to delete: a pass that found no lens match would
    /// then wipe good semantic rows. So the caller says which it means.
    ///
    /// Returns the number of rows removed, so a receipt can report a withdrawal
    /// that actually happened.
    pub fn clear_feed_relevance(&self, feed_id: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "DELETE FROM {}_feed_relevance WHERE feed_id = ?1",
                self.prefix
            ),
            params![&feed_id],
        )?)
    }

    /// Replace every profile result for one item in one transaction. Removed
    /// or renamed TELOS lenses therefore cannot leave stale matches behind.
    pub fn replace_feed_relevance(
        &self,
        feed_id: &str,
        matches: &[RelevanceMatch],
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let incoming_tier = matches
            .first()
            .map(|matched| provenance::ranking_tier(&matched.mode))
            .unwrap_or("deterministic");
        let current = transaction
            .query_row(
                &format!(
                    "SELECT tier FROM {}_feed_evaluations WHERE feed_id = ?1",
                    self.prefix
                ),
                params![&feed_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if current
            .is_some_and(|tier| provenance::tier_rank(incoming_tier) < provenance::tier_rank(&tier))
        {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "DELETE FROM {}_feed_relevance WHERE feed_id = ?1",
                self.prefix
            ),
            params![&feed_id],
        )?;
        for relevance in matches {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_feed_relevance
                        (feed_id, profile_key, profile_label, score, rationale, mode, profile_revision, scored_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,{now})",
                    prefix = self.prefix,
                now = sjel_store::NOW
            ),
                params![
                    &feed_id,
                    &relevance.profile_key,
                    &relevance.profile_label,
                    relevance.score,
                    &relevance.rationale,
                    &relevance.mode,
                    &relevance.profile_revision,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn feed_relevance(
        &self,
        feed_id: &str,
    ) -> Result<Vec<RelevanceMatch>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}_feed_relevance WHERE feed_id = ?1 ORDER BY score DESC",
                self.prefix
            ),
            params![&feed_id],
            |row| {
                Ok(RelevanceMatch {
                    profile_key: row.get(0)?,
                    profile_label: row.get(1)?,
                    score: row.get(2)?,
                    rationale: row.get(3)?,
                    mode: row.get(4)?,
                    profile_revision: row.get(5)?,
                })
            },
        )?)
    }
}
