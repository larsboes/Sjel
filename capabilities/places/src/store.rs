//! Persistence for the place registry, the permanent geocode cache, the
//! transaction links and the companion register (`README.md` schema section).
//!
//! Writes happen only under this store's own table prefix (ISA anti-claim A2).
//! Cross-capability reads live in `layers.rs` and `backfill.rs` as plain
//! SELECTs. PRD Q45 (2026-08-27) made those same-file joins across table
//! prefixes rather than across schemas; they are joins either way, which is the
//! property the shared instance existed for.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sjel_store::QueryAll;

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub struct PlacesStore {
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `places` here means `places_places` and its three siblings.
    prefix: String,
}

/// A registry row. `external_ref` holds a stable foreign identity such as an
/// EVA code (`eva:8000207`) or an OSM id (`osm:node/123`), which is what makes
/// every backfill idempotent: the same source identity is the same place.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: String,
    pub external_ref: Option<String>,
}

/// One recorded visit: the operator was here, then, and rated it so.
///
/// Distinct from `Place`, which describes somewhere that exists. Nothing here is
/// a property of the place -- two people disagree about the same restaurant, and a
/// registry that held one rating would be wrong for one of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceVisit {
    pub id: String,
    pub place_id: String,
    /// The export's own timestamp. Its date is what a planner asks about.
    pub visited_on: Option<String>,
    pub rating: Option<i64>,
    pub source: String,
    pub external_ref: Option<String>,
}

/// A visit joined to the place it happened at, which is the only shape a reader
/// wants: a rating with no name is a number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceVisitRow {
    pub place_id: String,
    pub place_name: String,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub visited_on: Option<String>,
    pub rating: Option<i64>,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: String,
    pub response: Option<String>,
    pub place_id: Option<String>,
}

/// One calendar month of a place's climate normal. Every measure is optional
/// because a provider that reports nothing for a month must store nothing:
/// a 0.0 that means "not observed" is the failure mode the nullable columns and
/// `days_observed` exist to prevent (Packs/travel ISA: a guess that looks like a
/// measurement is worse than a blank).
#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyNormal {
    pub month: u32,
    pub t_max_mean: Option<f64>,
    pub t_min_mean: Option<f64>,
    pub rain_days_mean: Option<f64>,
    pub precipitation_mm_mean: Option<f64>,
    pub daylight_hours_mean: Option<f64>,
    pub sunshine_hours_mean: Option<f64>,
    pub days_observed: i64,
}

/// What window the twelve rows were folded from, and when. `years_covered` alone
/// cannot tell 2015-2024 from 1995-2004, so the window is named as well as counted.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalsMeta {
    pub years_covered: i64,
    pub period_start: String,
    pub period_end: String,
    pub source: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone)]
pub struct PersonPlaceRow {
    pub id: String,
    pub person: String,
    pub place_id: String,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub confidence_bp: i16,
    pub source: String,
    pub state: String,
}

/// The only two states the review route may write. `Proposed` is deliberately
/// absent: proposals are created by `propose_person_place`, which hardcodes it,
/// and nothing else in this crate can spell `confirmed` into the table
/// (ISA PLC-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    Confirmed,
    Dismissed,
}

impl Review {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Dismissed => "dismissed",
        }
    }
}

/// A table prefix reaches SQL by interpolation, because SQL has no bind
/// parameter for an identifier. Copied deliberately from `finance`/`trips`:
/// the validation is the reason interpolating it is safe. `pub(crate)` because
/// the cross-capability readers in `backfill.rs`/`layers.rs` take the source
/// prefix as a parameter (tests point them at scratch prefixes) and interpolate
/// it under the same rule. Same character rule it had as `validate_schema`.
pub(crate) fn validate_prefix(prefix: &str) -> Fallible<()> {
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

impl PlacesStore {
    pub fn open(database_path: &Path) -> Fallible<Self> {
        Self::open_with_prefix(database_path, "places")
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

    pub(crate) fn conn(&self) -> Fallible<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The cheapest statement that proves this store can reach its database —
    /// what `/ready` promises, rather than mere liveness (#126).
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    /// The current shape of the five tables. `places` is declared before the three
    /// that reference it, because a batch executes in order.
    fn run_migration(conn: &Connection, prefix: &str) -> Fallible<()> {
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS {prefix}_places (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL
                    CHECK (kind IN ('venue','city','station','address','region')),
                address TEXT,
                city TEXT,
                country_code TEXT,
                latitude REAL,
                longitude REAL,
                source TEXT NOT NULL,
                external_ref TEXT,
                created_at TEXT NOT NULL
            );
            -- Index names carry the prefix too: one file is one namespace now.
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{prefix}_places_external_ref
                ON {prefix}_places(external_ref) WHERE external_ref IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_{prefix}_places_kind
                ON {prefix}_places(kind, name);

            -- Permanent by design (README D3): a repeat query is served from
            -- here and never leaves the host again. No TTL column on purpose.
            --
            -- `response` was JSONB and is TEXT holding JSON here -- one of the two
            -- measured columns in the repo with no SQLite equivalent (PRD Q45).
            -- Nothing ever queried inside it: every reader took `response::text`
            -- and parsed it in Rust, so the JSONB was buying validation, not
            -- indexing. That validation now happens where the value is produced.
            CREATE TABLE IF NOT EXISTS {prefix}_geocode_cache (
                query_hash TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                query TEXT NOT NULL,
                response TEXT,
                place_id TEXT,
                status TEXT NOT NULL CHECK (status IN ('hit','miss','error')),
                fetched_at TEXT NOT NULL
            );

            -- source_id is the finance journal's SHA-256 candidate fingerprint
            -- (capabilities/finance/src/import.rs), the one identity that
            -- survives projection rebuilds (README D2). One link per
            -- transaction: the PRIMARY KEY makes a venue link and a later city
            -- guess mutually exclusive without any code having to check.
            CREATE TABLE IF NOT EXISTS {prefix}_transaction_places (
                source_id TEXT PRIMARY KEY,
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                precision TEXT NOT NULL CHECK (precision IN ('venue','city')),
                confidence_bp INTEGER NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_transaction_places_place
                ON {prefix}_transaction_places(place_id);

            -- The companion register, PRD 8.2 / README D4. C2: never seeded
            -- into axon_demo, and no write path here sets 'confirmed' except
            -- review_person_place (ISA PLC-7).
            CREATE TABLE IF NOT EXISTS {prefix}_person_places (
                id TEXT PRIMARY KEY,
                person TEXT NOT NULL,
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                date_start TEXT,
                date_end TEXT,
                confidence_bp INTEGER NOT NULL,
                source TEXT NOT NULL,
                state TEXT NOT NULL
                    CHECK (state IN ('proposed','confirmed','dismissed')),
                created_at TEXT NOT NULL,
                reviewed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_person_places_state
                ON {prefix}_person_places(state, person);

            -- Climate normals, twelve rows per place (README D5, ISA F4).
            -- The table IS the cache: permanent by design like the geocode cache
            -- above, so there is no TTL column and no second cache table. A
            -- re-fetch is DELETE-then-twelve-INSERTs in one transaction, so a
            -- partial refresh never leaves a stale eleventh month behind.
            --
            -- Every measure is nullable because a provider that reports no
            -- sunshine duration at some latitude must store NULL, not 0.
            -- `days_observed` is NOT NULL so a short month is visible rather
            -- than silently averaged, and `years_covered` alone cannot tell
            -- 2015-2024 from 1995-2004, which is what the period columns say.
            -- `best_month` is deliberately not a column: the numbers are data,
            -- the rule that reads them is code (climate.rs::best_months).
            -- One time the operator was somewhere, as THEIR record rather than
            -- the place's.
            --
            -- A separate table on purpose. The review export carries a date and a
            -- star rating, and the first version of this importer dropped both with
            -- the reasoning that they are not place attributes -- which is right,
            -- and is exactly why they belong in their own table instead of in a
            -- column on the registry. A shared place has no opinion about whether
            -- anyone liked it; a person does, and the two must not be one row.
            CREATE TABLE IF NOT EXISTS {prefix}_place_visits (
                id TEXT PRIMARY KEY,
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                -- The export's timestamp, kept whole: its date is what a planner
                -- asks about, and truncating here would throw away the time.
                visited_on TEXT,
                rating INTEGER CHECK (rating IS NULL OR (rating >= 1 AND rating <= 5)),
                source TEXT NOT NULL,
                external_ref TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_place_visits_place
                ON {prefix}_place_visits(place_id);

            CREATE TABLE IF NOT EXISTS {prefix}_climate_normals (
                place_id TEXT NOT NULL REFERENCES {prefix}_places(id),
                month INTEGER NOT NULL CHECK (month BETWEEN 1 AND 12),
                t_max_mean REAL,
                t_min_mean REAL,
                rain_days_mean REAL,
                precipitation_mm_mean REAL,
                daylight_hours_mean REAL,
                sunshine_hours_mean REAL,
                days_observed INTEGER NOT NULL,
                years_covered INTEGER NOT NULL,
                period_start TEXT NOT NULL,
                period_end TEXT NOT NULL,
                source TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                PRIMARY KEY (place_id, month)
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_climate_normals_fetched
                ON {prefix}_climate_normals(fetched_at);
            "
        ))?;
        Self::type_ids(conn, prefix)
    }

    /// Renames `place_<hex>`, `pp_<hex>` and `visit_<hex>` to `<kind>:<hex>` (libs/links/ISA.md D3,
    /// LNK-12). The hex is unchanged, so a backfill that re-derives an id lands on the renamed row.
    ///
    /// Four tables reference `places.id` and foreign keys are on for every connection
    /// (`libs/sjel-store`), so the check is deferred to the commit of the migration transaction:
    /// parent and children change together or not at all. Each statement matches only the old
    /// shape at its exact length, so a second run changes nothing.
    fn type_ids(conn: &Connection, prefix: &str) -> Fallible<()> {
        let place = "GLOB 'place_[0-9a-f]*' AND length({col}) = 22";
        let rename = |table: &str, col: &str, old: &str, kind: &str, guard: &str| {
            format!(
                "UPDATE {prefix}_{table} SET {col} = '{kind}:' || substr({col}, {cut}) \
                 WHERE {col} {guard};\n",
                cut = old.len() + 1,
                guard = guard.replace("{col}", col),
            )
        };
        let mut batch = String::from("PRAGMA defer_foreign_keys = ON;\n");
        batch += &rename("places", "id", "place_", "place", place);
        for table in [
            "geocode_cache",
            "transaction_places",
            "person_places",
            "climate_normals",
            "place_visits",
        ] {
            batch += &rename(table, "place_id", "place_", "place", place);
        }
        batch += &rename(
            "person_places",
            "id",
            "pp_",
            "pp",
            "GLOB 'pp_[0-9a-f]*' AND length({col}) = 19",
        );
        batch += &rename(
            "place_visits",
            "id",
            "visit_",
            "visit",
            "GLOB 'visit_[0-9a-f]*' AND length({col}) = 22",
        );
        conn.execute_batch(&batch)?;
        Ok(())
    }

    /// Insert a place, or recognise one already registered. Identity is the
    /// stable id (and the unique `external_ref` behind it), so every backfill
    /// re-run is a counted no-op. Returns whether a row was created.
    pub fn upsert_place(&self, place: &Place, today: &str) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_places
                    (id, name, kind, address, city, country_code, latitude, longitude,
                     source, external_ref, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &place.id,
                &place.name,
                &place.kind,
                &place.address,
                &place.city,
                &place.country_code,
                &place.latitude,
                &place.longitude,
                &place.source,
                &place.external_ref,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    /// Record one visit. Returns whether it was new.
    ///
    /// Idempotent by id, which the caller derives from `external_ref` -- for a
    /// Takeout review that is the Google Maps URL, the one identity two exports of
    /// the same review agree on. A re-import is therefore a no-op rather than a
    /// second visit.
    pub fn put_visit(&self, visit: &PlaceVisit, today: &str) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_place_visits
                    (id, place_id, visited_on, rating, source, external_ref, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &visit.id,
                &visit.place_id,
                &visit.visited_on,
                &visit.rating,
                &visit.source,
                &visit.external_ref,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    /// Every recorded visit, newest first, joined to the place it happened at.
    ///
    /// Joined rather than returned bare because a rating with no name is a number:
    /// the reader's question is always "where was this, and what did I think".
    pub fn visits(&self) -> Fallible<Vec<PlaceVisitRow>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT v.place_id, p.name, p.city, p.country_code, p.latitude, p.longitude,
                        v.visited_on, v.rating, v.source
                 FROM {prefix}_place_visits v
                 JOIN {prefix}_places p ON p.id = v.place_id
                 ORDER BY v.visited_on DESC, p.name"
            ),
            [],
            |row| {
                Ok(PlaceVisitRow {
                    place_id: row.get(0)?,
                    place_name: row.get(1)?,
                    city: row.get(2)?,
                    country_code: row.get(3)?,
                    latitude: row.get(4)?,
                    longitude: row.get(5)?,
                    visited_on: row.get(6)?,
                    rating: row.get(7)?,
                    source: row.get(8)?,
                })
            },
        )?)
    }

    pub fn place(&self, id: &str) -> Fallible<Option<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {prefix}_places WHERE id = ?1"
                ),
                params![&id],
                row_to_place,
            )
            .optional()?)
    }

    pub fn place_by_external_ref(&self, external_ref: &str) -> Fallible<Option<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT id, name, kind, address, city, country_code, latitude,
                            longitude, source, external_ref
                     FROM {prefix}_places WHERE external_ref = ?1"
                ),
                params![&external_ref],
                row_to_place,
            )
            .optional()?)
    }

    /// List/search the registry. Both filters optional; bounded so an
    /// unfiltered call stays an inventory rather than a dump.
    ///
    /// `LIKE` where Postgres had `ILIKE`: SQLite's LIKE ignores case already,
    /// but only over ASCII, so a needle typed with an upper-case umlaut misses
    /// a name spelled with a lower-case one.
    pub fn search_places(&self, q: Option<&str>, kind: Option<&str>) -> Fallible<Vec<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let pattern = q.map(|q| format!("%{q}%"));
        Ok(conn.query_all(
            &format!(
                "SELECT id, name, kind, address, city, country_code, latitude,
                        longitude, source, external_ref
                 FROM {prefix}_places
                 WHERE (?1 IS NULL OR name LIKE ?1 OR city LIKE ?1)
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY name, id
                 LIMIT 500"
            ),
            params![&pattern, &kind],
            row_to_place,
        )?)
    }

    /// Every place that carries a coordinate, for haversine matching in code —
    /// the no-PostGIS decision in `README.md` ("Deliberately not built").
    pub fn places_with_coordinates(&self) -> Fallible<Vec<Place>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, name, kind, address, city, country_code, latitude,
                        longitude, source, external_ref
                 FROM {prefix}_places
                 WHERE latitude IS NOT NULL AND longitude IS NOT NULL"
            ),
            [],
            row_to_place,
        )?)
    }

    /// The twelve stored months for one place, in calendar order. An empty
    /// vector means "never fetched", which the read routes report as such
    /// rather than as an empty grid.
    pub fn climate_get(&self, place_id: &str) -> Fallible<Vec<MonthlyNormal>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT month, t_max_mean, t_min_mean, rain_days_mean,
                        precipitation_mm_mean, daylight_hours_mean,
                        sunshine_hours_mean, days_observed
                 FROM {prefix}_climate_normals WHERE place_id = ?1
                 ORDER BY month"
            ),
            params![&place_id],
            row_to_normal,
        )?)
    }

    /// The window and the fetch stamp the stored months came from. `None` when
    /// the place has no normals, which is the same answer `climate_get` gives.
    pub fn climate_meta(&self, place_id: &str) -> Fallible<Option<NormalsMeta>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT years_covered, period_start, period_end, source, fetched_at
                     FROM {prefix}_climate_normals WHERE place_id = ?1
                     ORDER BY month LIMIT 1"
                ),
                params![&place_id],
                |row| {
                    Ok(NormalsMeta {
                        years_covered: row.get(0)?,
                        period_start: row.get(1)?,
                        period_end: row.get(2)?,
                        source: row.get(3)?,
                        fetched_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Replace a place's normals in one transaction: DELETE, then one INSERT per
    /// month the fold produced. One transaction because a refresh that failed
    /// halfway would otherwise leave a place with some months from 2015-2024 and
    /// some from an older window, which no column could tell apart afterwards.
    pub fn climate_put(
        &self,
        place_id: &str,
        months: &[MonthlyNormal],
        meta: &NormalsMeta,
    ) -> Fallible<usize> {
        let prefix = self.prefix.clone();
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            &format!("DELETE FROM {prefix}_climate_normals WHERE place_id = ?1"),
            params![&place_id],
        )?;
        {
            let mut insert = tx.prepare(&format!(
                "INSERT INTO {prefix}_climate_normals
                    (place_id, month, t_max_mean, t_min_mean, rain_days_mean,
                     precipitation_mm_mean, daylight_hours_mean, sunshine_hours_mean,
                     days_observed, years_covered, period_start, period_end,
                     source, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
            ))?;
            for month in months {
                insert.execute(params![
                    &place_id,
                    &month.month,
                    &month.t_max_mean,
                    &month.t_min_mean,
                    &month.rain_days_mean,
                    &month.precipitation_mm_mean,
                    &month.daylight_hours_mean,
                    &month.sunshine_hours_mean,
                    &month.days_observed,
                    &meta.years_covered,
                    &meta.period_start,
                    &meta.period_end,
                    &meta.source,
                    &meta.fetched_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(months.len())
    }

    /// Every place that carries a coordinate, paired with whether it has normals
    /// stored. The read routes resolve a bare coordinate against exactly this
    /// list, so "nearest place" means "nearest place that can actually answer".
    pub fn places_with_climate(&self) -> Fallible<Vec<(Place, bool)>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT p.id, p.name, p.kind, p.address, p.city, p.country_code,
                        p.latitude, p.longitude, p.source, p.external_ref,
                        EXISTS (SELECT 1 FROM {prefix}_climate_normals c
                                WHERE c.place_id = p.id) AS has_normals
                 FROM {prefix}_places p
                 WHERE p.latitude IS NOT NULL AND p.longitude IS NOT NULL"
            ),
            [],
            |row| Ok((row_to_place(row)?, row.get::<_, i64>(10)? == 1)),
        )?)
    }

    pub fn cache_get(&self, query_hash: &str) -> Fallible<Option<CacheEntry>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        // No `response::text`: the column is TEXT, so the cast the JSONB column
        // needed has nothing left to do.
        Ok(conn
            .query_row(
                &format!(
                    "SELECT status, response, place_id
                     FROM {prefix}_geocode_cache WHERE query_hash = ?1"
                ),
                params![&query_hash],
                |row| {
                    Ok(CacheEntry {
                        status: row.get(0)?,
                        response: row.get(1)?,
                        place_id: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// The cache is permanent: a hash already present is left untouched, so a
    /// second writer cannot turn a served answer back into an egress.
    #[allow(clippy::too_many_arguments)]
    pub fn cache_put(
        &self,
        query_hash: &str,
        provider: &str,
        query: &str,
        response: Option<&str>,
        place_id: Option<&str>,
        status: &str,
        fetched_at: &str,
    ) -> Fallible<()> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        // `$4::text::jsonb` loses its cast with the column type. That cast was
        // also the only thing rejecting a malformed body, so the caller's own
        // serializer is now the validator -- every writer holds a serde value.
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_geocode_cache
                    (query_hash, provider, query, response, place_id, status, fetched_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT (query_hash) DO NOTHING"
            ),
            params![
                &query_hash,
                &provider,
                &query,
                &response,
                &place_id,
                &status,
                &fetched_at,
            ],
        )?;
        Ok(())
    }

    /// Link a transaction to a place. Idempotent on `source_id`; a transaction
    /// already linked (at either precision) is left alone.
    pub fn link_transaction(
        &self,
        source_id: &str,
        place_id: &str,
        precision: &str,
        confidence_bp: i16,
        source: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_transaction_places
                    (source_id, place_id, precision, confidence_bp, source, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT (source_id) DO NOTHING"
            ),
            params![
                &source_id,
                &place_id,
                &precision,
                confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    /// The places linked to one finance transaction, with the link's precision (`venue` or
    /// `city`). `source_id` is finance's fingerprint, the same value finance's typed id
    /// `fin:tx:<source_id>` carries (libs/links/ISA.md D3).
    pub fn places_for_transaction(&self, source_id: &str) -> Fallible<Vec<(Place, String)>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT p.id, p.name, p.kind, p.address, p.city, p.country_code, p.latitude,
                        p.longitude, p.source, p.external_ref, t.precision
                 FROM {prefix}_transaction_places t
                 JOIN {prefix}_places p ON p.id = t.place_id
                 WHERE t.source_id = ?1"
            ),
            params![source_id],
            |row| Ok((row_to_place(row)?, row.get(10)?)),
        )?)
    }

    pub fn linked_source_ids(&self) -> Fallible<std::collections::HashSet<String>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn
            .query_all(
                &format!("SELECT source_id FROM {prefix}_transaction_places"),
                [],
                |row| row.get::<_, String>(0),
            )?
            .into_iter()
            .collect())
    }

    /// Write a register proposal. The state is the literal `'proposed'` and
    /// nothing the caller passes can change that — derivation never writes a
    /// confirmed row (README D4, ISA PLC-7).
    #[allow(clippy::too_many_arguments)]
    pub fn propose_person_place(
        &self,
        id: &str,
        person: &str,
        place_id: &str,
        date_start: Option<&str>,
        date_end: Option<&str>,
        confidence_bp: i16,
        source: &str,
        today: &str,
    ) -> Fallible<bool> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let inserted = conn.execute(
            &format!(
                "INSERT INTO {prefix}_person_places
                    (id, person, place_id, date_start, date_end, confidence_bp,
                     source, state, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'proposed',?8)
                 ON CONFLICT (id) DO NOTHING"
            ),
            params![
                &id,
                &person,
                &place_id,
                &date_start,
                &date_end,
                confidence_bp,
                &source,
                &today,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn person_places_in_state(&self, state: &str) -> Fallible<Vec<PersonPlaceRow>> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT id, person, place_id, date_start, date_end, confidence_bp,
                        source, state
                 FROM {prefix}_person_places
                 WHERE state = ?1
                 ORDER BY person, id"
            ),
            params![&state],
            |row| {
                Ok(PersonPlaceRow {
                    id: row.get(0)?,
                    person: row.get(1)?,
                    place_id: row.get(2)?,
                    date_start: row.get(3)?,
                    date_end: row.get(4)?,
                    confidence_bp: row.get(5)?,
                    source: row.get(6)?,
                    state: row.get(7)?,
                })
            },
        )?)
    }

    /// The one write path that can produce `state = 'confirmed'`, reached only
    /// from the explicit confirm/dismiss routes (ISA PLC-7).
    ///
    /// The guard is **asymmetric**, and that is the whole decision:
    ///
    /// | from | to | |
    /// |---|---|---|
    /// | `proposed` | `confirmed` | allowed |
    /// | `proposed` | `dismissed` | allowed |
    /// | `confirmed` | `dismissed` | allowed — the human can always withdraw |
    /// | `dismissed` | `confirmed` | **refused**, naming the state found |
    /// | any | itself | **refused** — a review that changes nothing |
    ///
    /// Before this, the UPDATE was unconditional on the id, so a stale id in an
    /// open tab or a replayed call turned a dismissed row back into a confirmed
    /// one and got a 200 for it. A *symmetric* `state = 'proposed'` guard would
    /// have been the other mistake: these two routes are the only writers of
    /// this column — there is no PATCH, no DELETE and no CLI arm — so it would
    /// have made a mis-clicked confirm permanent on the most sensitive store in
    /// the system, leaving hand-written SQL as the only repair, which is the
    /// write path Q73 forbids. PRD 2419-2421 prices it the other way round: "a
    /// wrong inference costs a dismissed proposal rather than a wrong trip".
    ///
    /// The UPDATE carries the state it read in its WHERE clause, so two
    /// concurrent reviews cannot both apply.
    pub fn review_person_place(
        &self,
        id: &str,
        review: Review,
        now: &str,
    ) -> Fallible<ReviewOutcome> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let Some(state): Option<String> = conn
            .query_row(
                &format!("SELECT state FROM {prefix}_person_places WHERE id = ?1"),
                params![&id],
                |row| row.get(0),
            )
            .optional()?
        else {
            return Ok(ReviewOutcome::NoSuchRow);
        };
        let allowed = matches!(
            (state.as_str(), review),
            ("proposed", _) | ("confirmed", Review::Dismissed)
        );
        if !allowed {
            return Ok(ReviewOutcome::Refused { state });
        }
        let changed = conn.execute(
            &format!(
                "UPDATE {prefix}_person_places
                 SET state = ?2, reviewed_at = ?3
                 WHERE id = ?1 AND state = ?4"
            ),
            params![&id, review.as_str(), &now, &state],
        )?;
        Ok(if changed == 1 {
            ReviewOutcome::Applied
        } else {
            // Another writer moved the row between the read and the write.
            ReviewOutcome::Refused { state }
        })
    }

    /// How many known companions are near a coordinate in a window, and for how
    /// many days — and nothing else.
    ///
    /// A §6.2 derived aggregate. NO person, NO place name, NO row id, NO
    /// confidence: this is the only shape in which the C2 register reaches a
    /// planner at all (PRD 2424-2429), and it is what lets `trips` hold the
    /// answer without ever holding the row.
    ///
    /// The radius is [`PRESENCE_RADIUS_KM`] and is NOT a caller parameter. How
    /// precisely places will answer about a person's location is places'
    /// disclosure policy, not a travel-matching rule — which is also why it
    /// does not collide with `DESTINATION_MATCH_RADIUS_KM = 75` in
    /// `dashboard/src/lib/travel/travel-candidates.ts`, a different question
    /// with its own single home.
    ///
    /// Confirmed rows only. A proposal is an inference nobody has agreed with,
    /// so it is not evidence that anyone is anywhere (ISA PLC-7).
    ///
    /// Overlap rule: a row with no `date_start` and no `date_end` means "lives
    /// there" and matches any window; one open end is open in that direction.
    pub fn confirmed_presence(
        &self,
        latitude: f64,
        longitude: f64,
        from: &str,
        to: &str,
    ) -> Fallible<Presence> {
        let prefix = &self.prefix;
        let conn = self.conn()?;
        let rows: Vec<RegisterSpan> = conn.query_all(
            &format!(
                "SELECT pp.person, pp.date_start, pp.date_end, pl.latitude, pl.longitude
                     FROM {prefix}_person_places pp
                     JOIN {prefix}_places pl ON pl.id = pp.place_id
                     WHERE pp.state = 'confirmed'
                       AND (pp.date_start IS NULL OR pp.date_start <= ?1)
                       AND (pp.date_end IS NULL OR pp.date_end >= ?2)"
            ),
            params![&to, &from],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;

        let (Some(window_start), Some(window_end)) =
            (crate::days_from_civil(from), crate::days_from_civil(to))
        else {
            return Err("from and to must be YYYY-MM-DD".into());
        };
        let span = (window_end - window_start + 1).max(0) as usize;
        let mut covered = vec![false; span];
        let mut people: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for (person, date_start, date_end, row_latitude, row_longitude) in rows {
            let (Some(row_latitude), Some(row_longitude)) = (row_latitude, row_longitude) else {
                // A confirmed relation to a place nobody has geocoded is not an
                // error; it simply cannot be measured against a coordinate.
                continue;
            };
            let distance =
                crate::backfill::haversine_km((latitude, longitude), (row_latitude, row_longitude));
            if distance > PRESENCE_RADIUS_KM {
                continue;
            }
            // The person set is counted, never returned. `BTreeSet` because two
            // rows for one person in one window are one companion.
            people.insert(person);
            let start = date_start
                .as_deref()
                .and_then(crate::days_from_civil)
                .unwrap_or(window_start)
                .max(window_start);
            let end = date_end
                .as_deref()
                .and_then(crate::days_from_civil)
                .unwrap_or(window_end)
                .min(window_end);
            for day in start..=end {
                if let Some(slot) = covered.get_mut((day - window_start) as usize) {
                    *slot = true;
                }
            }
        }
        Ok(Presence {
            known_companions: people.len() as u32,
            overlap_days: covered.iter().filter(|day| **day).count() as u32,
        })
    }
}

/// One confirmed register row, reduced to what the aggregate needs:
/// `(person, date_start, date_end, latitude, longitude)`. The person is counted
/// and never returned.
type RegisterSpan = (
    String,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<f64>,
);

/// How precisely places is willing to answer about a person's location.
///
/// A places constant, echoed in the response, and deliberately not a caller
/// parameter: a caller-chosen radius is a free parameter in a disclosure route.
pub const PRESENCE_RADIUS_KM: f64 = 50.0;

/// What a review actually did. Three outcomes, because "nothing happened" and
/// "no such row" are different answers and a caller acts on them differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    Applied,
    NoSuchRow,
    /// The transition is not allowed. Carries the state found, so the caller
    /// can say which one rather than claiming the row does not exist.
    Refused {
        state: String,
    },
}

/// The whole answer of the presence read: a count and an overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Presence {
    pub known_companions: u32,
    pub overlap_days: u32,
}

fn row_to_place(row: &Row) -> rusqlite::Result<Place> {
    Ok(Place {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        address: row.get(3)?,
        city: row.get(4)?,
        country_code: row.get(5)?,
        latitude: row.get(6)?,
        longitude: row.get(7)?,
        source: row.get(8)?,
        external_ref: row.get(9)?,
    })
}

fn row_to_normal(row: &Row) -> rusqlite::Result<MonthlyNormal> {
    Ok(MonthlyNormal {
        month: row.get(0)?,
        t_max_mean: row.get(1)?,
        t_min_mean: row.get(2)?,
        rain_days_mean: row.get(3)?,
        precipitation_mm_mean: row.get(4)?,
        daylight_hours_mean: row.get(5)?,
        sunshine_hours_mean: row.get(6)?,
        days_observed: row.get(7)?,
    })
}

/// A stable id from a stable source identity, so a re-run of any backfill on
/// any machine lands on the row it made before. Same purpose as finance's
/// path-derived subscription ids.
///
/// Typed `<kind>:<hex>` since 2026-10-06 (libs/links/ISA.md D3); it was `<kind>_<hex>` with the
/// same hex, and `run_migration` renames the stored ones.
pub fn stable_id(prefix: &str, identity: &str) -> String {
    sjel_links::stable_id(prefix, identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_prefix_that_could_carry_sql_is_refused() {
        assert!(validate_prefix("places").is_ok());
        assert!(validate_prefix("places_test_123").is_ok());
        assert!(validate_prefix("places; DROP TABLE places_places").is_err());
        assert!(validate_prefix("Places").is_err());
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn stable_ids_are_stable_and_distinct() {
        assert_eq!(
            stable_id("place", "eva:8000207"),
            stable_id("place", "eva:8000207")
        );
        assert_ne!(
            stable_id("place", "eva:8000207"),
            stable_id("place", "eva:1234567")
        );
        assert!(stable_id("place", "eva:8000207").starts_with("place:"));
    }
}

/// Database-backed tests, a temp file per test. `db_tests` is the one selector every
/// database-backed module in the workspace is named by: CI's hermetic job runs
/// `--skip db_tests::` and its store job runs `db_tests::`, so the module name IS the
/// suite membership. It was `pg_tests` until 2026-08-25 (PRD Q44) and `postgres_tests`
/// until PRD Q45 — the suite needs a temp file now, not a server.
#[cfg(test)]
pub(crate) mod db_tests {
    use super::*;

    /// A file per test, in a directory this process owns. It replaces the schema-drop
    /// guard: that guard existed because a leaked test schema reached a real
    /// `pg_dumpall`, and a temp file is neither shared nor backed up.
    pub(crate) fn test_database(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("places-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{name}.db"));
        // The pid is recycled eventually; the file starts empty, which is what the
        // old DROP SCHEMA was buying.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The store plus the path it opened, because the cross-capability readers in
    /// `layers.rs`/`backfill.rs` need to create neighbour tables in the same file.
    pub(crate) fn open_test_store(name: &str) -> (PlacesStore, std::path::PathBuf) {
        let path = test_database(name);
        let store = PlacesStore::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()));
        (store, path)
    }

    fn a_venue(id: &str, name: &str) -> Place {
        Place {
            id: id.into(),
            name: name.into(),
            kind: "venue".into(),
            address: None,
            city: Some("Bielefeld".into()),
            country_code: Some("DE".into()),
            latitude: Some(52.0),
            longitude: Some(8.5),
            source: "takeout".into(),
            external_ref: Some(format!("takeout:review:{id}")),
        }
    }

    #[test]
    fn old_ids_are_renamed_once_with_their_references() {
        let (store, _path) = open_test_store("type_ids");
        let old = "place_32b1af1b5a9b8869";
        let mut conn = store.conn().unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO places_places (id, name, kind, source, created_at)
                 VALUES ('{old}', 'Bonn Hbf', 'station', 'test', '2026-10-06');
             INSERT INTO places_transaction_places
                 (source_id, place_id, precision, confidence_bp, source, created_at)
                 VALUES ('abc', '{old}', 'city', 9000, 'test', '2026-10-06');
             INSERT INTO places_place_visits (id, place_id, source, created_at)
                 VALUES ('visit_0123456789abcdef', '{old}', 'test', '2026-10-06');"
        ))
        .unwrap();

        for _ in 0..2 {
            let tx = conn.transaction().unwrap();
            PlacesStore::type_ids(&tx, "places").unwrap();
            tx.commit().unwrap();
        }

        let new = "place:32b1af1b5a9b8869";
        assert_eq!(new, stable_id("place", "eva:8000207"));
        let one = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(one("SELECT id FROM places_places"), new);
        assert_eq!(one("SELECT place_id FROM places_transaction_places"), new);
        assert_eq!(
            one("SELECT id FROM places_place_visits"),
            "visit:0123456789abcdef"
        );
        let broken: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
    }

    #[test]
    fn a_visit_is_recorded_once_and_reads_back_with_its_place() {
        // The distinction the table exists for: a place exists, a visit says the
        // operator was there. Recording the same review twice must not produce two
        // visits, because a re-import is the normal case.
        let (store, _path) = open_test_store("visits");
        let place = a_venue("place:test-cafe", "Test Café");
        assert!(store.upsert_place(&place, "2026-09-23").unwrap());

        let visit = PlaceVisit {
            id: "visit:1".into(),
            place_id: place.id.clone(),
            visited_on: Some("2025-10-31T19:02:16.086083Z".into()),
            rating: Some(5),
            source: "takeout-review".into(),
            external_ref: place.external_ref.clone(),
        };
        assert!(
            store.put_visit(&visit, "2026-09-23").unwrap(),
            "first write is new"
        );
        assert!(
            !store.put_visit(&visit, "2026-09-23").unwrap(),
            "a re-import is not a second visit"
        );

        let rows = store.visits().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].place_name, "Test Café",
            "joined, so a rating has a name"
        );
        assert_eq!(rows[0].rating, Some(5));
        assert_eq!(rows[0].city.as_deref(), Some("Bielefeld"));
        assert_eq!(
            rows[0].visited_on.as_deref(),
            Some("2025-10-31T19:02:16.086083Z"),
            "the export's timestamp is kept whole"
        );
    }

    #[test]
    fn a_visit_with_no_rating_or_date_is_still_a_visit() {
        // Both fields are optional in the export, and a visit with neither is still
        // the fact that the operator was there.
        let (store, _path) = open_test_store("visits-bare");
        let place = a_venue("place:bare", "Somewhere");
        store.upsert_place(&place, "2026-09-23").unwrap();
        store
            .put_visit(
                &PlaceVisit {
                    id: "visit:bare".into(),
                    place_id: place.id.clone(),
                    visited_on: None,
                    rating: None,
                    source: "takeout-review".into(),
                    external_ref: None,
                },
                "2026-09-23",
            )
            .unwrap();
        let rows = store.visits().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rating, None);
        assert_eq!(rows[0].visited_on, None);
    }

    #[test]
    fn a_rating_outside_one_to_five_is_refused_by_the_database() {
        // The CHECK is the guard: a bad rating is a data error, and a silent 0
        // would read as "rated it nothing" rather than as a bug.
        let (store, _path) = open_test_store("visits-range");
        let place = a_venue("place:range", "Rated");
        store.upsert_place(&place, "2026-09-23").unwrap();
        for bad in [0_i64, 6] {
            let refused = store.put_visit(
                &PlaceVisit {
                    id: format!("visit:range:{bad}"),
                    place_id: place.id.clone(),
                    visited_on: None,
                    rating: Some(bad),
                    source: "takeout-review".into(),
                    external_ref: None,
                },
                "2026-09-23",
            );
            assert!(refused.is_err(), "rating {bad} must be refused");
        }
    }

    #[test]
    fn ddl_is_idempotent_and_the_store_answers_its_own_ping() {
        let (store, path) = open_test_store("ddl");
        store.ping().expect("a live store answers its own ping");
        // A second migration over the same prefix must be a no-op, not an error.
        let conn = Connection::open(&path).unwrap();
        PlacesStore::run_migration(&conn, store.prefix()).expect("re-running DDL is a no-op");
    }

    /// The readiness handler turns exactly this into a 503, where the stateless
    /// liveness handler answers 200 (#126). It replaces "port 1 is unreachable":
    /// there is no port any more, so an unusable path is the failure a deployment
    /// can actually have.
    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        let blocker = std::env::temp_dir().join(format!("places-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            PlacesStore::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    /// The cache is permanent by design (README D3), and its `response` column
    /// carried JSON that Postgres validated as JSONB. TEXT does not validate, so
    /// the round trip is pinned here instead.
    #[test]
    fn the_geocode_cache_is_write_once_and_returns_its_json_body() {
        let (store, _path) = open_test_store("cache");
        assert!(store.cache_get("h1").unwrap().is_none());

        store
            .cache_put(
                "h1",
                "nominatim",
                "Bonn",
                Some(r#"{"lat":"50.7"}"#),
                None,
                "hit",
                "2026-08-28",
            )
            .unwrap();
        // A second write must not turn a served answer back into an egress.
        store
            .cache_put(
                "h1",
                "nominatim",
                "Bonn",
                Some(r#"{"lat":"0"}"#),
                None,
                "miss",
                "2026-08-28",
            )
            .unwrap();

        let entry = store.cache_get("h1").unwrap().unwrap();
        assert_eq!(entry.status, "hit");
        let body: serde_json::Value =
            serde_json::from_str(entry.response.as_deref().unwrap()).unwrap();
        assert_eq!(body["lat"], "50.7");
    }

    #[test]
    fn places_and_links_are_idempotent_by_stable_identity() {
        let (store, _path) = open_test_store("idem");
        let place = Place {
            id: stable_id("place", "eva:8000001"),
            name: "Synthetic Hbf".into(),
            kind: "station".into(),
            address: None,
            city: None,
            country_code: Some("DE".into()),
            latitude: Some(50.0),
            longitude: Some(7.0),
            source: "test".into(),
            external_ref: Some("eva:8000001".into()),
        };
        assert!(store.upsert_place(&place, "2026-08-25").unwrap());
        assert!(!store.upsert_place(&place, "2026-08-25").unwrap());
        assert_eq!(
            store
                .place_by_external_ref("eva:8000001")
                .unwrap()
                .unwrap()
                .id,
            place.id
        );
        assert!(store
            .link_transaction("fp-1", &place.id, "venue", 9000, "test", "2026-08-25")
            .unwrap());
        assert!(!store
            .link_transaction("fp-1", &place.id, "city", 6000, "test", "2026-08-25")
            .unwrap());
        assert_eq!(store.linked_source_ids().unwrap().len(), 1);
    }

    #[test]
    fn a_proposal_is_born_proposed_and_only_review_confirms_it() {
        let (store, _path) = open_test_store("register");
        let place = Place {
            id: stable_id("place", "test:city"),
            name: "Synthetic City".into(),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: None,
            latitude: Some(48.0),
            longitude: Some(16.0),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&place, "2026-08-25").unwrap();
        assert!(store
            .propose_person_place(
                "pp_test",
                "Synthetic Person",
                &place.id,
                None,
                None,
                5000,
                "vault-frontmatter",
                "2026-08-25",
            )
            .unwrap());
        // Idempotent by id.
        assert!(!store
            .propose_person_place(
                "pp_test",
                "Synthetic Person",
                &place.id,
                None,
                None,
                5000,
                "vault-frontmatter",
                "2026-08-25",
            )
            .unwrap());
        let proposed = store.person_places_in_state("proposed").unwrap();
        assert_eq!(proposed.len(), 1);
        assert_eq!(proposed[0].state, "proposed");

        assert_eq!(
            store
                .review_person_place("pp_test", Review::Confirmed, "2026-08-25")
                .unwrap(),
            ReviewOutcome::Applied
        );
        assert_eq!(store.person_places_in_state("confirmed").unwrap().len(), 1);
        assert_eq!(
            store
                .review_person_place("pp_missing", Review::Dismissed, "2026-08-25")
                .unwrap(),
            ReviewOutcome::NoSuchRow
        );
    }

    /// Set up one confirmed row for a synthetic person at a synthetic place.
    fn a_register_row(
        store: &PlacesStore,
        id: &str,
        latitude: f64,
        longitude: f64,
        date_start: Option<&str>,
        date_end: Option<&str>,
    ) -> String {
        let place = Place {
            id: stable_id("place", id),
            name: format!("Synthetic City {id}"),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: None,
            latitude: Some(latitude),
            longitude: Some(longitude),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&place, "2026-08-25").unwrap();
        store
            .propose_person_place(
                id,
                &format!("Synthetic Person {id}"),
                &place.id,
                date_start,
                date_end,
                5000,
                "test",
                "2026-08-25",
            )
            .unwrap();
        store
            .review_person_place(id, Review::Confirmed, "2026-08-25")
            .unwrap();
        place.id
    }

    /// A dismissal is final. Without this guard a stale id in an open tab, or a
    /// replayed call, turned a dismissed row back into a confirmed one and got
    /// a 200 for it.
    #[test]
    fn a_dismissed_row_cannot_be_confirmed() {
        let (store, _path) = open_test_store("review_dismissed");
        a_register_row(&store, "pp_a", 50.0, 8.0, None, None);
        assert_eq!(
            store
                .review_person_place("pp_a", Review::Dismissed, "2026-08-26")
                .unwrap(),
            ReviewOutcome::Applied
        );
        assert_eq!(
            store
                .review_person_place("pp_a", Review::Confirmed, "2026-08-27")
                .unwrap(),
            ReviewOutcome::Refused {
                state: "dismissed".into()
            },
            "a dismissed row must not be confirmable, and the caller must be told which state it found"
        );
        assert_eq!(store.person_places_in_state("dismissed").unwrap().len(), 1);
        assert!(store
            .person_places_in_state("confirmed")
            .unwrap()
            .is_empty());
    }

    /// The half a symmetric `state = 'proposed'` guard would have broken: these
    /// two routes are the only writers of this column, so a mis-clicked confirm
    /// must stay withdrawable or hand-written SQL becomes the only repair.
    #[test]
    fn a_confirmed_row_can_still_be_dismissed() {
        let (store, _path) = open_test_store("review_withdraw");
        a_register_row(&store, "pp_b", 50.0, 8.0, None, None);
        assert_eq!(
            store
                .review_person_place("pp_b", Review::Dismissed, "2026-08-26")
                .unwrap(),
            ReviewOutcome::Applied
        );
        // And a review that would change nothing is refused rather than
        // silently succeeding.
        assert_eq!(
            store
                .review_person_place("pp_b", Review::Dismissed, "2026-08-27")
                .unwrap(),
            ReviewOutcome::Refused {
                state: "dismissed".into()
            }
        );
    }

    /// The presence read answers a count and an overlap. What it must NOT
    /// answer is checked as text against the serialised body, because the
    /// failure this guards against is a field added later without thinking.
    #[test]
    fn presence_answers_a_count_and_no_identity() {
        let (store, _path) = open_test_store("presence_identity");
        a_register_row(
            &store,
            "pp_near",
            50.0,
            8.0,
            Some("2026-10-03"),
            Some("2026-10-07"),
        );
        // Far away: about 550 km north, well outside the 50 km radius.
        a_register_row(
            &store,
            "pp_far",
            55.0,
            8.0,
            Some("2026-10-03"),
            Some("2026-10-07"),
        );

        let presence = store
            .confirmed_presence(50.0, 8.0, "2026-10-01", "2026-10-10")
            .expect("the presence query is valid");
        assert_eq!(presence.known_companions, 1, "only the nearby row counts");
        assert_eq!(presence.overlap_days, 5);

        let body = serde_json::to_string(&presence).unwrap();
        for forbidden in [
            "Synthetic Person",
            "Synthetic City",
            "pp_near",
            "confidence",
        ] {
            assert!(
                !body.contains(forbidden),
                "the presence body carried {forbidden}: {body}"
            );
        }
    }

    /// Both dates null means "lives there", which overlaps any window. A
    /// proposal is not evidence, so it never counts.
    #[test]
    fn an_open_ended_register_row_overlaps_any_window() {
        let (store, _path) = open_test_store("presence_open");
        a_register_row(&store, "pp_resident", 50.0, 8.0, None, None);
        let presence = store
            .confirmed_presence(50.0, 8.0, "2027-05-01", "2027-05-04")
            .expect("the presence query is valid");
        assert_eq!(presence.known_companions, 1);
        assert_eq!(presence.overlap_days, 4, "the whole window is covered");

        // One open end is open in that direction only.
        a_register_row(&store, "pp_since", 50.0, 8.0, Some("2027-05-03"), None);
        let later = store
            .confirmed_presence(50.0, 8.0, "2027-05-01", "2027-05-04")
            .expect("the presence query is valid");
        assert_eq!(later.known_companions, 2);

        // A proposal is an inference nobody agreed with, so it is not evidence.
        let (fresh, _path) = open_test_store("presence_proposed");
        let place = Place {
            id: stable_id("place", "proposed-city"),
            name: "Synthetic City".into(),
            kind: "city".into(),
            address: None,
            city: None,
            country_code: None,
            latitude: Some(50.0),
            longitude: Some(8.0),
            source: "test".into(),
            external_ref: None,
        };
        fresh.upsert_place(&place, "2026-08-25").unwrap();
        fresh
            .propose_person_place(
                "pp_p",
                "Synthetic Person",
                &place.id,
                None,
                None,
                9000,
                "test",
                "2026-08-25",
            )
            .unwrap();
        assert_eq!(
            fresh
                .confirmed_presence(50.0, 8.0, "2027-05-01", "2027-05-04")
                .unwrap()
                .known_companions,
            0
        );
    }
}
