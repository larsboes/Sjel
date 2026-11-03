//! Two tables in the shared database. `software_apps` is what the last scan found, one row per
//! bundle id; an app that disappears keeps its row with `present = 0`, so its decision history
//! still has something to belong to. `software_decisions` is append-only: the newest row per
//! bundle id is the current decision, and the older ones are why it changed.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sjel_turso::{libsql, Store, NOW};

use crate::discover::Found;

pub const PREFIX: &str = "software";
pub type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// What the operator decided about one app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// In use, keep it.
    Use,
    /// Not needed; uninstall it.
    Remove,
    /// Keep for now, but look for a better app.
    Replace,
    /// Keep for now, but build the part that matters into Sjel.
    Rebuild,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Use => "use",
            Self::Remove => "remove",
            Self::Replace => "replace",
            Self::Rebuild => "rebuild",
        }
    }
}

/// One app with its current decision, as the API returns it.
#[derive(Clone, Debug, Serialize)]
pub struct App {
    /// `app`, `leftover` or `tool` (`discover::Found::kind`).
    pub kind: String,
    pub bundle_id: String,
    pub name: String,
    pub path: String,
    pub version: String,
    pub source: String,
    pub app_bytes: u64,
    pub data_bytes: u64,
    pub data_paths: Vec<String>,
    pub last_used: Option<String>,
    pub present: bool,
    pub seen_at: String,
    pub decision: Option<String>,
    pub note: Option<String>,
    pub decided_at: Option<String>,
    /// Why this row is due another look, or `None` (`review_reason`).
    pub review: Option<&'static str>,
}

/// How long a decision holds before the row asks again.
pub const DECISION_DAYS: i64 = 180;
/// How long a kept app may go unopened before the row asks whether it is still used.
pub const UNUSED_DAYS: i64 = 90;

/// Why a row is due review on `today`, or `None`. Only present rows are asked about.
///
/// - `remove` but still installed: the uninstall has not happened.
/// - a kept app (`use`) Spotlight says has not been opened in `UNUSED_DAYS`, when the decision is
///   that old too. A recent `use` was made knowing the date, so it is not asked again until it
///   ages: OrcaSlicer, last opened 2026-07-09 and kept on 2026-10-22, would otherwise ask forever.
///   A missing date asks nothing: shell-launched apps never get one, so its absence is not
///   evidence.
/// - any keep (`use`, `replace`, `rebuild`) older than `DECISION_DAYS`: replacements improve and
///   rebuilds land, so a standing decision expires rather than living forever.
pub fn review_reason(
    present: bool,
    kind: &str,
    decision: Option<&str>,
    decided_at: Option<&str>,
    last_used: Option<&str>,
    today: chrono::NaiveDate,
) -> Option<&'static str> {
    let age = |stamp: Option<&str>| -> Option<i64> {
        let date = chrono::NaiveDate::parse_from_str(stamp?.get(..10)?, "%Y-%m-%d").ok()?;
        Some((today - date).num_days())
    };
    if !present {
        return None;
    }
    match decision? {
        "remove" => Some("marked remove, still installed"),
        "use"
            if kind == "app"
                && age(last_used).is_some_and(|d| d >= UNUSED_DAYS)
                && age(decided_at).is_some_and(|d| d >= UNUSED_DAYS) =>
        {
            Some("kept, but not opened in 90 days")
        }
        _ if age(decided_at).is_some_and(|d| d >= DECISION_DAYS) => {
            Some("decided more than 180 days ago")
        }
        _ => None,
    }
}

pub struct SoftwareStore {
    db: Store,
}

impl SoftwareStore {
    /// # Errors
    /// The database cannot be opened or migrated.
    pub async fn open(path: &Path) -> Fallible<Self> {
        Ok(Self {
            db: Store::open(path, PREFIX, migrate).await?,
        })
    }

    pub async fn ping(&self) -> Fallible<()> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT 1", ()).await?;
        rows.next().await?.ok_or("`SELECT 1` returned no row")?;
        Ok(())
    }

    /// Record one scan: upsert what was found, and mark every other known app absent.
    pub async fn record_scan(&self, found: &[Found]) -> Fallible<()> {
        let tx = self.db.begin().await?;
        tx.execute("UPDATE software_apps SET present = 0", ())
            .await?;
        for app in found {
            tx.execute(
                &format!(
                    "INSERT INTO software_apps (bundle_id, name, path, version, source, app_bytes,
                         data_bytes, data_paths, last_used, kind, present, first_seen, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, {NOW}, {NOW})
                     ON CONFLICT(bundle_id) DO UPDATE SET name = excluded.name, kind = excluded.kind,
                         path = excluded.path, version = excluded.version,
                         source = excluded.source, app_bytes = excluded.app_bytes,
                         data_bytes = excluded.data_bytes, data_paths = excluded.data_paths,
                         last_used = excluded.last_used, present = 1, seen_at = excluded.seen_at"
                ),
                libsql::params![
                    app.bundle_id.as_str(),
                    app.name.as_str(),
                    app.path.as_str(),
                    app.version.as_str(),
                    app.source.as_str(),
                    i64::try_from(app.app_bytes)?,
                    i64::try_from(app.data_bytes)?,
                    serde_json::to_string(&app.data_paths)?,
                    app.last_used.clone(),
                    app.kind,
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Append a decision. `false` when no scan has seen that bundle id.
    pub async fn decide(&self, bundle_id: &str, decision: Decision, note: &str) -> Fallible<bool> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM software_apps WHERE bundle_id = ?1",
                [bundle_id],
            )
            .await?;
        let known = rows.next().await?.is_some();
        drop(rows);
        if !known {
            return Ok(false);
        }
        conn.execute(
            &format!(
                "INSERT INTO software_decisions (bundle_id, decision, note, at)
                 VALUES (?1, ?2, ?3, {NOW})"
            ),
            (bundle_id, decision.as_str(), note),
        )
        .await?;
        Ok(true)
    }

    /// Every known app with its newest decision, largest footprint first.
    pub async fn apps(&self) -> Fallible<Vec<App>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT a.bundle_id, a.name, a.path, a.version, a.source, a.app_bytes,
                        a.data_bytes, a.data_paths, a.last_used, a.present, a.seen_at,
                        d.decision, d.note, d.at, a.kind
                 FROM software_apps a
                 LEFT JOIN software_decisions d ON d.id = (
                     SELECT id FROM software_decisions
                     WHERE bundle_id = a.bundle_id ORDER BY id DESC LIMIT 1)
                 ORDER BY a.present DESC, a.app_bytes + a.data_bytes DESC, a.name",
                (),
            )
            .await?;
        let today = chrono::Local::now().date_naive();
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let data_paths: String = row.get(7)?;
            let mut app = App {
                kind: row.get(14)?,
                bundle_id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                version: row.get(3)?,
                source: row.get(4)?,
                app_bytes: u64::try_from(row.get::<i64>(5)?)?,
                data_bytes: u64::try_from(row.get::<i64>(6)?)?,
                data_paths: serde_json::from_str(&data_paths)?,
                last_used: row.get(8)?,
                present: row.get::<i64>(9)? != 0,
                seen_at: row.get(10)?,
                decision: row.get(11)?,
                note: row.get(12)?,
                decided_at: row.get(13)?,
                review: None,
            };
            app.review = review_reason(
                app.present,
                &app.kind,
                app.decision.as_deref(),
                app.decided_at.as_deref(),
                app.last_used.as_deref(),
                today,
            );
            out.push(app);
        }
        Ok(out)
    }

    /// One row by its key, with its current decision.
    pub async fn get(&self, bundle_id: &str) -> Fallible<Option<App>> {
        Ok(self
            .apps()
            .await?
            .into_iter()
            .find(|app| app.bundle_id == bundle_id))
    }
}

async fn migrate(conn: libsql::Connection) -> Fallible<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS software_apps (
            bundle_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            version TEXT NOT NULL,
            source TEXT NOT NULL,
            app_bytes INTEGER NOT NULL,
            data_bytes INTEGER NOT NULL,
            data_paths TEXT NOT NULL,
            last_used TEXT,
            present INTEGER NOT NULL,
            first_seen TEXT NOT NULL,
            seen_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS software_decisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            bundle_id TEXT NOT NULL REFERENCES software_apps(bundle_id),
            decision TEXT NOT NULL CHECK (decision IN ('use', 'remove', 'replace', 'rebuild')),
            note TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS software_decisions_by_app
            ON software_decisions (bundle_id, id);",
    )
    .await?;
    // `kind` arrived with leftovers and tooling (2026-10-22). A table created before it has
    // every row an app, which is what the default says.
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('software_apps') WHERE name = 'kind'",
            (),
        )
        .await?;
    let has_kind = rows.next().await?.is_some();
    drop(rows);
    if !has_kind {
        conn.execute(
            "ALTER TABLE software_apps ADD COLUMN kind TEXT NOT NULL DEFAULT 'app'",
            (),
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(bundle_id: &str, bytes: u64) -> Found {
        Found {
            kind: "app",
            bundle_id: bundle_id.into(),
            name: bundle_id.into(),
            path: format!("/Applications/{bundle_id}.app"),
            version: "1.0".into(),
            source: "other".into(),
            app_bytes: bytes,
            data_bytes: 0,
            data_paths: vec![],
            last_used: None,
        }
    }

    async fn scratch(name: &str) -> (SoftwareStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sjel-software-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (
            SoftwareStore::open(&dir.join("axon.db")).await.unwrap(),
            dir,
        )
    }

    #[tokio::test]
    async fn a_vanished_app_keeps_its_row_and_its_decision() {
        let (store, dir) = scratch("vanish").await;
        store
            .record_scan(&[found("a.small", 1), found("b.big", 9)])
            .await
            .unwrap();
        assert!(store
            .decide("a.small", Decision::Use, "daily")
            .await
            .unwrap());
        assert!(store
            .decide("a.small", Decision::Remove, "replaced by b")
            .await
            .unwrap());
        store.record_scan(&[found("b.big", 9)]).await.unwrap();

        let apps = store.apps().await.unwrap();
        assert_eq!(apps[0].bundle_id, "b.big", "present apps sort first");
        let a = &apps[1];
        assert!(!a.present);
        assert_eq!(
            a.decision.as_deref(),
            Some("remove"),
            "newest decision wins"
        );
        assert_eq!(a.note.as_deref(), Some("replaced by b"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn review_asks_about_stale_keeps_unused_apps_and_pending_removals() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap();
        let r = |kind, decision, decided, used| {
            review_reason(true, kind, decision, decided, used, today)
        };
        assert_eq!(
            r("app", None, None, None),
            None,
            "undecided is its own list"
        );
        assert_eq!(
            r("app", Some("remove"), Some("2026-10-22 10:00:00"), None),
            Some("marked remove, still installed")
        );
        assert_eq!(
            r(
                "app",
                Some("use"),
                Some("2026-07-01 10:00:00"),
                Some("2026-07-09 10:00:00 +0000")
            ),
            Some("kept, but not opened in 90 days")
        );
        assert_eq!(
            r(
                "app",
                Some("use"),
                Some("2026-10-22 10:00:00"),
                Some("2026-07-09 10:00:00 +0000")
            ),
            None,
            "a recent keep was made knowing the date"
        );
        assert_eq!(
            r("app", Some("use"), Some("2026-10-01 10:00:00"), None),
            None,
            "no Spotlight date is not evidence"
        );
        assert_eq!(
            r("tool", Some("use"), Some("2026-04-01 10:00:00"), None),
            Some("decided more than 180 days ago")
        );
        assert_eq!(
            review_reason(false, "app", Some("remove"), None, None, today),
            None,
            "a removed row asks nothing"
        );
    }

    #[tokio::test]
    async fn a_decision_for_an_unknown_app_is_refused() {
        let (store, dir) = scratch("unknown").await;
        assert!(!store.decide("never.seen", Decision::Use, "").await.unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
