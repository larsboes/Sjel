use super::*;

/// Safe local outcome for one message's event-analysis pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailEventScan {
    pub triage_id: String,
    pub candidate: bool,
    pub state: String,
    pub failure_code: Option<String>,
    pub attempts: u32,
    pub attempted_at: String,
}

impl Store {
    pub fn mail_event_scan_due(&self, triage_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let due = conn
            .query_row(
                &format!(
                    "SELECT CASE
                       WHEN state = 'failed' AND attempts < 5
                       THEN datetime(attempted_at, '+' || MIN(attempts * 30, 1440) || ' minutes') <= datetime('now')
                       WHEN state = 'deferred'
                       THEN datetime(attempted_at, '+60 minutes') <= datetime('now')
                       ELSE 0
                     END
                     FROM {}_mail_event_scans WHERE triage_id = ?1",
                    self.prefix
                ),
                [triage_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?;
        Ok(due.unwrap_or(true))
    }

    pub fn record_mail_event_scan(
        &self,
        triage_id: &str,
        candidate: bool,
        state: &str,
        failure_code: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {}_mail_event_scans
                   (triage_id, candidate, state, failure_code, attempts, attempted_at)
                 VALUES (?1, ?2, ?3, ?4, 1, {})
                 ON CONFLICT(triage_id) DO UPDATE SET
                   candidate = excluded.candidate,
                   state = excluded.state,
                   failure_code = excluded.failure_code,
                   attempts = {}_mail_event_scans.attempts + 1,
                   attempted_at = excluded.attempted_at",
                self.prefix,
                sjel_store::NOW,
                self.prefix,
            ),
            params![triage_id, candidate, state, failure_code],
        )?;
        Ok(())
    }

    pub fn mail_event_scans(
        &self,
        ids: &[String],
    ) -> Result<Vec<MailEventScan>, Box<dyn std::error::Error>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let placeholders = (1..=ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT triage_id, candidate, state, failure_code, attempts, attempted_at
               FROM {}_mail_event_scans WHERE triage_id IN ({placeholders})
               ORDER BY attempted_at DESC",
            self.prefix
        );
        let values = ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect::<Vec<_>>();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(values.as_slice(), |row| {
            Ok(MailEventScan {
                triage_id: row.get(0)?,
                candidate: row.get(1)?,
                state: row.get(2)?,
                failure_code: row.get(3)?,
                attempts: row.get(4)?,
                attempted_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
