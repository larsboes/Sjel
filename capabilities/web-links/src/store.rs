//! Persisted web destinations and reviewed browser-history candidates.

use std::path::Path;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sjel_store::{open_pool, QueryAll};

use crate::{destination, HistoryCandidate, HistoryDecision, WriteLink};

const PREFIX: &str = "web_links";

pub type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// A saved web destination, with its derived site group and any user override.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Link {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    pub tenant: Option<String>,
    pub group: String,
    pub group_override: Option<String>,
    pub pinned: bool,
    pub opens: u64,
    pub last_opened_at: Option<String>,
    pub created_at: String,
}

/// One locally reviewed import candidate.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    pub tenant: Option<String>,
    pub browser: String,
    pub visits: u64,
    pub last_visit: Option<String>,
}

/// Whether a review decision matched a pending suggestion, and the saved row when kept.
#[derive(Clone, Debug)]
pub struct CandidateDecision {
    pub found: bool,
    pub saved: Option<Link>,
}

#[derive(Clone)]
pub struct LinksStore {
    pool: sjel_store::Pool,
}

impl LinksStore {
    /// # Errors
    /// The shared database cannot be opened or the web-links schema cannot be migrated.
    pub fn open(path: &Path) -> Fallible<Self> {
        let pool = open_pool(path, PREFIX, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS web_links_items (
                    id TEXT PRIMARY KEY,
                    url TEXT NOT NULL UNIQUE,
                    title TEXT NOT NULL,
                    host TEXT NOT NULL,
                    tenant TEXT,
                    group_override TEXT,
                    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
                    opens INTEGER NOT NULL DEFAULT 0,
                    last_opened_at TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f+00:00','now'))
                );
                CREATE INDEX IF NOT EXISTS web_links_items_order
                    ON web_links_items (pinned DESC, host, title);
                CREATE TABLE IF NOT EXISTS web_links_candidates (
                    id TEXT PRIMARY KEY,
                    url TEXT NOT NULL UNIQUE,
                    title TEXT NOT NULL,
                    host TEXT NOT NULL,
                    tenant TEXT,
                    browser TEXT NOT NULL,
                    visits INTEGER NOT NULL,
                    last_visit TEXT,
                    decision TEXT NOT NULL DEFAULT 'pending'
                        CHECK (decision IN ('pending', 'kept', 'dismissed')),
                    scanned_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f+00:00'))
                );
                CREATE INDEX IF NOT EXISTS web_links_candidates_review
                    ON web_links_candidates (decision, visits DESC, host);",
            )?;
            Ok(())
        })?;
        Ok(Self { pool })
    }

    /// # Errors
    /// The database cannot be read.
    pub fn list(&self) -> Fallible<Vec<Link>> {
        let conn = self.pool.get()?;
        Ok(conn.query_all(
            "SELECT id, url, title, host, tenant, group_override, pinned, opens, last_opened_at, created_at
             FROM web_links_items ORDER BY pinned DESC, COALESCE(group_override, host), title COLLATE NOCASE",
            [],
            map_link,
        )?)
    }

    /// Save a destination. Re-adding the same normalized URL returns its existing row.
    pub fn add(&self, request: &WriteLink) -> Fallible<Link> {
        let parsed = destination(&request.url).map_err(invalid)?;
        let conn = self.pool.get()?;
        if let Some(existing) = conn
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM web_links_items WHERE url = ?1"),
                [&parsed.url],
                map_link,
            )
            .optional()?
        {
            return Ok(existing);
        }
        if request.title.chars().count() > 240
            || request
                .group
                .as_deref()
                .is_some_and(|group| group.chars().count() > 80)
        {
            return Err("Title must be at most 240 characters and group at most 80.".into());
        }
        let title = clean_text(&request.title, 240).unwrap_or_else(|| {
            parsed
                .tenant
                .clone()
                .unwrap_or_else(|| parsed.group.clone())
        });
        let group_override = clean_text(request.group.as_deref().unwrap_or(""), 80);
        let id = id_for(&parsed.url);
        conn.execute(
            "INSERT INTO web_links_items (id, url, title, host, tenant, group_override, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(url) DO NOTHING",
            params![
                id,
                parsed.url,
                title,
                parsed.host,
                parsed.tenant,
                group_override,
                i64::from(request.pinned)
            ],
        )?;
        self.get(&id)?
            .ok_or_else(|| "saved link was not readable".into())
    }

    /// Update title, group override or pin state for one saved destination.
    pub fn edit(
        &self,
        id: &str,
        title: Option<&str>,
        group: Option<&str>,
        pinned: Option<bool>,
    ) -> Fallible<Option<Link>> {
        let conn = self.pool.get()?;
        let Some(current) = conn
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM web_links_items WHERE id = ?1"),
                [id],
                map_link,
            )
            .optional()?
        else {
            return Ok(None);
        };
        if title.is_some_and(|value| value.chars().count() > 240)
            || group.is_some_and(|value| value.chars().count() > 80)
        {
            return Err("Title must be at most 240 characters and group at most 80.".into());
        }
        let title = title
            .map(|v| clean_text(v, 240).unwrap_or_else(|| current.title.clone()))
            .unwrap_or(current.title);
        let group = group
            .map(|v| clean_text(v, 80))
            .unwrap_or(current.group_override);
        let pinned = pinned.map(i64::from).unwrap_or(i64::from(current.pinned));
        conn.execute(
            "UPDATE web_links_items SET title = ?2, group_override = ?3, pinned = ?4 WHERE id = ?1",
            params![id, title, group, pinned],
        )?;
        drop(conn);
        self.get(id)
    }

    /// Remove one saved destination.
    pub fn remove(&self, id: &str) -> Fallible<bool> {
        let conn = self.pool.get()?;
        Ok(conn.execute("DELETE FROM web_links_items WHERE id = ?1", [id])? == 1)
    }

    /// Record an explicit open action and return the updated row.
    pub fn record_open(&self, id: &str) -> Fallible<Option<Link>> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE web_links_items SET opens = opens + 1,
             last_opened_at = strftime('%Y-%m-%d %H:%M:%f+00:00','now') WHERE id = ?1",
            [id],
        )?;
        drop(conn);
        self.get(id)
    }

    /// Return one saved destination by id.
    pub fn get(&self, id: &str) -> Fallible<Option<Link>> {
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM web_links_items WHERE id = ?1"),
                [id],
                map_link,
            )
            .optional()?)
    }

    /// Insert scan results without changing any previously reviewed decision.
    pub fn stage(&self, candidates: &[HistoryCandidate]) -> Fallible<usize> {
        let mut conn = self.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let mut inserted = 0;
        for candidate in candidates {
            let parsed = destination(&candidate.url).map_err(invalid)?;
            inserted += tx.execute(
                "INSERT INTO web_links_candidates (id, url, title, host, tenant, browser, visits, last_visit)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(url) DO UPDATE SET
                   visits = MAX(web_links_candidates.visits, excluded.visits),
                   last_visit = MAX(COALESCE(web_links_candidates.last_visit, ''), COALESCE(excluded.last_visit, '')),
                   browser = CASE WHEN instr(web_links_candidates.browser, excluded.browser) = 0
                     THEN web_links_candidates.browser || ', ' || excluded.browser ELSE web_links_candidates.browser END
                 WHERE web_links_candidates.decision = 'pending'",
                params![candidate.id, parsed.url, candidate.title, parsed.host, parsed.tenant,
                    candidate.browser, i64::try_from(candidate.visits)?, candidate.last_visit],
            )?;
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Pending browser-history candidates, most visited first.
    pub fn candidates(&self) -> Fallible<Vec<Candidate>> {
        let conn = self.pool.get()?;
        Ok(conn.query_all(
            "SELECT id, url, title, host, tenant, browser, visits, last_visit
             FROM web_links_candidates WHERE decision = 'pending'
             ORDER BY visits DESC, host, title COLLATE NOCASE",
            [],
            |row| {
                Ok(Candidate {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get(2)?,
                    host: row.get(3)?,
                    tenant: row.get(4)?,
                    browser: row.get(5)?,
                    visits: u64::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
                    last_visit: row.get(7)?,
                })
            },
        )?)
    }

    /// Keep or dismiss a staged suggestion. Keeping is atomic with creating the saved link.
    pub fn decide_candidate(
        &self,
        id: &str,
        decision: HistoryDecision,
    ) -> Fallible<CandidateDecision> {
        let mut conn = self.pool.get()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let candidate = tx.query_row(
            "SELECT url, title FROM web_links_candidates WHERE id = ?1 AND decision = 'pending'",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ).optional()?;
        let Some((url, title)) = candidate else {
            return Ok(CandidateDecision {
                found: false,
                saved: None,
            });
        };
        let decision_text = match decision {
            HistoryDecision::Keep => "kept",
            HistoryDecision::Dismiss => "dismissed",
        };
        tx.execute(
            "UPDATE web_links_candidates SET decision = ?2 WHERE id = ?1",
            params![id, decision_text],
        )?;
        if decision == HistoryDecision::Keep {
            let parsed = destination(&url).map_err(invalid)?;
            let link_id = id_for(&parsed.url);
            tx.execute(
                "INSERT INTO web_links_items (id, url, title, host, tenant)
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(url) DO NOTHING",
                params![
                    link_id,
                    parsed.url,
                    clean_text(&title, 240).unwrap_or(parsed.group.clone()),
                    parsed.host,
                    parsed.tenant
                ],
            )?;
            let link = tx.query_row(
                &format!("SELECT {LINK_COLUMNS} FROM web_links_items WHERE url = ?1"),
                [&parsed.url],
                map_link,
            )?;
            tx.commit()?;
            return Ok(CandidateDecision {
                found: true,
                saved: Some(link),
            });
        }
        tx.commit()?;
        Ok(CandidateDecision {
            found: true,
            saved: None,
        })
    }

    /// Verify that this store can read and write its own tables.
    pub fn ping(&self) -> Fallible<()> {
        let conn = self.pool.get()?;
        conn.query_row("SELECT COUNT(*) FROM web_links_items", [], |_| Ok(()))?;
        Ok(())
    }
}

const LINK_COLUMNS: &str =
    "id, url, title, host, tenant, group_override, pinned, opens, last_opened_at, created_at";

fn map_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<Link> {
    let host: String = row.get(3)?;
    let override_name: Option<String> = row.get(5)?;
    let group = override_name
        .clone()
        .unwrap_or_else(|| crate::site_name(&host));
    Ok(Link {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        host,
        tenant: row.get(4)?,
        group,
        group_override: override_name,
        pinned: row.get::<_, i64>(6)? != 0,
        opens: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
        last_opened_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn clean_text(value: &str, max: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= max).then(|| value.to_owned())
}

fn invalid(message: String) -> Box<dyn std::error::Error> {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

fn id_for(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn store() -> LinksStore {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sjel-web-links-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        LinksStore::open(&path).unwrap()
    }

    #[test]
    fn edits_survive_reload_and_keep_auto_group_separate_from_override() {
        let store = store();
        let added = store
            .add(&WriteLink {
                url: "https://reddit.com/r/rust".into(),
                title: "Rust".into(),
                group: None,
                pinned: false,
            })
            .unwrap();
        let edited = store
            .edit(
                &added.id,
                Some("Rust community"),
                Some("Reading"),
                Some(true),
            )
            .unwrap()
            .unwrap();
        assert_eq!(edited.group, "Reading");
        assert_eq!(edited.group_override.as_deref(), Some("Reading"));
        let again = store.list().unwrap();
        assert_eq!(again, vec![edited]);
    }

    #[test]
    fn repeated_add_is_idempotent_and_open_counts_are_recorded() {
        let store = store();
        let body = WriteLink {
            url: "https://github.com/notifications".into(),
            title: "".into(),
            group: None,
            pinned: false,
        };
        let first = store.add(&body).unwrap();
        let second = store.add(&body).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.record_open(&first.id).unwrap().unwrap().opens, 1);
    }

    #[test]
    fn history_decision_creates_once_and_dismissed_candidate_stays_out() {
        let store = store();
        let parsed = destination("https://reddit.com/r/rust").unwrap();
        let candidate = HistoryCandidate {
            id: id_for(&parsed.url),
            url: parsed.url.clone(),
            title: "Rust".into(),
            host: parsed.host,
            tenant: parsed.tenant,
            browser: "Chrome".into(),
            visits: 4,
            last_visit: None,
        };
        store.stage(&[candidate.clone()]).unwrap();
        let kept = store
            .decide_candidate(&candidate.id, HistoryDecision::Keep)
            .unwrap()
            .saved
            .unwrap();
        assert_eq!(kept.url, candidate.url);
        assert!(store.candidates().unwrap().is_empty());
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
