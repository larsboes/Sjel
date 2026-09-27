//! Persistence under the table prefix `scouting` in the one shared SQLite file
//! (PRD Q45) — see `capabilities/store/README.md`'s correlation section,
//! Phase 2 and this crate's own README's Verdict section. This capability
//! shipped a SQLite version first (single-user tool, avoid unnecessary
//! machinery), then moved to a Postgres schema on a shared instance once
//! cross-capability correlation with `transit` became the actual point of
//! Phase 2. PRD Q45 kept the sharing and dropped the server: one file, one
//! table prefix per capability, and the correlation joins are unchanged.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use sjel_store::QueryAll;

use crate::opportunity::Opportunity;

pub struct Store {
    /// Shared with every other store in this process on the same file, so
    /// opening one is a checkout rather than an open.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `scouting` here means `scouting_opportunities` and its three siblings.
    prefix: String,
}

impl Store {
    pub fn open(database_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_prefix(database_path, "scouting")
    }

    /// `prefix` is always either the literal `"scouting"` (production, via
    /// `open()`) or a test-generated name (see `db_tests`) — never user input.
    /// SQL has no parametrized-identifier syntax for `CREATE TABLE`, so
    /// prefixed table names are built via `format!` throughout this file;
    /// that's safe specifically because the prefix's origin is always one of
    /// those two controlled cases, not because interpolation into SQL is safe
    /// in general.
    fn open_with_prefix(
        database_path: &Path,
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A pool checkout, and the migration runs once per process per (file,
        // prefix) rather than once per open -- libs/sjel-store/README.md has why.
        let pool = sjel_store::open_pool(database_path, prefix, |conn| {
            Self::init_schema(conn, prefix)
        })?;
        Ok(Self {
            pool,
            prefix: prefix.to_string(),
        })
    }

    /// A connection from the shared pool, for the duration of one statement.
    ///
    /// A `Result` where this used to be `self.conn.lock().unwrap()`: that unwrap
    /// could only fail on a poisoned mutex, whereas a checkout can genuinely fail
    /// when the file is unreachable or every connection is busy.
    fn conn(&self) -> Result<sjel_store::PooledClient, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// The cheapest statement that proves this store can actually reach its database.
    ///
    /// A checkout from the pool is not enough on its own — the point is to fail exactly when a
    /// real query would, which is what the readiness surface promises its caller (#126).
    pub fn ping(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the four tables, not the history that produced them.
    ///
    /// The `ALTER TABLE ... ADD COLUMN IF NOT EXISTS latitude/longitude` retrofit is
    /// folded into `CREATE TABLE`. It existed because a Postgres database already held
    /// rows written before the coordinate columns did; no SQLite file predates this
    /// migration, and SQLite has no `ADD COLUMN IF NOT EXISTS` to express it with.
    fn init_schema(conn: &Connection, prefix: &str) -> Result<(), Box<dyn std::error::Error>> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_opportunities (
                id TEXT PRIMARY KEY,
                opportunity_type TEXT NOT NULL,
                source TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                url TEXT,
                title TEXT NOT NULL,
                starts_at TEXT,
                ends_at TEXT,
                location TEXT,
                city TEXT,
                country_code TEXT,
                latitude REAL,
                longitude REAL,
                raw TEXT,
                fetched_at TEXT,
                score REAL,
                matched_focus TEXT,
                rationale TEXT,
                vault_link TEXT,
                status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','dismissed','saved')),
                first_seen TEXT,
                last_seen TEXT
            );

            CREATE TABLE IF NOT EXISTS {prefix}_links (
                opportunity_id TEXT NOT NULL,
                vault_path TEXT NOT NULL,
                link_type TEXT NOT NULL,
                created_at TEXT,
                PRIMARY KEY (opportunity_id, vault_path)
            );

            -- Phase 1 correlation-engine memory: per-adapter run bookkeeping.
            -- `cursor` is scaffolding only for now -- nothing reads it back to
            -- skip fetch work yet (no adapter does incremental/since-last-run
            -- fetching). Documented honestly in README, same as the
            -- only-1-of-4-adapters-live-verified gap.
            CREATE TABLE IF NOT EXISTS {prefix}_source_state (
                adapter_name TEXT PRIMARY KEY,
                last_run_at TEXT,
                cursor TEXT
            );

            -- Somewhere for a candidate source to land. A Splash hub id or a
            -- `cal-…` noticed mid-run was lost between sessions, because the
            -- only place a source could exist was the overlay's `sources[]`
            -- and nothing at runtime may write there.
            --
            -- This table cannot make anything run. `create_adapter` is only
            -- ever called on `Config::sources`, read from the overlay file; a
            -- row here is inert until a human copies it across. That is the
            -- whole design: discovery earns a suggestion, never a fetch. See
            -- the README section on sources being declared, never discovered.
            --
            -- `status` borrows comms' word for the same idea one level lower,
            -- items there and sources here, rather than inventing a second.
            CREATE TABLE IF NOT EXISTS {prefix}_proposed_sources (
                id TEXT PRIMARY KEY,
                adapter TEXT NOT NULL,
                locator TEXT NOT NULL,
                label TEXT,
                found_by TEXT NOT NULL,
                found_at TEXT NOT NULL,
                note TEXT,
                status TEXT NOT NULL DEFAULT 'proposed'
                    CHECK (status IN ('proposed','dismissed'))
            );

            -- Index names carry the prefix too: one file is one namespace now, so
            -- `idx_opp_type` would collide where two schemas kept it apart.
            CREATE INDEX IF NOT EXISTS idx_{prefix}_opp_type
                ON {prefix}_opportunities(opportunity_type);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_opp_city
                ON {prefix}_opportunities(city);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_opp_status
                ON {prefix}_opportunities(status);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_proposed_status
                ON {prefix}_proposed_sources(status);
            "
        ))?;
        Ok(())
    }

    /// Valid `status` values -- matches the CHECK constraint above.
    /// Kept in sync manually (single-user tool, not worth generating).
    pub const VALID_STATUSES: [&'static str; 3] = ["new", "dismissed", "saved"];

    /// Sets an opportunity's status. Returns `Ok(false)` if no row matched
    /// `id` (not an error -- the id just doesn't exist). Returns `Err` for an
    /// invalid status string rather than silently no-op'ing on a typo.
    pub fn set_status(&self, id: &str, status: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if !Self::VALID_STATUSES.contains(&status) {
            return Err(format!(
                "invalid status '{status}' -- must be one of: {}",
                Self::VALID_STATUSES.join(", ")
            )
            .into());
        }
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_opportunities SET status = ?1 WHERE id = ?2",
                self.prefix
            ),
            params![&status, &id],
        )?;
        Ok(affected > 0)
    }

    /// Looks up a single opportunity's current status, if it exists.
    pub fn get_status(&self, id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status FROM {}_opportunities WHERE id = ?1",
                    self.prefix
                ),
                params![&id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Upserts per-adapter run bookkeeping: `last_run_at` is always bumped to
    /// now; `cursor` is only overwritten when `Some` (passing `None` preserves
    /// whatever cursor was recorded last time, rather than clobbering it).
    pub fn record_run(
        &self,
        adapter_name: &str,
        cursor: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = chrono_now();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_source_state (adapter_name, last_run_at, cursor)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (adapter_name) DO UPDATE SET
                     last_run_at = excluded.last_run_at,
                     cursor = COALESCE(excluded.cursor, {prefix}_source_state.cursor)",
                prefix = self.prefix
            ),
            params![&adapter_name, &now, &cursor],
        )?;
        Ok(())
    }

    /// Reads back per-adapter run bookkeeping (round-trip counterpart to
    /// `record_run`).
    pub fn get_source_state(
        &self,
        adapter_name: &str,
    ) -> Result<Option<SourceState>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT adapter_name, last_run_at, cursor FROM {}_source_state
                     WHERE adapter_name = ?1",
                    self.prefix
                ),
                params![&adapter_name],
                |row| {
                    Ok(SourceState {
                        adapter_name: row.get(0)?,
                        last_run_at: row.get(1)?,
                        cursor: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    // -----------------------------------------------------------------------
    // Proposed sources — the inbox a discovered candidate lands in.
    //
    // Nothing here can start a fetch. The only thing that makes a source run is
    // an entry in the overlay's `sources[]`, which no code path in this crate
    // writes. A proposal is a note to a human, and the human moves it.
    // -----------------------------------------------------------------------

    /// Record a candidate source, or refresh the one already recorded for the
    /// same `(adapter, locator)`.
    ///
    /// Identity is the pair, not a generated key: noticing the same Splash hub
    /// on three separate runs is one proposal seen three times, and an inbox
    /// that grows a row per sighting is an inbox nobody reads. A re-sighting
    /// keeps the original `found_at` — when it first showed up is the useful
    /// fact — and leaves a dismissal dismissed, because re-proposing something
    /// the operator already said no to is how an inbox stops being trusted.
    ///
    /// Returns true when this is the first sighting.
    pub fn propose_source(
        &self,
        adapter: &str,
        locator: &str,
        label: Option<&str>,
        found_by: &str,
        note: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if adapter.trim().is_empty() || locator.trim().is_empty() {
            return Err("a proposed source needs both an adapter and a locator".into());
        }
        if found_by.trim().is_empty() {
            return Err("a proposed source needs to say what found it".into());
        }
        let id = proposed_source_id(adapter, locator);
        let now = chrono_now();
        let prefix = &self.prefix;
        let mut conn = self.conn()?;

        // Two statements where Postgres had one. `RETURNING (xmax = 0)` asked
        // Postgres directly whether the row it just touched was an insert or an
        // update; SQLite exposes no such system column, and the two candidate
        // one-statement answers are both wrong. `changes()` reports 1 for either
        // branch of an upsert, and comparing the stored `found_at` to `now`
        // reports the second of two sightings inside one second as new.
        //
        // Insert-or-nothing first, then update only if nothing was inserted. The
        // transaction is what makes the pair one decision.
        let transaction = sjel_store::write_transaction(&mut conn)?;
        let inserted = transaction.execute(
            &format!(
                "INSERT INTO {prefix}_proposed_sources
                     (id, adapter, locator, label, found_by, found_at, note, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'proposed')
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &id,
                adapter.trim(),
                locator.trim(),
                &label,
                found_by.trim(),
                &now,
                &note,
            ],
        )? == 1;
        if !inserted {
            // `status` and `found_at` are absent on purpose, exactly as they were
            // absent from the DO UPDATE list: a dismissal stays dismissed, and
            // when it first showed up is the fact worth keeping.
            transaction.execute(
                &format!(
                    "UPDATE {prefix}_proposed_sources SET
                         label    = COALESCE(?2, label),
                         found_by = ?3,
                         note     = COALESCE(?4, note)
                     WHERE id = ?1"
                ),
                params![&id, &label, found_by.trim(), &note],
            )?;
        }
        transaction.commit()?;
        Ok(inserted)
    }

    /// Every proposal with the given status, newest sighting first.
    pub fn list_proposed_sources(
        &self,
        status: &str,
    ) -> Result<Vec<ProposedSource>, Box<dyn std::error::Error>> {
        if !Self::PROPOSAL_STATUSES.contains(&status) {
            return Err(format!(
                "invalid proposal status '{status}' -- must be one of: {}",
                Self::PROPOSAL_STATUSES.join(", ")
            )
            .into());
        }
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, adapter, locator, label, found_by, found_at, note, status
                 FROM {}_proposed_sources WHERE status = ?1
                 ORDER BY found_at DESC, id",
                self.prefix
            ),
            params![&status],
            |r| {
                Ok(ProposedSource {
                    id: r.get(0)?,
                    adapter: r.get(1)?,
                    locator: r.get(2)?,
                    label: r.get(3)?,
                    found_by: r.get(4)?,
                    found_at: r.get(5)?,
                    note: r.get(6)?,
                    status: r.get(7)?,
                })
            },
        )?)
    }

    /// Take a proposal out of the inbox. `Ok(false)` means no such id, which is
    /// not an error: dismissing something twice is the same wish twice.
    pub fn dismiss_proposed_source(&self, id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let affected = conn.execute(
            &format!(
                "UPDATE {}_proposed_sources SET status = 'dismissed' WHERE id = ?1",
                self.prefix
            ),
            params![&id],
        )?;
        Ok(affected > 0)
    }

    /// Valid proposal statuses. There is deliberately no `promoted`: promotion
    /// happens in the overlay's config file, which this process cannot write,
    /// so a status claiming it happened would be this table's opinion rather
    /// than a fact. `declared` is derived on read instead — see
    /// `ProposedSource::is_declared_by`.
    pub const PROPOSAL_STATUSES: [&'static str; 2] = ["proposed", "dismissed"];

    pub fn upsert(
        &self,
        opp: &Opportunity,
        score: f64,
        matched_focus: Option<&str>,
        rationale: &str,
        vault_link: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let now = chrono_now();
        let raw_json = serde_json::to_string(&opp.raw)?;
        let matched = matched_focus.unwrap_or("");
        let opp_type = format!("{:?}", opp.opportunity_type);
        let src_kind = format!("{:?}", opp.source_kind);

        let conn = self.conn()?;

        let is_new = conn
            .query_row(
                &format!("SELECT id FROM {}_opportunities WHERE id = ?1", self.prefix),
                params![&opp.id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_none();

        // Critical: `status` is set to 'new' ONLY on first insert (literal in
        // the VALUES list, not a bound param -- upsert() has no status input,
        // by design) and is deliberately ABSENT from the ON CONFLICT DO
        // UPDATE SET list below. A human's dismiss/save decision must survive
        // the same opportunity being re-fetched from its source tomorrow --
        // that's the actual point of Phase 1. See
        // `upsert_preserves_status_across_refetch` in db_tests for the proof.
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_opportunities (id, opportunity_type, source, source_kind, url, title,
                    starts_at, ends_at, location, city, country_code, latitude, longitude, raw, fetched_at, score,
                    matched_focus, rationale, vault_link, status, first_seen, last_seen)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,'new',?20,?21)
                ON CONFLICT (id) DO UPDATE SET
                    fetched_at = excluded.fetched_at,
                    score = excluded.score,
                    matched_focus = excluded.matched_focus,
                    rationale = excluded.rationale,
                    vault_link = excluded.vault_link,
                    last_seen = excluded.last_seen",
                prefix = self.prefix
            ),
            params![
                &opp.id,
                &opp_type,
                &opp.source,
                &src_kind,
                &opp.url,
                &opp.title,
                &opp.starts_at,
                &opp.ends_at,
                &opp.location,
                &opp.city,
                &opp.country_code,
                &opp.latitude,
                &opp.longitude,
                &raw_json,
                &opp.fetched_at,
                score,
                matched,
                &rationale,
                &vault_link,
                &now,
                &now,
            ],
        )?;

        if let Some(vp) = vault_link {
            conn.execute(
                &format!(
                    "INSERT INTO {prefix}_links (opportunity_id, vault_path, link_type, created_at)
                    VALUES (?1, ?2, ?3, ?4)
                    ON CONFLICT (opportunity_id, vault_path) DO NOTHING",
                    prefix = self.prefix
                ),
                params![&opp.id, &vp, "matched", &now],
            )?;
        }
        Ok(is_new)
    }

    pub fn count(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            &format!("SELECT COUNT(*) FROM {}_opportunities", self.prefix),
            [],
            |row| row.get(0),
        )?)
    }

    /// Ranked backlog. Excludes `status = 'dismissed'` by default -- the
    /// whole point of Phase 1 is that dismissed items stop coming back as if
    /// they were fresh. Pass `include_dismissed = true` for debugging/
    /// visibility (e.g. `--backlog --include-dismissed`); `saved` and `new`
    /// always show either way.
    pub fn list_top(
        &self,
        limit: usize,
        include_dismissed: bool,
    ) -> Result<Vec<RankedRow>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let filter = if include_dismissed {
            ""
        } else {
            "WHERE status != 'dismissed'"
        };
        let sql = format!(
            "SELECT id, opportunity_type, source, title, city, starts_at, ends_at, location, score, matched_focus, rationale, url, vault_link, status, country_code, latitude, longitude, raw
             FROM {prefix}_opportunities {filter} ORDER BY score DESC LIMIT ?1",
            prefix = self.prefix
        );
        Ok(conn.query_all(&sql, params![limit as i64], |row| {
            Ok(RankedRow {
                id: row.get(0)?,
                opportunity_type: row.get::<_, String>(1)?.to_lowercase(),
                source: row.get(2)?,
                title: row.get(3)?,
                city: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                starts_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                ends_at: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                location: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                score: row.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                matched_focus: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                rationale: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
                url: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                vault_link: row.get(12)?,
                status: row.get(13)?,
                country_code: row.get(14)?,
                latitude: row.get(15)?,
                longitude: row.get(16)?,
                raw: row.get(17)?,
            })
        })?)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RankedRow {
    pub id: String,
    pub opportunity_type: String,
    pub source: String,
    pub title: String,
    pub city: String,
    pub starts_at: String,
    pub ends_at: String,
    pub location: String,
    pub score: f64,
    pub matched_focus: String,
    pub rationale: String,
    pub url: String,
    pub vault_link: Option<String>,
    pub status: String,
    /// Whatever the source called the country. Deliberately not normalised
    /// here: Luma says "Germany", meetup says "de", and inventing a mapping in
    /// this file would put country knowledge in code. The geo policy matches
    /// case-insensitively against the tokens listed in config.
    pub country_code: Option<String>,
    /// Set only when the source gave a complete pair. A consumer computing a
    /// distance has to fall back to `country_code` when this is `None`, which
    /// is the common case for everything except Luma and the event hubs.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// The source's own payload. Carried so a policy can reason about what the
    /// provider actually said rather than a projection of it -- Luma leaves
    /// `geo_address_info` null for some events but still names an IANA
    /// timezone, which is enough to know a thing is not in Europe.
    pub raw: Option<String>,
}

/// Per-adapter run bookkeeping (`record_run`/`get_source_state`). `cursor` is
/// scaffolding for now -- see the `source_state` table comment in
/// `init_schema` and the README's honest gap note.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceState {
    pub adapter_name: String,
    pub last_run_at: String,
    pub cursor: Option<String>,
}

/// A candidate source somebody noticed, waiting for a human to decide.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProposedSource {
    pub id: String,
    /// Which adapter would read it, if it were ever declared. Not validated
    /// against `create_adapter`'s arms on the way in: a hub on a platform Axon
    /// has no adapter for yet is exactly the kind of thing worth remembering,
    /// and refusing it would throw away the note to write the adapter.
    pub adapter: String,
    /// The URL, path or platform id that identifies it.
    pub locator: String,
    pub label: Option<String>,
    /// What found it — a run, a source id, or `manual` when it was typed in.
    pub found_by: String,
    pub found_at: String,
    pub note: Option<String>,
    pub status: String,
}

impl ProposedSource {
    /// Whether a declared source already covers this proposal.
    ///
    /// Derived rather than stored, because the fact lives in the overlay's
    /// config file and this table would only ever hold a stale copy of it. The
    /// locator is compared to whichever field carries it for that adapter: a
    /// URL for network sources, the root path for file ones.
    pub fn is_declared_by(&self, declared: &[crate::sources::SourceManifest]) -> bool {
        declared.iter().any(|source| {
            let same_adapter = source.adapter.eq_ignore_ascii_case(&self.adapter);
            let by_url = source
                .url
                .as_deref()
                .is_some_and(|url| url.eq_ignore_ascii_case(&self.locator));
            let by_path = source
                .root_path
                .as_ref()
                .is_some_and(|path| path.to_string_lossy() == self.locator);
            same_adapter && (by_url || by_path)
        })
    }
}

/// Stable id for a candidate: the pair that identifies it, not a counter.
/// Lowercased so the same hub noticed with different casing is one proposal.
fn proposed_source_id(adapter: &str, locator: &str) -> String {
    format!(
        "{}:{}",
        adapter.trim().to_lowercase(),
        locator.trim().to_lowercase()
    )
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// Every test here needs a database, which is what the module name declares: `db_tests`
/// is the one selector CI splits on (CONTRIBUTING.md, "Validate the changed boundary").
/// Renamed from `tests` on 2026-08-25 (PRD Q44), then from `postgres_tests` at PRD Q45 --
/// Cargo has one filter, so a database test not named this way runs in the hermetic job
/// and is red on every runner.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::opportunity::{Opportunity, OpportunityType, SourceKind};

    /// A file per test, in a directory this process owns. It replaces the per-pid
    /// schema and its drop guard: that guard existed because four abandoned test
    /// schemas were sitting in the shared database on 2026-07-28 and would have gone
    /// into the next pg_dumpall. A temp file is neither shared nor backed up.
    fn open_test_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("scouting-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        // The pid is recycled eventually, so the file starts empty.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        Store::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    /// The readiness probe has to reach the database, not merely hold a pool handle —
    /// a check that passes without touching the file is the bug #126 is about.
    #[test]
    fn ping_reaches_the_database() {
        open_test_store("ping")
            .ping()
            .expect("a live store answers its own ping");
    }

    /// The readiness handler turns exactly this into a 503. It replaces "port 1 is
    /// unreachable": there is no port any more, so an unusable path is the failure a
    /// deployment can actually have.
    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        let blocker = std::env::temp_dir().join(format!("scouting-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            Store::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    fn mk_opp(id: &str, title: &str) -> Opportunity {
        Opportunity {
            id: id.into(),
            opportunity_type: OpportunityType::Event,
            source: "test".into(),
            source_kind: SourceKind::Api,
            url: "https://test.example".into(),
            title: title.into(),
            starts_at: None,
            ends_at: None,
            location: None,
            city: Some("Berlin".into()),
            country_code: Some("DE".into()),
            latitude: Some(52.52),
            longitude: Some(13.405),
            raw: serde_json::Value::Null,
            fetched_at: "123".into(),
        }
    }

    #[test]
    fn upsert_is_idempotent() {
        let store = open_test_store("idempotent");

        let opp = mk_opp("evt:test:1", "Test Hack");
        let new1 = store
            .upsert(&opp, 0.5, Some("Polymath"), "match", None)
            .unwrap();
        assert!(new1, "first insert should be new");

        let new2 = store
            .upsert(&opp, 0.6, Some("Polymath"), "match v2", None)
            .unwrap();
        assert!(!new2, "second insert should NOT be new (dedupe by id)");

        assert_eq!(
            store.count().unwrap(),
            1,
            "only 1 row after upsert of same id"
        );

        let rows = store.list_top(10, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Test Hack");
        assert!(
            (rows[0].score - 0.6).abs() < 1e-9,
            "score should be updated to 0.6"
        );
        assert_eq!(
            rows[0].status, "new",
            "freshly upserted opportunity defaults to 'new'"
        );
    }

    #[test]
    fn preserves_existing_on_new_insert() {
        let store = open_test_store("preserve");

        let opp_a = mk_opp("evt:test:a", "Event A");
        let opp_b = mk_opp("evt:test:b", "Event B");

        store
            .upsert(&opp_a, 0.7, Some("Polymath"), "a match", None)
            .unwrap();
        store
            .upsert(&opp_b, 0.3, Some("Career"), "b match", None)
            .unwrap();

        assert_eq!(store.count().unwrap(), 2);
        let rows = store.list_top(10, false).unwrap();
        assert_eq!(rows[0].title, "Event A", "higher score ranked first");
        assert_eq!(rows[1].title, "Event B");
        assert_eq!(rows[0].matched_focus, "Polymath");
        assert_eq!(rows[1].matched_focus, "Career");
    }

    #[test]
    fn set_status_updates_row() {
        let store = open_test_store("set_status");
        let opp = mk_opp("evt:test:status1", "Status Test");
        store.upsert(&opp, 0.5, None, "match", None).unwrap();

        let matched = store.set_status("evt:test:status1", "saved").unwrap();
        assert!(matched, "should match the existing row");

        let rows = store.list_top(10, false).unwrap();
        assert_eq!(rows[0].status, "saved");
    }

    #[test]
    fn set_status_unknown_id_returns_false_not_error() {
        let store = open_test_store("set_status_unknown");

        let matched = store.set_status("evt:does-not-exist", "dismissed").unwrap();
        assert!(!matched, "no row should match a nonexistent id");
    }

    // -----------------------------------------------------------------------
    // Proposed sources. The invariant under all of these: a proposal is a note
    // to a human and cannot make anything run.
    // -----------------------------------------------------------------------

    #[test]
    fn a_proposed_source_lands_in_the_inbox_with_its_provenance() {
        let store = open_test_store("propose_lands");

        let first = store
            .propose_source(
                "splash-hub",
                "142966",
                Some("A brand event hub"),
                "manual",
                Some("noticed while reading an event page"),
            )
            .unwrap();
        assert!(first, "the first sighting is new");

        let inbox = store.list_proposed_sources("proposed").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].adapter, "splash-hub");
        assert_eq!(inbox[0].locator, "142966");
        assert_eq!(
            inbox[0].found_by, "manual",
            "a proposal records what found it, or it is a fact with no origin"
        );
        assert!(!inbox[0].found_at.is_empty());
    }

    /// Seeing the same hub on three runs is one proposal, not three rows.
    #[test]
    fn a_second_sighting_updates_rather_than_filling_the_inbox() {
        let store = open_test_store("propose_resight");

        store
            .propose_source("splash-hub", "142966", None, "manual", None)
            .unwrap();
        let first_seen = store.list_proposed_sources("proposed").unwrap()[0]
            .found_at
            .clone();

        let again = store
            .propose_source(
                "splash-hub",
                "142966",
                Some("Now with a name"),
                "sweep",
                None,
            )
            .unwrap();
        assert!(!again, "the second sighting is not new");

        let inbox = store.list_proposed_sources("proposed").unwrap();
        assert_eq!(inbox.len(), 1, "one candidate is one row");
        assert_eq!(inbox[0].label.as_deref(), Some("Now with a name"));
        assert_eq!(inbox[0].found_by, "sweep", "the latest sighting says so");
        assert_eq!(
            inbox[0].found_at, first_seen,
            "when it first showed up is the fact worth keeping"
        );
    }

    #[test]
    fn casing_does_not_split_one_candidate_into_two() {
        let store = open_test_store("propose_casing");
        store
            .propose_source("luma-calendar", "cal-ABC123", None, "manual", None)
            .unwrap();
        store
            .propose_source("Luma-Calendar", "cal-abc123", None, "manual", None)
            .unwrap();
        assert_eq!(store.list_proposed_sources("proposed").unwrap().len(), 1);
    }

    /// The one that keeps the inbox worth opening.
    #[test]
    fn a_dismissed_candidate_does_not_come_back_on_the_next_sighting() {
        let store = open_test_store("propose_dismiss");
        store
            .propose_source("splash-hub", "999", None, "manual", None)
            .unwrap();
        assert!(store.dismiss_proposed_source("splash-hub:999").unwrap());
        assert!(store.list_proposed_sources("proposed").unwrap().is_empty());

        store
            .propose_source("splash-hub", "999", None, "sweep", None)
            .unwrap();
        assert!(
            store.list_proposed_sources("proposed").unwrap().is_empty(),
            "re-proposing what the operator refused is how an inbox stops being read"
        );
        assert_eq!(store.list_proposed_sources("dismissed").unwrap().len(), 1);
    }

    #[test]
    fn dismissing_something_that_is_not_there_is_not_an_error() {
        let store = open_test_store("propose_dismiss_missing");
        assert!(!store.dismiss_proposed_source("splash-hub:nope").unwrap());
    }

    #[test]
    fn a_proposal_with_no_origin_or_no_locator_is_refused() {
        let store = open_test_store("propose_incomplete");
        assert!(store
            .propose_source("splash-hub", "", None, "manual", None)
            .is_err());
        assert!(store
            .propose_source("", "142966", None, "manual", None)
            .is_err());
        assert!(store
            .propose_source("splash-hub", "142966", None, "  ", None)
            .is_err());
    }

    #[test]
    fn an_unknown_status_is_an_error_rather_than_an_empty_list() {
        let store = open_test_store("propose_bad_status");
        assert!(store.list_proposed_sources("promoted").is_err());
    }

    /// Promotion is a human editing the overlay. The table never claims it
    /// happened; the listing derives it from what is actually declared.
    #[test]
    fn a_proposal_knows_when_a_declared_source_already_covers_it() {
        use crate::sources::SourceEntry;

        let declared: Vec<crate::sources::SourceManifest> = serde_json::from_value::<
            Vec<SourceEntry>,
        >(serde_json::json!([
            { "id": "claude-community", "adapter": "luma-calendar", "url": "cal-TOpA5LAFfuDeFpu" }
        ]))
        .unwrap()
        .iter()
        .map(SourceEntry::resolve)
        .collect();

        let store = open_test_store("propose_declared");
        store
            .propose_source("luma-calendar", "cal-TOpA5LAFfuDeFpu", None, "manual", None)
            .unwrap();
        store
            .propose_source("luma-calendar", "cal-somethingelse", None, "manual", None)
            .unwrap();

        let inbox = store.list_proposed_sources("proposed").unwrap();
        let covered: Vec<bool> = inbox.iter().map(|p| p.is_declared_by(&declared)).collect();
        assert_eq!(covered.iter().filter(|c| **c).count(), 1);
    }

    #[test]
    fn set_status_invalid_value_is_an_error() {
        let store = open_test_store("set_status_invalid");
        let opp = mk_opp("evt:test:status2", "Status Test 2");
        store.upsert(&opp, 0.5, None, "match", None).unwrap();

        let result = store.set_status("evt:test:status2", "archived");
        assert!(
            result.is_err(),
            "an unrecognized status string must error, not silently no-op"
        );

        // Confirm the typo didn't silently do nothing to the actual status.
        let rows = store.list_top(10, false).unwrap();
        assert_eq!(rows[0].status, "new");
    }

    /// The critical Phase 1 correctness fix: a human's dismiss/save decision
    /// must survive the same opportunity being re-fetched from its source on
    /// a later run. upsert()'s ON CONFLICT clause must not touch `status`.
    #[test]
    fn upsert_preserves_status_across_refetch() {
        let store = open_test_store("preserve_status");

        let opp = mk_opp("evt:test:refetch", "Refetched Hack");
        store
            .upsert(&opp, 0.4, Some("Polymath"), "first sighting", None)
            .unwrap();
        store.set_status("evt:test:refetch", "dismissed").unwrap();

        // Simulate the adapter re-fetching the "same" opportunity tomorrow --
        // same id, but a fresh score/rationale/fetched_at, exactly what a real
        // re-run produces.
        let mut refetched = mk_opp("evt:test:refetch", "Refetched Hack");
        refetched.fetched_at = "999".into();
        let is_new = store
            .upsert(&refetched, 0.9, Some("Career"), "re-scored higher", None)
            .unwrap();
        assert!(!is_new, "still the same id, not a new row");

        let status_after = store.get_status("evt:test:refetch").unwrap();
        assert_eq!(
            status_after.as_deref(),
            Some("dismissed"),
            "dismiss decision must survive re-fetch"
        );

        // Other fields legitimately DO update on re-fetch (score, rationale) --
        // only status is protected.
        let rows = store.list_top(10, true).unwrap();
        assert!(
            (rows[0].score - 0.9).abs() < 1e-9,
            "score should still update on re-fetch"
        );
    }

    #[test]
    fn list_top_excludes_dismissed_by_default_includes_with_override() {
        let store = open_test_store("list_top_dismissed");

        let opp_a = mk_opp("evt:test:visible", "Visible Event");
        let opp_b = mk_opp("evt:test:hidden", "Hidden Event");
        store.upsert(&opp_a, 0.5, None, "match", None).unwrap();
        store.upsert(&opp_b, 0.8, None, "match", None).unwrap();
        store.set_status("evt:test:hidden", "dismissed").unwrap();

        let default_rows = store.list_top(10, false).unwrap();
        assert_eq!(default_rows.len(), 1, "dismissed row excluded by default");
        assert_eq!(default_rows[0].title, "Visible Event");

        let all_rows = store.list_top(10, true).unwrap();
        assert_eq!(all_rows.len(), 2, "include_dismissed=true shows both");
        assert!(all_rows
            .iter()
            .any(|r| r.title == "Hidden Event" && r.status == "dismissed"));
    }

    #[test]
    fn record_run_source_state_round_trip() {
        let store = open_test_store("source_state");

        assert!(
            store.get_source_state("euro_hackathons").unwrap().is_none(),
            "nothing recorded yet"
        );

        store
            .record_run("euro_hackathons", Some("cursor-abc"))
            .unwrap();
        let state = store
            .get_source_state("euro_hackathons")
            .unwrap()
            .expect("should exist now");
        assert_eq!(state.adapter_name, "euro_hackathons");
        assert_eq!(state.cursor.as_deref(), Some("cursor-abc"));
        assert!(!state.last_run_at.is_empty());

        // A later run with cursor=None preserves the previously recorded cursor.
        store.record_run("euro_hackathons", None).unwrap();
        let state2 = store.get_source_state("euro_hackathons").unwrap().unwrap();
        assert_eq!(
            state2.cursor.as_deref(),
            Some("cursor-abc"),
            "cursor preserved when not given"
        );
        assert!(
            state2.last_run_at >= state.last_run_at,
            "last_run_at should be bumped"
        );
    }
}
