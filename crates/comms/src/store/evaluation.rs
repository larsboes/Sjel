//! Stored feed evaluations and the travel-context snapshot they consume.

use super::*;

impl Store {
    /// Store the complete evaluation and its factors atomically. The factor
    /// table is normalized so future trip/deadline factors can be added without
    /// a schema migration or an opaque JSON payload.
    ///
    /// `Ok(false)` means the tier gate refused the write: a `deterministic`
    /// producer never overwrites a stored `model` row.
    pub fn replace_feed_evaluation(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.write_feed_evaluation(evaluation, true)
    }

    /// Store a class refusal, past the tier gate.
    ///
    /// A refusal is a withdrawal, not a weaker producer, so it must not lose to
    /// the row it withdraws. `provenance::ranking_tier` maps the refusal's
    /// `unscored` mode to `deterministic`, so escalating an already-scored item
    /// to c3 used to leave its model-derived score, rationale and factors
    /// exactly where they were while the pass reported `refused_class: 1` --
    /// the gate reported a refusal the database never took.
    pub fn replace_feed_evaluation_refusal(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.write_feed_evaluation(evaluation, false)
    }

    fn write_feed_evaluation(
        &self,
        evaluation: &FeedEvaluation,
        enforce_tier: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let tier = provenance::ranking_tier(&evaluation.mode);
        let gate = if enforce_tier {
            format!(
                "WHERE CASE excluded.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END >=
                       CASE {prefix}_feed_evaluations.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                prefix = self.prefix
            )
        } else {
            String::new()
        };
        let affected = transaction.execute(
            &format!(
                "INSERT INTO {prefix}_feed_evaluations
                    (feed_id, overall_score, explanation, mode, item_revision,
                     context_revision, evaluator_revision, tier, evaluated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,{now})
                 ON CONFLICT (feed_id) DO UPDATE SET
                    overall_score = excluded.overall_score,
                    explanation = excluded.explanation,
                    mode = excluded.mode,
                    item_revision = excluded.item_revision,
                    context_revision = excluded.context_revision,
                    evaluator_revision = excluded.evaluator_revision,
                    tier = excluded.tier,
                    evaluated_at = {now}
                 {gate}",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            params![
                &evaluation.feed_id,
                evaluation.overall_score,
                &evaluation.explanation,
                &evaluation.mode,
                &evaluation.item_revision,
                &evaluation.context_revision,
                &evaluation.evaluator_revision,
                &tier,
            ],
        )?;
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "DELETE FROM {}_feed_evaluation_factors WHERE feed_id = ?1",
                self.prefix
            ),
            params![&evaluation.feed_id],
        )?;
        for (position, factor) in evaluation.factors.iter().enumerate() {
            let context_json = factor
                .context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_feed_evaluation_factors
                        (feed_id, factor_key, label, score, weight, rationale, context_json, position)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    prefix = self.prefix
                ),
                params![
                    &evaluation.feed_id,
                    &factor.key,
                    &factor.label,
                    factor.score,
                    factor.weight,
                    &factor.rationale,
                    &context_json,
                    position as i32,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn feed_evaluation(
        &self,
        feed_id: &str,
    ) -> Result<Option<FeedEvaluation>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `evaluated_at::text` loses its cast: the column is TEXT now.
        let evaluation = conn
            .query_row(
                &format!(
                    "SELECT overall_score, explanation, mode, item_revision,
                            context_revision, evaluator_revision, evaluated_at
                     FROM {}_feed_evaluations WHERE feed_id = ?1",
                    self.prefix
                ),
                params![&feed_id],
                |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    ))
                },
            )
            .optional()?;
        let Some((
            overall_score,
            explanation,
            mode,
            item_revision,
            context_revision,
            evaluator_revision,
            evaluated_at,
        )) = evaluation
        else {
            return Ok(None);
        };
        let factors = conn.query_all(
            &format!(
                "SELECT factor_key, label, score, weight, rationale, context_json
                 FROM {}_feed_evaluation_factors
                 WHERE feed_id = ?1 ORDER BY position",
                self.prefix
            ),
            params![&feed_id],
            |factor| {
                Ok(EvaluationFactor {
                    key: factor.get(0)?,
                    label: factor.get(1)?,
                    score: factor.get(2)?,
                    weight: factor.get(3)?,
                    rationale: factor.get(4)?,
                    context: factor.get::<_, Option<String>>(5)?.and_then(|value| {
                        serde_json::from_str::<EvaluationFactorContext>(&value).ok()
                    }),
                })
            },
        )?;
        Ok(Some(FeedEvaluation {
            feed_id: feed_id.to_string(),
            overall_score,
            explanation,
            mode,
            item_revision,
            context_revision,
            evaluator_revision,
            evaluated_at,
            factors,
        }))
    }

    pub fn evaluation_summary(&self) -> Result<EvaluationSummary, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // `COUNT(*) FILTER (WHERE ...)` is supported verbatim; the `::bigint` casts
        // go because a SQLite integer is already 64-bit.
        Ok(conn.query_row(
            &format!(
                "SELECT COUNT(*),
                        COUNT(*) FILTER (WHERE mode = 'reranked'),
                        COUNT(*) FILTER (WHERE mode = 'semantic'),
                        COUNT(*) FILTER (WHERE mode = 'lexical'),
                        COUNT(*) FILTER (WHERE mode = 'unscored')
                 FROM {}_feed_evaluations",
                self.prefix
            ),
            [],
            |row| {
                Ok(EvaluationSummary {
                    evaluated: row.get(0)?,
                    reranked: row.get(1)?,
                    semantic: row.get(2)?,
                    lexical: row.get(3)?,
                    unscored: row.get(4)?,
                })
            },
        )?)
    }

    /// Store one revisioned blob under its kind.
    ///
    /// Generalised from the travel-only pair, because the learned feedback model
    /// is the same shape: a bounded, revisioned payload that feeds
    /// `context_revision`. The table is already keyed on `context_kind`
    /// (migrations.rs), so a second kind needs no migration at all — which
    /// matters, because `CREATE TABLE IF NOT EXISTS` never revisits an installed
    /// table and `axon.db` already holds this one.
    pub fn replace_context_snapshot(
        &self,
        kind: &str,
        revision: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_feed_context_snapshots
                    (context_kind, revision, payload, refreshed_at)
                 VALUES (?1,?2,?3,{now})
                 ON CONFLICT (context_kind) DO UPDATE SET
                    revision = excluded.revision,
                    payload = excluded.payload,
                    refreshed_at = {now}",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            params![&kind, &revision, &payload],
        )?;
        Ok(())
    }

    pub fn context_snapshot(
        &self,
        kind: &str,
    ) -> Result<Option<ContextSnapshot>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT revision, payload, refreshed_at
                     FROM {}_feed_context_snapshots WHERE context_kind = ?1",
                    self.prefix
                ),
                params![&kind],
                |row| {
                    Ok(ContextSnapshot {
                        revision: row.get(0)?,
                        payload: row.get(1)?,
                        refreshed_at: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }

    /// Two-line delegates, so `travel.rs` does not move for a change that is
    /// not about travel.
    pub fn replace_travel_context_snapshot(
        &self,
        revision: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.replace_context_snapshot("travel", revision, payload)
    }

    pub fn travel_context_snapshot(
        &self,
    ) -> Result<Option<TravelContextSnapshot>, Box<dyn std::error::Error>> {
        Ok(self
            .context_snapshot("travel")?
            .map(|snapshot| TravelContextSnapshot {
                revision: snapshot.revision,
                payload: snapshot.payload,
                refreshed_at: snapshot.refreshed_at,
            }))
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::evaluation;
    use crate::mail_evaluation;
    use crate::store::db_tests::{mk_triage, open_test_store};

    fn semantic_match() -> RelevanceMatch {
        RelevanceMatch {
            profile_key: "lens".into(),
            profile_label: "Career & Visibility".into(),
            score: 0.9,
            rationale: "match".into(),
            mode: "semantic".into(),
            profile_revision: "revision".into(),
        }
    }

    /// The defect the tier gate created for the class gate.
    ///
    /// `provenance::ranking_tier` maps the refusal's `unscored` mode to
    /// `deterministic`, so escalating an item that already carried a `model`
    /// row to c3 lost both writes: `replace_feed_relevance(id, &[])` computed a
    /// `deterministic` incoming tier and deleted nothing, and
    /// `replace_feed_evaluation` was refused by the same rule. The pass then
    /// reported `refused_class: 1` over a row nothing had been removed from,
    /// and every later pass failed identically. A refusal is a withdrawal, not
    /// a weaker producer.
    #[test]
    fn escalating_a_model_scored_feed_item_to_c3_withdraws_its_score() {
        let store = open_test_store("evaluation_c3_withdraws_feed");
        let mut item = FeedItem::new("https://example.com/withdrawn", "news", "article");
        item.title = Some("A Title".into());
        item.summary = Some("A summary".into());
        store.upsert_feed(&item).expect("the fixture stores");

        let matched = semantic_match();
        assert!(store
            .replace_feed_relevance(&item.id, std::slice::from_ref(&matched))
            .expect("the matches store"));
        let scored = evaluation::evaluate(&item, Some(&matched), "context", &[], false, None);
        assert!(store
            .replace_feed_evaluation(&scored)
            .expect("the model-tier evaluation stores"));

        store
            .set_feed_data_class(&item.id, "c3", Some("a human lowered it"))
            .expect("the escalation lands");

        // The write the refusal branch used to make, and why `clear_` exists:
        // an empty replacement is a `deterministic` producer and loses.
        assert!(!store
            .replace_feed_relevance(&item.id, &[])
            .expect("the tier gate answers"));

        // Exactly what the refusal branch of `relevance_refresh_handler` runs.
        let cleared = store
            .clear_feed_relevance(&item.id)
            .expect("the withdrawal runs");
        assert_eq!(cleared, 1, "the derived match must actually be deleted");
        let refusal = evaluation::evaluate(&item, None, "context", &[], true, None);
        assert!(store
            .replace_feed_evaluation_refusal(&refusal)
            .expect("the refusal writes"));

        let stored = store
            .feed_evaluation(&item.id)
            .expect("the evaluation reads")
            .expect("a refusal is a row, not an absence");
        assert_eq!(stored.mode, "unscored");
        assert!(!stored.explanation.contains("Interest fit"));
        assert!(store
            .feed_relevance_map(&[item.id.clone()])
            .expect("the map reads")
            .get(&item.id)
            .map(Vec::is_empty)
            .unwrap_or(true));

        // And the refusal is final: no reachable embedder can upgrade it, so a
        // second pass must leave it alone rather than rewrite it forever.
        assert!(evaluation::refusal_is_current(
            Some(&stored),
            &evaluation::item_revision(&item),
            "context"
        ));
    }

    #[test]
    fn escalating_a_model_scored_mail_to_c3_withdraws_its_score() {
        let store = open_test_store("evaluation_c3_withdraws_mail");
        let mut item = mk_triage("thread:withdrawn", "aktiv");
        item.internal_date_text = Some("2026-09-04 08:00:00+00:00".into());
        store.upsert_triage(&item).expect("the fixture stores");

        let matched = semantic_match();
        store
            .replace_triage_relevance(&item.id, std::slice::from_ref(&matched))
            .expect("the matches store");
        let scored = mail_evaluation::evaluate(&item, Some(&matched), None, "context", false);
        assert!(store
            .replace_triage_evaluation(&scored)
            .expect("the model-tier evaluation stores"));

        item.data_class = "c3".into();
        let refusal = mail_evaluation::evaluate(&item, None, None, "context", true);
        assert!(
            store
                .replace_triage_evaluation_refusal(&refusal)
                .expect("the refusal writes"),
            "a class refusal must not lose to the model row it withdraws"
        );
        let stored = store
            .triage_evaluation(&item.id)
            .expect("the evaluation reads")
            .expect("a refusal is a row");
        assert_eq!(stored.mode, "unscored");
        // The measured defect: a refused mail must never outscore one that was
        // actually read.
        assert!(stored.overall_score <= scored.overall_score);
        assert!(mail_evaluation::refusal_is_current(
            Some(&stored),
            &mail_evaluation::item_revision(&item),
            "context"
        ));
    }
}
