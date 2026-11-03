use super::*;

const MAX_TITLE_CHARS: usize = 200;
const MAX_BODY_BYTES: usize = 64 * 1024;
/// A queue the user empties by hand; more than this is an agent writing in a loop.
const MAX_QUEUED: i64 = 50;

/// A prompt an agent drafted for the user to paste into another tool.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PromptDraft {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: String,
}

/// Why a draft was refused.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DraftError {
    #[error("a draft needs a title and a body")]
    Invalid,
    #[error("the draft exceeds the size limit")]
    TooLarge,
    #[error("the outbox is full")]
    Full,
    #[error("draft storage unavailable")]
    Storage,
}

impl Store {
    /// Queue a draft.
    ///
    /// # Errors
    /// The title or body is empty or too large, the outbox is full, or storage fails.
    pub fn add_prompt_draft(&self, title: &str, body: &str) -> Result<PromptDraft, DraftError> {
        let title = title.trim();
        if title.is_empty() || body.trim().is_empty() {
            return Err(DraftError::Invalid);
        }
        if title.chars().count() > MAX_TITLE_CHARS || body.len() > MAX_BODY_BYTES {
            return Err(DraftError::TooLarge);
        }
        let conn = self.conn().map_err(|_| DraftError::Storage)?;
        let inserted = conn
            .execute(
                &format!(
                    "INSERT INTO {p}_prompt_drafts (title, body)
                     SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM {p}_prompt_drafts) < ?3",
                    p = self.prefix
                ),
                params![title, body, MAX_QUEUED],
            )
            .map_err(|_| DraftError::Storage)?;
        if inserted == 0 {
            return Err(DraftError::Full);
        }
        let id = conn.last_insert_rowid();
        self.prompt_draft(&conn, id)?.ok_or(DraftError::Storage)
    }

    /// Every queued draft, oldest first.
    ///
    /// # Errors
    /// Storage is unavailable.
    pub fn prompt_drafts(&self) -> Result<Vec<PromptDraft>, DraftError> {
        let conn = self.conn().map_err(|_| DraftError::Storage)?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT id, title, body, created_at FROM {}_prompt_drafts ORDER BY id",
                self.prefix
            ))
            .map_err(|_| DraftError::Storage)?;
        let rows = statement
            .query_map([], read_draft)
            .map_err(|_| DraftError::Storage)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| DraftError::Storage)
    }

    /// Remove a draft and return it: the user copied it, so it leaves the outbox.
    /// `None` when it was already taken.
    ///
    /// # Errors
    /// Storage is unavailable.
    pub fn take_prompt_draft(&self, id: i64) -> Result<Option<PromptDraft>, DraftError> {
        let conn = self.conn().map_err(|_| DraftError::Storage)?;
        conn.query_row(
            &format!(
                "DELETE FROM {}_prompt_drafts WHERE id = ?1 RETURNING id, title, body, created_at",
                self.prefix
            ),
            params![id],
            read_draft,
        )
        .optional()
        .map_err(|_| DraftError::Storage)
    }

    fn prompt_draft(
        &self,
        conn: &rusqlite::Connection,
        id: i64,
    ) -> Result<Option<PromptDraft>, DraftError> {
        conn.query_row(
            &format!(
                "SELECT id, title, body, created_at FROM {}_prompt_drafts WHERE id = ?1",
                self.prefix
            ),
            params![id],
            read_draft,
        )
        .optional()
        .map_err(|_| DraftError::Storage)
    }
}

fn read_draft(row: &rusqlite::Row<'_>) -> rusqlite::Result<PromptDraft> {
    Ok(PromptDraft {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        created_at: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "prompt-drafts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(&dir.join("comms.db")).unwrap(), dir)
    }

    #[test]
    fn a_taken_draft_leaves_the_outbox_once() {
        let (store, dir) = store();
        let first = store.add_prompt_draft(" Watch ", "Compare\n  watches").unwrap();
        store.add_prompt_draft("Mac", "Compare minis").unwrap();
        assert_eq!(first.title, "Watch");
        assert_eq!(store.prompt_drafts().unwrap().len(), 2);
        let taken = store.take_prompt_draft(first.id).unwrap().unwrap();
        assert_eq!(taken.body, "Compare\n  watches", "the body is kept exactly");
        assert_eq!(store.take_prompt_draft(first.id).unwrap(), None);
        let left = store.prompt_drafts().unwrap();
        assert_eq!(left.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), ["Mac"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_oversized_and_overflowing_drafts_are_refused() {
        let (store, dir) = store();
        assert_eq!(store.add_prompt_draft(" ", "x"), Err(DraftError::Invalid));
        assert_eq!(store.add_prompt_draft("t", " "), Err(DraftError::Invalid));
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        assert_eq!(store.add_prompt_draft("t", &big), Err(DraftError::TooLarge));
        for n in 0..MAX_QUEUED {
            store.add_prompt_draft(&format!("t{n}"), "b").unwrap();
        }
        assert_eq!(store.add_prompt_draft("one more", "b"), Err(DraftError::Full));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
