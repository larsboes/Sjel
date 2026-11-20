//! Durable operator decisions and the filing states needed to reopen them.

use super::{
    check_id, engine, fs, hex, params, write_atomic, Connection, Fallible, Library, LibraryError,
    OptionalExtension, Sha256,
};
use crate::review::{DecisionAction, DuplicateDecision};
use documents_api::{DuplicateReopenRequest, DuplicateSourceLocation};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DecisionRecord {
    pub decision: DuplicateDecision,
    pub decided_at: String,
    pub reopened_at: Option<String>,
    prior_status: Vec<(String, String)>,
    pub locations: Vec<DuplicateSourceLocation>,
}

impl DecisionRecord {
    pub(crate) fn can_reopen(&self) -> bool {
        self.reopened_at.is_none()
            && match self.decision.action {
                DecisionAction::ConfirmGroup { .. } => {
                    self.prior_status.len() == self.decision.absorbed_ids().len()
                }
                DecisionAction::Dismiss => true,
                DecisionAction::Split { .. } => false,
            }
    }
}

#[derive(Serialize, Deserialize)]
struct DecisionSidecar {
    version: u32,
    history: Vec<DecisionRecord>,
}

fn parse(json: &str, decided_at: String) -> Fallible<DecisionSidecar> {
    let fail =
        |error: serde_json::Error| LibraryError::Invalid(format!("duplicate decision: {error}"));
    if let Ok(sidecar) = serde_json::from_str::<DecisionSidecar>(json) {
        if sidecar.version == 1 && !sidecar.history.is_empty() {
            return Ok(sidecar);
        }
        return Err(LibraryError::Invalid(
            "unsupported duplicate decision sidecar".into(),
        ));
    }
    let decision = serde_json::from_str(json).map_err(fail)?;
    Ok(DecisionSidecar {
        version: 1,
        history: vec![DecisionRecord {
            decision,
            decided_at,
            reopened_at: None,
            locations: Vec::new(),
            prior_status: Vec::new(),
        }],
    })
}

const DECISION_ROWS: &str = "SELECT candidate_key, history_json AS record_json, updated_at
    FROM documents_duplicate_history
    UNION ALL
    SELECT current.candidate_key, current.decision_json, current.updated_at
    FROM documents_duplicate_reviews current
    WHERE NOT EXISTS (
        SELECT 1 FROM documents_duplicate_history history
        WHERE history.candidate_key = current.candidate_key
    )";

fn read_one(conn: &Connection, key: &str) -> Fallible<Option<DecisionSidecar>> {
    let sql =
        format!("SELECT record_json, updated_at FROM ({DECISION_ROWS}) WHERE candidate_key = ?1");
    conn.query_row(&sql, params![key], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .optional()?
    .map(|(json, timestamp)| parse(&json, timestamp))
    .transpose()
}

fn store(conn: &Connection, key: &str, json: &str, current: &DecisionRecord) -> Fallible<()> {
    let timestamp = current
        .reopened_at
        .as_deref()
        .unwrap_or(&current.decided_at);
    conn.execute(
        "INSERT INTO documents_duplicate_history (candidate_key, history_json, updated_at)
         VALUES (?1, ?2, ?3) ON CONFLICT (candidate_key) DO UPDATE SET
             history_json = excluded.history_json, updated_at = excluded.updated_at",
        params![key, json, timestamp],
    )?;
    if current.reopened_at.is_some() {
        conn.execute(
            "DELETE FROM documents_duplicate_reviews WHERE candidate_key = ?1",
            params![key],
        )?;
    } else {
        let active = serde_json::to_string(&current.decision)
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        conn.execute(
            "INSERT INTO documents_duplicate_reviews (candidate_key, decision_json, updated_at)
             VALUES (?1, ?2, ?3) ON CONFLICT (candidate_key) DO UPDATE SET
                 decision_json = excluded.decision_json, updated_at = excluded.updated_at",
            params![key, active.as_str(), timestamp],
        )?;
    }
    Ok(())
}

fn encode(sidecar: &DecisionSidecar) -> Fallible<String> {
    serde_json::to_string(sidecar).map_err(|error| LibraryError::Invalid(error.to_string()))
}

fn sidecar_name(key: &str) -> String {
    format!("{}.json", hex(&Sha256::digest(key.as_bytes())))
}

impl Library {
    fn decision_sidecars(&self, member: Option<&str>) -> Fallible<Vec<DecisionSidecar>> {
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let sql = format!(
            "SELECT record_json, updated_at FROM ({DECISION_ROWS})
            WHERE ?1 IS NULL OR instr(record_json, ?1) > 0 ORDER BY candidate_key"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params![member], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (json, timestamp) = row?;
            parse(&json, timestamp)
        })
        .collect()
    }

    /// Active decisions used by the duplicate review queue.
    pub fn duplicate_decisions(&self) -> Fallible<Vec<DuplicateDecision>> {
        let conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let mut statement = conn.prepare(
            "SELECT decision_json FROM documents_duplicate_reviews ORDER BY candidate_key",
        )?;
        let rows = statement.query_map(params![], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            serde_json::from_str(&row?).map_err(|error| {
                LibraryError::Invalid(format!("active duplicate decision: {error}"))
            })
        })
        .collect()
    }

    pub(crate) fn duplicate_history(&self) -> Fallible<Vec<DecisionRecord>> {
        let mut history: Vec<_> = self
            .decision_sidecars(None)?
            .into_iter()
            .flat_map(|sidecar| sidecar.history)
            .collect();
        history.sort_by(|left, right| right.decided_at.cmp(&left.decided_at));
        Ok(history)
    }

    fn persist_decision_sidecar(&self, key: &str, json: &str) -> Fallible<()> {
        let directory = self.root.join("records/duplicates");
        fs::create_dir_all(&directory)?;
        write_atomic(&directory.join(sidecar_name(key)), json.as_bytes())
    }

    fn persist_decision_members(&self, ids: &[String]) -> Fallible<()> {
        for id in ids {
            let document = self
                .get(id)?
                .ok_or_else(|| LibraryError::Invalid("duplicate member disappeared".into()))?;
            self.write_record(&document)?;
        }
        Ok(())
    }

    pub fn save_duplicate_decision(&self, decision: &DuplicateDecision) -> Fallible<Vec<String>> {
        let _stale = self.stale_after();
        let _lock = self.lock_duplicate_writes()?;
        decision
            .validate()
            .map_err(|error| LibraryError::Invalid(error.to_string()))?;
        let absorbed = decision.absorbed_ids();
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = engine::write_transaction(&mut conn)?;
        let key = &decision.candidate_key;
        let mut sidecar = read_one(&transaction, key)?.unwrap_or_else(|| DecisionSidecar {
            version: 1,
            history: Vec::new(),
        });
        if let Some(current) = sidecar.history.last() {
            if current.reopened_at.is_none() && current.decision == *decision {
                let json = encode(&sidecar)?;
                transaction.commit()?;
                drop(conn);
                self.persist_decision_members(&absorbed)?;
                self.persist_decision_sidecar(key, &json)?;
                return Ok(absorbed);
            }
        }
        let mut prior_status = Vec::with_capacity(absorbed.len());
        let mut locations = Vec::new();
        {
            let mut read_paths = transaction
                .prepare("SELECT path FROM documents_sources WHERE id = ?1 ORDER BY path")?;
            for id in &decision.member_ids {
                let status: String = transaction
                    .query_row(
                        "SELECT status FROM documents_items WHERE id = ?1",
                        params![id.as_str()],
                        |row| row.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| {
                        LibraryError::Invalid(
                            "duplicate candidate includes a missing document".into(),
                        )
                    })?;
                if absorbed.contains(id) && status != "archived" {
                    prior_status.push((id.clone(), status));
                    transaction.execute(
                        "UPDATE documents_items SET status = 'archived' WHERE id = ?1",
                        params![id.as_str()],
                    )?;
                }
                for path in read_paths.query_map(params![id.as_str()], |row| row.get(0))? {
                    locations.push(DuplicateSourceLocation {
                        document_id: id.clone(),
                        path: path?,
                    });
                }
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let record = DecisionRecord {
            decision: decision.clone(),
            decided_at: now,
            reopened_at: None,
            prior_status,
            locations,
        };
        sidecar.history.push(record.clone());
        let json = encode(&sidecar)?;
        store(&transaction, key, &json, &record)?;
        transaction.commit()?;
        drop(conn);
        self.persist_decision_members(&absorbed)?;
        self.persist_decision_sidecar(key, &json)?;
        Ok(absorbed)
    }

    pub(crate) fn reopen_duplicate(
        &self,
        request: &DuplicateReopenRequest,
    ) -> Fallible<Vec<String>> {
        let _stale = self.stale_after();
        let _lock = self.lock_duplicate_writes()?;
        let key = &request.candidate_key;
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = engine::write_transaction(&mut conn)?;
        let mut sidecar = read_one(&transaction, key)?
            .ok_or_else(|| LibraryError::Invalid("duplicate decision does not exist".into()))?;
        let current = sidecar
            .history
            .last_mut()
            .filter(|record| record.decided_at == request.decided_at)
            .ok_or_else(|| {
                LibraryError::Invalid("duplicate decision changed; reload history".into())
            })?;
        let ids: Vec<_> = current
            .prior_status
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        if current.reopened_at.is_none() {
            if !current.can_reopen() {
                return Err(LibraryError::Invalid(
                    "this decision has no saved state to restore".into(),
                ));
            }
            for (id, status) in &current.prior_status {
                transaction.execute(
                    "UPDATE documents_items SET status = ?2 WHERE id = ?1 AND status = 'archived'",
                    params![id.as_str(), status.as_str()],
                )?;
            }
            current.reopened_at = Some(chrono::Utc::now().to_rfc3339());
        }
        let current = current.clone();
        let json = encode(&sidecar)?;
        store(&transaction, key, &json, &current)?;
        transaction.commit()?;
        drop(conn);
        self.persist_decision_members(&ids)?;
        self.persist_decision_sidecar(key, &json)?;
        Ok(ids)
    }

    pub(crate) fn backfill_duplicate_sidecars(&self) -> Fallible<()> {
        let _lock = self.lock_duplicate_writes()?;
        let mut by_id = std::collections::HashMap::<String, Vec<String>>::new();
        for (id, path) in self.sources()? {
            by_id.entry(id).or_default().push(path);
        }
        for mut sidecar in self.decision_sidecars(None)? {
            let Some(current) = sidecar.history.last_mut() else {
                continue;
            };
            let key = current.decision.candidate_key.clone();
            for id in &current.decision.member_ids {
                if let Some(paths) = by_id.get(id) {
                    for path in paths {
                        if !current
                            .locations
                            .iter()
                            .any(|source| source.document_id == *id && source.path == *path)
                        {
                            current.locations.push(DuplicateSourceLocation {
                                document_id: id.clone(),
                                path: path.clone(),
                            });
                        }
                    }
                }
            }
            self.persist_decision_sidecar(&key, &encode(&sidecar)?)?;
        }
        Ok(())
    }

    pub(super) fn release_duplicate_archive(&self, id: &str) -> Fallible<()> {
        // The caller holds the same lock as save and reopen.
        let mut changed = Vec::new();
        for mut sidecar in self.decision_sidecars(Some(id))? {
            let Some(current) = sidecar.history.last_mut() else {
                continue;
            };
            if current.reopened_at.is_some() {
                continue;
            }
            let count = current.prior_status.len();
            current.prior_status.retain(|(member, _)| member != id);
            if current.prior_status.len() != count {
                let key = current.decision.candidate_key.clone();
                let current = current.clone();
                changed.push((key, current, encode(&sidecar)?));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = engine::write_transaction(&mut conn)?;
        for (key, current, json) in &changed {
            store(&transaction, key, json, current)?;
        }
        transaction.commit()?;
        for (key, _, json) in changed {
            self.persist_decision_sidecar(&key, &json)?;
        }
        Ok(())
    }

    pub(super) fn restore_duplicate_sidecars(&self) -> Fallible<()> {
        let _lock = self.lock_duplicate_writes()?;
        let directory = self.root.join("records/duplicates");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut conn = self
            .pool
            .get()
            .map_err(|error| LibraryError::Store(error.to_string()))?;
        let transaction = engine::write_transaction(&mut conn)?;
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().is_none_or(|ext| ext != "json")
            {
                continue;
            }
            let json = fs::read_to_string(entry.path())?;
            let sidecar = parse(&json, String::new())?;
            let current = sidecar
                .history
                .last()
                .ok_or_else(|| LibraryError::Invalid("empty duplicate history".into()))?;
            let key = &current.decision.candidate_key;
            if entry.file_name() != sidecar_name(key).as_str() {
                return Err(LibraryError::Invalid(
                    "duplicate sidecar filename does not match its key".into(),
                ));
            }
            if read_one(&transaction, key)?.is_none() {
                store(&transaction, key, &json, current)?;
            }
            for record in &sidecar.history {
                record
                    .decision
                    .validate()
                    .map_err(|error| LibraryError::Invalid(error.to_string()))?;
                if &record.decision.candidate_key != key {
                    return Err(LibraryError::Invalid(
                        "inconsistent duplicate sidecar key".into(),
                    ));
                }
                let absorbed = record.decision.absorbed_ids();
                for (member, status) in &record.prior_status {
                    if !absorbed.contains(member) || !matches!(status.as_str(), "inbox" | "filed") {
                        return Err(LibraryError::Invalid(
                            "invalid saved duplicate archive state".into(),
                        ));
                    }
                }
                for source in &record.locations {
                    check_id(&source.document_id)?;
                    if !record.decision.member_ids.contains(&source.document_id) {
                        return Err(LibraryError::Invalid(
                            "duplicate source is not a member".into(),
                        ));
                    }
                    transaction.execute(
                        "INSERT OR IGNORE INTO documents_sources (id, path, seen_at) VALUES (?1, ?2, ?3)",
                        params![source.document_id.as_str(), source.path.as_str(), record.decided_at.as_str()],
                    )?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn lock_duplicate_writes(&self) -> Fallible<std::sync::MutexGuard<'_, ()>> {
        self.duplicate_write_lock
            .lock()
            .map_err(|error| LibraryError::Store(error.to_string()))
    }
}
