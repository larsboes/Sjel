//! Durable, revision-guarded mail evidence scan receipts.

use super::*;

const MAX_PAGE: usize = 50;
// The indexed timestamp is stored as a canonical millisecond-resolution SQLite stamp.
const INDEXED_MS: &str = "(CAST(strftime('%s', i.internal_date) AS INTEGER) * 1000 + CAST(substr(strftime('%f', i.internal_date), 4, 3) AS INTEGER))";

#[derive(Clone, PartialEq, Eq)]
pub struct MailEvidenceWork {
    pub triage_id: String,
    pub internal_date_ms: Option<i64>,
    pub data_class: String,
}

/// Aggregate scan state; no message identities or content are returned.
#[derive(Debug, serde::Serialize)]
pub struct MailEvidenceCounts {
    pub indexed: i64,
    pub pending: i64,
    pub completed: i64,
    pub refused: i64,
    pub failed: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailEvidenceOutcome {
    Completed,
    Refused,
    Failed,
}

impl MailEvidenceOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Refused => "refused",
            Self::Failed => "failed",
        }
    }
}

impl Store {
    /// Count current indexed revisions and their scan outcomes.
    ///
    /// # Errors
    /// Returns an error if the store cannot be queried.
    pub fn mail_evidence_counts(&self) -> Result<MailEvidenceCounts, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let sql = format!(
            "WITH indexed AS (SELECT i.id, i.data_class, {INDEXED_MS} AS date_ms FROM {p}_triage_items i),
             state AS (SELECT CASE WHEN s.triage_id IS NULL OR s.internal_date_ms IS NOT i.date_ms
                 OR s.data_class != i.data_class THEN 'pending' ELSE s.state END AS state
                 FROM indexed i LEFT JOIN {p}_mail_evidence_scans s ON s.triage_id=i.id)
             SELECT count(*), coalesce(sum(state='pending'),0), coalesce(sum(state='completed'),0),
                    coalesce(sum(state='refused'),0), coalesce(sum(state='failed'),0) FROM state",
            p = self.prefix,
        );
        Ok(conn.query_row(&sql, [], |row| {
            Ok(MailEvidenceCounts {
                indexed: row.get(0)?,
                pending: row.get(1)?,
                completed: row.get(2)?,
                refused: row.get(3)?,
                failed: row.get(4)?,
            })
        })?)
    }

    /// Select at most 50 indexed revisions, ordered by the time each became eligible.
    /// No cursor is advanced.
    pub fn mail_evidence_due(
        &self,
        limit: usize,
    ) -> Result<Vec<MailEvidenceWork>, Box<dyn std::error::Error>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let sql = format!(
            "WITH indexed AS (
            SELECT i.id, i.data_class, i.first_seen, {INDEXED_MS} AS date_ms
            FROM {p}_triage_items i
        )
        SELECT i.id, i.date_ms, i.data_class FROM indexed i
        LEFT JOIN {p}_mail_evidence_scans s ON s.triage_id = i.id
        WHERE s.triage_id IS NULL OR s.internal_date_ms IS NOT i.date_ms
            OR s.data_class != i.data_class OR (s.state = 'failed' AND s.retry_at <= {now})
        ORDER BY CASE WHEN s.triage_id IS NULL OR s.internal_date_ms IS NOT i.date_ms
                    OR s.data_class != i.data_class THEN i.first_seen ELSE s.retry_at END,
                 i.id
        LIMIT ?1",
            p = self.prefix,
            now = sjel_store::NOW
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([limit.min(MAX_PAGE) as i64], |r| {
            Ok(MailEvidenceWork {
                triage_id: r.get(0)?,
                internal_date_ms: r.get(1)?,
                data_class: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Returns false if the indexed revision changed, disappeared, or a completed receipt would
    /// be replaced by a failure for the same revision. A failure retries after 30 minutes,
    /// doubling per consecutive failure to a 24-hour ceiling.
    pub fn record_mail_evidence(
        &self,
        work: &MailEvidenceWork,
        outcome: MailEvidenceOutcome,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let sql = format!("INSERT INTO {p}_mail_evidence_scans
            (triage_id, internal_date_ms, data_class, state, failure_count, retry_at)
            SELECT i.id, {INDEXED_MS}, i.data_class, ?4,
                CASE WHEN ?4 = 'failed' THEN 1 ELSE 0 END,
                CASE WHEN ?4 = 'failed' THEN strftime('{fmt}', 'now', '+30 minutes') ELSE NULL END
            FROM {p}_triage_items i
            WHERE i.id = ?1 AND {INDEXED_MS} IS ?2 AND i.data_class = ?3
            ON CONFLICT(triage_id) DO UPDATE SET
                internal_date_ms = excluded.internal_date_ms,
                data_class = excluded.data_class,
                state = excluded.state,
                failure_count = CASE WHEN excluded.state = 'failed' THEN
                    CASE WHEN {p}_mail_evidence_scans.state = 'failed'
                        AND {p}_mail_evidence_scans.internal_date_ms IS excluded.internal_date_ms
                        AND {p}_mail_evidence_scans.data_class = excluded.data_class
                    THEN MIN({p}_mail_evidence_scans.failure_count + 1, 7) ELSE 1 END ELSE 0 END,
                retry_at = CASE WHEN excluded.state = 'failed' THEN
                    strftime('{fmt}', 'now', '+' || CASE
                        WHEN {p}_mail_evidence_scans.state = 'failed'
                            AND {p}_mail_evidence_scans.internal_date_ms IS excluded.internal_date_ms
                            AND {p}_mail_evidence_scans.data_class = excluded.data_class
                        THEN MIN(30 * (1 << MIN({p}_mail_evidence_scans.failure_count, 6)), 1440)
                        ELSE 30 END || ' minutes') ELSE NULL END
            WHERE NOT ({p}_mail_evidence_scans.state = 'completed'
                AND excluded.state = 'failed'
                AND {p}_mail_evidence_scans.internal_date_ms IS excluded.internal_date_ms)", p=self.prefix, fmt=sjel_store::STAMP_FORMAT);
        let changed = conn.execute(
            &sql,
            params![
                work.triage_id,
                work.internal_date_ms,
                work.data_class,
                outcome.as_str()
            ],
        )?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(store: &Store, id: &str, date: Option<i64>, class: &str) {
        let conn = store.conn().unwrap();
        conn.execute(&format!("INSERT INTO {}_triage_items (id,internal_date,stream,rationale,data_class,first_seen,last_seen) VALUES (?1, strftime('{}', ?2 / 1000.0, 'unixepoch'), 'aktiv', '', ?3, '2020-01-01', '2020-01-01')", store.prefix, sjel_store::STAMP_FORMAT), params![id,date,class]).unwrap();
    }
    fn update(store: &Store, id: &str, date: Option<i64>, class: &str) {
        store.conn().unwrap().execute(&format!("UPDATE {}_triage_items SET internal_date = strftime('{}', ?2 / 1000.0, 'unixepoch'), data_class = ?3 WHERE id = ?1", store.prefix, sjel_store::STAMP_FORMAT), params![id,date,class]).unwrap();
    }
    fn ids(items: &[MailEvidenceWork]) -> Vec<&str> {
        items.iter().map(|w| w.triage_id.as_str()).collect()
    }

    #[test]
    fn restart_changes_and_optimistic_guard() {
        let path = crate::store::db_tests::test_database("mail_evidence_restart");
        let store = Store::open(&path).unwrap();
        insert(&store, "a", Some(1_700_000_000_123), "c1");
        assert_eq!(store.mail_evidence_counts().unwrap().pending, 1);
        let old = store.mail_evidence_due(5).unwrap().remove(0);
        assert_eq!(old.internal_date_ms, Some(1_700_000_000_123));
        assert!(store
            .record_mail_evidence(&old, MailEvidenceOutcome::Completed)
            .unwrap());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.mail_evidence_due(5).unwrap().is_empty());
        assert_eq!(store.mail_evidence_counts().unwrap().completed, 1);
        update(&store, "a", Some(1_700_000_000_124), "c1");
        assert_eq!(store.mail_evidence_counts().unwrap().pending, 1);
        assert!(!store
            .record_mail_evidence(&old, MailEvidenceOutcome::Completed)
            .unwrap());
        let newer = store.mail_evidence_due(5).unwrap().remove(0);
        assert_eq!(newer.internal_date_ms, Some(1_700_000_000_124));
        assert!(store
            .record_mail_evidence(&newer, MailEvidenceOutcome::Completed)
            .unwrap());
        update(&store, "a", Some(1_700_000_000_124), "c3");
        assert!(!store
            .record_mail_evidence(&newer, MailEvidenceOutcome::Completed)
            .unwrap());
        let secret = store.mail_evidence_due(5).unwrap().remove(0);
        assert_eq!(secret.data_class, "c3");
        assert!(store
            .record_mail_evidence(&secret, MailEvidenceOutcome::Refused)
            .unwrap());
        assert!(store.mail_evidence_due(5).unwrap().is_empty());
    }

    #[test]
    fn a_failure_never_overwrites_a_completed_receipt() {
        let path = crate::store::db_tests::test_database("mail_evidence_regress");
        let store = Store::open(&path).unwrap();
        insert(&store, "a", Some(1_700_000_000_000), "c1");
        let work = store.mail_evidence_due(5).unwrap().remove(0);
        assert!(store
            .record_mail_evidence(&work, MailEvidenceOutcome::Completed)
            .unwrap());
        assert!(!store
            .record_mail_evidence(&work, MailEvidenceOutcome::Failed)
            .unwrap());
        let counts = store.mail_evidence_counts().unwrap();
        assert_eq!((counts.completed, counts.failed, counts.pending), (1, 0, 0));
    }

    #[test]
    fn retry_backoff_and_due_order() {
        let path = crate::store::db_tests::test_database("mail_evidence_fair");
        let store = Store::open(&path).unwrap();
        insert(&store, "a", None, "c2");
        let failed = store.mail_evidence_due(5).unwrap().remove(0);
        assert!(store
            .record_mail_evidence(&failed, MailEvidenceOutcome::Failed)
            .unwrap());
        assert!(store.mail_evidence_due(5).unwrap().is_empty());
        insert(&store, "b", Some(1_700_000_000_000), "c1");
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE comms_mail_evidence_scans SET retry_at = '2000-01-01 00:00:00.000+00:00'",
                [],
            )
            .unwrap();
        assert_eq!(ids(&store.mail_evidence_due(1).unwrap()), vec!["a"]);
        assert_eq!(ids(&store.mail_evidence_due(5).unwrap()), vec!["a", "b"]);
        assert!(store
            .record_mail_evidence(&failed, MailEvidenceOutcome::Failed)
            .unwrap());
        let conn = store.conn().unwrap();
        let (count, retry): (i64, String) = conn.query_row("SELECT failure_count, retry_at FROM comms_mail_evidence_scans WHERE triage_id = 'a'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(count, 2);
        assert!(retry.as_str() > "2000");
        drop(conn);
        update(&store, "a", None, "c3");
        assert_eq!(ids(&store.mail_evidence_due(5).unwrap()), vec!["a", "b"]);
        assert_eq!(store.mail_evidence_due(0).unwrap().len(), 0);
    }
}
