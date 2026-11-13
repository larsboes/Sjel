use super::*;

const MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCUMENT_KEY_BYTES: usize = 5 + 1024 + 1 + 512;

/// A locally retained browser capture. The payload is not sent for enrichment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebClip {
    pub request_id: String,
    pub url: String,
    pub document_key: String,
    pub behavior: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

/// A browser-generated retry key and the complete versioned capture payload.
///
/// Payload version 1 carries `behavior`, `markdown`, and `destination.path` and
/// `destination.name` (or `destination.date` for a daily behavior). Every other field is
/// kept as sent.
#[derive(Debug, Clone, Deserialize)]
pub struct WebClipInput {
    pub request_id: String,
    pub url: String,
    pub payload: serde_json::Value,
}

/// Whether the submission created a capture or repeated an identical request.
#[derive(Debug, Clone, PartialEq)]
pub enum WebClipWrite {
    Created(WebClip),
    Replayed(WebClip),
}

/// One addressed clip document: every save to it, folded into its current Markdown.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WebClipDocument {
    pub document_key: String,
    pub title: Option<String>,
    pub url: String,
    pub versions: usize,
    pub markdown: String,
    pub updated_at: String,
}

/// Clip metadata for discovery without returning document bodies.
#[derive(Debug, Serialize)]
pub struct WebClipSummary {
    pub document_key: String,
    pub title: Option<String>,
    pub url: String,
    pub versions: i64,
    pub updated_at: String,
    pub data_class: &'static str,
}

/// Failure to validate or persist a clip.
#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("invalid clip request")]
    Invalid,
    #[error("clip payload exceeds the inline limit")]
    TooLarge,
    #[error("request id already belongs to a different clip")]
    Conflict,
    #[error("clip storage unavailable")]
    Storage,
}

/// The Obsidian Web Clipper template behaviors, mapped onto clip documents.
const BEHAVIORS: [&str; 6] = [
    "create",
    "overwrite",
    "append-specific",
    "prepend-specific",
    "append-daily",
    "prepend-daily",
];

/// The document a save addresses. `create` always starts a document of its own; the other
/// behaviors address the template's path and note name, or the day for a daily behavior.
fn document_key(request_id: &str, behavior: &str, payload: &serde_json::Value) -> Option<String> {
    let field = |name: &str| {
        payload
            .get("destination")
            .and_then(|d| d.get(name))
            .and_then(serde_json::Value::as_str)
    };
    match behavior {
        "create" => Some(format!("clip:{request_id}")),
        "append-daily" | "prepend-daily" => {
            let date = field("date")?;
            let parts: Vec<&str> = date.split('-').collect();
            let valid = parts.len() == 3
                && [4, 2, 2]
                    .iter()
                    .zip(&parts)
                    .all(|(n, p)| p.len() == *n && p.bytes().all(|b| b.is_ascii_digit()))
                && (1..=12).contains(&parts[1].parse::<u8>().unwrap_or(0))
                && (1..=31).contains(&parts[2].parse::<u8>().unwrap_or(0));
            if !valid {
                return None;
            }
            Some(format!("daily:{date}"))
        }
        _ => {
            let path = field("path").unwrap_or("").trim().trim_matches('/');
            let name = field("name")?.trim();
            if name.is_empty() || name.len() > 512 || path.len() > 1024 {
                return None;
            }
            Some(if path.is_empty() {
                format!("note:{name}")
            } else {
                format!("note:{path}/{name}")
            })
        }
    }
}

/// Fold saves in arrival order: create and overwrite replace, append and prepend combine.
fn fold(versions: &[(String, String)]) -> String {
    versions
        .iter()
        .fold(String::new(), |body, (behavior, markdown)| {
            if body.is_empty() {
                return markdown.clone();
            }
            match behavior.as_str() {
                b if b.starts_with("append") => format!("{body}\n\n{markdown}"),
                b if b.starts_with("prepend") => format!("{markdown}\n\n{body}"),
                _ => markdown.clone(),
            }
        })
}

type ClipRow = (String, String, String, String, String, String);

fn clip_from(row: ClipRow) -> Result<WebClip, ClipError> {
    Ok(WebClip {
        request_id: row.0,
        url: row.1,
        document_key: row.2,
        behavior: row.3,
        payload: serde_json::from_str(&row.4).map_err(|_| ClipError::Storage)?,
        created_at: row.5,
    })
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

const COLUMNS: &str = "request_id, url, document_key, behavior, payload, created_at";

impl Store {
    /// Read a stored clip by its retry key.
    ///
    /// # Errors
    /// Returns an error if storage is unavailable or the stored payload is invalid.
    pub fn web_clip(&self, request_id: &str) -> Result<Option<WebClip>, ClipError> {
        let conn = self.conn().map_err(|_| ClipError::Storage)?;
        conn.query_row(
            &format!(
                "SELECT {COLUMNS} FROM {}_web_clips WHERE request_id = ?1",
                self.prefix
            ),
            params![request_id],
            read_row,
        )
        .optional()
        .map_err(|_| ClipError::Storage)?
        .map(clip_from)
        .transpose()
    }

    /// Store a local-only clip. Reusing an id with different content is an error.
    ///
    /// # Errors
    /// Returns an error for invalid input, a conflicting retry, or storage failure.
    pub fn put_web_clip(&self, input: WebClipInput) -> Result<WebClipWrite, ClipError> {
        let behavior = input
            .payload
            .get("behavior")
            .and_then(serde_json::Value::as_str)
            .filter(|b| BEHAVIORS.contains(b))
            .ok_or(ClipError::Invalid)?
            .to_string();
        if input.request_id.is_empty()
            || input.request_id.len() > 128
            || !input
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || input.url.len() > 8192
            || !reqwest::Url::parse(&input.url).is_ok_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.username().is_empty()
                    && u.password().is_none()
            })
            || input
                .payload
                .get("version")
                .and_then(serde_json::Value::as_u64)
                != Some(1)
            || input
                .payload
                .get("markdown")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(ClipError::Invalid);
        }
        let document_key =
            document_key(&input.request_id, &behavior, &input.payload).ok_or(ClipError::Invalid)?;
        let payload = serde_json::to_string(&input.payload).map_err(|_| ClipError::Invalid)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ClipError::TooLarge);
        }
        let conn = self.conn().map_err(|_| ClipError::Storage)?;
        let inserted = conn
            .execute(
                &format!(
                    "INSERT INTO {}_web_clips (request_id, url, document_key, behavior, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (request_id) DO NOTHING",
                    self.prefix
                ),
                params![input.request_id, input.url, document_key, behavior, payload],
            )
            .map_err(|_| ClipError::Storage)?;
        let clip = clip_from(
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM {}_web_clips WHERE request_id = ?1",
                    self.prefix
                ),
                params![input.request_id],
                read_row,
            )
            .map_err(|_| ClipError::Storage)?,
        )?;
        if clip.url != input.url || clip.payload != input.payload {
            return Err(ClipError::Conflict);
        }
        Ok(if inserted == 1 {
            WebClipWrite::Created(clip)
        } else {
            WebClipWrite::Replayed(clip)
        })
    }

    /// The current state of one clip document, or `None` when nothing was saved to it.
    ///
    /// # Errors
    /// Returns an error if storage is unavailable or a stored payload is invalid.
    pub fn web_clip_document(
        &self,
        document_key: &str,
    ) -> Result<Option<WebClipDocument>, ClipError> {
        let conn = self.conn().map_err(|_| ClipError::Storage)?;
        let mut statement = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM {}_web_clips WHERE document_key = ?1 ORDER BY rowid",
                self.prefix
            ))
            .map_err(|_| ClipError::Storage)?;
        let clips = statement
            .query_map(params![document_key], read_row)
            .map_err(|_| ClipError::Storage)?
            .map(|row| row.map_err(|_| ClipError::Storage).and_then(clip_from))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(latest) = clips.last() else {
            return Ok(None);
        };
        let versions: Vec<(String, String)> = clips
            .iter()
            .map(|c| {
                let markdown = c.payload["markdown"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                (c.behavior.clone(), markdown)
            })
            .collect();
        Ok(Some(WebClipDocument {
            document_key: document_key.to_string(),
            title: latest.payload["title"].as_str().map(str::to_string),
            url: latest.url.clone(),
            versions: clips.len(),
            markdown: fold(&versions),
            updated_at: latest.created_at.clone(),
        }))
    }

    /// Clip documents, most recently saved first.
    ///
    /// # Errors
    /// Returns an error if storage is unavailable or a stored payload is invalid.
    pub fn web_clip_documents(&self, limit: usize) -> Result<Vec<WebClipDocument>, ClipError> {
        let keys = {
            let conn = self.conn().map_err(|_| ClipError::Storage)?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT document_key FROM {}_web_clips GROUP BY document_key
                     ORDER BY MAX(rowid) DESC LIMIT ?1",
                    self.prefix
                ))
                .map_err(|_| ClipError::Storage)?;
            let keys = statement
                .query_map(params![limit as i64], |row| row.get::<_, String>(0))
                .map_err(|_| ClipError::Storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ClipError::Storage)?;
            keys
        };
        keys.iter()
            .filter_map(|key| self.web_clip_document(key).transpose())
            .collect()
    }

    /// Search saved versions by literal substring, returning current metadata newest first.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or pagination exceeds SQLite's range.
    pub fn search_web_clip_documents(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WebClipSummary>, ClipError> {
        let limit = i64::try_from(limit).map_err(|_| ClipError::Invalid)?;
        let offset = i64::try_from(offset).map_err(|_| ClipError::Invalid)?;
        let conn = self.conn().map_err(|_| ClipError::Storage)?;
        let mut statement = conn.prepare(&format!(
            "SELECT latest.document_key, json_extract(latest.payload, '$.title'),
                    latest.url, grouped.versions, latest.created_at
             FROM (SELECT document_key, MAX(rowid) AS latest_id, COUNT(*) AS versions
                   FROM {0}_web_clips GROUP BY document_key) grouped
             JOIN {0}_web_clips latest ON latest.rowid = grouped.latest_id
             WHERE ?1 = '' OR EXISTS (
                 SELECT 1 FROM {0}_web_clips saved
                 WHERE saved.document_key = grouped.document_key AND (
                     instr(lower(saved.url), lower(?1)) > 0 OR
                     instr(lower(COALESCE(json_extract(saved.payload, '$.title'), '')), lower(?1)) > 0 OR
                     instr(lower(COALESCE(json_extract(saved.payload, '$.markdown'), '')), lower(?1)) > 0))
             ORDER BY grouped.latest_id DESC LIMIT ?2 OFFSET ?3", self.prefix
        )).map_err(|_| ClipError::Storage)?;
        let rows = statement
            .query_map(params![query, limit, offset], |row| {
                Ok(WebClipSummary {
                    document_key: row.get(0)?,
                    title: row.get(1)?,
                    url: row.get(2)?,
                    versions: row.get(3)?,
                    updated_at: row.get(4)?,
                    data_class: "c2",
                })
            })
            .map_err(|_| ClipError::Storage)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| ClipError::Storage)
    }

    /// Delete every stored version belonging to one clip document.
    ///
    /// # Errors
    /// Returns an error when the key is invalid or storage is unavailable.
    pub fn delete_web_clip_document(&self, document_key: &str) -> Result<usize, ClipError> {
        if document_key.trim().is_empty() || document_key.len() > MAX_DOCUMENT_KEY_BYTES {
            return Err(ClipError::Invalid);
        }
        self.conn()
            .map_err(|_| ClipError::Storage)?
            .execute(
                &format!(
                    "DELETE FROM {}_web_clips WHERE document_key = ?1",
                    self.prefix
                ),
                params![document_key],
            )
            .map_err(|_| ClipError::Storage)
    }

    /// Delete every stored clip version.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable.
    pub fn delete_all_web_clips(&self) -> Result<usize, ClipError> {
        self.conn()
            .map_err(|_| ClipError::Storage)?
            .execute(&format!("DELETE FROM {}_web_clips", self.prefix), [])
            .map_err(|_| ClipError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db_tests::open_test_store;

    fn clip(id: &str, behavior: &str, markdown: &str) -> WebClipInput {
        WebClipInput {
            request_id: id.into(),
            url: "https://example.org/".into(),
            payload: serde_json::json!({
                "version": 1,
                "behavior": behavior,
                "title": "Example",
                "markdown": markdown,
                "destination": {"path": "Clippings/", "name": "Example", "date": "2026-10-28"},
            }),
        }
    }

    #[test]
    fn retries_do_not_replace_a_distinct_capture() {
        let store = open_test_store("web_clips_retry");
        let input = clip("request-1", "create", "one");
        let first = store.put_web_clip(input.clone()).unwrap();
        assert!(
            matches!(&first, WebClipWrite::Created(clip) if store.web_clip("request-1").unwrap().as_ref() == Some(clip))
        );
        assert!(store.web_clip("missing").unwrap().is_none());
        let retry = store.put_web_clip(input.clone()).unwrap();
        assert!(
            matches!((&first, &retry), (WebClipWrite::Created(a), WebClipWrite::Replayed(b)) if a == b)
        );
        let mut changed = input.clone();
        changed.payload["markdown"] = serde_json::json!("two");
        assert!(matches!(
            store.put_web_clip(changed.clone()),
            Err(ClipError::Conflict)
        ));
        changed.request_id = "request-2".into();
        assert!(matches!(
            store.put_web_clip(changed),
            Ok(WebClipWrite::Created(_))
        ));
        assert!(matches!(
            store.put_web_clip(input),
            Ok(WebClipWrite::Replayed(_))
        ));
        assert_eq!(store.web_clip_documents(10).unwrap().len(), 2);
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let store = open_test_store("web_clips_invalid");
        let valid = clip("valid", "create", "x");
        let mut input = valid.clone();
        input.url = "file:///etc/passwd".into();
        assert!(matches!(store.put_web_clip(input), Err(ClipError::Invalid)));
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("behavior", serde_json::json!("delete")),
            ("markdown", serde_json::Value::Null),
        ] {
            let mut input = valid.clone();
            input.payload[field] = value;
            assert!(
                matches!(store.put_web_clip(input), Err(ClipError::Invalid)),
                "{field}"
            );
        }
        let mut input = clip("valid", "append-daily", "x");
        input.payload["destination"]["date"] = serde_json::json!("28.10.2026");
        assert!(matches!(store.put_web_clip(input), Err(ClipError::Invalid)));
        let mut input = clip("valid", "overwrite", "x");
        input.payload["destination"]["name"] = serde_json::json!(" ");
        assert!(matches!(store.put_web_clip(input), Err(ClipError::Invalid)));
        let input = clip("valid", "create", &"x".repeat(MAX_PAYLOAD_BYTES));
        assert!(matches!(
            store.put_web_clip(input),
            Err(ClipError::TooLarge)
        ));
    }

    #[test]
    fn deleting_a_document_removes_all_versions_and_leaves_other_documents() {
        let store = open_test_store("web_clips_delete_document");
        store.put_web_clip(clip("a", "overwrite", "one")).unwrap();
        store
            .put_web_clip(clip("b", "append-specific", "two"))
            .unwrap();
        store.put_web_clip(clip("other", "create", "keep")).unwrap();

        assert_eq!(
            store
                .delete_web_clip_document("note:Clippings/Example")
                .unwrap(),
            2
        );
        assert!(store
            .web_clip_document("note:Clippings/Example")
            .unwrap()
            .is_none());
        assert!(store.web_clip("a").unwrap().is_none());
        assert_eq!(store.web_clip_documents(10).unwrap().len(), 1);
        assert!(matches!(
            store.delete_web_clip_document(" "),
            Err(ClipError::Invalid)
        ));
    }

    #[test]
    fn deleting_a_document_accepts_the_longest_valid_key() {
        let store = open_test_store("web_clips_delete_long_key");
        let mut input = clip("long-key", "overwrite", "body");
        input.payload["destination"]["path"] = serde_json::json!("p".repeat(1024));
        input.payload["destination"]["name"] = serde_json::json!("n".repeat(512));
        let clip = store.put_web_clip(input).unwrap();
        let document_key = match clip {
            WebClipWrite::Created(clip) | WebClipWrite::Replayed(clip) => clip.document_key,
        };

        assert_eq!(document_key.len(), MAX_DOCUMENT_KEY_BYTES);
        assert_eq!(store.delete_web_clip_document(&document_key).unwrap(), 1);
        assert!(store.web_clip_document(&document_key).unwrap().is_none());
    }

    #[test]
    fn delete_all_removes_every_clip_version() {
        let store = open_test_store("web_clips_delete_all");
        store.put_web_clip(clip("a", "create", "one")).unwrap();
        store.put_web_clip(clip("b", "create", "two")).unwrap();

        assert_eq!(store.delete_all_web_clips().unwrap(), 2);
        assert!(store.web_clip_documents(10).unwrap().is_empty());
        assert_eq!(store.delete_all_web_clips().unwrap(), 0);
    }

    #[test]
    fn behaviors_fold_into_one_document_without_losing_a_version() {
        let store = open_test_store("web_clips_documents");
        store.put_web_clip(clip("a", "overwrite", "body")).unwrap();
        store
            .put_web_clip(clip("b", "append-specific", "after"))
            .unwrap();
        store
            .put_web_clip(clip("c", "prepend-specific", "before"))
            .unwrap();
        let doc = store
            .web_clip_document("note:Clippings/Example")
            .unwrap()
            .unwrap();
        assert_eq!(doc.markdown, "before\n\nbody\n\nafter");
        assert_eq!(doc.versions, 3);
        store.put_web_clip(clip("d", "overwrite", "fresh")).unwrap();
        let doc = store
            .web_clip_document("note:Clippings/Example")
            .unwrap()
            .unwrap();
        assert_eq!((doc.markdown.as_str(), doc.versions), ("fresh", 4));
        assert!(store.web_clip("a").unwrap().is_some());

        store
            .put_web_clip(clip("e", "append-daily", "one"))
            .unwrap();
        store
            .put_web_clip(clip("f", "prepend-daily", "zero"))
            .unwrap();
        let day = store
            .web_clip_document("daily:2026-10-28")
            .unwrap()
            .unwrap();
        assert_eq!(day.markdown, "zero\n\none");

        store
            .put_web_clip(clip("g", "create", "same name"))
            .unwrap();
        store
            .put_web_clip(clip("h", "create", "same name"))
            .unwrap();
        let keys: Vec<String> = store
            .web_clip_documents(10)
            .unwrap()
            .into_iter()
            .map(|d| d.document_key)
            .collect();
        assert_eq!(
            keys,
            [
                "clip:h",
                "clip:g",
                "daily:2026-10-28",
                "note:Clippings/Example"
            ]
        );
        assert!(store.web_clip_document("note:missing").unwrap().is_none());
    }
}
