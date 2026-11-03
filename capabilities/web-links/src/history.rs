//! Explicit, read-only browser-history discovery. Each source is snapshotted with SQLite's online
//! backup API before the candidate query runs; browser databases are never edited.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{backup::Backup, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::destination;

/// A supported browser-history source.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Browser {
    Chrome,
    Safari,
}

impl Browser {
    fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Safari => "Safari",
        }
    }
}

/// A proposed destination extracted from local browser history; it is not saved until reviewed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HistoryCandidate {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    pub tenant: Option<String>,
    pub browser: String,
    pub visits: u64,
    pub last_visit: Option<String>,
}

/// Supported browser profile database paths, in deterministic order.
pub fn sources(home: &Path) -> Vec<(Browser, PathBuf)> {
    let mut out = Vec::new();
    let chrome = home.join("Library/Application Support/Google/Chrome");
    if let Ok(entries) = fs::read_dir(&chrome) {
        let mut profiles: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "Default" || name.starts_with("Profile "))
            })
            .collect();
        profiles.sort();
        out.extend(
            profiles
                .into_iter()
                .map(|profile| (Browser::Chrome, profile.join("History")))
                .filter(|(_, path)| path.is_file()),
        );
    }
    let safari = home.join("Library/Safari/History.db");
    if safari.is_file() {
        out.push((Browser::Safari, safari));
    }
    out
}

/// Read the configured browser sources and return destinations visited often enough recently.
pub fn collect(home: &Path, days: u16, min_visits: u16) -> Result<Vec<HistoryCandidate>, String> {
    if !(1..=365).contains(&days) || !(1..=100).contains(&min_visits) {
        return Err("History scan bounds must be 1–365 days and 1–100 visits.".into());
    }
    let browser_sources = sources(home);
    if browser_sources.is_empty() {
        return Err("No Chrome or Safari history database was found.".into());
    }
    let mut out = Vec::new();
    let mut readable = 0;
    for (browser, path) in browser_sources {
        match read_source(browser, &path, days, min_visits) {
            Ok(mut rows) => {
                readable += 1;
                out.append(&mut rows);
            }
            Err(error) => eprintln!(
                "web-links: could not read {} history: {error}",
                browser.label()
            ),
        }
    }
    if readable == 0 {
        return Err("Available browser history databases could not be read. Check browser access and try again.".into());
    }
    out.sort_by(|a, b| {
        b.visits
            .cmp(&a.visits)
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.url.cmp(&b.url))
    });
    Ok(out)
}

fn read_source(
    browser: Browser,
    path: &Path,
    days: u16,
    min_visits: u16,
) -> Result<Vec<HistoryCandidate>, String> {
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let mut snapshot = Snapshot::new()?;
    Backup::new(&source, &mut snapshot.conn)
        .and_then(|backup| backup.run_to_completion(128, Duration::from_millis(20), None))
        .map_err(|e| e.to_string())?;
    let rows = match browser {
        Browser::Chrome => read_chrome(&snapshot.conn, days, min_visits),
        Browser::Safari => read_safari(&snapshot.conn, days, min_visits),
    }?;
    Ok(rows
        .into_iter()
        .filter_map(|(raw_url, title, visits, last_visit)| {
            candidate(browser, raw_url, title, visits, last_visit)
        })
        .collect())
}

/// One history row: URL, title, visit count, last visit as the browser stores it.
type HistoryRow = (String, String, u64, Option<String>);

fn read_chrome(conn: &Connection, days: u16, min_visits: u16) -> Result<Vec<HistoryRow>, String> {
    let chrome_epoch_delta = 11_644_473_600_i64;
    let cutoff = unix_now()
        .saturating_sub(i64::from(days) * 86_400)
        .saturating_add(chrome_epoch_delta)
        .saturating_mul(1_000_000);
    let mut stmt = conn
        .prepare(
            "SELECT u.url, COALESCE(u.title, ''), COUNT(v.id), CAST(MAX(v.visit_time) AS TEXT)
         FROM urls u JOIN visits v ON v.url = u.id
         WHERE v.visit_time >= ?1
         GROUP BY u.id HAVING COUNT(v.id) >= ?2
         ORDER BY COUNT(v.id) DESC LIMIT 5000",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([cutoff, i64::from(min_visits)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                row.get(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn read_safari(conn: &Connection, days: u16, min_visits: u16) -> Result<Vec<HistoryRow>, String> {
    let cutoff = unix_now()
        .saturating_sub(i64::from(days) * 86_400)
        .saturating_sub(978_307_200) as f64;
    let mut stmt = conn
        .prepare(
            "SELECT i.url,
                COALESCE((SELECT latest.title FROM history_visits latest
                    WHERE latest.history_item = i.id
                    ORDER BY latest.visit_time DESC, latest.id DESC LIMIT 1), ''),
                COUNT(v.id), CAST(MAX(v.visit_time) AS TEXT)
         FROM history_items i JOIN history_visits v ON v.history_item = i.id
         WHERE v.visit_time >= ?1 GROUP BY i.id HAVING COUNT(v.id) >= ?2
         ORDER BY COUNT(v.id) DESC LIMIT 5000",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([cutoff, f64::from(min_visits)], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                row.get(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn candidate(
    browser: Browser,
    raw_url: String,
    title: String,
    visits: u64,
    last_visit: Option<String>,
) -> Option<HistoryCandidate> {
    let mut url = url::Url::parse(&raw_url).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    let parsed = destination(url.as_str()).ok()?;
    let id = {
        let digest = Sha256::digest(parsed.url.as_bytes());
        let mut value = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(value, "{byte:02x}");
        }
        value
    };
    let title = title.trim();
    let title = if title.chars().count() > 240 {
        title.chars().take(240).collect()
    } else {
        title.to_owned()
    };
    Some(HistoryCandidate {
        id,
        url: parsed.url,
        title,
        host: parsed.host,
        tenant: parsed.tenant,
        browser: browser.label().into(),
        visits,
        last_visit,
    })
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

struct Snapshot {
    path: PathBuf,
    conn: Connection,
}

impl Snapshot {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "sjel-web-links-history-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        Ok(Self { path, conn })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.path.with_extension("db-wal"));
        let _ = fs::remove_file(self.path.with_extension("db-shm"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_import_drops_query_and_fragment_data() {
        let row = candidate(
            Browser::Chrome,
            "https://reddit.com/r/rust?token=private#top".into(),
            "Rust".into(),
            8,
            None,
        )
        .unwrap();
        assert_eq!(row.url, "https://reddit.com/r/rust");
        assert_eq!(row.tenant.as_deref(), Some("r/rust"));
    }

    #[test]
    fn unsupported_schemes_never_become_candidates() {
        assert!(candidate(
            Browser::Safari,
            "file:///tmp/private".into(),
            "private".into(),
            30,
            None
        )
        .is_none());
        assert!(candidate(
            Browser::Chrome,
            "https://user:pass@example.com/".into(),
            "private".into(),
            30,
            None
        )
        .is_none());
    }

    #[test]
    fn chrome_source_is_snapshotted_and_query_data_is_not_imported() {
        let path = fixture_path("chrome");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, last_visit_time INTEGER); CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);").unwrap();
        let now = (unix_now() + 11_644_473_600) * 1_000_000;
        let outside_range = now - 90 * 86_400 * 1_000_000 - 1;
        conn.execute(
            "INSERT INTO urls VALUES (1, ?1, ?2, ?3, ?4)",
            ("https://reddit.com/r/rust?search=private", "Rust", 100, now),
        )
        .unwrap();
        for id in 1..=98 {
            conn.execute("INSERT INTO visits VALUES (?1, 1, ?2)", (id, outside_range))
                .unwrap();
        }
        for id in 99..=100 {
            conn.execute("INSERT INTO visits VALUES (?1, 1, ?2)", (id, now))
                .unwrap();
        }
        assert!(read_source(Browser::Chrome, &path, 90, 3)
            .unwrap()
            .is_empty());
        conn.execute("INSERT INTO visits VALUES (101, 1, ?1)", [now])
            .unwrap();
        drop(conn);
        let rows = read_source(Browser::Chrome, &path, 90, 3).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url, "https://reddit.com/r/rust");
        assert_eq!(rows[0].visits, 3);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn safari_source_is_snapshotted_and_returns_recent_visits() {
        let path = fixture_path("safari");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE history_items (id INTEGER PRIMARY KEY, url TEXT); CREATE TABLE history_visits (id INTEGER PRIMARY KEY, history_item INTEGER, visit_time REAL, title TEXT);").unwrap();
        let recent = unix_now() as f64 - 978_307_200.0;
        conn.execute(
            "INSERT INTO history_items VALUES (1, ?1)",
            ["https://x.com/example"],
        )
        .unwrap();
        for id in 1..=4 {
            conn.execute(
                "INSERT INTO history_visits VALUES (?1, 1, ?2, ?3)",
                (id, recent + f64::from(id), format!("Visit {id}")),
            )
            .unwrap();
        }
        drop(conn);
        let rows = read_source(Browser::Safari, &path, 90, 3).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Visit 4");
        assert_eq!(rows[0].tenant.as_deref(), Some("@example"));
        let _ = fs::remove_file(path);
    }

    fn fixture_path(browser: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sjel-web-links-{browser}-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
