//! Mail triage state, Gmail lifecycle actions, and triage relevance.

use super::*;

impl Store {
    // -- triage ----------------------------------------------------------

    pub const TRIAGE_STATUSES: [&'static str; 7] = [
        "proposed",
        "approved",
        "executed",
        "archived",
        "trashed",
        "missing",
        "dismissed",
    ];

    /// Upsert a triage proposal observed in the Gmail Inbox, with no record of
    /// which deterministic rung decided it.
    ///
    /// The shape every caller outside the two sweep paths wants. The sweep
    /// itself calls [`Store::upsert_triage_with_rules`], which is the same
    /// transaction plus the `{prefix}_triage_rules` row.
    pub fn upsert_triage(&self, item: &TriageItem) -> Result<bool, Box<dyn std::error::Error>> {
        self.upsert_triage_inner(item, None)
    }

    /// [`Store::upsert_triage`] plus the deterministic verdict that produced
    /// the row, written inside the same transaction.
    ///
    /// One transaction rather than two writes, because a crash between them
    /// would leave a row whose rung nothing records — and the rung is what
    /// decides whether the model rung may look at it. It is also the parameter
    /// the stream guard needs: a rule that actually **fired** may take a row
    /// back from the model, a rule that merely fell through to `aktiv` may not.
    pub fn upsert_triage_with_rules(
        &self,
        item: &TriageItem,
        verdict: &crate::rules::Verdict,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.upsert_triage_inner(item, Some(verdict))
    }

    /// One transaction, because two of the columns it writes are governed by a
    /// third: the class decides whether `subject` and `snippet` may be stored
    /// as they arrived, and that class is only known after the stored row has
    /// been read. See `class_after_upsert` below for the split the two rules
    /// make — redaction follows the winning class, freshness follows the thread.
    fn upsert_triage_inner(
        &self,
        item: &TriageItem,
        verdict: Option<&crate::rules::Verdict>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Gmail internalDate is epoch-ms; convert to fractional epoch-seconds so
        // the bound param is a plain double for the `unixepoch` modifier below.
        let internal_secs: Option<f64> = item.internal_date_ms.map(|ms| ms as f64 / 1000.0);
        // One predicate, four columns: keep the stored classification when a
        // human set it, and also when the incoming one would be *less* strict.
        // A resweep re-runs the rules, so without the second half an edit that
        // made the classifier less suspicious would walk the whole inbox
        // quietly downgrading rows it had previously called Secret -- a rule
        // lowering a class, which is the one thing the escalation rule forbids.
        // `t.` was an INSERT alias (`INSERT INTO x AS t`), which SQLite has no
        // syntax for: inside DO UPDATE it refers to the existing row by the
        // table's own name. So the predicate is built with the prefix.
        let table = format!("{}_triage_items", self.prefix);
        let preserve_class = format!(
            "{table}.data_classification_method = 'human' OR \
             (CASE excluded.data_class WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 \
              ELSE 0 END) < \
             (CASE {table}.data_class WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 \
              ELSE 0 END)"
        );
        // The same shape for the CATEGORY axis, and it did not exist before the
        // model rung did. Four columns tested `= 'human'` as a bare literal, so
        // a row written `classification_method = 'model'` was reverted by the
        // next deterministic sweep — silently, with the rationale column then
        // reading as a rules result. The unattended sweep is enabled in this
        // overlay (`inbox_sweep_minutes`), so the window was one sweep interval.
        //
        // This is NOT `preserve_class` with a different column list.
        // `preserve_class` ranks the class VALUE, which is why a sweep may still
        // escalate a model-written class; this ranks the METHOD, and the four
        // inline ranks equal `content_item::method_rank` exactly (legacy 0,
        // deterministic 10, model 20, human 30).
        //
        // The third clause is what a bare method rank would get wrong. Without
        // it, one model write would freeze the thread against every future
        // overlay rule — including a rule written specifically to correct the
        // model. `?14` is the incoming `decided_by`: a deterministic verdict a
        // rule actually FIRED for (`config_rule`, `heuristic`) takes the row
        // back, a deterministic FALLBACK does not, because the model rung only
        // ran on the rows the rules fell through on. It is NULL for every caller
        // that passes no verdict, and `COALESCE` rather than a bare `IN` is
        // load-bearing: `NULL IN (…)` is NULL, `x AND NULL` is NULL, and a NULL
        // predicate takes the ELSE arm — so without it the missing verdict
        // would OVERWRITE the model row, which is the opposite of the default
        // this clause is for.
        let method_rank = |side: &str| {
            format!(
                "(CASE {side}.classification_method WHEN 'human' THEN 30 WHEN 'model' THEN 20 \
                  WHEN 'deterministic' THEN 10 ELSE 0 END)"
            )
        };
        let preserve_stream = format!(
            "{table}.classification_method = 'human' \
             OR ( {incoming} < {stored} \
                  AND NOT ({table}.classification_method = 'model' \
                           AND COALESCE(?14, 'fallback') IN ('config_rule','heuristic')) )",
            incoming = method_rank("excluded"),
            stored = method_rank(&table),
        );
        // `?5` is Unix seconds; the column holds the canonical stamp, so the
        // conversion is SQL rather than Rust.
        let internal_date = format!("strftime('{}', ?5, 'unixepoch')", sjel_store::STAMP_FORMAT);
        let mut conn = self.conn()?;
        // BEGIN IMMEDIATE, not the default deferred begin: this reads the stored
        // class and then writes, and SQLite answers a failed upgrade to the
        // writer lock with SQLITE_BUSY that `busy_timeout` deliberately does not
        // retry (`sjel_store::migrate_once`). Two sweeps and a dashboard write
        // reach this at once.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<(String, String)> = transaction
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method
                       FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&item.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let is_new = stored.is_none();

        // The class decides the review fields, one layer below the CASE above.
        // `intake` redacts subject and snippet *before* it builds the row, so an
        // incoming c0/c1 row carries verbatim Gmail text by construction, and
        // writing that over a stored c2/c3 row would undo the redaction while
        // the class column stays strict -- a row marked Redacted in the
        // dashboard holding the text it says it removed. The live way in is
        // ruling 3: the named-person escalation reads the people registry, so a
        // sweep run with the overlay unmounted answers c1 for a thread the last
        // refresh raised to c2 (`people_registry::State::Absent`).
        //
        // The answer is to redact the *incoming* text rather than keep the old,
        // because the two properties belong to different things. Redaction
        // follows the class that wins; freshness follows the thread. Freezing
        // the stored pair bought the first at the cost of the second: from_addr,
        // internal_date and stream keep advancing, so the row would show one
        // message's date beside an older message's subject, for the life of the
        // thread and with no path back short of a human de-escalation.
        let winner = class_after_upsert(stored.as_ref(), &item.data_class);
        let remediation =
            crate::intake::remediate(winner, item.subject.as_deref(), item.snippet.as_deref());
        let (subject, snippet) = match &remediation {
            Some(remediation) => (
                remediation.subject.as_deref(),
                remediation.snippet.as_deref(),
            ),
            None => (item.subject.as_deref(), item.snippet.as_deref()),
        };

        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_triage_items
                    (id, from_addr, subject, snippet, internal_date, stream, rationale,
                     classification_method, classification_version, data_class,
                     data_class_rationale, data_classification_method,
                     data_classification_version, status, gmail_location,
                     gmail_observed_at, gmail_sync_status, first_seen, last_seen)
                 VALUES (?1,?2,?3,?4, {internal_date}, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         'proposed', 'inbox', {now}, 'synced', {now}, {now})
                 ON CONFLICT (id) DO UPDATE SET
                     from_addr = excluded.from_addr,
                     subject = excluded.subject,
                     snippet = excluded.snippet,
                     internal_date = excluded.internal_date,
                     stream = CASE WHEN {preserve_stream}
                        THEN {table}.stream ELSE excluded.stream END,
                     rationale = CASE WHEN {preserve_stream}
                        THEN {table}.rationale ELSE excluded.rationale END,
                     classification_method = CASE WHEN {preserve_stream}
                        THEN {table}.classification_method ELSE excluded.classification_method END,
                     classification_version = CASE WHEN {preserve_stream}
                        THEN {table}.classification_version ELSE excluded.classification_version END,
                     data_class = CASE WHEN {preserve_class}
                        THEN {table}.data_class ELSE excluded.data_class END,
                     data_class_rationale = CASE WHEN {preserve_class}
                        THEN {table}.data_class_rationale ELSE excluded.data_class_rationale END,
                     data_classification_method = CASE WHEN {preserve_class}
                        THEN {table}.data_classification_method ELSE excluded.data_classification_method END,
                     data_classification_version = CASE WHEN {preserve_class}
                        THEN {table}.data_classification_version ELSE excluded.data_classification_version END,
                     status = CASE WHEN {table}.status IN ('archived','trashed','missing','executed')
                        THEN 'proposed' ELSE {table}.status END,
                     gmail_location = 'inbox',
                     gmail_observed_at = {now},
                     gmail_sync_status = 'synced',
                     gmail_sync_error = NULL,
                     purge_after = NULL,
                     last_seen = {now}",
                prefix = self.prefix,
                table = table,
                preserve_class = preserve_class,
                preserve_stream = preserve_stream,
                internal_date = internal_date,
                now = sjel_store::NOW
            ),
            params![&item.id,
                &item.from_addr,
                &subject,
                &snippet,
                &internal_secs,
                &item.stream,
                &item.rationale,
                &item.classification_method,
                &item.classification_version,
                &item.data_class,
                &item.data_class_rationale,
                &item.data_classification_method,
                &item.data_classification_version,
                &verdict.map(|verdict| verdict.decided_by.as_str()),
            ],
        )?;
        // Inside the transaction the row itself is written in, not in a second
        // connection after it: the rung is what decides whether the model rung
        // may look at this thread, and a crash between the two writes would
        // leave a row nothing records a rung for. `ON CONFLICT DO UPDATE`
        // rather than `INSERT OR IGNORE`, because a resweep after a rule edit
        // is exactly when the stored rung stops being true.
        if let Some(verdict) = verdict {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_triage_rules
                        (triage_id, decided_by, stream, rationale, rules_version, decided_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, {now})
                     ON CONFLICT (triage_id) DO UPDATE SET
                         decided_by = excluded.decided_by,
                         stream = excluded.stream,
                         rationale = excluded.rationale,
                         rules_version = excluded.rules_version,
                         decided_at = excluded.decided_at",
                    prefix = self.prefix,
                    now = sjel_store::NOW
                ),
                params![
                    &item.id,
                    verdict.decided_by.as_str(),
                    &verdict.stream,
                    &verdict.rationale,
                    crate::rules::MAIL_RULES_VERSION,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(is_new)
    }

    /// The deterministic verdict stored beside one thread, if a sweep that knew
    /// about rungs has written it.
    ///
    /// `None` for a row swept before `{prefix}_triage_rules` existed and for a
    /// row whose only writer was a human. Read by the model rung's eligibility
    /// query and by `POST /triage/classify/revert`.
    pub fn triage_rules_verdict(
        &self,
        triage_id: &str,
    ) -> Result<Option<RulesVerdictRow>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT triage_id, decided_by, stream, rationale, rules_version
                       FROM {}_triage_rules WHERE triage_id = ?1",
                    self.prefix
                ),
                params![&triage_id],
                |row| {
                    Ok(RulesVerdictRow {
                        triage_id: row.get(0)?,
                        decided_by: row.get(1)?,
                        stream: row.get(2)?,
                        rationale: row.get(3)?,
                        rules_version: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Record a human category correction without resolving the proposal. The
    /// separate classification provenance is what prevents the next sweep from
    /// overwriting the correction with a deterministic rule result.
    ///
    /// One transaction over three writes, because the category decides the
    /// other two. `steuern` and `belege` are Others by rule
    /// (`content_item::mail_others_reason`), so moving a thread into one of
    /// them raises its class — and a row at the new `c2` still holding the
    /// verbatim subject `c2` exists to hide is exactly the gap
    /// `set_triage_data_class` was given a transaction to close. This is the
    /// human path, and it is the ONLY path allowed to raise a class here:
    /// `apply_model_stream` refuses a class-escalating proposal outright,
    /// because the class UPDATE is escalation-only and the narrowing is
    /// deliberately not a delete.
    ///
    /// `classification` is re-derived by the caller from the stored row and the
    /// new stream, the way `refresh_one_triage_class` does it — the people
    /// registry lives above the store, and a store that read it would be a
    /// second classifier.
    pub fn set_triage_stream(
        &self,
        id: &str,
        stream: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<StreamWrite, Box<dyn std::error::Error>> {
        if !crate::rules::STREAMS.contains(&stream) {
            return Err(format!(
                "invalid triage stream '{stream}' -- must be one of: {}",
                crate::rules::STREAMS.join(", ")
            )
            .into());
        }
        let mut conn = self.conn()?;
        // Immediate, for the reason `upsert_triage` states: this writes, then
        // reads the settled class back, then writes again.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    stream = ?1,
                    rationale = 'Category set manually in Axon.',
                    classification_method = 'human',
                    classification_version = 'manual-v1'
                 WHERE id = ?2",
                self.prefix
            ),
            params![&stream, &id],
        )?;
        if affected == 0 {
            return Ok(StreamWrite::default());
        }
        let class_changed = transaction.execute(
            &self.refresh_data_class_sql(),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )? > 0;
        // Read back rather than assume the argument: the escalation guard can
        // refuse the write, and what the row NOW HOLDS is what the redaction is
        // judged against.
        let Some((stored_class, subject, snippet)) = transaction
            .query_row(
                &format!(
                    "SELECT data_class, subject, snippet FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(StreamWrite::default());
        };
        let narrowed = narrow_stored_material(
            &transaction,
            &self.prefix,
            id,
            &stored_class,
            subject.as_deref(),
            snippet.as_deref(),
        )?;
        transaction.commit()?;
        Ok(StreamWrite {
            changed: true,
            class_changed,
            narrowed,
        })
    }

    /// Set a mail's class by hand. Same rule as [`Store::set_feed_data_class`],
    /// decided by the same function, on the other table.
    ///
    /// One transaction over both halves, for the reason the sweep redacts
    /// before it writes and the refresh pass narrows in the same pass: an
    /// operator selecting **Others** or **Secret** in the dashboard is saying
    /// the stored subject is material this row may not hold. Writing only the
    /// class would leave the row labelled Redacted -- which is what the
    /// dashboard prints from the class alone -- while the one-time code the
    /// rules never matched stays in `subject` until somebody remembers a second
    /// endpoint. The receipt says whether it narrowed, so the operator sees
    /// which of the two happened rather than assuming.
    pub fn set_triage_data_class(
        &self,
        id: &str,
        data_class: &str,
        rationale: Option<&str>,
    ) -> Result<ClassWrite, Box<dyn std::error::Error>> {
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
        let mut conn = self.conn()?;
        // Immediate, for the reason `upsert_triage` states: read then write.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((stored_class, stored_method, subject, snippet)) = transaction
            .query_row(
                &format!(
                    "SELECT data_class, data_classification_method, subject, snippet
                       FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(ClassWrite::default());
        };
        // A refused reclassification returns here, and the transaction rolls
        // back with it: nothing was written, so there is nothing to undo.
        let classification =
            human_reclassification(&stored_class, &stored_method, data_class, rationale)?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
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
        let narrowed = narrow_stored_material(
            &transaction,
            &self.prefix,
            id,
            &classification.value,
            subject.as_deref(),
            snippet.as_deref(),
        )?;
        transaction.commit()?;
        Ok(ClassWrite {
            changed: affected > 0,
            narrowed,
        })
    }

    /// Refresh a rule-produced data class while preserving an explicit human
    /// override. Returns false for a missing item or a preserved override.
    ///
    /// The rank comparison in the WHERE clause is the escalation rule, and it
    /// is here rather than only in the human path because this is the one that
    /// runs unattended: a rule edit that made the classifier *less* suspicious
    /// would otherwise walk the whole table quietly downgrading rows it had
    /// previously called Secret. It may raise a class, never lower one, and
    /// the human override is preserved on top of that.
    pub fn refresh_triage_data_class(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &self.refresh_data_class_sql(),
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

    /// [`Store::refresh_triage_data_class`] and the narrowing that class
    /// demands, as one transaction.
    ///
    /// They are one decision, and the sweep already takes them as one: classify,
    /// then redact before the row is written. Split across two connections a
    /// crash between them leaves the row at the new `c2` holding the verbatim
    /// subject `c2` exists to hide -- the exact state the pass set out to
    /// remove, and invisible in the dashboard, which prints "Redacted" from the
    /// class alone.
    ///
    /// The class the redaction is judged against is read back inside the
    /// transaction rather than assumed from the argument: the escalation guard
    /// can refuse the write (a stored `c3` against a re-derived `c2`), and what
    /// the row *now holds* is what the row is.
    pub fn refresh_triage_data_class_and_redact(
        &self,
        id: &str,
        classification: &crate::content_item::DataClass,
    ) -> Result<ClassWrite, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        // Immediate, for the reason `upsert_triage` states: this one writes then
        // reads back and writes again, which is the same upgrade hazard.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = transaction.execute(
            &self.refresh_data_class_sql(),
            params![
                &classification.value,
                &classification.rationale,
                &classification.method,
                &classification.version,
                &id,
            ],
        )?;
        let Some((stored_class, subject, snippet)) = transaction
            .query_row(
                &format!(
                    "SELECT data_class, subject, snippet FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(ClassWrite::default());
        };
        let narrowed = narrow_stored_material(
            &transaction,
            &self.prefix,
            id,
            &stored_class,
            subject.as_deref(),
            snippet.as_deref(),
        )?;
        transaction.commit()?;
        Ok(ClassWrite {
            changed: affected > 0,
            narrowed,
        })
    }

    /// The escalation-guarded class UPDATE, written once because two callers
    /// issue it. The rank comparison in the WHERE clause *is* the escalation
    /// rule; a second copy is how one of them would come to be missing it.
    fn refresh_data_class_sql(&self) -> String {
        format!(
            "UPDATE {}_triage_items SET
                data_class = ?1,
                data_class_rationale = ?2,
                data_classification_method = ?3,
                data_classification_version = ?4
             WHERE id = ?5 AND data_classification_method <> 'human'
               AND (CASE ?1 WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 ELSE 0 END)
                 >= (CASE data_class
                         WHEN 'c3' THEN 30 WHEN 'c2' THEN 20 WHEN 'c1' THEN 10 ELSE 0 END)",
            self.prefix
        )
    }

    /// Overwrite a stored row's review fields with their redacted form, and
    /// narrow the model verdict the same class governs.
    ///
    /// Deliberately the only write that narrows these two columns, and
    /// deliberately not a delete: the proposal, its decision and its Gmail
    /// identity all stay reviewable — only the material that should never have
    /// been persisted goes. A resweep cannot undo it, because the sweep now
    /// redacts before it writes (see `intake`).
    ///
    /// `data_class` is a parameter because this is the route that exists to
    /// remediate persisted `c2` and `c3` review fields, and a model verdict's
    /// two sentences are review fields of exactly that kind. Both writes commit
    /// together: a route whose whole contract is "run it twice and the second
    /// run reports zero" must not be able to finish half of one row.
    pub fn redact_triage_review_fields(
        &self,
        id: &str,
        data_class: &str,
        subject: Option<&str>,
        snippet: Option<&str>,
    ) -> Result<RedactWrite, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET subject = ?1, snippet = ?2 WHERE id = ?3",
                self.prefix
            ),
            params![&subject, &snippet, &id],
        )?;
        let verdict_narrowed = narrow_model_verdict(&transaction, &self.prefix, id, data_class)?;
        transaction.commit()?;
        Ok(RedactWrite {
            changed: affected > 0,
            verdict_narrowed,
        })
    }

    // -- the model rung -----------------------------------------------------

    /// The threads the model rung may look at, newest first, with whatever
    /// verdict each already carries.
    ///
    /// Four filters, and each one is load-bearing:
    /// - `decided_by = 'fallback'` — the rung exists for the rows the rules did
    ///   not decide. Today those are the same 102 rows as `stream = 'aktiv'`,
    ///   but the moment an overlay rule declares `aktiv` the two sets diverge,
    ///   which is why the rung is persisted rather than string-matched.
    /// - `classification_method = 'deterministic'` — not merely `<> 'human'`.
    ///   An already-applied model row is not re-prompted, which is what closes
    ///   the second-pass loop.
    /// - `status IN ('proposed','approved')` — an archived or trashed thread
    ///   has no category decision left to make.
    /// - a joined verdict, so the caller can compare `item_revision`.
    ///
    /// `limit` is applied by the CALLER, after the `item_revision` comparison,
    /// because SQL cannot compute that hash. The coarse filters already cut the
    /// table to the fallback rows, so this reads a hundred-odd rows and not the
    /// whole file.
    pub fn model_rung_candidates(&self) -> Result<Vec<ModelCandidate>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT i.id, i.from_addr, i.subject, i.snippet, i.data_class,
                        r.stream, r.rationale, r.decided_by,
                        v.producer, v.prompt_revision, v.item_revision, v.state,
                        v.mode, v.attempts,
                        (v.next_attempt IS NULL OR v.next_attempt <= {now})
                   FROM {prefix}_triage_items i
                   JOIN {prefix}_triage_rules r ON r.triage_id = i.id
                   LEFT JOIN {prefix}_triage_model_verdicts v ON v.triage_id = i.id
                  WHERE r.decided_by = 'fallback'
                    AND i.classification_method = 'deterministic'
                    AND i.status IN ('proposed','approved')
                  ORDER BY i.internal_date DESC NULLS LAST",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            [],
            |row| {
                let producer: Option<String> = row.get(8)?;
                Ok(ModelCandidate {
                    id: row.get(0)?,
                    from_addr: row.get(1)?,
                    subject: row.get(2)?,
                    snippet: row.get(3)?,
                    data_class: row.get(4)?,
                    rule_stream: row.get(5)?,
                    rule_rationale: row.get(6)?,
                    rule_decided_by: row.get(7)?,
                    stored: match producer {
                        Some(producer) => Some(StoredVerdictState {
                            producer,
                            prompt_revision: row.get(9)?,
                            item_revision: row.get(10)?,
                            state: row.get(11)?,
                            mode: row.get(12)?,
                            attempts: row.get(13)?,
                            backoff_expired: row.get(14)?,
                        }),
                        None => None,
                    },
                })
            },
        )?)
    }

    /// What classified the open mailbox, as counts per method.
    ///
    /// Largest first, then by name, so a heading a human compares against
    /// yesterday's does not reshuffle. The same `status` filter the board uses,
    /// so the number beside "Current method" counts the rows on screen.
    ///
    /// In SQL rather than in the dashboard: frontend renders, backend computes.
    pub fn triage_classification_methods(
        &self,
    ) -> Result<Vec<(String, usize)>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT classification_method, COUNT(*) AS n
                   FROM {}_triage_items
                  WHERE status IN ('proposed','approved')
                  GROUP BY classification_method
                  ORDER BY n DESC, classification_method ASC",
                self.prefix
            ),
            [],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)?.max(0) as usize)),
        )?)
    }

    /// How many threads the rung may look at at all, without reading one.
    ///
    /// The same three filters as [`Store::model_rung_candidates`], and it
    /// exists because the report only ever wanted the number: the report route
    /// states that the query behind it must be unable to return mail text, and
    /// counting rows by materialising every `from_addr`, `subject` and
    /// `snippet` undercut its own guarantee (review, 2026-09-05).
    pub fn model_rung_candidate_count(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*)
                   FROM {prefix}_triage_items i
                   JOIN {prefix}_triage_rules r ON r.triage_id = i.id
                  WHERE r.decided_by = 'fallback'
                    AND i.classification_method = 'deterministic'
                    AND i.status IN ('proposed','approved')",
                prefix = self.prefix
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    }

    /// Store one verdict, replacing whatever this thread carried.
    ///
    /// Whole-row replacement rather than a partial update: a row with a new
    /// state and a previous run's rationale would describe two runs at once,
    /// and the report reads both columns.
    ///
    /// `next_attempt` is DB-owned and the struct's value is ignored on write,
    /// and `applied_at` is derived here too: a verdict stored as `applied`
    /// carries the stamp of the write that moved the category, unless it
    /// already had one. The deadline is
    /// derived here from the state and the attempt count, in the canonical
    /// stamp format the column's other values are in — `sjel_store::now_offset`
    /// exists because `datetime('now','+1 minute')` renders 19 characters into
    /// a column that holds 29, and one column at two widths stops `ORDER BY`
    /// being time order.
    pub fn upsert_model_verdict(
        &self,
        verdict: &ModelVerdict,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // ?3 is the state and ?18 the attempt count: a retryable state below the
        // cap arms a growing backoff, everything else clears it. `MIN(...,5)`
        // is the ceiling the Gmail action queue already uses, so the two
        // ledgers back off alike. Built here rather than inline, because a
        // `format!` inside a `format!` argument is a lint and reads worse.
        let backoff_arm = format!(
            "CASE WHEN ?3 IN ({retryable}) AND ?18 < {cap} THEN {offset} ELSE NULL END",
            retryable = retryable_model_verdict_states_sql(),
            cap = MAX_MODEL_VERDICT_ATTEMPTS,
            offset = sjel_store::now_offset("'+' || MIN(?18 + 1, 5) || ' minutes'"),
        );
        // ?2 is the mode. `applied_at` is DB-owned for the same reason
        // `next_attempt` is: a machine write onto the category axis has to
        // leave a stamp, and a stamp assembled in Rust would be a second
        // clock and a second format in one column. The struct's value is kept
        // when it has one, so re-storing an applied verdict does not restamp
        // it, and `revert_model_streams` clearing the column still means what
        // it says (review, 2026-09-05).
        let applied_arm = format!(
            "CASE WHEN ?2 = 'applied' THEN COALESCE(?21,{now}) ELSE ?21 END",
            now = sjel_store::NOW
        );
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_triage_model_verdicts
                    (triage_id, mode, state, rule_decided_by, rule_stream, model_stream,
                     confidence_bp, urgency_bp, rationale, urgency_rationale, redactions,
                     data_class, redaction_class, producer, item_revision, prompt_revision,
                     classification_version, attempts, last_error, next_attempt, held_reason,
                     applied_at, decided_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,
                         {backoff_arm},?20,{applied_arm},{now})
                 ON CONFLICT (triage_id) DO UPDATE SET
                     mode = excluded.mode,
                     state = excluded.state,
                     rule_decided_by = excluded.rule_decided_by,
                     rule_stream = excluded.rule_stream,
                     model_stream = excluded.model_stream,
                     confidence_bp = excluded.confidence_bp,
                     urgency_bp = excluded.urgency_bp,
                     rationale = excluded.rationale,
                     urgency_rationale = excluded.urgency_rationale,
                     redactions = excluded.redactions,
                     data_class = excluded.data_class,
                     redaction_class = excluded.redaction_class,
                     producer = excluded.producer,
                     item_revision = excluded.item_revision,
                     prompt_revision = excluded.prompt_revision,
                     classification_version = excluded.classification_version,
                     attempts = excluded.attempts,
                     last_error = excluded.last_error,
                     next_attempt = excluded.next_attempt,
                     held_reason = excluded.held_reason,
                     applied_at = excluded.applied_at,
                     decided_at = excluded.decided_at",
                prefix = self.prefix,
                now = sjel_store::NOW,
                backoff_arm = backoff_arm,
                applied_arm = applied_arm,
            ),
            params![
                &verdict.triage_id,
                &verdict.mode,
                &verdict.state,
                &verdict.rule_decided_by,
                &verdict.rule_stream,
                &verdict.model_stream,
                &verdict.confidence_bp,
                &verdict.urgency_bp,
                &verdict.rationale,
                &verdict.urgency_rationale,
                &verdict.redactions,
                &verdict.data_class,
                &verdict.redaction_class,
                &verdict.producer,
                &verdict.item_revision,
                &verdict.prompt_revision,
                &verdict.classification_version,
                &verdict.attempts,
                &verdict.last_error,
                &verdict.held_reason,
                &verdict.applied_at,
            ],
        )?;
        Ok(())
    }

    /// Write one accepted model proposal onto the category axis.
    ///
    /// The stream and nothing else. It refuses a proposal that would raise the
    /// data class before it opens a transaction, because the caller must have
    /// stored that verdict `held` instead: the class UPDATE is escalation-only
    /// and the narrowing that follows it "is deliberately not a delete", so a
    /// wrong `belege` from an uncalibrated model on a thread nobody read would
    /// leave a permanently Others row with a permanently redacted subject. A
    /// machine may not open that door; the human confirm path
    /// ([`Store::set_triage_stream`]) may, and says so in the button.
    ///
    /// The `WHERE` clause is the method ladder for an incoming `model`: rank 20
    /// takes a legacy, deterministic or older model row and loses to a human.
    pub fn apply_model_stream(
        &self,
        verdict: &ModelVerdict,
    ) -> Result<ModelWrite, Box<dyn std::error::Error>> {
        let Some(stream) = verdict.model_stream.as_deref() else {
            return Err("a verdict with no model stream cannot be applied".into());
        };
        if !crate::rules::STREAMS.contains(&stream) {
            return Err(format!("invalid triage stream '{stream}'").into());
        }
        if crate::content_item::class_rank(&verdict.redaction_class)
            > crate::content_item::class_rank(&verdict.data_class)
        {
            return Err(format!(
                "applying '{stream}' would raise this mail from {} to {}; \
                 a class-raising proposal is held for a human",
                verdict.data_class, verdict.redaction_class
            )
            .into());
        }
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {prefix}_triage_items SET
                    stream = ?1,
                    rationale = ?2,
                    classification_method = 'model',
                    classification_version = ?3
                 WHERE id = ?4
                   AND (CASE classification_method WHEN 'human' THEN 30 WHEN 'model' THEN 20
                        WHEN 'deterministic' THEN 10 ELSE 0 END) <= 20",
                prefix = self.prefix
            ),
            params![
                &stream,
                verdict
                    .rationale
                    .as_deref()
                    .unwrap_or("Classified by the local model rung."),
                &verdict.classification_version,
                &verdict.triage_id,
            ],
        )?;
        transaction.commit()?;
        Ok(ModelWrite {
            stream_changed: affected > 0,
        })
    }

    /// Put the deterministic verdict back on every row this rung moved.
    ///
    /// The bulk rollback: "the model was wrong about a class of mail" has to be
    /// one command rather than a hundred clicks. It restores stream, rationale,
    /// classification_method and classification_version from
    /// `{prefix}_triage_rules` and marks the verdict `shadow` again.
    ///
    /// It cannot restore a lowered class or an un-narrowed subject, and it does
    /// not pretend to: `refresh_data_class_sql` is escalation-only and the
    /// narrowing is permanent. That is the whole reason apply never escalates,
    /// and the route says it in words.
    ///
    /// Returns `(reverted, skipped_not_model, skipped_no_rules_row)`.
    pub fn revert_model_streams(
        &self,
        ids: Option<&[String]>,
    ) -> Result<(usize, usize, usize), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let selected: Vec<(String, String, Option<String>)> = {
            let mut statement = transaction.prepare(&format!(
                "SELECT i.id, i.classification_method, r.stream
                   FROM {prefix}_triage_items i
                   LEFT JOIN {prefix}_triage_rules r ON r.triage_id = i.id
                  WHERE i.id IN (SELECT triage_id FROM {prefix}_triage_model_verdicts
                                  WHERE mode = 'applied')
                     OR i.classification_method = 'model'",
                prefix = self.prefix
            ))?;
            let rows = statement.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, Option<String>>(2)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let wanted: Option<std::collections::HashSet<&str>> =
            ids.map(|ids| ids.iter().map(String::as_str).collect());

        let mut reverted = 0usize;
        let mut skipped_not_model = 0usize;
        let mut skipped_no_rules_row = 0usize;
        for (id, method, rules_stream) in &selected {
            if let Some(wanted) = &wanted {
                if !wanted.contains(id.as_str()) {
                    continue;
                }
            }
            if method != crate::content_item::METHOD_MODEL {
                skipped_not_model += 1;
                continue;
            }
            if rules_stream.is_none() {
                // No deterministic verdict to put back. Leaving the model row
                // in place is the honest answer: overwriting it with a guess
                // would be a third classifier nobody asked for.
                skipped_no_rules_row += 1;
                continue;
            }
            transaction.execute(
                &format!(
                    "UPDATE {prefix}_triage_items SET
                        stream = (SELECT stream FROM {prefix}_triage_rules WHERE triage_id = ?1),
                        rationale = (SELECT rationale FROM {prefix}_triage_rules WHERE triage_id = ?1),
                        classification_method = 'deterministic',
                        classification_version =
                            (SELECT rules_version FROM {prefix}_triage_rules WHERE triage_id = ?1)
                     WHERE id = ?1 AND classification_method = 'model'",
                    prefix = self.prefix
                ),
                params![&id],
            )?;
            transaction.execute(
                &format!(
                    "UPDATE {prefix}_triage_model_verdicts
                        SET mode = 'shadow', applied_at = NULL
                      WHERE triage_id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
            )?;
            reverted += 1;
        }
        transaction.commit()?;
        Ok((reverted, skipped_not_model, skipped_no_rules_row))
    }

    /// Every stored verdict, as the report counts them.
    ///
    /// The SELECT list carries no `rationale` and no `urgency_rationale` on
    /// purpose. The report body is meant to be safe to log and to paste into a
    /// decision record, and the cheapest way to keep it so is for the query
    /// that feeds it to be unable to return the text.
    pub fn model_verdict_summaries(
        &self,
        mode: Option<&str>,
    ) -> Result<Vec<ModelVerdictSummary>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let base = format!(
            "SELECT triage_id, mode, state, rule_stream, model_stream, confidence_bp,
                    urgency_bp, data_class, held_reason, producer, prompt_revision
               FROM {}_triage_model_verdicts",
            self.prefix
        );
        let read = |row: &rusqlite::Row| {
            Ok(ModelVerdictSummary {
                triage_id: row.get(0)?,
                mode: row.get(1)?,
                state: row.get(2)?,
                rule_stream: row.get(3)?,
                model_stream: row.get(4)?,
                confidence_bp: row.get(5)?,
                urgency_bp: row.get(6)?,
                data_class: row.get(7)?,
                held_reason: row.get(8)?,
                producer: row.get(9)?,
                prompt_revision: row.get(10)?,
            })
        };
        Ok(match mode {
            Some(mode) => conn.query_all(
                &format!("{base} WHERE mode = ?1 ORDER BY decided_at DESC"),
                params![&mode],
                read,
            )?,
            None => conn.query_all(&format!("{base} ORDER BY decided_at DESC"), [], read)?,
        })
    }

    /// Every stored verdict in full, keyed by thread, for the reader contract.
    ///
    /// One query before the loop rather than one per item: `triage_handler`
    /// already issues a relevance query per item, and a second per-item query
    /// would double that over the whole table for no reason.
    pub fn model_verdicts(
        &self,
    ) -> Result<std::collections::HashMap<String, ModelVerdict>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let rows: Vec<ModelVerdict> = conn.query_all(
            &format!(
                "SELECT triage_id, mode, state, rule_decided_by, rule_stream, model_stream,
                        confidence_bp, urgency_bp, rationale, urgency_rationale, redactions,
                        data_class, redaction_class, producer, item_revision, prompt_revision,
                        classification_version, attempts, last_error, next_attempt, held_reason,
                        applied_at
                   FROM {}_triage_model_verdicts",
                self.prefix
            ),
            [],
            |row| {
                Ok(ModelVerdict {
                    triage_id: row.get(0)?,
                    mode: row.get(1)?,
                    state: row.get(2)?,
                    rule_decided_by: row.get(3)?,
                    rule_stream: row.get(4)?,
                    model_stream: row.get(5)?,
                    confidence_bp: row.get(6)?,
                    urgency_bp: row.get(7)?,
                    rationale: row.get(8)?,
                    urgency_rationale: row.get(9)?,
                    redactions: row.get(10)?,
                    data_class: row.get(11)?,
                    redaction_class: row.get(12)?,
                    producer: row.get(13)?,
                    item_revision: row.get(14)?,
                    prompt_revision: row.get(15)?,
                    classification_version: row.get(16)?,
                    attempts: row.get(17)?,
                    last_error: row.get(18)?,
                    next_attempt: row.get(19)?,
                    held_reason: row.get(20)?,
                    applied_at: row.get(21)?,
                })
            },
        )?;
        Ok(rows
            .into_iter()
            .map(|verdict| (verdict.triage_id.clone(), verdict))
            .collect())
    }

    /// Record the `Waiting` label locally, after Gmail has already accepted it.
    ///
    /// Ordering is the point: Gmail first, this second. A local flag written
    /// optimistically and then a failed modify call leaves the dashboard showing
    /// a state the mailbox does not have, and the operator's inbox is the thing
    /// they actually look at.
    ///
    /// `waiting_at` is cleared rather than kept on unset. "Waiting since" is only
    /// meaningful while it is true, and a stale timestamp on a cleared row reads
    /// like a currently-blocked thread in any query that forgets to check the
    /// boolean.
    pub fn set_triage_waiting(
        &self,
        id: &str,
        waiting: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items
                    SET waiting = ?1,
                        waiting_at = CASE WHEN ?1 THEN {now} ELSE NULL END
                  WHERE id = ?2",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&waiting, &id],
        )?;
        Ok(affected > 0)
    }

    pub fn set_triage_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::TRIAGE_STATUSES.contains(&status) {
            return Err(format!(
                "invalid triage status '{status}' -- must be one of: {}",
                Self::TRIAGE_STATUSES.join(", ")
            )
            .into());
        }
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET status = ?1 WHERE id = ?2",
                self.prefix
            ),
            params![&status, &id],
        )?;
        Ok(affected > 0)
    }

    /// Persist the local half of a Gmail lifecycle action. Callers must invoke
    /// this only after Gmail has confirmed the matching mutation.
    pub fn record_gmail_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !matches!(action, "archive" | "trash" | "restore") {
            return Err("Gmail action must be archive, trash, or restore".into());
        }
        let conn = self.conn()?;
        let affected = match action {
            "archive" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'archived', gmail_action = 'archive',
                        gmail_action_at = {now}, purge_after = NULL,
                        gmail_location = 'archive', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW
                ),
                params![&id],
            )?,
            "trash" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'trashed', gmail_action = 'trash',
                        gmail_action_at = {now}, purge_after = {purge},
                        gmail_location = 'trash', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW,
                    purge = sjel_store::now_offset("'+30 days'")
                ),
                params![&id],
            )?,
            "restore" => conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'proposed', gmail_action = 'restore',
                        gmail_action_at = {now}, purge_after = NULL,
                        gmail_location = 'inbox', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL
                     WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW
                ),
                params![&id],
            )?,
            _ => unreachable!(),
        };
        Ok(affected > 0)
    }

    /// Write intent before contacting Gmail. A single queued job per thread
    /// prevents conflicting retries while allowing completed history to remain.
    pub fn queue_gmail_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<GmailActionJob, Box<dyn std::error::Error>> {
        if !matches!(action, "archive" | "trash" | "restore") {
            return Err("Gmail action must be archive, trash, or restore".into());
        }
        let mut conn = self.conn()?;
        // No `FOR UPDATE`: SQLite has no row locks and needs none here. The
        // transaction is the lock, because there is exactly one writer.
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let Some(source_status) = transaction
            .query_row(
                &format!(
                    "SELECT status FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Err("mail proposal not found".into());
        };
        let allowed = match action {
            "archive" | "trash" => matches!(source_status.as_str(), "proposed" | "approved"),
            "restore" => matches!(source_status.as_str(), "archived" | "trashed"),
            _ => false,
        };
        if !allowed {
            return Err(format!("cannot {action} mail in {source_status} state").into());
        }
        if transaction
            .query_row(
                &format!(
                    "SELECT job_id FROM {}_gmail_action_jobs
                     WHERE triage_id = ?1 AND state = 'queued'",
                    self.prefix
                ),
                params![&id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some()
        {
            return Err("a Gmail action is already queued for this mail".into());
        }
        let job = transaction.query_row(
            &format!(
                "INSERT INTO {}_gmail_action_jobs (triage_id, action, source_status)
                 VALUES (?1,?2,?3)
                 RETURNING job_id, triage_id, action, source_status, attempts",
                self.prefix
            ),
            params![&id, &action, &source_status],
            |job| {
                Ok(GmailActionJob {
                    job_id: job.get(0)?,
                    triage_id: job.get(1)?,
                    action: job.get(2)?,
                    source_status: job.get(3)?,
                    attempts: job.get(4)?,
                })
            },
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(job)
    }

    /// Complete both halves of local state atomically after Gmail is known to
    /// be at the requested location. Replaying a completed job is harmless.
    pub fn complete_gmail_action(&self, job_id: i64) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let Some((id, action, state)) = transaction
            .query_row(
                &format!(
                    "SELECT triage_id, action, state FROM {}_gmail_action_jobs
                     WHERE job_id = ?1",
                    self.prefix
                ),
                params![&job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(false);
        };
        if state == "completed" {
            return Ok(true);
        }
        if state != "queued" {
            return Err("Gmail action job is no longer retryable".into());
        }
        let affected = match action.as_str() {
            "archive" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'archived', gmail_action = 'archive', gmail_action_at = {now},
                        purge_after = NULL, gmail_location = 'archive', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW
                ),
                params![&id],
            )?,
            "trash" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'trashed', gmail_action = 'trash', gmail_action_at = {now},
                        purge_after = COALESCE(purge_after, {purge}),
                        gmail_location = 'trash', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW,
                    purge = sjel_store::now_offset("'+30 days'")
                ),
                params![&id],
            )?,
            "restore" => transaction.execute(
                &format!(
                    "UPDATE {}_triage_items SET
                        status = 'proposed', gmail_action = 'restore', gmail_action_at = {now},
                        purge_after = NULL, gmail_location = 'inbox', gmail_observed_at = {now},
                        gmail_sync_status = 'synced', gmail_sync_error = NULL WHERE id = ?1",
                    self.prefix,
                    now = sjel_store::NOW
                ),
                params![&id],
            )?,
            _ => return Err("stored Gmail action is invalid".into()),
        };
        if affected == 0 {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'completed', updated_at = {now}, completed_at = {now}, last_error = NULL
                 WHERE job_id = ?1",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn fail_gmail_action(
        &self,
        job_id: i64,
        error: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let bounded_error = error.chars().take(240).collect::<String>();
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        // `LEAST(attempts + 1, 5)` becomes SQLite's two-argument `MIN`, and the
        // whole `now() + interval '1 minute' * n` becomes one `now_offset` with a
        // computed modifier -- so the deadline lands in the canonical format the
        // column's other values are in.
        let Some((triage_id, state)) = transaction
            .query_row(
                &format!(
                    "UPDATE {}_gmail_action_jobs SET
                        attempts = attempts + 1,
                        state = CASE WHEN attempts + 1 >= 5 THEN 'abandoned' ELSE 'queued' END,
                        last_error = ?2, updated_at = {now},
                        next_attempt = {backoff}
                     WHERE job_id = ?1 AND state = 'queued'
                     RETURNING triage_id, state",
                    self.prefix,
                    now = sjel_store::NOW,
                    backoff = sjel_store::now_offset("'+' || MIN(attempts + 1, 5) || ' minutes'")
                ),
                params![&job_id, &bounded_error],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Err("Gmail action job is not queued".into());
        };
        let sync_status = if state == "abandoned" {
            "attention"
        } else {
            "retrying"
        };
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET gmail_sync_status = ?1, gmail_sync_error = ?2
                 WHERE id = ?3",
                self.prefix
            ),
            params![&sync_status, &bounded_error, &triage_id],
        )?;
        transaction.commit()?;
        Ok(state)
    }

    /// Reset the newest attention job after an explicit operator decision.
    /// The action and its original source state are preserved; only the bounded
    /// attempt window is reopened.
    pub fn retry_abandoned_gmail_action(
        &self,
        id: &str,
    ) -> Result<GmailActionJob, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let Some(job) = transaction
            .query_row(
                &format!(
                    "SELECT job_id, triage_id, action, source_status
                     FROM {}_gmail_action_jobs
                     WHERE triage_id = ?1 AND state = 'abandoned'
                     ORDER BY job_id DESC LIMIT 1",
                    self.prefix
                ),
                params![&id],
                |row| {
                    Ok(GmailActionJob {
                        job_id: row.get(0)?,
                        triage_id: row.get(1)?,
                        action: row.get(2)?,
                        source_status: row.get(3)?,
                        attempts: 0,
                    })
                },
            )
            .optional()?
        else {
            return Err("no Gmail action needs operator attention".into());
        };
        let job_id = job.job_id;
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'queued', attempts = 0, last_error = NULL,
                    next_attempt = {now}, updated_at = {now}, completed_at = NULL
                 WHERE job_id = ?1",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&job_id],
        )?;
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET gmail_sync_status = 'queued', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(job)
    }

    /// Cancel only an abandoned job. Queued jobs may already be in flight in
    /// the maintenance worker, so canceling them would create an ambiguous
    /// Gmail/local split.
    pub fn cancel_abandoned_gmail_action(
        &self,
        id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let canceled = transaction
            .query_row(
                &format!(
                    "UPDATE {}_gmail_action_jobs SET
                        state = 'canceled', updated_at = {now}, completed_at = {now}
                     WHERE job_id = (
                        SELECT job_id FROM {}_gmail_action_jobs
                        WHERE triage_id = ?1 AND state = 'abandoned'
                        ORDER BY job_id DESC LIMIT 1
                     )
                     RETURNING triage_id",
                    self.prefix,
                    self.prefix,
                    now = sjel_store::NOW
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if canceled.is_none() {
            return Ok(false);
        }
        transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    gmail_sync_status = CASE WHEN gmail_location IS NULL THEN NULL ELSE 'synced' END,
                    gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn pending_gmail_actions(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailActionJob>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT job_id, triage_id, action, source_status, attempts
                 FROM {}_gmail_action_jobs
                 WHERE state = 'queued' AND next_attempt <= {now}
                 ORDER BY next_attempt, job_id LIMIT ?1",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![limit.clamp(1, 100)],
            |row| {
                Ok(GmailActionJob {
                    job_id: row.get(0)?,
                    triage_id: row.get(1)?,
                    action: row.get(2)?,
                    source_status: row.get(3)?,
                    attempts: row.get(4)?,
                })
            },
        )?)
    }

    pub fn gmail_reconcile_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<GmailReconcileCandidate>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, status FROM {}_triage_items t
                 WHERE status <> 'dismissed'
                   AND NOT EXISTS (
                     SELECT 1 FROM {}_gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                   )
                 ORDER BY gmail_observed_at ASC NULLS FIRST, last_seen DESC
                 LIMIT ?1",
                self.prefix, self.prefix
            ),
            params![limit.clamp(1, 500)],
            |row| {
                Ok(GmailReconcileCandidate {
                    triage_id: row.get(0)?,
                    status: row.get(1)?,
                })
            },
        )?)
    }

    pub fn observe_gmail_location(
        &self,
        id: &str,
        location: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !matches!(location, "inbox" | "archive" | "trash") {
            return Err("Gmail location must be inbox, archive, or trash".into());
        }
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    status = CASE
                      WHEN ?1 = 'trash' THEN 'trashed'
                      WHEN ?1 = 'archive' THEN 'archived'
                      WHEN status IN ('archived','trashed','missing','executed') THEN 'proposed'
                      ELSE status
                    END,
                    purge_after = CASE
                      WHEN ?1 = 'trash' THEN COALESCE(purge_after, {purge})
                      ELSE NULL
                    END,
                    gmail_location = ?1, gmail_observed_at = {now},
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = ?2",
                self.prefix,
                now = sjel_store::NOW,
                purge = sjel_store::now_offset("'+30 days'")
            ),
            params![&location, &id],
        )?;
        Ok(affected > 0)
    }

    /// Record an authoritative Gmail 404/410 without discarding Axon's local
    /// metadata. Any queued or attention action is closed because Gmail can no
    /// longer apply it. A Trash retention deadline, if present, remains active.
    pub fn observe_gmail_missing(&self, id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(
            &format!(
                "UPDATE {}_gmail_action_jobs SET
                    state = 'canceled', updated_at = {now}, completed_at = {now}
                 WHERE triage_id = ?1 AND state IN ('queued','abandoned')",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&id],
        )?;
        let affected = transaction.execute(
            &format!(
                "UPDATE {}_triage_items SET
                    status = 'missing', gmail_location = 'missing', gmail_observed_at = {now},
                    gmail_sync_status = 'synced', gmail_sync_error = NULL
                 WHERE id = ?1",
                self.prefix,
                now = sjel_store::NOW
            ),
            params![&id],
        )?;
        transaction.commit()?;
        Ok(affected > 0)
    }

    /// Remove expired Trash content and any staged cloud copy. Gmail owns its
    /// own Trash retention; this cleanup is strictly Axon's local copy.
    pub fn purge_expired_trashed(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(
            &format!(
                "DELETE FROM {prefix}_content_cloud_jobs
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {prefix}_triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= {now}
                 )",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            [],
        )?;
        transaction.execute(
            &format!(
                "DELETE FROM {prefix}_content_cloud_derivatives
                 WHERE source = 'mail' AND item_id IN (
                    SELECT id FROM {prefix}_triage_items
                    WHERE status IN ('trashed','missing') AND purge_after <= {now}
                 )",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            [],
        )?;
        let purged = transaction.execute(
            &format!(
                "DELETE FROM {}_triage_items
                 WHERE status IN ('trashed','missing') AND purge_after <= {now}",
                self.prefix,
                now = sjel_store::NOW
            ),
            [],
        )?;
        transaction.commit()?;
        Ok(purged as u64)
    }

    pub fn get_triage_status(
        &self,
        id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status FROM {}_triage_items WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// List triage items, optionally filtered by status, newest first.
    pub fn list_triage(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<TriageItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // The `::text` casts on every timestamp column are gone: they are TEXT now.
        let base = format!(
            "SELECT id, from_addr, subject, snippet, internal_date, stream, rationale,
                    status, first_seen, last_seen,
                    classification_method, classification_version, data_class,
                    data_class_rationale, data_classification_method,
                    data_classification_version, gmail_action,
                    gmail_action_at, purge_after, gmail_location,
                    gmail_observed_at, gmail_sync_status,
                    (SELECT action FROM {prefix}_gmail_action_jobs j
                     WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                     ORDER BY job_id DESC LIMIT 1),
                    gmail_sync_error, waiting, waiting_at
             FROM {prefix}_triage_items t",
            prefix = self.prefix
        );
        Ok(match status {
            Some(s) => conn.query_all(
                &format!("{base} WHERE status = ?1 ORDER BY internal_date DESC NULLS LAST"),
                params![&s],
                row_to_triage,
            )?,
            None => conn.query_all(
                &format!("{base} ORDER BY internal_date DESC NULLS LAST"),
                [],
                row_to_triage,
            )?,
        })
    }

    /// Read one mail proposal for the shared content reader. Gmail-specific
    /// category and action state stays on `triage_items`; the HTTP adapter
    /// projects it into the same content contract as a normal Feed item.
    pub fn get_triage(&self, id: &str) -> Result<Option<TriageItem>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, from_addr, subject, snippet, internal_date, stream, rationale,
                        status, first_seen, last_seen,
                        classification_method, classification_version, data_class,
                        data_class_rationale, data_classification_method,
                        data_classification_version, gmail_action,
                        gmail_action_at, purge_after, gmail_location,
                        gmail_observed_at, gmail_sync_status,
                        (SELECT action FROM {prefix}_gmail_action_jobs j
                         WHERE j.triage_id = t.id AND j.state IN ('queued','abandoned')
                         ORDER BY job_id DESC LIMIT 1),
                        gmail_sync_error, waiting, waiting_at
                     FROM {prefix}_triage_items t WHERE id = ?1",
                    prefix = self.prefix
                ),
                params![&id],
                row_to_triage,
            )
            .optional()?)
    }

    /// Replace the TELOS matches for one mail proposal. This is relevance
    /// annotation only: it never changes the category, proposal status, or a
    /// TELOS source file.
    pub fn replace_triage_relevance(
        &self,
        triage_id: &str,
        matches: &[RelevanceMatch],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(
            &format!(
                "DELETE FROM {}_triage_relevance WHERE triage_id = ?1",
                self.prefix
            ),
            params![&triage_id],
        )?;
        for relevance in matches {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_triage_relevance
                        (triage_id, profile_key, profile_label, score, rationale, mode,
                         profile_revision, scored_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,{now})",
                    prefix = self.prefix,
                    now = sjel_store::NOW
                ),
                params![
                    &triage_id,
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
        Ok(())
    }

    pub fn triage_relevance(
        &self,
        triage_id: &str,
    ) -> Result<Vec<RelevanceMatch>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT profile_key, profile_label, score, rationale, mode, profile_revision
                 FROM {}_triage_relevance WHERE triage_id = ?1 ORDER BY score DESC",
                self.prefix
            ),
            params![&triage_id],
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

/// Which class the row holds once an upsert lands: the stored one, or the
/// incoming one.
///
/// The Rust twin of the `preserve_class` predicate `upsert_triage` builds into
/// its SQL -- the same two clauses in the same order. A human's decision stands,
/// and a rule may never lower a class it once raised. It exists in Rust as well
/// because the answer decides more than a column: the two review fields have to
/// be redacted against the class that wins *before* the row is written, and SQL
/// cannot run the redactor.
///
/// `class_rank` answers `None` for a value outside the vocabulary, and `None`
/// sorts below every `Some` -- which is the `ELSE 0` arm of the SQL CASE, so an
/// unknown class loses to a known one on either side, exactly as it does there.
fn class_after_upsert<'a>(stored: Option<&'a (String, String)>, incoming: &'a str) -> &'a str {
    match stored {
        Some((stored_class, stored_method))
            if stored_method == crate::content_item::METHOD_HUMAN
                || crate::content_item::class_rank(incoming)
                    < crate::content_item::class_rank(stored_class) =>
        {
            stored_class
        }
        _ => incoming,
    }
}

/// Narrow one row's two review fields to what its class admits, on a connection
/// the caller owns.
///
/// A connection rather than the pool, so the class write and this one commit
/// together or not at all. Returns whether anything was removed -- false both
/// for a class that governs nothing (`c0`, `c1`) and for a row already clean,
/// which is what makes a second pass report zero.
fn narrow_review_fields(
    conn: &Connection,
    prefix: &str,
    id: &str,
    data_class: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(remediation) = crate::intake::remediate(data_class, subject, snippet) else {
        return Ok(false);
    };
    if !remediation.changed {
        return Ok(false);
    }
    conn.execute(
        &format!("UPDATE {prefix}_triage_items SET subject = ?1, snippet = ?2 WHERE id = ?3"),
        params![&remediation.subject, &remediation.snippet, &id],
    )?;
    Ok(true)
}

/// Narrow everything this database holds about one mail to what its class
/// admits: the item's two review fields, and the model verdict's two sentences.
///
/// One function rather than two calls at each site, because the second table is
/// exactly the one three callers forgot.
fn narrow_stored_material(
    conn: &Connection,
    prefix: &str,
    id: &str,
    data_class: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let narrowed = narrow_review_fields(conn, prefix, id, data_class, subject, snippet)?;
    // Judged on its own: a row whose subject was already clean can still carry
    // a verdict written while the row sat at a lower class.
    narrow_model_verdict(conn, prefix, id, data_class)?;
    Ok(narrowed)
}

/// Narrow one thread's stored model verdict to what the item's class NOW
/// admits, on a connection the caller owns.
///
/// The second half of a narrowing, and it was missing. `rationale` and
/// `urgency_rationale` are model sentences about the mail, redacted once
/// against the class the row held when the verdict was stored. A row whose
/// class later rises — a human moving it to `belege`, the people registry
/// escalating it on a resweep — left those sentences behind at the old class,
/// and neither `narrow_review_fields` nor `POST /triage/redact` reached them,
/// because both only ever wrote `{prefix}_triage_items` (review, 2026-09-05).
///
/// `data_class` on the verdict row is left alone on purpose: it is the class at
/// PROMPT time, which is what a receipt proves "no Secret mail was prompted"
/// from. `redaction_class` is the one that says what the stored text was
/// narrowed against, so that is the one this moves.
///
/// A class that refuses prompts outright loses the text rather than narrowing
/// it: the rung would never have produced these sentences for a `c3` row, and
/// keeping a redacted derivative of a mail the gate now refuses would be the
/// gap in a smaller form.
fn narrow_model_verdict(
    conn: &Connection,
    prefix: &str,
    id: &str,
    data_class: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some((rationale, urgency_rationale, redaction_class)): Option<(
        Option<String>,
        Option<String>,
        String,
    )> = conn
        .query_row(
            &format!(
                "SELECT rationale, urgency_rationale, redaction_class
                   FROM {prefix}_triage_model_verdicts WHERE triage_id = ?1"
            ),
            params![&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    if crate::content_item::class_rank(data_class)
        <= crate::content_item::class_rank(&redaction_class)
    {
        return Ok(false);
    }
    let (rationale, urgency_rationale) = if crate::content_item::local_prompt_allowed(data_class) {
        let mut findings = Vec::new();
        (
            crate::cloud_derivative::redact_review_field(rationale.as_deref(), &mut findings),
            crate::cloud_derivative::redact_review_field(
                urgency_rationale.as_deref(),
                &mut findings,
            ),
        )
    } else {
        (None, None)
    };
    conn.execute(
        &format!(
            "UPDATE {prefix}_triage_model_verdicts
                SET rationale = ?1, urgency_rationale = ?2, redaction_class = ?3
              WHERE triage_id = ?4"
        ),
        params![&rationale, &urgency_rationale, &data_class, &id],
    )?;
    Ok(true)
}

// -- mail evaluations ----------------------------------------------------
//
// feed-personalization 2026-09-03. Mirrors store/evaluation.rs column for
// column and gate for gate, against the triage tables: same currency check,
// same tier gate, same contract type. Appended at the end of the file rather
// than filed beside `replace_triage_relevance`, because a second stream edits
// the top of this file tonight.

impl Store {
    /// Store one mail evaluation and its factors atomically.
    ///
    /// `Ok(false)` means the tier gate refused it: a `deterministic` write never
    /// replaces a stored `model` row, which is the same rule the feed keeps and
    /// the reason `ranking_tier` decides the tier from the relevance mode alone.
    pub fn replace_triage_evaluation(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.write_triage_evaluation(evaluation, true)
    }

    /// Store a class refusal, past the tier gate. Same rule and same reason as
    /// [`Store::replace_feed_evaluation_refusal`]: escalating a scored mail to
    /// c3 must withdraw the model-derived score, and a refusal that loses to the
    /// row it withdraws leaves the score on the surface forever.
    pub fn replace_triage_evaluation_refusal(
        &self,
        evaluation: &FeedEvaluation,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.write_triage_evaluation(evaluation, false)
    }

    fn write_triage_evaluation(
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
                       CASE {prefix}_triage_evaluations.tier WHEN 'human' THEN 30 WHEN 'model' THEN 20 WHEN 'deterministic' THEN 10 ELSE 0 END",
                prefix = self.prefix
            )
        } else {
            String::new()
        };
        let affected = transaction.execute(
            &format!(
                "INSERT INTO {prefix}_triage_evaluations
                    (triage_id, overall_score, explanation, mode, item_revision,
                     context_revision, evaluator_revision, tier, evaluated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,{now})
                 ON CONFLICT (triage_id) DO UPDATE SET
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
                "DELETE FROM {}_triage_evaluation_factors WHERE triage_id = ?1",
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
                    "INSERT INTO {prefix}_triage_evaluation_factors
                        (triage_id, factor_key, label, score, weight, rationale, context_json, position)
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

    pub fn triage_evaluation(
        &self,
        triage_id: &str,
    ) -> Result<Option<FeedEvaluation>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let evaluation = conn
            .query_row(
                &format!(
                    "SELECT overall_score, explanation, mode, item_revision,
                            context_revision, evaluator_revision, evaluated_at
                     FROM {}_triage_evaluations WHERE triage_id = ?1",
                    self.prefix
                ),
                params![&triage_id],
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
                 FROM {}_triage_evaluation_factors
                 WHERE triage_id = ?1 ORDER BY position",
                self.prefix
            ),
            params![&triage_id],
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
            feed_id: triage_id.to_string(),
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

    /// Every stored mail score, in one query.
    ///
    /// `GET /triage` renders a list; asking per item would be one read per row
    /// against a table the list already knows the keys of.
    pub fn triage_score_map(
        &self,
    ) -> Result<BTreeMap<String, (f64, String)>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let rows = conn.query_all(
            &format!(
                "SELECT triage_id, overall_score, evaluated_at
                 FROM {}_triage_evaluations",
                self.prefix
            ),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            },
        )?;
        Ok(rows
            .into_iter()
            .map(|(id, score, evaluated_at)| (id, (score, evaluated_at)))
            .collect())
    }
}

#[cfg(test)]
mod mail_evaluation_db_tests {
    use super::*;
    use crate::store::db_tests::{mk_triage, open_test_store};

    fn evaluation(id: &str, mode: &str, score: f64) -> FeedEvaluation {
        FeedEvaluation {
            feed_id: id.into(),
            overall_score: score,
            explanation: "Strongest signal: Category (100%).".into(),
            mode: mode.into(),
            item_revision: "item".into(),
            context_revision: "context".into(),
            evaluator_revision: crate::mail_evaluation::MAIL_EVALUATOR_REVISION.into(),
            evaluated_at: String::new(),
            factors: vec![EvaluationFactor {
                key: "category".into(),
                label: "Category".into(),
                score: 1.0,
                weight: 1.0,
                rationale: "Active correspondence (aktiv)".into(),
                context: None,
            }],
        }
    }

    #[test]
    fn a_mail_evaluation_round_trips_and_keeps_its_tier_gate() {
        let store = open_test_store("mail_evaluation_round_trip");
        let item = mk_triage("thread:evaluated", "aktiv");
        store.upsert_triage(&item).expect("the fixture stores");

        assert!(store
            .replace_triage_evaluation(&evaluation(&item.id, "semantic", 0.62))
            .expect("the first write lands"));
        let stored = store
            .triage_evaluation(&item.id)
            .expect("read back")
            .expect("a row");
        assert_eq!(stored.evaluator_revision, "mail-evaluator-v1");
        assert_eq!(stored.factors.len(), 1);
        assert_eq!(stored.factors[0].key, "category");

        // The tier gate: a `deterministic` write never replaces a stored
        // `model` row. `reranked` is the model tier; `lexical` is not.
        assert!(store
            .replace_triage_evaluation(&evaluation(&item.id, "reranked", 0.80))
            .expect("a model-tier write lands"));
        assert!(!store
            .replace_triage_evaluation(&evaluation(&item.id, "lexical", 0.10))
            .expect("a weaker write is refused, not an error"));
        let held = store
            .triage_evaluation(&item.id)
            .expect("read back")
            .expect("a row");
        assert_eq!(held.mode, "reranked");
        assert!((held.overall_score - 0.80).abs() < 1e-9);

        let map = store.triage_score_map().expect("the batched read");
        assert_eq!(map.len(), 1);
        let (score, evaluated_at) = map.get(&item.id).expect("the item's score");
        assert!((score - 0.80).abs() < 1e-9);
        assert!(!evaluated_at.is_empty(), "the row stamps its own time");
    }
}
