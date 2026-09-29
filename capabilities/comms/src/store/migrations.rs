//! Connection ownership and the ordered comms migration.

use std::time::Duration;

use super::*;

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "comms")
    }

    /// `prefix` is always either the literal `"comms"` (production) or a
    /// test-generated name -- never user input. SQL has no
    /// parametrized-identifier syntax for CREATE TABLE, so prefixed names are
    /// built via `format!`; that is safe specifically because the prefix's
    /// origin is one of those two controlled cases, not because SQL
    /// interpolation is safe in general.
    pub(super) fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Ahead of the pool, not inside `run_migration`: see
        // `rebuild_data_class_vocabulary` for why the dance cannot run there.
        rebuild_data_class_vocabulary(database_path, prefix)?;
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/sjel-store/README.md has why.
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// Returns a `Result` where this used to be `self.conn.lock().unwrap()`. That
    /// unwrap could only fail on a poisoned mutex, which is to say never in
    /// practice; a checkout can genuinely fail — the file is unreachable, or every
    /// connection is busy — and a store method has somewhere to put that.
    pub(super) fn conn(&self) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126). `pub`
    /// rather than `pub(super)` because the server's `/ready` handler is the caller.
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the nineteen tables, not the history that produced them.
    ///
    /// This is where PRD Q45's DDL rule does the most work. The Postgres migration
    /// replayed 62 `ALTER TABLE`s, 21 DROP/ADD CONSTRAINT pairs, seven backfill
    /// `UPDATE`s and one PL/pgSQL `DO` block, because `CREATE TABLE IF NOT EXISTS`
    /// never touches an installed table and every widened CHECK had to be re-stated
    /// for databases that already held rows. SQLite has none of those forms: no
    /// conditional ADD COLUMN, no alterable constraint, no procedural block. So each
    /// retrofitted column is declared here and each widened CHECK is the inline one.
    ///
    /// That is only correct because no deployed SQLite file predates this migration.
    /// The backfills are gone for the same reason — every one of them described rows
    /// written before a column existed, and there are none.
    ///
    /// Order is load-bearing: a batch executes top to bottom, so a referenced table
    /// is declared before the table that references it.
    fn run_migration(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            {triage_items}

            {feed_items}

            -- Outcome of the last pass, so an unattended schedule can be read
            -- rather than trusted. Counts and an error class only: a scheduler
            -- log that quotes a subject is the same leak the sweep gate closes.
            CREATE TABLE IF NOT EXISTS {prefix}_source_state (
                source_name TEXT PRIMARY KEY,
                last_run_at TEXT,
                cursor TEXT,
                last_success_at TEXT,
                last_failure_at TEXT,
                last_error TEXT,
                considered_count INTEGER NOT NULL DEFAULT 0,
                new_count INTEGER NOT NULL DEFAULT 0,
                consecutive_failures INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_relevance (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score REAL NOT NULL,
                rationale TEXT NOT NULL,
                -- #71: cross-encoder scores are distinct from bi-encoder cosine
                -- scores, so 'reranked' is its own mode rather than 'semantic'.
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_triage_relevance (
                triage_id TEXT NOT NULL REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score REAL NOT NULL,
                rationale TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (triage_id, profile_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_origins (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                source_id TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                label TEXT,
                first_seen TEXT NOT NULL DEFAULT ({now}),
                last_seen TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, source_id, source_ref)
            );

            -- #86: extraction output, kept beside the normalized transcript so a
            -- normalization rule change re-runs from here instead of re-fetching.
            -- Its own table, not a column: feed list queries have no business
            -- dragging 20k-character bodies they never read.
            CREATE TABLE IF NOT EXISTS {prefix}_feed_raw_content (
                feed_id TEXT PRIMARY KEY REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                raw TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (tier IN ('legacy','deterministic','model','human')),
                revision TEXT NOT NULL DEFAULT 'legacy-unknown',
                extracted_at TEXT NOT NULL DEFAULT ({now})
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_evaluations (
                feed_id TEXT PRIMARY KEY REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                overall_score REAL NOT NULL CHECK (overall_score BETWEEN 0 AND 1),
                explanation TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical','unscored')),
                item_revision TEXT NOT NULL,
                context_revision TEXT NOT NULL,
                evaluator_revision TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (tier IN ('legacy','deterministic','model','human')),
                evaluated_at TEXT NOT NULL DEFAULT ({now})
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_evaluation_factors (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_evaluations(feed_id) ON DELETE CASCADE,
                factor_key TEXT NOT NULL,
                label TEXT NOT NULL,
                score REAL NOT NULL CHECK (score BETWEEN 0 AND 1),
                weight REAL NOT NULL CHECK (weight BETWEEN 0 AND 1),
                rationale TEXT NOT NULL,
                context_json TEXT,
                position INTEGER NOT NULL,
                PRIMARY KEY (feed_id, factor_key)
            );

            CREATE TABLE IF NOT EXISTS {prefix}_feed_context_snapshots (
                context_kind TEXT PRIMARY KEY,
                revision TEXT NOT NULL,
                payload TEXT NOT NULL,
                refreshed_at TEXT NOT NULL DEFAULT ({now})
            );

            -- #79: deterministic suggestions for the human review queue. The
            -- computation replaces this set explicitly; reading it has no side
            -- effects and never invokes an inference provider.
            CREATE TABLE IF NOT EXISTS {prefix}_feed_quality_flags (
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                signal TEXT NOT NULL,
                reason TEXT NOT NULL,
                evidence TEXT NOT NULL,
                derived_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, signal)
            );

            -- What the local model wrote about an item, for every source.
            --
            -- One table rather than a summary column on each item table.
            -- libs/content-item keeps *storage* apart because the items have
            -- genuinely different invariants -- calendar's exclusive ends_at,
            -- mail's retention window -- and a merged table could enforce
            -- neither. A digest has none of those: it is derived data with the
            -- same axes and the same refine action everywhere, so three
            -- migrations and three upsert paths would buy nothing but drift.
            --
            -- `text` is null unless `state` = 'generated'. `shape` is the rung
            -- the length ladder landed on and `depth` records whether an
            -- operator asked for one more; both are stored rather than
            -- re-derived, because source_chars alone cannot tell you that a
            -- short item was digested on purpose.
            --
            -- No raw source is kept here. A mail body is fetched, digested and
            -- dropped inside one call; this row is the only thing that survives
            -- it.
            CREATE TABLE IF NOT EXISTS {prefix}_content_digests (
                source TEXT NOT NULL CHECK (source IN ('feed','mail','calendar')),
                item_id TEXT NOT NULL,
                text TEXT,
                state TEXT NOT NULL,
                shape TEXT NOT NULL CHECK (shape IN ('none','brief','standard','sectioned')),
                depth TEXT NOT NULL DEFAULT 'standard' CHECK (depth IN ('standard','detailed')),
                focus TEXT NOT NULL DEFAULT '',
                producer TEXT NOT NULL,
                source_chars INTEGER NOT NULL DEFAULT 0 CHECK (source_chars >= 0),
                redactions INTEGER NOT NULL DEFAULT 0 CHECK (redactions >= 0),
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
                last_error TEXT,
                diagram TEXT,
                diagram_state TEXT,
                diagram_error TEXT,
                diagram_producer TEXT,
                -- The extracted table, as chart-data JSON. Not a Vega-Lite
                -- spec: the reader compiles one, which is what keeps the model
                -- out of the rendering layer. Every value in here appeared
                -- verbatim in the source -- see libs/summarize/src/chart.rs.
                chart TEXT,
                chart_state TEXT,
                chart_error TEXT,
                chart_producer TEXT,
                -- Earliest time a failed digest may be retried. Null means
                -- eligible immediately. See RETRYABLE_DIGEST_STATES.
                next_attempt TEXT,
                generated_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (source, item_id)
            );

            {cloud_derivatives}

            -- One reviewed cloud intent and its bounded execution ledger. The
            -- joined derivative, never the original source, is the provider input.
            CREATE TABLE IF NOT EXISTS {prefix}_content_cloud_jobs (
                job_id TEXT PRIMARY KEY,
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                task TEXT NOT NULL DEFAULT 'content-analysis-v1',
                status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
                queued_at TEXT NOT NULL DEFAULT ({now}),
                provider_calls INTEGER NOT NULL DEFAULT 0 CHECK (provider_calls BETWEEN 0 AND 5),
                started_at TEXT,
                completed_at TEXT,
                last_error TEXT,
                result_json TEXT,
                -- `task` is part of the intent, not decoration: one reviewed
                -- derivative can be asked two different questions (an analysis
                -- and a digest), and without the task in the key the second
                -- upsert would silently overwrite the first job's task.
                CONSTRAINT content_cloud_jobs_intent_key
                    UNIQUE (source, item_id, preview_hash, provider_role, task)
            );

            -- One row per actual provider request. Policy-disabled candidates
            -- never enter this ledger because no request was made. The exact
            -- approved hash follows every attempt, including failover.
            --
            -- AUTOINCREMENT rather than the bare rowid alias BIGSERIAL maps to:
            -- an attempt id is quoted in a ledger, and a rowid reused after the
            -- highest row is deleted would make two attempts share one.
            CREATE TABLE IF NOT EXISTS {prefix}_content_cloud_attempts (
                attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL REFERENCES {prefix}_content_cloud_jobs(job_id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 5),
                provider_role TEXT NOT NULL CHECK (provider_role LIKE 'cloud\\_%' ESCAPE '\\'),
                model TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','succeeded','failed')),
                started_at TEXT NOT NULL DEFAULT ({now}),
                completed_at TEXT,
                last_error TEXT,
                result_json TEXT,
                UNIQUE (job_id, sequence)
            );

            -- Egress audit log for outbound cloud model calls (PRD §6, ISC-23, ISC-22).
            -- Every outbound model call appears here with its token count and cost.
            -- The exact document_payload is preserved so an audit query can verify
            -- that no raw C2 data about other people was sent off-host.
            CREATE TABLE IF NOT EXISTS {prefix}_egress_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT ({now}),
                job_id TEXT,
                task TEXT NOT NULL,
                provider TEXT NOT NULL,
                provider_role TEXT NOT NULL,
                model TEXT NOT NULL,
                data_class TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                document_payload TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cost_cents REAL NOT NULL DEFAULT 0.0,
                status TEXT NOT NULL CHECK (status IN ('succeeded','failed')),
                error TEXT
            );

            -- Durable intent for Gmail mutations. The thread id is already the
            -- triage primary key; no message content is copied into this ledger.
            CREATE TABLE IF NOT EXISTS {prefix}_gmail_action_jobs (
                job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                triage_id TEXT NOT NULL REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                action TEXT NOT NULL CHECK (action IN ('archive','trash','restore')),
                source_status TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'queued'
                    CHECK (state IN ('queued','completed','abandoned','canceled')),
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
                last_error TEXT,
                next_attempt TEXT NOT NULL DEFAULT ({now}),
                created_at TEXT NOT NULL DEFAULT ({now}),
                updated_at TEXT NOT NULL DEFAULT ({now}),
                completed_at TEXT
            );

            -- Index names carry the prefix too: one file is one namespace now, so
            -- `idx_feed_status` would collide where two schemas kept it apart.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_stream ON {prefix}_triage_items(stream);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_status ON {prefix}_triage_items(status);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_purge_after
                ON {prefix}_triage_items(purge_after) WHERE purge_after IS NOT NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_gmail_action_jobs_one_queued
                ON {prefix}_gmail_action_jobs(triage_id) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_{prefix}_gmail_action_jobs_retry
                ON {prefix}_gmail_action_jobs(next_attempt) WHERE state = 'queued';
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_stream ON {prefix}_feed_items(stream);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_status ON {prefix}_feed_items(status);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_day ON {prefix}_feed_items(day);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_relevance_score
                ON {prefix}_feed_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_relevance_score
                ON {prefix}_triage_relevance(score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_origins_source
                ON {prefix}_feed_origins(source_id);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_evaluations_score
                ON {prefix}_feed_evaluations(overall_score DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_evaluations_revision
                ON {prefix}_feed_evaluations(context_revision, evaluator_revision);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_quality_flags_derived
                ON {prefix}_feed_quality_flags(derived_at DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_attempts_role_started
                ON {prefix}_content_cloud_attempts(provider_role, started_at);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_derivatives_approved
                ON {prefix}_content_cloud_derivatives(approved_at DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_content_cloud_jobs_queued
                ON {prefix}_content_cloud_jobs(queued_at ASC) WHERE status = 'queued';
            CREATE INDEX IF NOT EXISTS idx_{prefix}_egress_log_timestamp
                ON {prefix}_egress_log(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_egress_log_provider
                ON {prefix}_egress_log(provider);

            -- mail-llm-rung 2026-09-03 ---------------------------------------
            -- Appended as one delimited block at the END of the batch: the feed
            -- personalization stream edits the same function on the same night.

            -- Which deterministic rung decided one thread's stream, and what it
            -- decided. `decided_by`, not `rung`: `crate::quiet::Rung` already
            -- owns that word in this crate for the inference ladder.
            --
            -- A table rather than a column on {prefix}_triage_items, because
            -- CREATE TABLE IF NOT EXISTS is a no-op against the installed table
            -- and a new column would need the swap_in_rebuilt_table dance below.
            -- ON DELETE CASCADE covers the Trash purge with no new retention
            -- mechanism (`PRAGMA foreign_keys = ON` is set per connection in
            -- libs/sjel-store).
            CREATE TABLE IF NOT EXISTS {prefix}_triage_rules (
                triage_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                decided_by TEXT NOT NULL
                    CHECK (decided_by IN ('config_rule','heuristic','fallback')),
                stream TEXT NOT NULL
                    CHECK (stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                rationale TEXT NOT NULL,
                rules_version TEXT NOT NULL DEFAULT 'mail-rules-v1',
                decided_at TEXT NOT NULL DEFAULT ({now})
            );

            -- One backfill, deliberately, against this file's own rule that the
            -- backfills are gone (see run_migration's doc comment). That rule
            -- says every backfill described rows written before a COLUMN
            -- existed, and there are none. These rows were written before this
            -- TABLE existed, and the rationale literals partition them exactly:
            -- measured on a copy of the live file 2026-09-05, fallback 102,
            -- heuristic 111, config_rule 24, total 237. The four heuristic
            -- literals are `rules::classify`'s own, and
            -- `the_backfill_literals_match_the_classifier` asserts the two lists
            -- still agree. INSERT OR IGNORE makes a re-run free.
            INSERT OR IGNORE INTO {prefix}_triage_rules
                (triage_id, decided_by, stream, rationale)
            SELECT id,
                   CASE
                     WHEN rationale = 'No rule matched; kept active as the conservative default.'
                       THEN 'fallback'
                     WHEN rationale IN (
                       'List-Unsubscribe plus a shopping signal in the subject; classified as advertising.',
                       'List-Unsubscribe plus a development or technology signal in the subject; classified as a Feed newsletter.',
                       'A no-reply sender plus a receipt or invoice signal in the subject; classified as a receipt.',
                       'List-Unsubscribe is present, but no specific rule matched; classified as other.'
                     ) THEN 'heuristic'
                     ELSE 'config_rule'
                   END,
                   stream, rationale
              FROM {prefix}_triage_items
             WHERE classification_method = 'deterministic';

            -- UNLIKE {prefix}_gmail_action_jobs, which states that no message
            -- content is copied into it, this table DOES hold message-derived
            -- text: `rationale` and `urgency_rationale` are a model's sentences
            -- about a subject and a snippet, and `last_error` can carry a word
            -- the model invented or a message the local server returned. All
            -- three are redacted before insert against `redaction_class` = the
            -- higher of the row's class and the class the PROPOSED stream
            -- implies, so an escalation at confirm time cannot leave them
            -- under-redacted; `last_error` is capped harder still, because
            -- nothing reads it for content (mail_model::stored_error).
            --
            -- An escalation that arrives LATER — a human moving the row to
            -- belege, a resweep the people registry escalates — is remediated
            -- by `narrow_model_verdict`, which every path that narrows the
            -- item's own review fields now also runs.
            --
            -- `agree` is deliberately NOT a column: it is rule_stream =
            -- model_stream, derived in the report, and a stored copy would be a
            -- second place for it to be wrong.
            --
            -- `state` carries no CHECK, following {prefix}_content_digests. The
            -- vocabulary is enumerated by
            -- `mail_model::tests::every_stored_state_is_in_the_documented_set`
            -- instead, because it is the union of `summarize::Outcome::state()`
            -- and four states this rung owns, and a CHECK here would drift from
            -- that enum silently.
            CREATE TABLE IF NOT EXISTS {prefix}_triage_model_verdicts (
                triage_id TEXT PRIMARY KEY
                    REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                mode TEXT NOT NULL CHECK (mode IN ('shadow','applied','held')),
                state TEXT NOT NULL,
                rule_decided_by TEXT NOT NULL
                    CHECK (rule_decided_by IN ('config_rule','heuristic','fallback')),
                rule_stream TEXT NOT NULL
                    CHECK (rule_stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                model_stream TEXT
                    CHECK (model_stream IS NULL OR model_stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                confidence_bp INTEGER
                    CHECK (confidence_bp IS NULL OR confidence_bp BETWEEN 0 AND 10000),
                urgency_bp INTEGER
                    CHECK (urgency_bp IS NULL OR urgency_bp BETWEEN 0 AND 10000),
                rationale TEXT,
                urgency_rationale TEXT,
                redactions INTEGER NOT NULL DEFAULT 0 CHECK (redactions >= 0),
                data_class TEXT NOT NULL CHECK (data_class IN ('c0','c1','c2','c3')),
                redaction_class TEXT NOT NULL CHECK (redaction_class IN ('c0','c1','c2','c3')),
                producer TEXT NOT NULL,
                item_revision TEXT NOT NULL,
                prompt_revision TEXT NOT NULL DEFAULT 'mail-stream-v1-english',
                classification_version TEXT NOT NULL DEFAULT 'mail-model-v1',
                attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
                last_error TEXT,
                next_attempt TEXT,
                held_reason TEXT,
                applied_at TEXT,
                decided_at TEXT NOT NULL DEFAULT ({now})
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_model_state
                ON {prefix}_triage_model_verdicts(state);
            -- end mail-llm-rung 2026-09-03 -----------------------------------

            -- feed-personalization 2026-09-03 ---------------------------------
            -- Appended as one block at the end of the batch rather than beside
            -- the tables it belongs with: a second stream edits this same file
            -- tonight, and one delimited region is one merge conflict instead
            -- of three. Order still holds -- every table referenced here is
            -- declared above.

            -- What the operator did with a feed item, and when. The status
            -- column it complements is a mutable enum with no history, which
            -- is why the vault projection cannot render a saved date
            -- (projection.rs) and why nothing learned could ever decay.
            --
            -- One writer per verb. `kept`, `dismissed` and `unkept` are
            -- written only by `set_feed_status`, in the same transaction as
            -- the UPDATE, so a decision can neither be lost nor counted twice.
            -- `POST /feed/:id/interactions` refuses those three with 400 and
            -- accepts `opened` and `reopened` only. One row per status CHANGE:
            -- `set_feed_status` reads the stored status inside the same
            -- transaction and writes nothing when the press does not move it,
            -- because an UPDATE to the value a column already holds still
            -- reports one row affected.
            --
            -- Reading the label: an item's label is its most recent row whose
            -- event is in ('kept','dismissed'), retracted by a later `unkept`.
            --
            -- `shared` ships with no writer on purpose, and `home` has none
            -- until the Home surface passes its own name to
            -- `POST /feed/:id/status` (the route already takes `surface`).
            -- SQLite has no alterable constraint, so widening this CHECK later
            -- costs the table-rebuild dance this file documents below;
            -- declaring a value now is free.
            CREATE TABLE IF NOT EXISTS {prefix}_feed_interactions (
                interaction_id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id TEXT NOT NULL REFERENCES {prefix}_feed_items(id) ON DELETE CASCADE,
                event TEXT NOT NULL
                    CHECK (event IN ('opened','kept','dismissed','reopened','unkept','shared')),
                -- Where the press happened. No content and no text: ids and
                -- verbs only, which is what keeps this row c1 and keeps it
                -- trainable.
                surface TEXT NOT NULL DEFAULT 'api'
                    CHECK (surface IN ('inbox','reader','library','home','cli','api')),
                occurred_at TEXT NOT NULL DEFAULT ({now})
            );

            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_interactions_item
                ON {prefix}_feed_interactions(feed_id, occurred_at DESC);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_feed_interactions_time
                ON {prefix}_feed_interactions(occurred_at DESC);

            -- Mail's own evaluation, deliberately the same nine columns as
            -- {prefix}_feed_evaluations. Same currency check, same tier gate,
            -- same contract type, against a different table -- because the feed
            -- evaluator reads a FeedItem and a trip snapshot, and a triage row
            -- has neither.
            --
            -- `mode = 'unscored'` is the c3 refusal state, and it is why the
            -- refusal cannot live on {prefix}_triage_relevance: that table's
            -- mode CHECK admits only reranked, semantic and lexical.
            CREATE TABLE IF NOT EXISTS {prefix}_triage_evaluations (
                triage_id TEXT PRIMARY KEY REFERENCES {prefix}_triage_items(id) ON DELETE CASCADE,
                overall_score REAL NOT NULL CHECK (overall_score BETWEEN 0 AND 1),
                explanation TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical','unscored')),
                item_revision TEXT NOT NULL,
                context_revision TEXT NOT NULL,
                evaluator_revision TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (tier IN ('legacy','deterministic','model','human')),
                evaluated_at TEXT NOT NULL DEFAULT ({now})
            );

            -- Mail-shaped factors: interest, category, age, and a reserved
            -- urgency slot the model rung fills. There is no correspondent
            -- factor, and that is a ruling rather than an omission -- the people
            -- registry is asked per token of subject and snippet, never over the
            -- sender field (PRD Q72 rule 2).
            CREATE TABLE IF NOT EXISTS {prefix}_triage_evaluation_factors (
                triage_id TEXT NOT NULL
                    REFERENCES {prefix}_triage_evaluations(triage_id) ON DELETE CASCADE,
                factor_key TEXT NOT NULL,
                label TEXT NOT NULL,
                score REAL NOT NULL CHECK (score BETWEEN 0 AND 1),
                weight REAL NOT NULL CHECK (weight BETWEEN 0 AND 1),
                rationale TEXT NOT NULL,
                context_json TEXT,
                position INTEGER NOT NULL,
                PRIMARY KEY (triage_id, factor_key)
            );

            CREATE INDEX IF NOT EXISTS idx_{prefix}_triage_evaluations_score
                ON {prefix}_triage_evaluations(overall_score DESC);
            -- end feed-personalization 2026-09-03 -----------------------------
            ",
            prefix = prefix,
            now = sjel_store::NOW,
            triage_items = triage_items_ddl(&format!("{prefix}_triage_items")),
            feed_items = feed_items_ddl(&format!("{prefix}_feed_items")),
            cloud_derivatives =
                cloud_derivatives_ddl(&format!("{prefix}_content_cloud_derivatives")),
        ))?;
        Ok(())
    }
}

/// The three class-carrying tables state their DDL as a function of the table
/// name, because the C0-C3 rebuild below has to create the same shape under a
/// scratch name. A second copy of a forty-column CREATE TABLE is the drift this
/// avoids: the rebuild would install last month's schema.
///
/// None of the three may write an old class name as a quoted literal, comments
/// included. `holds_old_data_classes` reads the DDL SQLite stored, and would
/// read such a comment as a table still waiting to be rebuilt.
fn triage_items_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (
                id TEXT PRIMARY KEY,
                from_addr TEXT,
                subject TEXT,
                snippet TEXT,
                internal_date TEXT,
                stream TEXT NOT NULL CHECK (stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                rationale TEXT NOT NULL,
                classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (classification_method IN ('legacy','deterministic','model','human')),
                classification_version TEXT NOT NULL DEFAULT 'mail-rules-v1',
                data_class TEXT NOT NULL DEFAULT 'c1'
                    CHECK (data_class IN ('c0','c1','c2','c3')),
                data_class_rationale TEXT NOT NULL DEFAULT 'Mail metadata is Mine by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-rules-v2',
                -- The widened set: archived/trashed/missing arrived with the Gmail
                -- mirror and were a DROP/ADD CONSTRAINT pair under Postgres.
                status TEXT NOT NULL DEFAULT 'proposed'
                    CHECK (status IN ('proposed','approved','executed','archived','trashed','missing','dismissed')),
                gmail_action TEXT
                    CHECK (gmail_action IS NULL OR gmail_action IN ('archive','trash','restore')),
                gmail_action_at TEXT,
                purge_after TEXT,
                gmail_location TEXT
                    CHECK (gmail_location IS NULL OR gmail_location IN ('inbox','archive','trash','missing')),
                gmail_observed_at TEXT,
                gmail_sync_status TEXT
                    CHECK (gmail_sync_status IS NULL OR gmail_sync_status IN ('synced','queued','retrying','attention')),
                gmail_sync_error TEXT,
                -- The doctrine's one state label, mirrored locally so the dashboard
                -- can rank on it without asking Gmail. Gmail stays authoritative:
                -- this is only written after its modify call succeeds, and a sweep
                -- that sees the label gone clears it. INTEGER because SQLite has no
                -- boolean; rusqlite writes 0/1 and reads a `bool` back.
                waiting INTEGER NOT NULL DEFAULT 0,
                waiting_at TEXT,
                first_seen TEXT NOT NULL,
                last_seen TEXT NOT NULL
            );"
    )
}

fn feed_items_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (
                id TEXT PRIMARY KEY,
                stream TEXT NOT NULL CHECK (stream IN ('news','media')),
                -- The widened set: the share-link extractors (github/arxiv/reddit/
                -- huggingface) were a DROP/ADD CONSTRAINT pair under Postgres.
                kind TEXT NOT NULL CHECK (kind IN ('youtube','instagram','podcast','article','mail','github','arxiv','reddit','huggingface')),
                title TEXT,
                url TEXT NOT NULL,
                author TEXT,
                summary TEXT,
                transcript TEXT,
                day TEXT NOT NULL,
                created_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','keeper','dismissed')),
                -- #74: enrichment state -- content_status and the summary retry ledger.
                content_status TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (content_status IN ('full','thin','none','unknown')),
                summary_attempts INTEGER NOT NULL DEFAULT 0,
                summary_last_error TEXT,
                summary_next_attempt TEXT,
                summary_attempt_revision TEXT,
                -- #81: which client handed the content over. NULL means the server
                -- fetched it. Free-form rather than a CHECK: the set of clients is
                -- open (extension, CLI, a future share sheet) and a constraint here
                -- would need a migration every time one is added.
                captured_via TEXT,
                -- #78: what the stored text IS, as against how much of it there is.
                -- Constrained, unlike captured_via, because this set is closed by
                -- the enum that writes it (provenance::TranscriptSource).
                transcript_source TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (transcript_source IN ('full-text','abstract','unknown')),
                -- #77: producer provenance lives beside each stage value.
                normalization_tier TEXT
                    CHECK (normalization_tier IS NULL OR normalization_tier IN ('legacy','deterministic','model','human')),
                normalization_revision TEXT,
                normalization_completed_at TEXT,
                summary_tier TEXT
                    CHECK (summary_tier IS NULL OR summary_tier IN ('legacy','deterministic','model','human')),
                summary_revision TEXT,
                summary_completed_at TEXT,
                -- Data classification. The default is the fail-closed pair, and both
                -- halves carry weight: an item nobody classified is c1, so a
                -- collector cannot produce a cloud-eligible row by omission, and its
                -- method is 'legacy' (rank 0) so the first real decision of any kind
                -- outranks it. c0 has to be positively declared.
                data_class TEXT NOT NULL DEFAULT 'c1'
                    CHECK (data_class IN ('c0','c1','c2','c3')),
                data_class_rationale TEXT NOT NULL
                    DEFAULT 'No collector declared a class for this item; Mine by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                -- The legacy stamp names the decision's provenance, not the
                -- current rule set, so it survives the C0-C3 translation.
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-legacy-v1'
            );"
    )
}

fn cloud_derivatives_ddl(table: &str) -> String {
    format!(
        "-- A human-approved derivative is staged locally before a cloud job
            -- can consume it. Staging has no provider identity or side effect.
            --
            -- Neither class column admits c2 or c3: `cloud_derivative::prepare`
            -- returns Err for both, so no code can produce one to stage, and a
            -- vocabulary that accepts a value only a binary predating that refusal
            -- could write describes history rather than what may exist.
            CREATE TABLE IF NOT EXISTS {table} (
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                original_data_class TEXT NOT NULL CHECK (original_data_class IN ('c0','c1')),
                derivative_data_class TEXT NOT NULL CHECK (derivative_data_class IN ('c0','c1')),
                transformation TEXT NOT NULL,
                document TEXT NOT NULL,
                redaction_count INTEGER NOT NULL CHECK (redaction_count >= 0),
                approved_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (source, item_id)
            );",
        now = sjel_store::NOW
    )
}

/// One-time translation of the three-class vocabulary into C0-C3 (Q27, Q72).
///
/// The doctrine above holds only while no deployed file predates a constraint
/// change. `axon.db` does: it holds triage, feed and derivative rows written
/// under `CHECK (data_class IN (...))` with the old vocabulary, and
/// `CREATE TABLE IF NOT EXISTS` never revisits an installed table. So this is
/// the table-rebuild dance SQLite's own documentation prescribes, run once,
/// behind a probe of what the file actually holds.
///
/// It runs before the pool rather than inside [`Store::run_migration`] because
/// the dance needs foreign keys off. `sjel_store::migrate_once` hands the
/// migration an already-open transaction, `PRAGMA foreign_keys` is a documented
/// no-op inside one, and `DROP TABLE` with enforcement on performs an implicit
/// DELETE that fires `ON DELETE CASCADE` -- which would take the seven tables
/// keyed to a triage or feed row with it. A fresh connection has enforcement
/// off, which is the state the dance requires. Nothing references the rebuilt
/// tables' own foreign keys in return, and every primary key is copied
/// verbatim, so no child row's reference changes.
fn rebuild_data_class_vocabulary(
    database_path: &Path,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // The migration's own once-per-process discipline, under its own key so
    // neither guard can answer for the other.
    sjel_store::once_per_target(
        &database_path.to_string_lossy(),
        &format!("{prefix}-data-class-c0-c3"),
        || rebuild_data_class_vocabulary_now(database_path, prefix),
    )
}

/// The rebuild without the once-guard, so a test can run it twice and see the
/// second pass find nothing to do.
fn rebuild_data_class_vocabulary_now(
    database_path: &Path,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // `Connection::open` creates the *file* and no directory above it, and this
    // runs before `sjel_store::open_pool`, which is where the mkdir normally
    // happens (`canonical_key`, libs/sjel-store/src/lib.rs:180-190). Without
    // this line a first run against a path whose directory does not exist yet
    // fails with SQLITE_CANTOPEN, on the one code path nobody re-runs by hand.
    // Mirrored rather than shared because that helper is private to the pool.
    if let Some(parent) = database_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut conn = Connection::open(database_path)?;
    conn.busy_timeout(Duration::from_millis(5_000))?;
    // Every pooled connection in this process gets `foreign_keys = ON` from
    // sjel-store's CONNECTION_PRAGMAS (libs/sjel-store/src/lib.rs:165), because
    // the pragma is per-connection and SQLite leaves it off unless asked
    // (documented at libs/sjel-store/src/lib.rs:60). This rebuild does not use
    // that pool — it owns a raw connection — and it turns the pragma explicitly
    // OFF, because the drop/rename dance below would otherwise cascade:
    // `comms_content_cloud_attempts` references `comms_content_cloud_jobs`, and
    // dropping the old table would take its children with it. Set here rather
    // than inside the transaction, where SQLite ignores it.
    conn.pragma_update(None, "foreign_keys", false)?;
    // One transaction for all three tables: a file with translated feed rows and
    // untranslated triage rows is a state no later run can recognise, because
    // the probe is per table.
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let triage = format!("{prefix}_triage_items");
    if holds_old_data_classes(&transaction, &triage)? {
        refuse_unmapped_class(
            &transaction,
            &triage,
            "data_class",
            &["public", "personal", "vault"],
        )?;
        swap_in_rebuilt_table(
            &transaction,
            &triage,
            triage_items_ddl,
            TRIAGE_COLUMNS,
            TRIAGE_SELECT,
        )?;
    }

    let feed = format!("{prefix}_feed_items");
    if holds_old_data_classes(&transaction, &feed)? {
        // No 'vault' arm, so a vault feed row aborts the rebuild rather than
        // being guessed at. The old CHECK admitted one; nothing ever wrote one,
        // and there is no stored trace to re-derive a c2/c3 split from.
        refuse_unmapped_class(&transaction, &feed, "data_class", &["public", "personal"])?;
        swap_in_rebuilt_table(
            &transaction,
            &feed,
            feed_items_ddl,
            FEED_COLUMNS,
            FEED_SELECT,
        )?;
    }

    let derivatives = format!("{prefix}_content_cloud_derivatives");
    if holds_old_data_classes(&transaction, &derivatives)? {
        for column in ["original_data_class", "derivative_data_class"] {
            refuse_unmapped_class(&transaction, &derivatives, column, &["public", "personal"])?;
        }
        swap_in_rebuilt_table(
            &transaction,
            &derivatives,
            cloud_derivatives_ddl,
            DERIVATIVE_COLUMNS,
            DERIVATIVE_SELECT,
        )?;
    }

    transaction.commit()?;
    Ok(())
}

/// Whether this table is installed and still names a class from the old
/// vocabulary.
///
/// Reads the DDL SQLite stored, which is the only durable record of which
/// CHECK a table was created under. An empty table has to be rebuilt too: its
/// constraint would refuse the first c1 row written after the deploy.
fn holds_old_data_classes(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    let ddl: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(ddl.is_some_and(|sql| {
        sql.contains("'public'") || sql.contains("'personal'") || sql.contains("'vault'")
    }))
}

/// Abort rather than guess. A stored value the CASE ladder does not name would
/// map to NULL and land as a NOT NULL failure naming a column; this names the
/// value, and it names it before a single row has been copied.
fn refuse_unmapped_class(
    conn: &Connection,
    table: &str,
    column: &str,
    mapped: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let mapped = mapped
        .iter()
        .map(|class| format!("'{class}'"))
        .collect::<Vec<_>>()
        .join(",");
    let stray: Option<String> = conn
        .query_row(
            &format!("SELECT DISTINCT {column} FROM {table} WHERE {column} NOT IN ({mapped})"),
            [],
            |row| row.get(0),
        )
        .optional()?;
    match stray {
        Some(value) => Err(format!(
            "{table}.{column} holds {value:?}, which the C0-C3 rebuild has no mapping for"
        )
        .into()),
        None => Ok(()),
    }
}

/// Create the table's new shape beside it, copy every row through the mapping,
/// drop the old one and take its name.
///
/// Indexes and triggers are read back and replayed rather than left to the
/// migration batch: the batch declares the ones this repository ships, and a
/// DROP takes every index the deployed file actually carries.
fn swap_in_rebuilt_table(
    conn: &Connection,
    table: &str,
    ddl: fn(&str) -> String,
    columns: &str,
    select: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let scratch = format!("{table}_c0_c3");
    let attached: Vec<String> = conn.query_all(
        "SELECT sql FROM sqlite_master
         WHERE tbl_name = ?1 AND type IN ('index','trigger') AND sql IS NOT NULL",
        params![table],
        |row| row.get(0),
    )?;
    // A crashed earlier attempt would have left the scratch table behind, and
    // its DDL says IF NOT EXISTS.
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {scratch};"))?;
    conn.execute_batch(&ddl(&scratch))?;
    conn.execute_batch(&format!(
        "INSERT INTO {scratch} ({columns}) SELECT {select} FROM {table};
         DROP TABLE {table};
         ALTER TABLE {scratch} RENAME TO {table};"
    ))?;
    for statement in attached {
        conn.execute_batch(&statement)?;
    }
    Ok(())
}

const TRIAGE_COLUMNS: &str = "id, from_addr, subject, snippet, internal_date, stream, rationale,
     classification_method, classification_version, data_class, data_class_rationale,
     data_classification_method, data_classification_version, status, gmail_action,
     gmail_action_at, purge_after, gmail_location, gmail_observed_at, gmail_sync_status,
     gmail_sync_error, waiting, waiting_at, first_seen, last_seen";

/// A vault row splits by its stored rationale, which is a 1:1 trace of the
/// classifier branch that fired and, on such a row, the only trace left: the
/// subject and snippet were redacted before they were written, so re-running
/// the text rules would be blind (Q27). Authentication becomes c3; tax,
/// receipts, financial and health become c2; a rationale no rule of ours wrote
/// becomes c3, the stricter reading.
///
/// The rewritten sentences are the ones `libs/content-item` returns today, so
/// the first `POST /triage/data-class/refresh` after the deploy writes the same
/// text rather than churning the row. `data_classification_version` is left
/// alone for the opposite reason: no rule re-ran here, so the stamp still names
/// the rule set that decided. A human's sentence is never rewritten at all —
/// it is the one rationale nobody can reproduce.
const TRIAGE_SELECT: &str = "id, from_addr, subject, snippet, internal_date, stream, rationale,
     classification_method, classification_version,
     CASE
         WHEN data_class = 'public' THEN 'c0'
         WHEN data_class = 'personal' THEN 'c1'
         WHEN data_class_rationale = 'Authentication or account-recovery metadata is Private.'
             THEN 'c3'
         WHEN data_class_rationale IN (
             'Tax-related mail is Private by default.',
             'Receipts and invoices are Private by default.',
             'Financial or insurance metadata is Private.',
             'Health-related metadata is Private.'
         ) THEN 'c2'
         ELSE 'c3'
     END,
     CASE
         WHEN data_class <> 'vault' THEN data_class_rationale
         WHEN data_classification_method = 'human' THEN data_class_rationale
         WHEN data_class_rationale = 'Authentication or account-recovery metadata is Private.'
             THEN 'Authentication or account-recovery metadata is Secret.'
         WHEN data_class_rationale = 'Tax-related mail is Private by default.'
             THEN 'Tax-related mail is Others by default.'
         WHEN data_class_rationale = 'Receipts and invoices are Private by default.'
             THEN 'Receipts and invoices are Others by default.'
         WHEN data_class_rationale = 'Financial or insurance metadata is Private.'
             THEN 'Financial or insurance metadata is Others.'
         WHEN data_class_rationale = 'Health-related metadata is Private.'
             THEN 'Health-related metadata is Others.'
         ELSE 'No rule wrote this rationale, so the row is Secret rather than guessed. Recorded reason: '
             || data_class_rationale
     END,
     data_classification_method, data_classification_version, status, gmail_action,
     gmail_action_at, purge_after, gmail_location, gmail_observed_at, gmail_sync_status,
     gmail_sync_error, waiting, waiting_at, first_seen, last_seen";

const FEED_COLUMNS: &str = "id, stream, kind, title, url, author, summary, transcript, day,
     created_at, status, content_status, summary_attempts, summary_last_error,
     summary_next_attempt, summary_attempt_revision, captured_via, transcript_source,
     normalization_tier, normalization_revision, normalization_completed_at, summary_tier,
     summary_revision, summary_completed_at, data_class, data_class_rationale,
     data_classification_method, data_classification_version";

/// A feed row's class is a declaration, not a derivation, so the translation
/// carries the rationale, method and version across untouched: the same
/// decision in the new vocabulary is not a reclassification, and
/// `admit_reclassification` is deliberately not consulted.
const FEED_SELECT: &str = "id, stream, kind, title, url, author, summary, transcript, day,
     created_at, status, content_status, summary_attempts, summary_last_error,
     summary_next_attempt, summary_attempt_revision, captured_via, transcript_source,
     normalization_tier, normalization_revision, normalization_completed_at, summary_tier,
     summary_revision, summary_completed_at,
     CASE data_class WHEN 'public' THEN 'c0' ELSE 'c1' END,
     data_class_rationale, data_classification_method, data_classification_version";

const DERIVATIVE_COLUMNS: &str = "source, item_id, source_revision, preview_hash,
     original_data_class, derivative_data_class, transformation, document, redaction_count,
     approved_at";

const DERIVATIVE_SELECT: &str = "source, item_id, source_revision, preview_hash,
     CASE original_data_class WHEN 'public' THEN 'c0' ELSE 'c1' END,
     CASE derivative_data_class WHEN 'public' THEN 'c0' ELSE 'c1' END,
     transformation, document, redaction_count, approved_at";

/// Database-backed; named for the selector CI splits on -- see CONTRIBUTING.md.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::store::db_tests::test_database;

    /// The schema the deployed file carries, quoted from its `sqlite_master`
    /// rather than from an older revision of this file. Anything that reads
    /// back differently would be testing a fiction.
    fn install_old_schema(path: &Path) -> Connection {
        let conn = Connection::open(path).expect("a writable temp database");
        conn.execute_batch(&format!(
            "
            CREATE TABLE comms_triage_items (
                id TEXT PRIMARY KEY,
                from_addr TEXT,
                subject TEXT,
                snippet TEXT,
                internal_date TEXT,
                stream TEXT NOT NULL CHECK (stream IN ('aktiv','issue','feed','werbung','belege','steuern','sonstiges')),
                rationale TEXT NOT NULL,
                classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (classification_method IN ('legacy','deterministic','model','human')),
                classification_version TEXT NOT NULL DEFAULT 'mail-rules-v1',
                data_class TEXT NOT NULL DEFAULT 'personal'
                    CHECK (data_class IN ('public','personal','vault')),
                data_class_rationale TEXT NOT NULL DEFAULT 'Mail metadata is Personal by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'deterministic'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-rules-v1',
                status TEXT NOT NULL DEFAULT 'proposed'
                    CHECK (status IN ('proposed','approved','executed','archived','trashed','missing','dismissed')),
                gmail_action TEXT CHECK (gmail_action IS NULL OR gmail_action IN ('archive','trash','restore')),
                gmail_action_at TEXT,
                purge_after TEXT,
                gmail_location TEXT CHECK (gmail_location IS NULL OR gmail_location IN ('inbox','archive','trash','missing')),
                gmail_observed_at TEXT,
                gmail_sync_status TEXT CHECK (gmail_sync_status IS NULL OR gmail_sync_status IN ('synced','queued','retrying','attention')),
                gmail_sync_error TEXT,
                waiting INTEGER NOT NULL DEFAULT 0,
                waiting_at TEXT,
                first_seen TEXT NOT NULL,
                last_seen TEXT NOT NULL
            );

            CREATE TABLE comms_feed_items (
                id TEXT PRIMARY KEY,
                stream TEXT NOT NULL CHECK (stream IN ('news','media')),
                kind TEXT NOT NULL CHECK (kind IN ('youtube','instagram','podcast','article','mail','github','arxiv','reddit','huggingface')),
                title TEXT, url TEXT NOT NULL, author TEXT, summary TEXT, transcript TEXT,
                day TEXT NOT NULL, created_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','keeper','dismissed')),
                content_status TEXT NOT NULL DEFAULT 'unknown' CHECK (content_status IN ('full','thin','none','unknown')),
                summary_attempts INTEGER NOT NULL DEFAULT 0,
                summary_last_error TEXT, summary_next_attempt TEXT, summary_attempt_revision TEXT,
                captured_via TEXT,
                transcript_source TEXT NOT NULL DEFAULT 'unknown' CHECK (transcript_source IN ('full-text','abstract','unknown')),
                normalization_tier TEXT CHECK (normalization_tier IS NULL OR normalization_tier IN ('legacy','deterministic','model','human')),
                normalization_revision TEXT, normalization_completed_at TEXT,
                summary_tier TEXT CHECK (summary_tier IS NULL OR summary_tier IN ('legacy','deterministic','model','human')),
                summary_revision TEXT, summary_completed_at TEXT,
                data_class TEXT NOT NULL DEFAULT 'personal' CHECK (data_class IN ('public','personal','vault')),
                data_class_rationale TEXT NOT NULL DEFAULT 'No collector declared a class for this item; Personal by default.',
                data_classification_method TEXT NOT NULL DEFAULT 'legacy'
                    CHECK (data_classification_method IN ('legacy','deterministic','model','human')),
                data_classification_version TEXT NOT NULL DEFAULT 'data-class-legacy-v1'
            );

            CREATE TABLE comms_content_cloud_derivatives (
                source TEXT NOT NULL CHECK (source IN ('feed','mail')),
                item_id TEXT NOT NULL,
                source_revision TEXT NOT NULL,
                preview_hash TEXT NOT NULL,
                original_data_class TEXT NOT NULL CHECK (original_data_class IN ('public','personal')),
                derivative_data_class TEXT NOT NULL CHECK (derivative_data_class IN ('public','personal')),
                transformation TEXT NOT NULL,
                document TEXT NOT NULL,
                redaction_count INTEGER NOT NULL CHECK (redaction_count >= 0),
                approved_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (source, item_id)
            );

            CREATE TABLE comms_triage_relevance (
                triage_id TEXT NOT NULL REFERENCES comms_triage_items(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                profile_label TEXT NOT NULL,
                score REAL NOT NULL,
                rationale TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('reranked','semantic','lexical')),
                profile_revision TEXT NOT NULL,
                scored_at TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (triage_id, profile_key)
            );

            CREATE TABLE comms_feed_origins (
                feed_id TEXT NOT NULL REFERENCES comms_feed_items(id) ON DELETE CASCADE,
                source_id TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                label TEXT,
                first_seen TEXT NOT NULL DEFAULT ({now}),
                last_seen TEXT NOT NULL DEFAULT ({now}),
                PRIMARY KEY (feed_id, source_id, source_ref)
            );

            CREATE INDEX idx_comms_triage_stream ON comms_triage_items(stream);
            CREATE INDEX idx_comms_triage_first_seen ON comms_triage_items(first_seen);
            ",
            now = sjel_store::NOW
        ))
        .expect("the old schema installs");
        conn
    }

    /// One mail row per branch the rebuild has to decide, plus the two rows a
    /// human owns. Addresses are `example.com` fictions (Q42).
    fn install_old_rows(conn: &Connection) {
        for (id, class, rationale, method) in [
            (
                "thread:public",
                "public",
                "Declared public by its collector.",
                "deterministic",
            ),
            (
                "thread:mine",
                "personal",
                "Mail metadata is Personal by default.",
                "deterministic",
            ),
            (
                "thread:auth",
                "vault",
                "Authentication or account-recovery metadata is Private.",
                "deterministic",
            ),
            (
                "thread:tax",
                "vault",
                "Tax-related mail is Private by default.",
                "deterministic",
            ),
            (
                "thread:receipt",
                "vault",
                "Receipts and invoices are Private by default.",
                "deterministic",
            ),
            (
                "thread:financial",
                "vault",
                "Financial or insurance metadata is Private.",
                "deterministic",
            ),
            (
                "thread:health",
                "vault",
                "Health-related metadata is Private.",
                "deterministic",
            ),
            (
                "thread:unknown",
                "vault",
                "A sentence no rule of ours ever wrote.",
                "deterministic",
            ),
            ("thread:human-vault", "vault", "I read it myself.", "human"),
            (
                "thread:human-personal",
                "personal",
                "Ordinary correspondence.",
                "human",
            ),
        ] {
            conn.execute(
                "INSERT INTO comms_triage_items
                    (id, from_addr, subject, snippet, stream, rationale, data_class,
                     data_class_rationale, data_classification_method, first_seen, last_seen)
                 VALUES (?1, 'erika@example.com', 'Subject', 'Snippet', 'aktiv', 'test',
                         ?2, ?3, ?4, '2026-09-01', '2026-09-01')",
                params![id, class, rationale, method],
            )
            .expect("a triage row inserts");
        }

        for (id, class, method) in [
            ("feed:public", "public", "deterministic"),
            ("feed:mine", "personal", "legacy"),
            ("feed:human", "personal", "human"),
        ] {
            conn.execute(
                "INSERT INTO comms_feed_items
                    (id, stream, kind, url, day, created_at, data_class,
                     data_class_rationale, data_classification_method)
                 VALUES (?1, 'news', 'article', 'https://example.com/' || ?1, '2026-09-01',
                         '2026-09-01', ?2, 'Declared by its collector.', ?3)",
                params![id, class, method],
            )
            .expect("a feed row inserts");
        }

        for (item, original, derivative) in [
            ("feed:public", "public", "public"),
            ("feed:mine", "personal", "personal"),
        ] {
            conn.execute(
                "INSERT INTO comms_content_cloud_derivatives
                    (source, item_id, source_revision, preview_hash, original_data_class,
                     derivative_data_class, transformation, document, redaction_count)
                 VALUES ('feed', ?1, 'rev', 'hash-' || ?1, ?2, ?3, 'bounded-public-v1', 'doc', 0)",
                params![item, original, derivative],
            )
            .expect("a derivative row inserts");
        }

        conn.execute(
            "INSERT INTO comms_triage_relevance
                (triage_id, profile_key, profile_label, score, rationale, mode, profile_revision)
             VALUES ('thread:auth', 'work', 'Work', 0.5, 'test', 'lexical', 'rev')",
            [],
        )
        .expect("a triage child row inserts");
        conn.execute(
            "INSERT INTO comms_feed_origins (feed_id, source_id, source_ref)
             VALUES ('feed:mine', 'arxiv', 'ref')",
            [],
        )
        .expect("a feed child row inserts");
    }

    fn stored(conn: &Connection, id: &str) -> (String, String, String) {
        conn.query_row(
            "SELECT data_class, data_class_rationale, data_classification_method
             FROM comms_triage_items WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the row survived the rebuild")
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).expect("a count")
    }

    /// The whole rebuild, against the schema the deployed file actually has:
    /// every mapping arm, the human rows, the children that must not be
    /// cascaded away, and a second pass that must find nothing to do.
    #[test]
    fn the_old_vocabulary_is_rebuilt_into_c0_c3_and_the_second_pass_is_a_no_op() {
        let path = test_database("data_class_rebuild");
        let old = install_old_schema(&path);
        install_old_rows(&old);
        drop(old);

        let store = Store::open(&path).expect("the rebuild runs on open");
        assert!(store.ping().is_ok(), "the store works after the rebuild");
        let conn = Connection::open(&path).unwrap();

        assert_eq!(
            stored(&conn, "thread:public").0,
            "c0",
            "a public mail row is c0"
        );
        assert_eq!(
            stored(&conn, "thread:mine"),
            (
                "c1".into(),
                "Mail metadata is Personal by default.".into(),
                "deterministic".into()
            ),
            "a personal row is c1 and keeps the rationale the rules wrote"
        );
        assert_eq!(
            stored(&conn, "thread:auth"),
            (
                "c3".into(),
                "Authentication or account-recovery metadata is Secret.".into(),
                "deterministic".into()
            ),
            "an authentication row is c3 and names the new class"
        );
        for (id, rationale) in [
            ("thread:tax", "Tax-related mail is Others by default."),
            (
                "thread:receipt",
                "Receipts and invoices are Others by default.",
            ),
            (
                "thread:financial",
                "Financial or insurance metadata is Others.",
            ),
            ("thread:health", "Health-related metadata is Others."),
        ] {
            assert_eq!(
                stored(&conn, id),
                ("c2".into(), rationale.into(), "deterministic".into()),
                "{id} is c2 and names the new class"
            );
        }
        let unknown = stored(&conn, "thread:unknown");
        assert_eq!(unknown.0, "c3", "a rationale no rule wrote maps strictly");
        assert!(
            unknown
                .1
                .ends_with("A sentence no rule of ours ever wrote."),
            "the original reason is kept, got {}",
            unknown.1
        );
        assert_eq!(
            stored(&conn, "thread:human-vault"),
            ("c3".into(), "I read it myself.".into(), "human".into()),
            "a human's vault row keeps its method and its own words"
        );
        assert_eq!(
            stored(&conn, "thread:human-personal").2,
            "human",
            "a human's personal row keeps its method"
        );

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_feed_items WHERE data_class = 'c0'"
            ),
            1
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_feed_items WHERE data_class = 'c1'"
            ),
            2
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_content_cloud_derivatives
                 WHERE original_data_class = 'c0' AND derivative_data_class = 'c0'"
            ),
            1
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_content_cloud_derivatives
                 WHERE original_data_class = 'c1' AND derivative_data_class = 'c1'"
            ),
            1
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM comms_triage_items"),
            10,
            "no mail row was lost"
        );

        // The dance drops the parent table, and a drop under enforced foreign
        // keys would have cascaded these away.
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM comms_triage_relevance"),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM comms_feed_origins"), 1);

        // The batch declares the first of these and not the second.
        for index in ["idx_comms_triage_stream", "idx_comms_triage_first_seen"] {
            assert_eq!(
                count(
                    &conn,
                    &format!(
                        "SELECT COUNT(*) FROM sqlite_master
                         WHERE type = 'index' AND name = '{index}'"
                    )
                ),
                1,
                "{index} did not survive the rebuild"
            );
        }

        // The once-guard makes the second `open` a no-op, so idempotence is
        // asked of the pass itself as well.
        Store::open(&path).expect("a second open");
        rebuild_data_class_vocabulary_now(&path, "comms").expect("a second rebuild pass");
        assert_eq!(
            stored(&conn, "thread:auth"),
            (
                "c3".into(),
                "Authentication or account-recovery metadata is Secret.".into(),
                "deterministic".into()
            ),
            "the second pass rewrote a row it should not have touched"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_triage_items WHERE data_class LIKE 'c%'"
            ),
            10
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM comms_triage_relevance"),
            1
        );
    }

    /// The probe reads the DDL SQLite stored, comments and all, so a table this
    /// migration creates must not name an old class anywhere inside it. A
    /// version of this file that did would rebuild, on every open, a table it
    /// had just created.
    #[test]
    fn a_fresh_database_is_never_mistaken_for_an_old_one() {
        let path = test_database("data_class_fresh");
        Store::open(&path).expect("a fresh store opens");
        let conn = Connection::open(&path).unwrap();
        for table in [
            "comms_triage_items",
            "comms_feed_items",
            "comms_content_cloud_derivatives",
        ] {
            assert!(
                !holds_old_data_classes(&conn, table).unwrap(),
                "{table} reads as still holding the old vocabulary"
            );
        }
    }

    /// The old CHECK admitted a vault feed row and the mapping has no arm for
    /// one, because no stored trace could tell c2 from c3. Refusing to open is
    /// the loud failure; translating on a guess is the quiet one.
    #[test]
    fn a_class_the_mapping_cannot_name_aborts_the_rebuild() {
        let path = test_database("data_class_rebuild_abort");
        let old = install_old_schema(&path);
        install_old_rows(&old);
        old.execute(
            "INSERT INTO comms_feed_items
                (id, stream, kind, url, day, created_at, data_class, data_class_rationale)
             VALUES ('feed:vault', 'news', 'article', 'https://example.com/v', '2026-09-01',
                     '2026-09-01', 'vault', 'Declared by its collector.')",
            [],
        )
        .unwrap();
        drop(old);

        let Err(error) = Store::open(&path) else {
            panic!("an unmapped class must not open");
        };
        let error = error.to_string();
        assert!(
            error.contains("comms_feed_items.data_class") && error.contains("vault"),
            "the error names neither the column nor the value: {error}"
        );

        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM comms_triage_items WHERE data_class IN ('public','personal','vault')"
            ),
            10,
            "the transaction did not roll back: mail rows were translated anyway"
        );
    }
}
