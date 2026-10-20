use std::error::Error;
use std::path::Path;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Ledger {
    pool: sjel_store::Pool,
}

fn ingest_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IngestRow> {
    Ok(IngestRow {
        id: row.get(0)?,
        source: row.get(1)?,
        staging: row.get(2)?,
        started: row.get(3)?,
        finished: row.get(4)?,
        considered: row.get(5)?,
        imported: row.get(6)?,
        duplicates: row.get(7)?,
        refused: row.get(8)?,
        failed: row.get(9)?,
        resumed: row.get(10)?,
        verified: row.get(11)?,
    })
}

pub struct Outcome<'a> {
    pub kind: &'static str,
    pub reason: &'a str,
    pub imported: Option<&'a str>,
    pub verified: bool,
}

impl<'a> Outcome<'a> {
    pub fn new(
        kind: &'static str,
        reason: &'a str,
        imported: Option<&'a str>,
        verified: bool,
    ) -> Self {
        Self {
            kind,
            reason,
            imported,
            verified,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Location {
    pub uuid: String,
    pub relpath: String,
    pub digest: String,
    pub size: i64,
    pub mtime_ns: i64,
}

/// One mounted volume as the ledger knows it: what it is called, how many of its paths are
/// indexed, how many bytes those are, and when a verb last saw it.
#[derive(Debug, Serialize)]
pub struct VolumeRow {
    pub uuid: String,
    pub label: String,
    pub locations: i64,
    pub bytes: i64,
    pub last_seen: String,
}

/// One ingest run's counts. `finished` is null while a run is still going.
#[derive(Debug, Serialize)]
pub struct IngestRow {
    pub id: i64,
    pub source: String,
    pub staging: String,
    pub started: String,
    pub finished: Option<String>,
    pub considered: i64,
    pub imported: i64,
    pub duplicates: i64,
    pub refused: i64,
    pub failed: i64,
    pub resumed: i64,
    pub verified: i64,
}

/// One path's disposition inside a run. These are the rows the counts above are made of; a
/// refusal names the path and the reason, which the counts alone cannot.
#[derive(Debug, Serialize)]
pub struct ItemRow {
    pub relpath: String,
    pub digest: Option<String>,
    pub size: Option<i64>,
    pub disposition: String,
    pub reason: String,
    pub imported_relpath: Option<String>,
    pub verified: bool,
}

/// One digest's perceptual hash in the advisory perceptual layer (`F4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhashRow {
    pub digest: String,
    pub phash: String,
    pub algorithm: String,
    pub computed_at: String,
}

/// One operator review decision over a pair of distinct digests (`F4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewRow {
    pub left_digest: String,
    pub right_digest: String,
    pub decision: String,
    pub note: String,
    pub decided_at: String,
}

/// One digest's embedded GPS coordinates and capture day (PRD N4: index and link, never copy).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeotagRow {
    pub digest: String,
    pub latitude: f64,
    pub longitude: f64,
    pub captured_at: Option<String>,
    pub indexed_at: String,
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Self> {
        let pool = sjel_store::open_pool(path, "media", |conn| {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS media_volumes (
                    uuid TEXT PRIMARY KEY, label TEXT NOT NULL,
                    first_seen TEXT NOT NULL DEFAULT ({now}), last_seen TEXT NOT NULL DEFAULT ({now})
                );
                CREATE TABLE IF NOT EXISTS media_files (
                    digest TEXT PRIMARY KEY, size INTEGER NOT NULL CHECK(size >= 0),
                    first_seen TEXT NOT NULL DEFAULT ({now})
                );
                CREATE TABLE IF NOT EXISTS media_locations (
                    uuid TEXT NOT NULL REFERENCES media_volumes(uuid),
                    relpath TEXT NOT NULL, digest TEXT NOT NULL REFERENCES media_files(digest),
                    size INTEGER NOT NULL, mtime_ns INTEGER NOT NULL,
                    first_seen TEXT NOT NULL DEFAULT ({now}), last_seen TEXT NOT NULL DEFAULT ({now}),
                    PRIMARY KEY (uuid, relpath)
                );
                CREATE INDEX IF NOT EXISTS media_locations_digest ON media_locations(digest, uuid);
                CREATE TABLE IF NOT EXISTS media_ingests (
                    id INTEGER PRIMARY KEY, source TEXT NOT NULL, staging TEXT NOT NULL,
                    manifest_json TEXT NOT NULL, started TEXT NOT NULL DEFAULT ({now}),
                    finished TEXT, considered INTEGER NOT NULL DEFAULT 0,
                    imported INTEGER NOT NULL DEFAULT 0, duplicates INTEGER NOT NULL DEFAULT 0,
                    refused INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0,
                    resumed INTEGER NOT NULL DEFAULT 0, verified INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS media_ingest_items (
                    ingest_id INTEGER NOT NULL REFERENCES media_ingests(id),
                    relpath TEXT NOT NULL, digest TEXT, size INTEGER,
                    disposition TEXT NOT NULL CHECK(disposition IN ('duplicate','imported','refused','failed','resumed')),
                    reason TEXT NOT NULL, imported_relpath TEXT,
                    verified INTEGER NOT NULL DEFAULT 0 CHECK(verified IN (0,1)),
                    PRIMARY KEY (ingest_id, relpath)
                );
                CREATE INDEX IF NOT EXISTS media_ingest_history ON media_ingests(staging, id);
                CREATE TABLE IF NOT EXISTS media_ingest_copies (
                    ingest_id INTEGER NOT NULL REFERENCES media_ingests(id),
                    relpath TEXT NOT NULL, uuid TEXT NOT NULL, library TEXT NOT NULL,
                    incoming_name TEXT NOT NULL UNIQUE,
                    PRIMARY KEY (ingest_id, relpath)
                );
                CREATE TABLE IF NOT EXISTS media_phashes (
                    digest TEXT PRIMARY KEY REFERENCES media_files(digest),
                    phash TEXT NOT NULL,
                    algorithm TEXT NOT NULL DEFAULT 'dhash64',
                    computed_at TEXT NOT NULL DEFAULT ({now})
                );
                CREATE TABLE IF NOT EXISTS media_reviews (
                    left_digest TEXT NOT NULL,
                    right_digest TEXT NOT NULL,
                    decision TEXT NOT NULL CHECK(decision IN ('near_duplicate','distinct','skip')),
                    note TEXT NOT NULL DEFAULT '',
                    decided_at TEXT NOT NULL DEFAULT ({now}),
                    PRIMARY KEY (left_digest, right_digest),
                    CHECK(left_digest < right_digest)
                );
                CREATE TABLE IF NOT EXISTS media_geotags (
                    digest TEXT PRIMARY KEY REFERENCES media_files(digest),
                    latitude REAL NOT NULL CHECK(latitude BETWEEN -90.0 AND 90.0),
                    longitude REAL NOT NULL CHECK(longitude BETWEEN -180.0 AND 180.0),
                    captured_at TEXT,
                    indexed_at TEXT NOT NULL DEFAULT ({now})
                );",
                now = sjel_store::NOW
            ))?;
            Ok(())
        })?;
        Ok(Self { pool })
    }

    pub fn register(&self, uuid: &str, label: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            &format!(
                "INSERT INTO media_volumes(uuid,label) VALUES(?1,?2)
                ON CONFLICT(uuid) DO UPDATE SET label=excluded.label,last_seen={}",
                sjel_store::NOW
            ),
            params![uuid, label],
        )?;
        Ok(())
    }

    pub fn location(&self, uuid: &str, relpath: &str) -> Result<Option<Location>> {
        let conn = self.pool.get()?;
        Ok(conn.query_row(
            "SELECT uuid,relpath,digest,size,mtime_ns FROM media_locations WHERE uuid=?1 AND relpath=?2",
            params![uuid, relpath],
            |row| Ok(Location { uuid: row.get(0)?, relpath: row.get(1)?, digest: row.get(2)?, size: row.get(3)?, mtime_ns: row.get(4)? }),
        ).optional()?)
    }

    /// Drop one recorded location, and with it any file row left with no location at all.
    ///
    /// This exists for a path that was recorded and should never have been: macOS writes
    /// `.DS_Store` and AppleDouble `._*` beside the media on its own schedule, so an index that
    /// admits them starts refusing on a file the operator never put there. Removing a location is
    /// not how a deleted media file is handled — that stays visible as an unindexed absence, which
    /// is the whole point of the audit.
    pub fn forget(&self, uuid: &str, relpath: &str) -> Result<bool> {
        let mut conn = self.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let removed = tx.execute(
            "DELETE FROM media_locations WHERE uuid=?1 AND relpath=?2",
            params![uuid, relpath],
        )?;
        if removed > 0 {
            tx.execute(
                "DELETE FROM media_files WHERE digest NOT IN (SELECT digest FROM media_locations)",
                [],
            )?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }

    pub fn has_digest(&self, digest: &str) -> Result<bool> {
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM media_files WHERE digest=?1",
                [digest],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn locations(&self, uuid: &str) -> Result<Vec<Location>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT uuid,relpath,digest,size,mtime_ns FROM media_locations WHERE uuid=?1 ORDER BY relpath")?;
        let rows = stmt.query_map([uuid], |row| {
            Ok(Location {
                uuid: row.get(0)?,
                relpath: row.get(1)?,
                digest: row.get(2)?,
                size: row.get(3)?,
                mtime_ns: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn counts(&self) -> Result<(i64, i64)> {
        let conn = self.pool.get()?;
        Ok(conn.query_row(
            "SELECT (SELECT count(*) FROM media_files), (SELECT count(*) FROM media_locations)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    pub fn volumes(&self) -> Result<Vec<VolumeRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT v.uuid, v.label, v.last_seen,
                    (SELECT count(*) FROM media_locations l WHERE l.uuid = v.uuid),
                    (SELECT coalesce(sum(size),0) FROM media_locations l WHERE l.uuid = v.uuid)
             FROM media_volumes v ORDER BY v.label, v.uuid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(VolumeRow {
                uuid: row.get(0)?,
                label: row.get(1)?,
                last_seen: row.get(2)?,
                locations: row.get(3)?,
                bytes: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn recent_ingests(&self, limit: i64) -> Result<Vec<IngestRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, staging, started, finished, considered, imported, duplicates,
                    refused, failed, resumed, verified
             FROM media_ingests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], ingest_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn ingest(&self, id: i64) -> Result<Option<IngestRow>> {
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                "SELECT id, source, staging, started, finished, considered, imported, duplicates,
                        refused, failed, resumed, verified
                 FROM media_ingests WHERE id = ?1",
                [id],
                ingest_row,
            )
            .optional()?)
    }

    pub fn ingest_items(&self, id: i64) -> Result<Vec<ItemRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT relpath, digest, size, disposition, reason, imported_relpath, verified
             FROM media_ingest_items WHERE ingest_id = ?1 ORDER BY disposition, relpath",
        )?;
        let rows = stmt.query_map([id], |row| {
            Ok(ItemRow {
                relpath: row.get(0)?,
                digest: row.get(1)?,
                size: row.get(2)?,
                disposition: row.get(3)?,
                reason: row.get(4)?,
                imported_relpath: row.get(5)?,
                verified: row.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // A file and its location commit together; a crashed indexer cannot leave an orphaned digest.
    pub fn record(&self, loc: &Location) -> Result<()> {
        let mut conn = self.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            "INSERT OR IGNORE INTO media_files(digest,size) VALUES(?1,?2)",
            params![loc.digest, loc.size],
        )?;
        let stored_size: i64 = tx.query_row(
            "SELECT size FROM media_files WHERE digest=?1",
            [&loc.digest],
            |r| r.get(0),
        )?;
        if stored_size != loc.size {
            return Err(format!("digest {} has conflicting sizes", loc.digest).into());
        }
        tx.execute(
            &format!(
                "INSERT INTO media_locations(uuid,relpath,digest,size,mtime_ns)
                VALUES(?1,?2,?3,?4,?5) ON CONFLICT(uuid,relpath) DO UPDATE SET
                digest=excluded.digest,size=excluded.size,mtime_ns=excluded.mtime_ns,last_seen={}",
                sjel_store::NOW
            ),
            params![loc.uuid, loc.relpath, loc.digest, loc.size, loc.mtime_ns],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn start_ingest(&self, source: &str, staging: &str, manifest_json: &str) -> Result<i64> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO media_ingests(source,staging,manifest_json) VALUES(?1,?2,?3)",
            params![source, staging, manifest_json],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Records the incoming directory owned by one staged path and target.
    ///
    /// # Errors
    /// Returns an error if the connection fails or the ownership row violates a constraint.
    pub fn own_incoming(
        &self,
        id: i64,
        relpath: &str,
        uuid: &str,
        library: &str,
        name: &str,
    ) -> Result<()> {
        self.pool.get()?.execute(
            "INSERT INTO media_ingest_copies(ingest_id,relpath,uuid,library,incoming_name) VALUES(?1,?2,?3,?4,?5)",
            params![id, relpath, uuid, library, name],
        )?;
        Ok(())
    }

    /// Lists owned incoming names, source paths, and digests for this staging/target pair.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read or its rows cannot be decoded.
    pub fn incoming_copies(
        &self,
        staging: &str,
        uuid: &str,
        library: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT c.incoming_name,c.relpath,i.digest FROM media_ingest_copies c
             JOIN media_ingests r ON r.id=c.ingest_id
             JOIN media_ingest_items i ON i.ingest_id=c.ingest_id AND i.relpath=c.relpath
             WHERE r.staging=?1 AND c.uuid=?2 AND c.library=?3",
        )?;
        let rows = stmt
            .query_map(params![staging, uuid, library], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn prior_verified(
        &self,
        staging: &str,
        uuid: &str,
        library: &str,
        relpath: &str,
        digest: &str,
    ) -> Result<Option<String>> {
        let conn = self.pool.get()?;
        Ok(conn.query_row("SELECT i.imported_relpath FROM media_ingest_items i
            JOIN media_ingests r ON r.id=i.ingest_id WHERE r.staging=?1 AND i.relpath=?2 AND i.digest=?3
            AND i.verified=1 AND i.disposition IN ('imported','resumed')
            AND EXISTS (SELECT 1 FROM media_ingest_copies c WHERE c.ingest_id=i.ingest_id AND c.relpath=i.relpath AND c.uuid=?4 AND c.library=?5) ORDER BY r.id DESC LIMIT 1",
            params![staging, relpath, digest, uuid, library], |r| r.get(0)).optional()?)
    }

    pub fn pending_paths(
        &self,
        staging: &str,
        uuid: &str,
        library: &str,
    ) -> Result<std::collections::BTreeSet<String>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT i.imported_relpath FROM media_ingest_items i
            JOIN media_ingests r ON r.id=i.ingest_id WHERE r.staging=?1 AND i.verified=0
            AND i.imported_relpath IS NOT NULL
            AND EXISTS (SELECT 1 FROM media_ingest_copies c WHERE c.ingest_id=i.ingest_id AND c.relpath=i.relpath AND c.uuid=?2 AND c.library=?3)",
        )?;
        let paths = stmt
            .query_map(params![staging, uuid, library], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
        Ok(paths)
    }

    pub fn prior_unverified(
        &self,
        staging: &str,
        uuid: &str,
        library: &str,
        relpath: &str,
        digest: &str,
    ) -> Result<Option<String>> {
        let conn = self.pool.get()?;
        Ok(conn.query_row("SELECT i.imported_relpath FROM media_ingest_items i
            JOIN media_ingests r ON r.id=i.ingest_id WHERE r.staging=?1 AND i.relpath=?2 AND i.digest=?3
            AND i.verified=0 AND i.imported_relpath IS NOT NULL
            AND EXISTS (SELECT 1 FROM media_ingest_copies c WHERE c.ingest_id=i.ingest_id AND c.relpath=i.relpath AND c.uuid=?4 AND c.library=?5) ORDER BY r.id DESC LIMIT 1",
            params![staging, relpath, digest, uuid, library], |r| r.get(0)).optional()?)
    }

    pub fn item(
        &self,
        id: i64,
        relpath: &str,
        digest: Option<&str>,
        size: Option<i64>,
        outcome: Outcome<'_>,
    ) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("INSERT INTO media_ingest_items(ingest_id,relpath,digest,size,disposition,reason,imported_relpath,verified)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![id, relpath, digest, size, outcome.kind, outcome.reason, outcome.imported, outcome.verified])?;
        Ok(())
    }

    pub fn reserve_path(&self, id: i64, relpath: &str, imported: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("UPDATE media_ingest_items SET disposition='imported',reason='new digest',imported_relpath=?3 WHERE ingest_id=?1 AND relpath=?2",
            params![id, relpath, imported])?;
        Ok(())
    }

    pub fn fail_item(&self, id: i64, relpath: &str, reason: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("UPDATE media_ingest_items SET disposition='failed',reason=?3 WHERE ingest_id=?1 AND relpath=?2",
            params![id, relpath, reason])?;
        Ok(())
    }

    pub fn verified(&self, id: i64, relpath: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE media_ingest_items SET verified=1 WHERE ingest_id=?1 AND relpath=?2",
            params![id, relpath],
        )?;
        Ok(())
    }

    pub fn finish(&self, id: i64) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(&format!("UPDATE media_ingests SET finished={now},
            considered=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1),
            imported=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND disposition='imported'),
            duplicates=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND disposition='duplicate'),
            refused=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND disposition='refused'),
            failed=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND disposition='failed'),
            resumed=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND disposition='resumed'),
            verified=(SELECT count(*) FROM media_ingest_items WHERE ingest_id=?1 AND verified=1)
            WHERE id=?1", now=sjel_store::NOW), [id])?;
        Ok(())
    }

    pub fn record_phash(&self, digest: &str, phash: u64, algorithm: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let hex = format!("{phash:016x}");
        conn.execute(
            &format!(
                "INSERT INTO media_phashes(digest, phash, algorithm)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(digest) DO UPDATE SET
                 phash = excluded.phash,
                 algorithm = excluded.algorithm,
                 computed_at = {}",
                sjel_store::NOW
            ),
            params![digest, hex, algorithm],
        )?;
        Ok(())
    }

    pub fn phashes(&self) -> Result<Vec<PhashRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT digest, phash, algorithm, computed_at
             FROM media_phashes
             ORDER BY digest",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PhashRow {
                digest: row.get(0)?,
                phash: row.get(1)?,
                algorithm: row.get(2)?,
                computed_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn all_locations(&self) -> Result<Vec<Location>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, relpath, digest, size, mtime_ns
             FROM media_locations
             ORDER BY uuid, relpath",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Location {
                uuid: row.get(0)?,
                relpath: row.get(1)?,
                digest: row.get(2)?,
                size: row.get(3)?,
                mtime_ns: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn record_review(
        &self,
        first_digest: &str,
        second_digest: &str,
        decision: &str,
        note: &str,
    ) -> Result<ReviewRow> {
        let first = first_digest.trim();
        let second = second_digest.trim();
        if first.is_empty() || second.is_empty() {
            return Err("review digests must not be empty".into());
        }
        if first == second {
            return Err("a perceptual review requires two distinct digests".into());
        }
        if !matches!(decision, "near_duplicate" | "distinct" | "skip") {
            return Err(format!(
                "invalid review decision {decision:?}; expected near_duplicate, distinct, or skip"
            )
            .into());
        }
        let (left, right) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let conn = self.pool.get()?;
        conn.execute(
            &format!(
                "INSERT INTO media_reviews(left_digest, right_digest, decision, note)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(left_digest, right_digest) DO UPDATE SET
                 decision = excluded.decision,
                 note = excluded.note,
                 decided_at = {}",
                sjel_store::NOW
            ),
            params![left, right, decision, note],
        )?;
        Ok(conn.query_row(
            "SELECT left_digest, right_digest, decision, note, decided_at
             FROM media_reviews
             WHERE left_digest = ?1 AND right_digest = ?2",
            params![left, right],
            |row| {
                Ok(ReviewRow {
                    left_digest: row.get(0)?,
                    right_digest: row.get(1)?,
                    decision: row.get(2)?,
                    note: row.get(3)?,
                    decided_at: row.get(4)?,
                })
            },
        )?)
    }

    pub fn reviews(&self) -> Result<Vec<ReviewRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT left_digest, right_digest, decision, note, decided_at
             FROM media_reviews
             ORDER BY left_digest, right_digest",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReviewRow {
                left_digest: row.get(0)?,
                right_digest: row.get(1)?,
                decision: row.get(2)?,
                note: row.get(3)?,
                decided_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn record_geotag(
        &self,
        digest: &str,
        latitude: f64,
        longitude: f64,
        captured_at: Option<&str>,
    ) -> Result<()> {
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            return Err(format!("coordinates out of bounds: ({latitude}, {longitude})").into());
        }
        let conn = self.pool.get()?;
        conn.execute(
            &format!(
                "INSERT INTO media_geotags(digest, latitude, longitude, captured_at, indexed_at)
                 VALUES(?1, ?2, ?3, ?4, {now})
                 ON CONFLICT(digest) DO UPDATE SET
                 latitude = excluded.latitude,
                 longitude = excluded.longitude,
                 captured_at = excluded.captured_at,
                 indexed_at = {now}",
                now = sjel_store::NOW
            ),
            params![digest, latitude, longitude, captured_at],
        )?;
        Ok(())
    }

    pub fn geotags(&self) -> Result<Vec<GeotagRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT digest, latitude, longitude, captured_at, indexed_at
             FROM media_geotags
             ORDER BY digest",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(GeotagRow {
                digest: row.get(0)?,
                latitude: row.get(1)?,
                longitude: row.get(2)?,
                captured_at: row.get(3)?,
                indexed_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
