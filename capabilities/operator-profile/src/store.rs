use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::model::{Profile, ProfileInput, SCHEMA_VERSION};

pub const PREFIX: &str = "operator_profile";
type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub enum PutOutcome {
    Stored(Profile),
    Stale { current_revision: u64 },
}

pub struct OperatorProfileStore {
    pool: sjel_store::Pool,
}

impl OperatorProfileStore {
    pub fn open(path: &Path) -> Fallible<Self> {
        let pool = sjel_store::open_pool(path, PREFIX, migrate)?;
        Ok(Self { pool })
    }

    fn conn(&self) -> Fallible<sjel_store::PooledClient> {
        Ok(self.pool.get()?)
    }

    pub fn ping(&self) -> Fallible<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    }

    pub fn get(&self) -> Fallible<Option<Profile>> {
        let conn = self.conn()?;
        let row: Option<(String, i64, String)> = conn
            .query_row(
                "SELECT document, revision, updated_at FROM operator_profile_profiles WHERE id = 'default'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(document, revision, updated_at)| {
            let input: ProfileInput = serde_json::from_str(&document)?;
            let revision = u64::try_from(revision)?;
            Ok(Profile {
                revision,
                input,
                updated_at,
            })
        })
        .transpose()
    }

    pub fn put(&self, input: &ProfileInput, expected_revision: u64) -> Fallible<PutOutcome> {
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT revision FROM operator_profile_profiles WHERE id = 'default'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let current_revision = current.map(u64::try_from).transpose()?.unwrap_or(0);
        if expected_revision != current_revision {
            drop(tx);
            return Ok(PutOutcome::Stale { current_revision });
        }
        let revision = current_revision
            .checked_add(1)
            .ok_or("profile revision overflow")?;
        let stored_revision = i64::try_from(revision)?;
        let document = serde_json::to_string(input)?;
        tx.execute(
            "INSERT INTO operator_profile_profiles (id, document, revision, updated_at)
             VALUES ('default', ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(id) DO UPDATE SET document = excluded.document,
                 revision = excluded.revision, updated_at = excluded.updated_at",
            params![document, stored_revision],
        )?;
        tx.commit()?;
        self.get()?
            .map(PutOutcome::Stored)
            .ok_or_else(|| std::io::Error::other("profile write disappeared").into())
    }
}

fn migrate(conn: &Connection) -> Fallible<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS operator_profile_profiles (
            id TEXT PRIMARY KEY CHECK (id = 'default'),
            document TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK (revision > 0),
            updated_at TEXT NOT NULL
        );",
    )?;
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

    fn scratch(name: &str) -> (OperatorProfileStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sjel-operator-profile-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = OperatorProfileStore::open(&dir.join("axon.db")).unwrap();
        (store, dir)
    }

    #[test]
    fn missing_profile_is_empty_and_first_write_is_revision_one() {
        let (store, dir) = scratch("first-write");
        assert!(store.get().unwrap().is_none());
        let PutOutcome::Stored(profile) = store.put(&sample(), 0).unwrap() else {
            panic!("fresh write is not stale")
        };
        assert_eq!(profile.revision, 1);
        assert_eq!(store.get().unwrap().unwrap().revision, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_write_does_not_replace_the_profile() {
        let (store, dir) = scratch("stale-write");
        store.put(&sample(), 0).unwrap();
        let mut changed = sample();
        changed.fields.get_mut("verbosity").unwrap().value = FieldValue::Text("detailed".into());
        assert!(matches!(
            store.put(&changed, 0).unwrap(),
            PutOutcome::Stale {
                current_revision: 1
            }
        ));
        assert_eq!(
            store.get().unwrap().unwrap().input.fields["verbosity"].value,
            FieldValue::Text("short".into())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
