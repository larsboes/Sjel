use std::path::Path;

use serde_json::Value;
use sjel_turso::{libsql, Store};

use crate::model::{Profile, ProfileInput, SCHEMA_VERSION};

pub const PREFIX: &str = "operator_profile";
type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub enum PutOutcome {
    Stored(Profile),
    Stale { current_revision: u64 },
}

pub struct OperatorProfileStore {
    db: Store,
}

impl OperatorProfileStore {
    /// Open the shared database and migrate this capability's table.
    ///
    /// The engine is `sjel-turso` (LibSQL) rather than `sjel-store` (rusqlite): the table lives in
    /// the same file, and the async connection is what lets the API await a read instead of
    /// handing one to `spawn_blocking`.
    ///
    /// # Errors
    /// The database cannot be opened or migrated.
    pub async fn open(path: &Path) -> Fallible<Self> {
        let db = Store::open(path, PREFIX, migrate).await?;
        Ok(Self { db })
    }

    pub async fn ping(&self) -> Fallible<()> {
        let conn = self.db.connect().await?;
        let mut rows = conn.query("SELECT 1", ()).await?;
        rows.next()
            .await?
            .ok_or_else(|| std::io::Error::other("`SELECT 1` returned no row"))?;
        Ok(())
    }

    pub async fn get(&self) -> Fallible<Option<Profile>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT document, revision, updated_at FROM operator_profile_profiles WHERE id = 'default'",
                (),
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let document: String = row.get(0)?;
        let revision: i64 = row.get(1)?;
        let updated_at: String = row.get(2)?;
        let input: ProfileInput = serde_json::from_str(&document)?;
        Ok(Some(Profile {
            revision: u64::try_from(revision)?,
            input,
            updated_at,
        }))
    }

    pub async fn put(&self, input: &ProfileInput, expected_revision: u64) -> Fallible<PutOutcome> {
        let tx = self.db.begin().await?;
        let mut rows = tx
            .query(
                "SELECT revision FROM operator_profile_profiles WHERE id = 'default'",
                (),
            )
            .await?;
        let current: Option<i64> = match rows.next().await? {
            Some(row) => Some(row.get(0)?),
            None => None,
        };
        // The read must finish before the write starts: LibSQL reads statement by statement on one
        // connection, and a pending row would deadlock the insert below.
        drop(rows);
        let current_revision = current.map(u64::try_from).transpose()?.unwrap_or(0);
        if expected_revision != current_revision {
            tx.rollback().await?;
            return Ok(PutOutcome::Stale { current_revision });
        }
        let revision = current_revision
            .checked_add(1)
            .ok_or("profile revision overflow")?;
        let stored_revision = i64::try_from(revision)?;
        let document = serde_json::to_string(input)?;
        // The stamp keeps this capability's existing `T`/`Z` shape: stored values and the rendered
        // managed section already carry it, and one row is not worth a format change. New tables
        // take `sjel_turso::NOW`.
        tx.execute(
            "INSERT INTO operator_profile_profiles (id, document, revision, updated_at)
             VALUES ('default', ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(id) DO UPDATE SET document = excluded.document,
                 revision = excluded.revision, updated_at = excluded.updated_at",
            (document, stored_revision),
        )
        .await?;
        tx.commit().await?;
        self.get()
            .await?
            .map(PutOutcome::Stored)
            .ok_or_else(|| std::io::Error::other("profile write disappeared").into())
    }
}

async fn migrate(conn: libsql::Connection) -> Fallible<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS operator_profile_profiles (
            id TEXT PRIMARY KEY CHECK (id = 'default'),
            document TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK (revision > 0),
            updated_at TEXT NOT NULL
        );",
    )
    .await?;
    Ok(())
}

/// Stable JSON shape for API and CLI consumers; profile values are never interpolated into logs.
pub fn profile_json(profile: Option<Profile>) -> Result<Value, serde_json::Error> {
    match profile {
        Some(profile) => serde_json::to_value(crate::model::StoredProfile {
            schema_version: profile.input.schema_version,
            revision: profile.revision,
            fields: profile.input.fields,
            statements: profile
                .input
                .statements
                .into_iter()
                .map(|(id, s)| {
                    (
                        id,
                        crate::model::ProfileStatement {
                            category: s.category,
                            text: s.text,
                            classification: s.classification,
                            export_to: s.export_to,
                        },
                    )
                })
                .collect(),
            updated_at: profile.updated_at,
        }),
        None => serde_json::to_value(serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "revision": 0,
            "fields": {},
            "statements": {},
            "stored": false
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Classification, FieldInput, FieldValue, Harness, ProfileCategory, StatementInput,
    };
    use std::collections::BTreeMap;

    fn sample() -> ProfileInput {
        ProfileInput {
            schema_version: SCHEMA_VERSION,
            fields: BTreeMap::from([(
                "verbosity".into(),
                FieldInput {
                    category: ProfileCategory::InteractionStyle,
                    value: FieldValue::Text("short".into()),
                    classification: Classification::C1,
                    export_to: vec![Harness::Claude],
                },
            )]),
            statements: BTreeMap::from([(
                "boundary".into(),
                StatementInput {
                    category: ProfileCategory::Boundary,
                    text: "Ask before sending.".into(),
                    classification: Classification::C1,
                    export_to: vec![Harness::Claude],
                },
            )]),
        }
    }

    async fn scratch(name: &str) -> (OperatorProfileStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sjel-operator-profile-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = OperatorProfileStore::open(&dir.join("axon.db"))
            .await
            .unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn missing_profile_is_empty_and_first_write_is_revision_one() {
        let (store, dir) = scratch("first-write").await;
        assert!(store.get().await.unwrap().is_none());
        let outcome = store.put(&sample(), 0).await.unwrap();
        let PutOutcome::Stored(profile) = outcome else {
            panic!("fresh write is not stale")
        };
        assert_eq!(profile.revision, 1);
        assert_eq!(store.get().await.unwrap().unwrap().revision, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn stale_write_does_not_replace_the_profile() {
        let (store, dir) = scratch("stale-write").await;
        store.put(&sample(), 0).await.unwrap();
        let mut changed = sample();
        changed.fields.get_mut("verbosity").unwrap().value = FieldValue::Text("detailed".into());
        assert!(matches!(
            store.put(&changed, 0).await.unwrap(),
            PutOutcome::Stale {
                current_revision: 1
            }
        ));
        assert_eq!(
            store.get().await.unwrap().unwrap().input.fields["verbosity"].value,
            FieldValue::Text("short".into())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
