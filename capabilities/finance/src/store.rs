//! Persistence for subscriptions and their two append-only series.
//!
//! The table shape is the model's guarantee made structural. There is no `price`
//! column on `subscriptions` and no `status` column either, because a column is a
//! thing that can be updated, and the entire point is that a price change appends.
//! What the current price *is* comes from `price_at()` over the series, not from a
//! row somebody has to remember to keep in sync.
//!
//! `total_cents` is likewise absent. A cached total is a second source of truth
//! that goes stale silently, and the series it summarises is already in memory by
//! the time anyone asks.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use sjel_store::QueryAll;

use crate::analytics::{TransactionKind, TransactionRow};
use crate::import::{CandidateState, TransactionCandidate};
use crate::investment::{
    Holding, HoldingsCoverage, Quantity, ReviewedHoldingsSnapshot, ReviewedHoldingsSource,
};
use crate::obsidian::ScannedNote;
use crate::price::{FetchAttempt, FetchStatus, FxObservation, PriceObservation};
use crate::subscription::{BillingCycle, PricePoint, State, StateChange, Subscription};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct FinanceStore {
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `finance` here means `finance_subscriptions` and its seven siblings.
    prefix: String,
}

/// A table prefix reaches SQL by interpolation, because SQL has no bind parameter
/// for an identifier. Copied deliberately from `trips`: the validation is the reason
/// interpolating it is safe, and dropping the check while keeping the interpolation
/// is how this becomes an injection.
fn validate_prefix(prefix: &str) -> Fallible<()> {
    let ok = !prefix.is_empty()
        && prefix.len() <= 63
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid table prefix: {prefix:?}").into())
    }
}

impl FinanceStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, "finance")
    }

    pub fn open_with_prefix(database_path: &Path, prefix: &str) -> Fallible<Self> {
        validate_prefix(prefix)?;
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::run_migration(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    fn conn(&self) -> Fallible<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can reach its database, which
    /// is what the readiness surface promises rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the eight tables, not the history that produced them.
    ///
    /// Fourteen `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` statements and two
    /// DROP/ADD CONSTRAINT pairs are folded into their `CREATE TABLE`s. SQLite has
    /// neither form -- no conditional ADD COLUMN, no alterable constraint -- and no
    /// deployed SQLite file predates this migration, so there is no history for the
    /// replay to describe. Each folded column keeps the nullability the ALTER gave
    /// it, because the code that reads it was written against that.
    ///
    /// `subscriptions` is declared before the two series that reference it, because
    /// a batch executes in order.
    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_subscriptions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_path TEXT NOT NULL UNIQUE,
                category TEXT,
                value_rating INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Append-only by intent, and by the absence of any code path that
            -- updates or deletes a row here. The (subscription, date, reason) key
            -- makes a re-import idempotent without making a genuine same-day
            -- correction impossible: a different reason is a different point.
            --
            -- AUTOINCREMENT rather than the bare rowid alias BIGSERIAL would map to:
            -- `list` orders by (valid_from, id), so two points on one day keep the
            -- order they were appended in, and a plain rowid is reused after the
            -- highest row is deleted.
            --
            -- `plan` is nullable because most subscriptions have exactly one tier.
            CREATE TABLE IF NOT EXISTS {prefix}_price_points (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subscription_id TEXT NOT NULL
                    REFERENCES {prefix}_subscriptions(id) ON DELETE CASCADE,
                valid_from TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                currency TEXT NOT NULL DEFAULT 'EUR',
                cycle TEXT NOT NULL
                    CHECK (cycle IN ('weekly','monthly','quarterly','yearly','one_off')),
                reason TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                plan TEXT,
                UNIQUE (subscription_id, valid_from, reason)
            );

            -- The CHECK is the widened one: 'covered' arrived as a DROP/ADD
            -- CONSTRAINT pair under Postgres and is simply in the list here.
            CREATE TABLE IF NOT EXISTS {prefix}_state_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subscription_id TEXT NOT NULL
                    REFERENCES {prefix}_subscriptions(id) ON DELETE CASCADE,
                effective TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('considering','trial','active','covered','paused','cancelled')),
                note TEXT NOT NULL DEFAULT '',
                recorded_at TEXT NOT NULL,
                UNIQUE (subscription_id, effective, state)
            );

            -- Index names carry the prefix too: one file is one namespace now.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_price_points_sub
                ON {prefix}_price_points(subscription_id, valid_from);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_state_changes_sub
                ON {prefix}_state_changes(subscription_id, effective);

            -- The nullable raw location columns are preserved from the export for
            -- the places capability, which reads candidates to link spend to venues
            -- (capabilities/places/README.md, D1/D2).
            CREATE TABLE IF NOT EXISTS {prefix}_transaction_candidates (
                id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                currency TEXT NOT NULL,
                source_account TEXT NOT NULL,
                source_reference TEXT,
                proposed_account TEXT NOT NULL,
                confidence_basis_points INTEGER NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('pending','confirmed','rejected','duplicate')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT,
                location_street TEXT,
                location_postal_code TEXT,
                location_city TEXT,
                location_country TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_candidates_state
                ON {prefix}_transaction_candidates(state, booked_at DESC);

            CREATE TABLE IF NOT EXISTS {prefix}_transaction_projection (
                id TEXT PRIMARY KEY,
                booked_at TEXT NOT NULL,
                description TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer')),
                account TEXT NOT NULL,
                category TEXT NOT NULL,
                amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
                currency TEXT NOT NULL,
                source_id TEXT,
                purpose TEXT,
                trip_id TEXT,
                cash_amount_cents INTEGER NOT NULL DEFAULT 0
                    CHECK (cash_amount_cents >= 0),
                shared_cents INTEGER NOT NULL DEFAULT 0
                    CHECK (shared_cents >= 0),
                reimbursement_for TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_projection_date
                ON {prefix}_transaction_projection(booked_at DESC);

            -- `singleton` is INTEGER because SQLite has no boolean type; the CHECK
            -- is what keeps the table to one row, exactly as it did as a BOOLEAN.
            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection_state (
                singleton INTEGER PRIMARY KEY DEFAULT 1 CHECK (singleton = 1),
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );

            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection (
                instrument TEXT PRIMARY KEY,
                quantity_mantissa INTEGER NOT NULL,
                quantity_scale INTEGER NOT NULL CHECK (quantity_scale BETWEEN 0 AND 12),
                price_mantissa INTEGER,
                price_scale INTEGER CHECK (price_scale BETWEEN 0 AND 12),
                currency TEXT NOT NULL,
                CHECK ((price_mantissa IS NULL) = (price_scale IS NULL))
            );

            CREATE TABLE IF NOT EXISTS {prefix}_holding_projection_sources (
                source_key TEXT PRIMARY KEY,
                snapshot_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                coverage TEXT NOT NULL DEFAULT 'complete'
            );

            -- ---------------------------------------------------------------
            -- Market observations and the decision ledger above them.
            -- ---------------------------------------------------------------

            -- An observed market price, never a correction of one. Shape copied
            -- from {prefix}_price_points above: AUTOINCREMENT so (observed_on, id)
            -- keeps append order, a UNIQUE tuple so a re-fetch is idempotent, and
            -- (mantissa, scale) rather than REAL because a float is where an exact
            -- decimal stops being exact.
            --
            -- NOT {prefix}_price_points: that series is Axon's own subscription
            -- pricing history (§9.2 dogfooding) and must not be conflated with
            -- market data.
            --
            -- No CHECK on `source`, deliberately. The provider set grows, SQLite
            -- cannot alter a CHECK, and this crate has no ALTER path at all (see
            -- the doc comment above). `price::PROVIDERS` is the enumeration
            -- instead, and a unit test asserts every registered name round-trips.
            CREATE TABLE IF NOT EXISTS {prefix}_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                instrument TEXT NOT NULL,
                observed_on TEXT NOT NULL,
                price_mantissa INTEGER NOT NULL,
                price_scale INTEGER NOT NULL CHECK (price_scale BETWEEN 0 AND 12),
                currency TEXT NOT NULL,
                source TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                UNIQUE (instrument, observed_on, source)
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_prices_instrument
                ON {prefix}_prices(instrument, observed_on);

            -- A published FX reference rate, quote units per one base unit,
            -- stored as published and never inverted at write time -- a division
            -- is where an exact decimal stops being exact.
            CREATE TABLE IF NOT EXISTS {prefix}_fx_rates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                base TEXT NOT NULL,
                quote TEXT NOT NULL,
                observed_on TEXT NOT NULL,
                rate_mantissa INTEGER NOT NULL,
                rate_scale INTEGER NOT NULL CHECK (rate_scale BETWEEN 0 AND 12),
                source TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                UNIQUE (base, quote, observed_on, source)
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_fx_rates_pair
                ON {prefix}_fx_rates(base, quote, observed_on);

            -- Every attempt, successful or not. Modelled on
            -- capabilities/places/src/store.rs's geocode cache status column: a
            -- miss or a refusal is a recorded row, so the question of why an
            -- instrument is stale has an answer that is not somebody's memory.
            -- `detail` carries a bounded reason -- an HTTP status, a note that
            -- the body was not CSV -- and never a response body, because a
            -- provider page can contain anything and this table is backed up.
            CREATE TABLE IF NOT EXISTS {prefix}_price_fetches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider TEXT NOT NULL,
                target TEXT NOT NULL,
                requested_on TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('ok','empty','refused','error')),
                detail TEXT NOT NULL DEFAULT '',
                rows_written INTEGER NOT NULL DEFAULT 0,
                fetched_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_price_fetches_recent
                ON {prefix}_price_fetches(provider, fetched_at DESC);

            -- The proposal. Everything that happens to it afterwards is a row in
            -- {prefix}_decision_events, because a column is a thing that can be
            -- updated, and a single UPDATEd verdict loses the date the call was
            -- actually made.
            --
            -- `data_class` carries NO CHECK: the vocabulary belongs to
            -- libs/content-item, it has already been renamed once (comms' own
            -- predecessor constraint needed a full table rebuild), and this crate
            -- cannot rebuild a table. `content_item::valid()` is called at every
            -- write site instead. `kind` and `rung` DO carry CHECKs -- those are
            -- closed sets this capability owns.
            CREATE TABLE IF NOT EXISTS {prefix}_decisions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL
                    CHECK (kind IN ('rebalance','contribute','sell','hold','review')),
                subject TEXT NOT NULL,
                rung TEXT NOT NULL CHECK (rung IN ('rule','model')),
                data_class TEXT NOT NULL DEFAULT 'c1',
                data_class_rationale TEXT NOT NULL
                    DEFAULT 'A proposal about the owner''s own allocation names no third party.',
                proposal_json TEXT NOT NULL,
                evidence_json TEXT NOT NULL,
                model_revision TEXT NOT NULL,
                proposed_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_decisions_subject
                ON {prefix}_decisions(kind, subject, proposed_at DESC);

            -- The append-only half. Status is derived, never stored: `open` when
            -- the proposal has no verdict and no supersession, otherwise the
            -- latest verdict, or superseded. `recorded_at` is second-granular,
            -- which is what keeps a same-day corrected verdict from hitting the
            -- UNIQUE tuple; two verdicts inside one second are answered as a
            -- named 409 rather than a 500.
            --
            -- Nothing here is ever deleted, including a supersession. A run
            -- asserts `superseded` -- the run that produced this no longer
            -- produces it -- and a later run that produces it again asserts
            -- `reinstated` beside it. The LATEST of that pair is the answer, so
            -- the history of a proposal leaving and re-entering the inbox is
            -- readable rather than overwritten; a `verdict` closes the row and
            -- outranks both, because a human answered these exact numbers.
            --
            -- The fourth CHECK value costs a table rebuild on a file that already
            -- carries the three-value shape, which `rebuild_decision_events_check`
            -- below performs once, behind a probe. That is the whole reason the
            -- earlier form deleted the row instead; the rebuild is the honest
            -- price of an append-only ledger and it is paid here.
            CREATE TABLE IF NOT EXISTS {prefix}_decision_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                decision_id TEXT NOT NULL
                    REFERENCES {prefix}_decisions(id) ON DELETE CASCADE,
                event TEXT NOT NULL
                    CHECK (event IN ('verdict','outcome','superseded','reinstated')),
                verdict TEXT CHECK (verdict IN ('accepted','rejected')),
                note TEXT NOT NULL DEFAULT '',
                outcome_json TEXT,
                recorded_at TEXT NOT NULL,
                CHECK ((event = 'verdict') = (verdict IS NOT NULL)),
                UNIQUE (decision_id, event, recorded_at)
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_decision_events_decision
                ON {prefix}_decision_events(decision_id, recorded_at);
            "
        ))?;
        rebuild_decision_events_check(conn, prefix)?;
        Ok(())
    }

    /// Every subscription, each with its full series attached.
    ///
    /// Three queries rather than one join, because a join across two one-to-many
    /// series multiplies the rows and then has to be de-duplicated in memory
    /// anyway. Three ordered scans are simpler to read and simpler to be right.
    pub fn list(&self) -> Fallible<Vec<Subscription>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;

        let mut subs: Vec<Subscription> = conn.query_all(
            &format!(
                "SELECT id, name, source_path, category, value_rating
                 FROM {prefix}_subscriptions ORDER BY name"
            ),
            [],
            row_to_subscription,
        )?;

        for sub in &mut subs {
            sub.prices = conn.query_all(
                &format!(
                    "SELECT valid_from, amount_cents, currency, cycle, plan, reason
                     FROM {prefix}_price_points
                     WHERE subscription_id = ?1 ORDER BY valid_from, id"
                ),
                params![&sub.id],
                row_to_price,
            )?;

            sub.states = conn
                .query_all(
                    &format!(
                        "SELECT effective, state, note
                         FROM {prefix}_state_changes
                         WHERE subscription_id = ?1 ORDER BY effective, id"
                    ),
                    params![&sub.id],
                    row_to_state,
                )?
                .into_iter()
                .flatten()
                .collect();
        }
        Ok(subs)
    }

    pub fn get(&self, id: &str) -> Fallible<Option<Subscription>> {
        Ok(self.list()?.into_iter().find(|s| s.id == id))
    }

    /// Import a scanned note, or recognise one already imported.
    ///
    /// Identity is the vault-relative path, so a second import of the same note is
    /// a no-op rather than a duplicate. Crucially it does **not** re-seed the
    /// series: the frontmatter's single cost figure was only ever a starting point,
    /// and re-applying it would silently discard every price change recorded since.
    ///
    /// Returns whether a new subscription was created.
    pub fn import_note(&self, note: &ScannedNote, today: &str) -> Fallible<(String, bool)> {
        let prefix = &self.prefix;
        let conn = self.conn()?;

        if let Some(existing) = conn
            .query_row(
                &format!("SELECT id FROM {prefix}_subscriptions WHERE source_path = ?1"),
                params![&note.source_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok((existing, false));
        }

        let seed = crate::obsidian::seed_from_note(note, today);
        let id = format!("sub_{:016x}", fnv1a64(note.source_path.as_bytes()));

        conn.execute(
            &format!(
                "INSERT INTO {prefix}_subscriptions
                    (id, name, source_path, category, value_rating, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?6)"
            ),
            params![
                &id,
                &seed.name,
                &seed.source_path,
                &seed.category,
                &seed.value_rating,
                &today,
            ],
        )?;

        for price in &seed.prices {
            insert_price(&conn, prefix, &id, price, today)?;
        }
        for state in &seed.states {
            insert_state(&conn, prefix, &id, state, today)?;
        }
        Ok((id, true))
    }

    /// Append a price point. Never updates: that is the guarantee.
    pub fn append_price(&self, id: &str, price: &PricePoint, today: &str) -> Fallible<bool> {
        let conn = self.conn()?;
        let created = insert_price(&conn, &self.prefix, id, price, today)?;
        touch(&conn, &self.prefix, id, today)?;
        Ok(created)
    }

    /// Append a state change. Never updates: that is the guarantee.
    pub fn append_state(&self, id: &str, change: &StateChange, today: &str) -> Fallible<bool> {
        let conn = self.conn()?;
        let created = insert_state(&conn, &self.prefix, id, change, today)?;
        touch(&conn, &self.prefix, id, today)?;
        Ok(created)
    }

    /// How many rows each series holds. Exists for the append-only regression test,
    /// which is otherwise reduced to trusting that no UPDATE was written.
    pub fn series_lengths(&self, id: &str) -> Fallible<(i64, i64)> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let prices: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {prefix}_price_points WHERE subscription_id = ?1"),
            params![&id],
            |row| row.get(0),
        )?;
        let states: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {prefix}_state_changes WHERE subscription_id = ?1"),
            params![&id],
            |row| row.get(0),
        )?;
        Ok((prices, states))
    }

    /// Stage normalized candidates. The CSV bytes never reach this store; the
    /// fingerprint makes importing the same export again a counted no-op. A
    /// changed mapping may refresh suggestions while a candidate is still pending;
    /// reviewed candidates remain untouched.
    pub fn stage_candidates(
        &self,
        candidates: &[TransactionCandidate],
        today: &str,
    ) -> Fallible<(usize, usize)> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let (mut created, mut existing) = (0, 0);
        for candidate in candidates {
            let confidence = i16::try_from(candidate.confidence_basis_points)?;
            let inserted = conn.execute(
                &format!(
                    "INSERT INTO {prefix}_transaction_candidates
                        (id, fingerprint, booked_at, description, amount_cents, currency,
                         source_account, source_reference, proposed_account,
                         confidence_basis_points, state, created_at,
                         location_street, location_postal_code, location_city,
                         location_country)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                     ON CONFLICT (fingerprint) DO NOTHING"
                ),
                params![
                    &candidate.id,
                    &candidate.fingerprint,
                    &candidate.booked_at,
                    &candidate.description,
                    candidate.amount_cents,
                    &candidate.currency,
                    &candidate.source_account,
                    &candidate.source_reference,
                    &candidate.proposed_account,
                    confidence,
                    candidate.state.as_str(),
                    &today,
                    &candidate.location_street,
                    &candidate.location_postal_code,
                    &candidate.location_city,
                    &candidate.location_country,
                ],
            )?;
            if inserted == 1 {
                created += 1;
            } else {
                // COALESCE, not plain assignment: a re-import through a mapping
                // that gained location columns fills them in, while one through
                // a mapping without them never erases what an earlier import
                // captured.
                conn.execute(
                    &format!(
                        "UPDATE {prefix}_transaction_candidates
                         SET proposed_account = ?2, confidence_basis_points = ?3,
                             location_street = COALESCE(?4, location_street),
                             location_postal_code = COALESCE(?5, location_postal_code),
                             location_city = COALESCE(?6, location_city),
                             location_country = COALESCE(?7, location_country)
                         WHERE fingerprint = ?1 AND state = 'pending'"
                    ),
                    params![
                        &candidate.fingerprint,
                        &candidate.proposed_account,
                        confidence,
                        &candidate.location_street,
                        &candidate.location_postal_code,
                        &candidate.location_city,
                        &candidate.location_country,
                    ],
                )?;
                existing += 1;
            }
        }
        Ok((created, existing))
    }

    pub fn list_candidates(&self) -> Fallible<Vec<TransactionCandidate>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT id, fingerprint, booked_at, description, amount_cents,
                            currency, source_account, source_reference, proposed_account,
                            confidence_basis_points, state, location_street,
                            location_postal_code, location_city, location_country
                     FROM {prefix}_transaction_candidates
                     ORDER BY booked_at DESC, id"
                ),
                [],
                row_to_candidate,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// One indexed read, not a scan.
    ///
    /// This loaded and deserialized every candidate row to find one, inside the
    /// per-item loop of both batch handlers -- so a batch of n cost n full table
    /// reads. The column list is `list_candidates`'s, so `row_to_candidate` reads
    /// the same shape from either door.
    pub fn candidate(&self, id: &str) -> Fallible<Option<TransactionCandidate>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, fingerprint, booked_at, description, amount_cents,
                            currency, source_account, source_reference, proposed_account,
                            confidence_basis_points, state, location_street,
                            location_postal_code, location_city, location_country
                     FROM {prefix}_transaction_candidates
                     WHERE id = ?1"
                ),
                params![&id],
                row_to_candidate,
            )
            .optional()?
            .flatten())
    }

    pub fn review_candidate(
        &self,
        id: &str,
        state: CandidateState,
        account: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = ?2, proposed_account = ?3, reviewed_at = ?4
                 WHERE id = ?1"
            ),
            params![&id, state.as_str(), &account, &today],
        )? == 1)
    }

    pub fn review_transfer_pair(
        &self,
        canonical_id: &str,
        duplicate_id: &str,
        canonical_account: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let canonical = transaction.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = 'confirmed', proposed_account = ?2, reviewed_at = ?3
                 WHERE id = ?1 AND state IN ('pending','confirmed')"
            ),
            params![&canonical_id, &canonical_account, &today],
        )?;
        let duplicate = transaction.execute(
            &format!(
                "UPDATE {prefix}_transaction_candidates
                 SET state = 'duplicate', reviewed_at = ?3
                 WHERE id = ?1 AND id <> ?2 AND state IN ('pending','duplicate')"
            ),
            params![&duplicate_id, &canonical_id, &today],
        )?;
        if canonical != 1 || duplicate != 1 {
            // Dropped without committing, so `Transaction`'s own Drop rolls back:
            // half a transfer pair is not a state this table may be left in.
            return Ok(false);
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Replace the disposable index in one database transaction. The journal is
    /// canonical, so a half-rebuilt projection is never observable.
    pub fn replace_transaction_projection(&self, rows: &[TransactionRow]) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(&format!("DELETE FROM {prefix}_transaction_projection"), [])?;
        {
            let mut insert = transaction.prepare(&format!(
                "INSERT INTO {prefix}_transaction_projection
                    (id, booked_at, description, kind, account, category, amount_cents,
                     currency, source_id, purpose, trip_id, cash_amount_cents, shared_cents,
                     reimbursement_for)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
            ))?;
            for row in rows {
                insert.execute(params![
                    &row.id,
                    &row.date,
                    &row.description,
                    row.kind.as_str(),
                    &row.account,
                    &row.category,
                    row.amount_cents,
                    &row.currency,
                    &row.source_id,
                    &row.purpose.map(|purpose| purpose.as_str()),
                    &row.trip_id,
                    row.cash_amount_cents,
                    row.shared_cents,
                    &row.reimbursement_for,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn transaction_projection(&self) -> Fallible<Vec<TransactionRow>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT id, booked_at, description, kind, account, category,
                            amount_cents, currency, source_id, purpose, trip_id,
                            cash_amount_cents, shared_cents, reimbursement_for
                     FROM {prefix}_transaction_projection
                     ORDER BY booked_at DESC, id"
                ),
                [],
                row_to_transaction,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Replace the disposable holdings index and its review marker together. The
    /// marker makes a reviewed empty portfolio distinguishable from no snapshot.
    pub fn replace_holding_projection(&self, snapshot: &ReviewedHoldingsSnapshot) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(&format!("DELETE FROM {prefix}_holding_projection"), [])?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_state"),
            [],
        )?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_sources"),
            [],
        )?;
        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_holding_projection_state
                    (singleton, snapshot_id, reviewed_at, coverage) VALUES (1, ?1, ?2, ?3)"
            ),
            params![
                &snapshot.snapshot_id,
                &snapshot.reviewed_at,
                snapshot.coverage.as_str(),
            ],
        )?;
        for holding in &snapshot.holdings {
            let quantity_scale = i32::try_from(holding.quantity.scale)?;
            let (price_mantissa, price_scale) = holding
                .latest_unit_price
                .as_ref()
                .map(|price| {
                    Ok::<_, Box<dyn std::error::Error>>((
                        Some(price.mantissa),
                        Some(i32::try_from(price.scale)?),
                    ))
                })
                .transpose()?
                .unwrap_or((None, None));
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_holding_projection
                        (instrument, quantity_mantissa, quantity_scale,
                         price_mantissa, price_scale, currency)
                     VALUES (?1,?2,?3,?4,?5,?6)"
                ),
                params![
                    &holding.instrument,
                    holding.quantity.mantissa,
                    quantity_scale,
                    price_mantissa,
                    price_scale,
                    &holding.currency,
                ],
            )?;
        }
        for source in &snapshot.sources {
            transaction.execute(
                &format!(
                    "INSERT INTO {prefix}_holding_projection_sources
                        (source_key, snapshot_id, reviewed_at, coverage) VALUES (?1,?2,?3,?4)"
                ),
                params![
                    &source.source_key,
                    &source.snapshot_id,
                    &source.reviewed_at,
                    source.coverage.as_str(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_holding_projection(&self) -> Fallible<()> {
        let prefix = &self.prefix;
        let mut conn = self.conn()?;
        let transaction = sjel_store::write_transaction(&mut conn)?;
        transaction.execute(&format!("DELETE FROM {prefix}_holding_projection"), [])?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_state"),
            [],
        )?;
        transaction.execute(
            &format!("DELETE FROM {prefix}_holding_projection_sources"),
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn holding_projection(&self) -> Fallible<Option<ReviewedHoldingsSnapshot>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let Some((snapshot_id, reviewed_at, stored_coverage)) = conn
            .query_row(
                &format!(
                    "SELECT snapshot_id, reviewed_at, coverage
                     FROM {prefix}_holding_projection_state WHERE singleton = 1"
                ),
                [],
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
            return Ok(None);
        };
        let mut holdings = Vec::new();
        for (
            instrument,
            quantity_mantissa,
            quantity_scale,
            price_mantissa,
            price_scale,
            currency,
        ) in conn.query_all(
            &format!(
                "SELECT instrument, quantity_mantissa, quantity_scale,
                            price_mantissa, price_scale, currency
                     FROM {prefix}_holding_projection ORDER BY instrument"
            ),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i32>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )? {
            let latest_unit_price = match (price_mantissa, price_scale) {
                (Some(mantissa), Some(scale)) => Some(Quantity {
                    mantissa,
                    scale: scale.try_into()?,
                }),
                (None, None) => None,
                _ => return Err("holding projection price is incomplete".into()),
            };
            holdings.push(Holding {
                instrument,
                quantity: Quantity {
                    mantissa: quantity_mantissa,
                    scale: quantity_scale.try_into()?,
                },
                latest_unit_price,
                currency,
            });
        }
        let sources = conn
            .query_all(
                &format!(
                    "SELECT source_key, snapshot_id, reviewed_at, coverage
                     FROM {prefix}_holding_projection_sources ORDER BY source_key"
                ),
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .into_iter()
            .map(|(source_key, snapshot_id, reviewed_at, coverage)| {
                Ok(ReviewedHoldingsSource {
                    source_key,
                    snapshot_id,
                    reviewed_at,
                    coverage: HoldingsCoverage::parse(&coverage)
                        .ok_or("holding projection source coverage is invalid")?,
                })
            })
            .collect::<Fallible<Vec<_>>>()?;
        let coverage = HoldingsCoverage::parse(&stored_coverage)
            .ok_or("holding projection coverage is invalid")?;
        Ok(Some(ReviewedHoldingsSnapshot {
            schema_version: if sources.is_empty() { 1 } else { 2 },
            snapshot_id,
            reviewed_at,
            coverage,
            holdings,
            sources,
        }))
    }

    // -----------------------------------------------------------------------
    // Market observations
    // -----------------------------------------------------------------------

    /// Append one observed market price. `Ok(false)` means the row was already
    /// there, which is what makes a re-fetch free.
    ///
    /// Named `append_market_price` and not `append_price`: this crate already has
    /// an `append_price` for a subscription's own price series, and the two must
    /// never be reachable through one name.
    pub fn append_market_price(&self, observation: &PriceObservation) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let scale = i32::try_from(observation.price.scale)?;
        let changed = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {prefix}_prices
                    (instrument, observed_on, price_mantissa, price_scale, currency,
                     source, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)"
            ),
            params![
                &observation.instrument,
                &observation.observed_on,
                observation.price.mantissa,
                scale,
                &observation.currency,
                &observation.source,
                &observation.fetched_at,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn append_fx_rate(&self, observation: &FxObservation) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let scale = i32::try_from(observation.rate.scale)?;
        let changed = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {prefix}_fx_rates
                    (base, quote, observed_on, rate_mantissa, rate_scale, source, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)"
            ),
            params![
                &observation.base,
                &observation.quote,
                &observation.observed_on,
                observation.rate.mantissa,
                scale,
                &observation.source,
                &observation.fetched_at,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Every attempt, successful or not.
    pub fn record_fetch(&self, attempt: &FetchAttempt) -> Fallible<()> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_price_fetches
                    (provider, target, requested_on, status, detail, rows_written, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)"
            ),
            params![
                &attempt.provider,
                &attempt.target,
                &attempt.requested_on,
                attempt.status.as_str(),
                &attempt.detail,
                attempt.rows_written,
                &attempt.fetched_at,
            ],
        )?;
        Ok(())
    }

    /// The newest observation per instrument.
    ///
    /// Newest by `(observed_on, fetched_at, id)` rather than by source priority:
    /// two sources may hold one instrument-day, and which one a reader saw is a
    /// fact the reader reports rather than a preference this layer bakes in.
    pub fn latest_prices(&self) -> Fallible<Vec<PriceObservation>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT instrument, observed_on, price_mantissa, price_scale,
                            currency, source, fetched_at
                     FROM {prefix}_prices AS outer_price
                     WHERE id = (
                         SELECT id FROM {prefix}_prices AS inner_price
                         WHERE inner_price.instrument = outer_price.instrument
                         ORDER BY observed_on DESC, fetched_at DESC, id DESC
                         LIMIT 1
                     )
                     ORDER BY instrument"
                ),
                [],
                row_to_market_price,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Every observation for one instrument, oldest first.
    pub fn price_series(&self, instrument: &str) -> Fallible<Vec<PriceObservation>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT instrument, observed_on, price_mantissa, price_scale,
                            currency, source, fetched_at
                     FROM {prefix}_prices WHERE instrument = ?1
                     ORDER BY observed_on, fetched_at, id"
                ),
                params![&instrument],
                row_to_market_price,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Every observation, oldest first. The risk model's whole input.
    pub fn all_prices(&self) -> Fallible<Vec<PriceObservation>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT instrument, observed_on, price_mantissa, price_scale,
                            currency, source, fetched_at
                     FROM {prefix}_prices ORDER BY instrument, observed_on, fetched_at, id"
                ),
                [],
                row_to_market_price,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// How many observations exist per instrument, and the newest date.
    pub fn price_counts(&self) -> Fallible<Vec<(String, i64)>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        conn.query_all(
            &format!(
                "SELECT instrument, COUNT(*) FROM {prefix}_prices
                 GROUP BY instrument ORDER BY instrument"
            ),
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(Into::into)
    }

    /// The newest published rate per (base, quote) pair.
    pub fn latest_fx_rates(&self) -> Fallible<Vec<FxObservation>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT base, quote, observed_on, rate_mantissa, rate_scale,
                            source, fetched_at
                     FROM {prefix}_fx_rates AS outer_rate
                     WHERE id = (
                         SELECT id FROM {prefix}_fx_rates AS inner_rate
                         WHERE inner_rate.base = outer_rate.base
                           AND inner_rate.quote = outer_rate.quote
                         ORDER BY observed_on DESC, fetched_at DESC, id DESC
                         LIMIT 1
                     )
                     ORDER BY base, quote"
                ),
                [],
                row_to_fx,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The newest fetch attempts, bounded.
    pub fn recent_fetches(&self, limit: i64) -> Fallible<Vec<FetchAttempt>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!(
                    "SELECT provider, target, requested_on, status, detail,
                            rows_written, fetched_at
                     FROM {prefix}_price_fetches
                     ORDER BY fetched_at DESC, id DESC LIMIT ?1"
                ),
                params![limit],
                row_to_fetch,
            )?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The most recent moment any provider delivered an observation, as the
    /// stamp it was written with. The whole body of `GET /__axon/freshness`.
    ///
    /// `status = 'ok'` and nothing else, for the reason
    /// `capabilities/comms/src/store/source_state.rs` gives for reading
    /// `last_success_at` rather than `last_run_at`: an attempt that refused
    /// still ran, so a run column stays fresh while nothing arrives. `refused`
    /// is Yahoo answering a gate page and `error` is a dead route, and neither
    /// may answer "yes, data is still reaching finance".
    ///
    /// `empty` is excluded with them, and that is the one judgement call here.
    /// `run_provider` writes `empty` when a provider answered correctly and
    /// produced no observation at all -- an instrument with no configured
    /// ticker, a series with no rows. Nothing was delivered, so nothing
    /// arrived. It is NOT the idempotent case: a nightly re-fetch that returns
    /// the same closes it returned yesterday is `ok` with `rows_written = 0`,
    /// because the provider handed over observations and the UNIQUE tuple
    /// dropped them. So the normal quiet night still moves this forward, which
    /// is what keeps the contract about the producer rather than about the
    /// market calendar.
    ///
    /// MAX over providers, not per provider: one working source means the
    /// capability is still being fed, and a single provider that has stopped is
    /// the narrower fault `GET /api/prices/status` already reports per provider.
    ///
    /// `MAX` on the TEXT column is chronological because every value comes from
    /// `clock::now_timestamp`: twenty characters, fixed width, UTC. A stamp in
    /// another shape would sort wrong here and is refused at the other end --
    /// `clock::epoch_seconds` answers `None` rather than guessing.
    ///
    /// `None` when no attempt has ever succeeded, which doctor reads as "never"
    /// rather than as "fresh".
    pub fn newest_price_arrival(&self) -> Fallible<Option<String>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!(
                "SELECT MAX(fetched_at) FROM {prefix}_price_fetches
                 WHERE status = 'ok' AND fetched_at <> ''"
            ),
            [],
            |row| row.get::<_, Option<String>>(0),
        )?)
    }

    // -----------------------------------------------------------------------
    // The decision ledger
    // -----------------------------------------------------------------------

    /// Every proposal matching `status`, with its events attached.
    ///
    /// Status is derived here and stored nowhere, which is the whole point of the
    /// two-table shape: `open` when a proposal has no verdict and no supersession,
    /// otherwise its latest verdict, or `superseded`.
    pub fn decisions(&self, status: Option<&str>) -> Fallible<Vec<StoredDecision>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let rows: Vec<StoredProposal> = conn
            .query_all(
                &format!(
                    "SELECT id, kind, subject, rung, data_class, data_class_rationale,
                            proposal_json, evidence_json, model_revision, proposed_at
                     FROM {prefix}_decisions ORDER BY proposed_at DESC, id"
                ),
                [],
                row_to_proposal,
            )?
            .into_iter()
            .flatten()
            .collect();
        let mut events: std::collections::BTreeMap<String, Vec<StoredDecisionEvent>> =
            std::collections::BTreeMap::new();
        for (decision_id, event) in conn
            .query_all(
                &format!(
                    "SELECT decision_id, event, verdict, note, outcome_json, recorded_at
                     FROM {prefix}_decision_events ORDER BY recorded_at, id"
                ),
                [],
                row_to_decision_event,
            )?
            .into_iter()
            .flatten()
        {
            events.entry(decision_id).or_default().push(event);
        }
        let mut decisions: Vec<StoredDecision> = rows
            .into_iter()
            .map(|proposal| {
                let events = events.remove(&proposal.id).unwrap_or_default();
                StoredDecision { proposal, events }
            })
            .collect();
        if let Some(status) = status.filter(|status| *status != "all") {
            decisions.retain(|decision| decision.status() == status);
        }
        Ok(decisions)
    }

    pub fn decision(&self, id: &str) -> Fallible<Option<StoredDecision>> {
        Ok(self
            .decisions(None)?
            .into_iter()
            .find(|decision| decision.proposal.id == id))
    }

    /// Append one event. `Ok(false)` is the UNIQUE collision -- two events of one
    /// kind on one proposal inside one second -- which the handler answers as a
    /// named 409 rather than a 500.
    pub fn append_decision_event(
        &self,
        decision_id: &str,
        event: &StoredDecisionEvent,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let changed = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO {prefix}_decision_events
                    (decision_id, event, verdict, note, outcome_json, recorded_at)
                 VALUES (?1,?2,?3,?4,?5,?6)"
            ),
            params![
                &decision_id,
                &event.event,
                &event.verdict,
                &event.note,
                &event.outcome_json,
                &event.recorded_at,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Reconcile one run's proposals against what is open, in one transaction.
    ///
    /// `BEGIN IMMEDIATE`, not the default deferred begin, and the reason is not
    /// style: `finance-cli decisions run` and the server can each call this, an
    /// in-process mutex cannot see another process, and the primary key does not
    /// save it -- two supersession events with different `recorded_at` both
    /// satisfy `UNIQUE (decision_id, event, recorded_at)`, so an open proposal
    /// could be superseded twice by two runs that each believed they were the
    /// replacement. Taking the writer lock up front makes the loser wait
    /// (`busy_timeout` is 5000 ms on every pooled connection) instead of erroring.
    pub fn reconcile_decisions(
        &self,
        minted: &[StoredProposal],
        recorded_at: &str,
    ) -> Fallible<DecisionRunOutcome> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let transaction =
            conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let open_ids: Vec<String> = {
            // "Currently open", derived exactly as `StoredDecision::status`
            // derives it, because a presence test would be wrong now that a
            // supersession can be answered by a later `reinstated`: a proposal
            // that left the inbox and came back carries BOTH events, and only
            // the later one is its state. A row with no verdict and no
            // supersession at all has never left, hence the COALESCE default.
            let mut statement = transaction.prepare(&format!(
                "SELECT d.id FROM {prefix}_decisions AS d
                 WHERE NOT EXISTS (
                     SELECT 1 FROM {prefix}_decision_events AS answered
                     WHERE answered.decision_id = d.id AND answered.event = 'verdict'
                 )
                 AND COALESCE((
                     SELECT e.event FROM {prefix}_decision_events AS e
                     WHERE e.decision_id = d.id
                       AND e.event IN ('superseded','reinstated')
                     ORDER BY e.recorded_at DESC, e.id DESC
                     LIMIT 1
                 ), 'reinstated') <> 'superseded'"
            ))?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        let mut outcome = DecisionRunOutcome::default();
        // Reinstate anything this run produces again, BEFORE the supersede loop,
        // so a proposal the current run produces is open by definition.
        //
        // The proposal id is a hash over BUCKETED numbers (decision.rs), so a
        // drift that crosses a band edge and comes back into the same 10 bp
        // bucket re-mints the id the ledger already carries. `INSERT OR IGNORE`
        // alone then dropped the row, the earlier `superseded` event survived,
        // and the proposal stayed invisible to the human FOR EVER -- no later run
        // can mint a different id for it -- while the run reported `unchanged`
        // and success. Reproduced end to end 2026-09-05: three runs left two live
        // rebalance proposals unreachable while the engine's own dry run still
        // produced both.
        //
        // The answer is a row, not a deletion. Both assertions stay on the
        // record and the later one is the state, so "this left the inbox on the
        // 5th and came back on the 6th" is readable a year later -- which is the
        // whole reason this ledger has no mutable column. A proposal a human has
        // answered is not reinstated at all: the verdict was given on these exact
        // numbers, and re-asking would be the ledger forgetting.
        let mut reopened: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for proposal in minted {
            let appended = transaction.execute(
                &format!(
                    "INSERT OR IGNORE INTO {prefix}_decision_events
                        (decision_id, event, verdict, note, outcome_json, recorded_at)
                     SELECT ?1, 'reinstated', NULL, ?2, NULL, ?3
                     WHERE NOT EXISTS (
                         SELECT 1 FROM {prefix}_decision_events AS answered
                         WHERE answered.decision_id = ?1 AND answered.event = 'verdict')
                       AND (
                         SELECT e.event FROM {prefix}_decision_events AS e
                         WHERE e.decision_id = ?1
                           AND e.event IN ('superseded','reinstated')
                         ORDER BY e.recorded_at DESC, e.id DESC
                         LIMIT 1
                       ) = 'superseded'"
                ),
                params![
                    &proposal.id,
                    "a later run produced this proposal again",
                    recorded_at
                ],
            )?;
            if appended == 1 {
                reopened.insert(proposal.id.as_str());
            }
        }
        for id in &open_ids {
            if minted.iter().any(|proposal| &proposal.id == id) {
                continue;
            }
            let changed = transaction.execute(
                &format!(
                    "INSERT OR IGNORE INTO {prefix}_decision_events
                        (decision_id, event, verdict, note, outcome_json, recorded_at)
                     VALUES (?1, 'superseded', NULL, ?2, NULL, ?3)"
                ),
                params![
                    id,
                    "the run that produced this no longer produces it",
                    recorded_at
                ],
            )?;
            outcome.superseded += usize::from(changed == 1);
        }
        for proposal in minted {
            let changed = transaction.execute(
                &format!(
                    "INSERT OR IGNORE INTO {prefix}_decisions
                        (id, kind, subject, rung, data_class, data_class_rationale,
                         proposal_json, evidence_json, model_revision, proposed_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
                ),
                params![
                    &proposal.id,
                    &proposal.kind,
                    &proposal.subject,
                    &proposal.rung,
                    &proposal.data_class,
                    &proposal.data_class_rationale,
                    &proposal.proposal_json,
                    &proposal.evidence_json,
                    &proposal.model_revision,
                    &proposal.proposed_at,
                ],
            )?;
            if changed == 1 {
                outcome.proposed += 1;
            } else if reopened.contains(proposal.id.as_str()) {
                outcome.reopened += 1;
            } else {
                outcome.unchanged += 1;
            }
        }
        transaction.commit()?;
        Ok(outcome)
    }
}

/// What one reconcile run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DecisionRunOutcome {
    pub proposed: usize,
    pub unchanged: usize,
    /// Already in the ledger, superseded by an earlier run, and produced again by
    /// this one, so a `reinstated` event was appended. Counted apart from
    /// `unchanged` because the row moved back into the human's inbox, which is a
    /// different fact from "nothing happened".
    pub reopened: usize,
    pub superseded: usize,
}

/// The stored proposal, exactly as its row holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProposal {
    pub id: String,
    pub kind: String,
    pub subject: String,
    pub rung: String,
    pub data_class: String,
    pub data_class_rationale: String,
    pub proposal_json: String,
    pub evidence_json: String,
    pub model_revision: String,
    pub proposed_at: String,
}

/// One appended fact about a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDecisionEvent {
    pub event: String,
    pub verdict: Option<String>,
    pub note: String,
    pub outcome_json: Option<String>,
    pub recorded_at: String,
}

/// A proposal with everything that happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDecision {
    pub proposal: StoredProposal,
    pub events: Vec<StoredDecisionEvent>,
}

impl StoredDecision {
    /// Derived, never stored. A column is a thing that can be updated, and a
    /// single UPDATEd verdict loses the date the call was actually made.
    pub fn status(&self) -> &str {
        if let Some(verdict) = self.latest_verdict() {
            return verdict.verdict.as_deref().unwrap_or("open");
        }
        // The LATEST of the supersede/reinstate pair, not the presence of either.
        // A proposal can leave the inbox and come back any number of times, and
        // every one of those assertions stays on the record; a presence test on
        // `superseded` would read a row that came back as still gone.
        match self.latest_reachability() {
            Some("superseded") => "superseded",
            _ => "open",
        }
    }

    pub fn latest_verdict(&self) -> Option<&StoredDecisionEvent> {
        self.events
            .iter()
            .filter(|event| event.event == "verdict")
            .max_by(|left, right| left.recorded_at.cmp(&right.recorded_at))
    }

    /// The later of this proposal's newest `superseded` and newest `reinstated`,
    /// which is what decides whether it is in the inbox.
    ///
    /// Ordered on `recorded_at` and then on arrival, because the two can share a
    /// second: `reconcile_decisions` stamps every event in one run with one
    /// timestamp, and a run that reinstates a proposal and a later run that
    /// supersedes it again inside the same second must still resolve.
    fn latest_reachability(&self) -> Option<&str> {
        self.events
            .iter()
            .enumerate()
            .filter(|(_, event)| matches!(event.event.as_str(), "superseded" | "reinstated"))
            .max_by(|left, right| {
                left.1
                    .recorded_at
                    .cmp(&right.1.recorded_at)
                    .then(left.0.cmp(&right.0))
            })
            .map(|(_, event)| event.event.as_str())
    }

    pub fn latest_outcome(&self) -> Option<&StoredDecisionEvent> {
        self.events
            .iter()
            .filter(|event| event.event == "outcome")
            .max_by(|left, right| left.recorded_at.cmp(&right.recorded_at))
    }
}

fn row_to_market_price(row: &Row) -> rusqlite::Result<Option<PriceObservation>> {
    let scale: i32 = row.get("price_scale")?;
    let Ok(scale) = u32::try_from(scale) else {
        return Ok(None);
    };
    Ok(Some(PriceObservation {
        instrument: row.get("instrument")?,
        observed_on: row.get("observed_on")?,
        price: Quantity {
            mantissa: row.get("price_mantissa")?,
            scale,
        },
        currency: row.get("currency")?,
        source: row.get("source")?,
        fetched_at: row.get("fetched_at")?,
    }))
}

fn row_to_fx(row: &Row) -> rusqlite::Result<Option<FxObservation>> {
    let scale: i32 = row.get("rate_scale")?;
    let Ok(scale) = u32::try_from(scale) else {
        return Ok(None);
    };
    Ok(Some(FxObservation {
        base: row.get("base")?,
        quote: row.get("quote")?,
        observed_on: row.get("observed_on")?,
        rate: Quantity {
            mantissa: row.get("rate_mantissa")?,
            scale,
        },
        source: row.get("source")?,
        fetched_at: row.get("fetched_at")?,
    }))
}

/// A status this binary does not know is dropped rather than guessed into the
/// nearest neighbour, the way `row_to_state` above already does it.
fn row_to_fetch(row: &Row) -> rusqlite::Result<Option<FetchAttempt>> {
    let Some(status) = FetchStatus::parse(row.get::<_, String>("status")?.as_str()) else {
        return Ok(None);
    };
    Ok(Some(FetchAttempt {
        provider: row.get("provider")?,
        target: row.get("target")?,
        requested_on: row.get("requested_on")?,
        status,
        detail: row.get("detail")?,
        rows_written: row.get("rows_written")?,
        fetched_at: row.get("fetched_at")?,
    }))
}

fn row_to_proposal(row: &Row) -> rusqlite::Result<Option<StoredProposal>> {
    Ok(Some(StoredProposal {
        id: row.get("id")?,
        kind: row.get("kind")?,
        subject: row.get("subject")?,
        rung: row.get("rung")?,
        data_class: row.get("data_class")?,
        data_class_rationale: row.get("data_class_rationale")?,
        proposal_json: row.get("proposal_json")?,
        evidence_json: row.get("evidence_json")?,
        model_revision: row.get("model_revision")?,
        proposed_at: row.get("proposed_at")?,
    }))
}

fn row_to_decision_event(row: &Row) -> rusqlite::Result<Option<(String, StoredDecisionEvent)>> {
    Ok(Some((
        row.get("decision_id")?,
        StoredDecisionEvent {
            event: row.get("event")?,
            verdict: row.get("verdict")?,
            note: row.get("note")?,
            outcome_json: row.get("outcome_json")?,
            recorded_at: row.get("recorded_at")?,
        },
    )))
}

/// Widen `{prefix}_decision_events`' `event` CHECK to admit `reinstated`, once,
/// on a file that already carries the three-value shape.
///
/// The doctrine above -- one `CREATE TABLE IF NOT EXISTS` batch, no ALTER path --
/// holds only while no deployed file predates a constraint change. Measured
/// 2026-09-06: the owner's `axon.db` already holds this table with
/// `CHECK (event IN ('verdict','outcome','superseded'))`, written by an earlier
/// run of this same code, and `CREATE TABLE IF NOT EXISTS` never revisits an
/// installed table. So this is the table-rebuild dance SQLite's own documentation
/// prescribes, behind a probe of what the file actually holds. The precedent is
/// `capabilities/comms/src/store/migrations.rs`, which does the same for the
/// C0-C3 class vocabulary.
///
/// Unlike that one, this runs INSIDE the migration transaction and does not turn
/// `foreign_keys` off, and the difference is a property of this table rather than
/// a shortcut. comms had to disable enforcement because `DROP TABLE` performs an
/// implicit DELETE that fires `ON DELETE CASCADE` on the dropped table's
/// children, and its tables have children. `{prefix}_decision_events` has none:
/// it is the child, `{prefix}_decisions` is the parent, and `sqlite_master`
/// carries no other `REFERENCES {prefix}_decision_events`. Dropping it therefore
/// cascades nothing, and every copied row keeps its `decision_id` verbatim, so no
/// reference changes.
///
/// Idempotent: the probe reads the installed DDL text, so a second pass finds the
/// widened CHECK and does nothing.
fn rebuild_decision_events_check(conn: &Connection, prefix: &str) -> Fallible<()> {
    let table = format!("{prefix}_decision_events");
    let installed: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![&table],
            |row| row.get(0),
        )
        .optional()?;
    // No table at all means the batch above just created it with the four-value
    // CHECK. A table whose DDL already names the value has been rebuilt.
    let Some(installed) = installed else {
        return Ok(());
    };
    if installed.contains("'reinstated'") {
        return Ok(());
    }
    // Read the indexes back and replay them rather than trusting the batch above
    // to have declared every one: a DROP takes every index the deployed file
    // actually carries, including one this repository no longer ships.
    let attached: Vec<String> = conn.query_all(
        "SELECT sql FROM sqlite_master
         WHERE tbl_name = ?1 AND type IN ('index','trigger') AND sql IS NOT NULL",
        params![&table],
        |row| row.get(0),
    )?;
    let scratch = format!("{table}_reinstated");
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS {scratch};
         CREATE TABLE {scratch} (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             decision_id TEXT NOT NULL
                 REFERENCES {prefix}_decisions(id) ON DELETE CASCADE,
             event TEXT NOT NULL
                 CHECK (event IN ('verdict','outcome','superseded','reinstated')),
             verdict TEXT CHECK (verdict IN ('accepted','rejected')),
             note TEXT NOT NULL DEFAULT '',
             outcome_json TEXT,
             recorded_at TEXT NOT NULL,
             CHECK ((event = 'verdict') = (verdict IS NOT NULL)),
             UNIQUE (decision_id, event, recorded_at)
         );
         INSERT INTO {scratch}
             (id, decision_id, event, verdict, note, outcome_json, recorded_at)
         SELECT id, decision_id, event, verdict, note, outcome_json, recorded_at
         FROM {table};
         DROP TABLE {table};
         ALTER TABLE {scratch} RENAME TO {table};"
    ))?;
    for statement in attached {
        // The batch's own index is `IF NOT EXISTS`; a replayed one from the file
        // may not be, and the DROP already took it.
        conn.execute_batch(&statement)?;
    }
    Ok(())
}

fn insert_price(
    conn: &Connection,
    prefix: &str,
    id: &str,
    price: &PricePoint,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {prefix}_price_points
                (subscription_id, valid_from, amount_cents, currency, cycle, plan, reason, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT (subscription_id, valid_from, reason) DO NOTHING"
        ),
        params![
            &id,
            &price.valid_from,
            price.amount_cents,
            &price.currency,
            cycle_str(price.cycle),
            &price.plan,
            &price.reason,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

fn insert_state(
    conn: &Connection,
    prefix: &str,
    id: &str,
    change: &StateChange,
    today: &str,
) -> Fallible<bool> {
    let inserted = conn.execute(
        &format!(
            "INSERT INTO {prefix}_state_changes
                (subscription_id, effective, state, note, recorded_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT (subscription_id, effective, state) DO NOTHING"
        ),
        params![
            &id,
            &change.effective,
            change.state.as_str(),
            &change.note,
            &today,
        ],
    )?;
    Ok(inserted == 1)
}

/// `updated_at` on the parent row is the one thing that does get updated, and it
/// describes the record rather than the money.
fn touch(conn: &Connection, prefix: &str, id: &str, today: &str) -> Fallible<()> {
    conn.execute(
        &format!("UPDATE {prefix}_subscriptions SET updated_at = ?2 WHERE id = ?1"),
        params![&id, &today],
    )?;
    Ok(())
}

fn cycle_str(cycle: BillingCycle) -> &'static str {
    match cycle {
        BillingCycle::Weekly => "weekly",
        BillingCycle::Monthly => "monthly",
        BillingCycle::Quarterly => "quarterly",
        BillingCycle::Yearly => "yearly",
        BillingCycle::OneOff => "one_off",
    }
}

fn cycle_from_str(raw: &str) -> BillingCycle {
    match raw {
        "weekly" => BillingCycle::Weekly,
        "quarterly" => BillingCycle::Quarterly,
        "yearly" => BillingCycle::Yearly,
        "one_off" => BillingCycle::OneOff,
        _ => BillingCycle::Monthly,
    }
}

fn row_to_subscription(row: &Row) -> rusqlite::Result<Subscription> {
    Ok(Subscription {
        id: row.get("id")?,
        name: row.get("name")?,
        source_path: row.get("source_path")?,
        category: row.get("category")?,
        value_rating: row.get("value_rating")?,
        prices: Vec::new(),
        states: Vec::new(),
    })
}

fn row_to_price(row: &Row) -> rusqlite::Result<PricePoint> {
    Ok(PricePoint {
        valid_from: row.get("valid_from")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        cycle: cycle_from_str(row.get::<_, String>("cycle")?.as_str()),
        plan: row.get("plan")?,
        reason: row.get("reason")?,
    })
}

/// A state the enum no longer knows is dropped rather than guessed into the
/// nearest neighbour. The CHECK constraint makes it unreachable today; if a later
/// migration widens it, a stale binary reporting the wrong state is worse than one
/// reporting a shorter history.
fn row_to_state(row: &Row) -> rusqlite::Result<Option<StateChange>> {
    let Some(state) = State::parse(row.get::<_, String>("state")?.as_str()) else {
        return Ok(None);
    };
    Ok(Some(StateChange {
        effective: row.get("effective")?,
        state,
        note: row.get("note")?,
    }))
}

/// `Ok(None)` for a row this binary cannot represent, which the caller drops.
/// Distinct from `Err`: a column that will not convert is a broken read, while a
/// state string the enum does not know is a row from a newer writer.
fn row_to_candidate(row: &Row) -> rusqlite::Result<Option<TransactionCandidate>> {
    let confidence: i16 = row.get("confidence_basis_points")?;
    let (Ok(confidence_basis_points), Some(state)) = (
        confidence.try_into(),
        CandidateState::parse(row.get::<_, String>("state")?.as_str()),
    ) else {
        return Ok(None);
    };
    Ok(Some(TransactionCandidate {
        id: row.get("id")?,
        fingerprint: row.get("fingerprint")?,
        booked_at: row.get("booked_at")?,
        description: row.get("description")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        source_account: row.get("source_account")?,
        source_reference: row.get("source_reference")?,
        proposed_account: row.get("proposed_account")?,
        confidence_basis_points,
        state,
        location_street: row.get("location_street")?,
        location_postal_code: row.get("location_postal_code")?,
        location_city: row.get("location_city")?,
        location_country: row.get("location_country")?,
    }))
}

fn row_to_transaction(row: &Row) -> rusqlite::Result<Option<TransactionRow>> {
    let Some(kind) = TransactionKind::parse(row.get::<_, String>("kind")?.as_str()) else {
        return Ok(None);
    };
    Ok(Some(TransactionRow {
        id: row.get("id")?,
        date: row.get("booked_at")?,
        description: row.get("description")?,
        kind,
        account: row.get("account")?,
        category: row.get("category")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        source_id: row.get("source_id")?,
        purpose: row
            .get::<_, Option<String>>("purpose")?
            .as_deref()
            .and_then(crate::allocation::SpendingPurpose::parse),
        trip_id: row.get("trip_id")?,
        cash_amount_cents: row.get("cash_amount_cents")?,
        shared_cents: row.get("shared_cents")?,
        reimbursement_for: row.get("reimbursement_for")?,
    }))
}

/// Stable id from the note's path, so the same note gets the same id on any
/// machine and a re-import after a database rebuild lands on the row it had.
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_prefix_that_could_carry_sql_is_refused() {
        assert!(validate_prefix("finance").is_ok());
        assert!(validate_prefix("finance_test_123").is_ok());
        assert!(validate_prefix("finance; DROP TABLE finance_subscriptions").is_err());
        assert!(validate_prefix("Finance").is_err());
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn cycles_round_trip_through_their_stored_spelling() {
        for cycle in [
            BillingCycle::Weekly,
            BillingCycle::Monthly,
            BillingCycle::Quarterly,
            BillingCycle::Yearly,
            BillingCycle::OneOff,
        ] {
            assert_eq!(cycle_from_str(cycle_str(cycle)), cycle);
        }
    }

    #[test]
    fn an_id_is_derived_from_the_path_so_it_survives_a_rebuild() {
        let a = fnv1a64(b"Atlas/Finance/Subscriptions/Example.md");
        let b = fnv1a64(b"Atlas/Finance/Subscriptions/Example.md");
        let c = fnv1a64(b"Atlas/Finance/Subscriptions/Other.md");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
