//! Capture inbox: a link or a thought, stored raw before anything else looks at it.

use super::*;

pub const CAPTURE_TAGS: [&str; 4] = ["buy", "look", "go", "learn"];
pub const CAPTURE_STATUSES: [&str; 3] = ["new", "done", "dismissed"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Capture {
    pub id: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub tag: Option<String>,
    pub status: String,
    /// Where the item went: `feed:<id>` or `inventory:<id>`.
    pub routed_to: Option<String>,
    pub client: Option<String>,
    pub created_at: String,
}

/// What `PATCH /captures/{id}` may change. An absent field stays as it is.
#[derive(Debug, Default, Deserialize)]
pub struct CapturePatch {
    pub text: Option<String>,
    pub tag: Option<String>,
    pub status: Option<String>,
    pub routed_to: Option<String>,
}

/// Splits `#buy Proper shoes` into the tag and the rest. Only a known tag is taken; any other
/// `#word` stays in the text.
pub fn split_tag(text: &str) -> (Option<String>, String) {
    let mut tag = None;
    let mut rest = Vec::new();
    for word in text.split_whitespace() {
        match word.strip_prefix('#') {
            Some(t) if tag.is_none() && CAPTURE_TAGS.contains(&t.to_ascii_lowercase().as_str()) => {
                tag = Some(t.to_ascii_lowercase())
            }
            _ => rest.push(word),
        }
    }
    (tag, rest.join(" "))
}

fn check_tag(tag: Option<&str>) -> Result<(), String> {
    match tag {
        Some(t) if !CAPTURE_TAGS.contains(&t) => Err(format!(
            "unknown tag '{t}', expected one of: {}",
            CAPTURE_TAGS.join(", ")
        )),
        _ => Ok(()),
    }
}

impl Store {
    pub fn add_capture(
        &self,
        text: Option<&str>,
        url: Option<&str>,
        tag: Option<&str>,
        client: Option<&str>,
    ) -> Result<Capture, Box<dyn std::error::Error>> {
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        let url = url.map(str::trim).filter(|u| !u.is_empty());
        if text.is_none() && url.is_none() {
            return Err("a capture needs text or a url".into());
        }
        check_tag(tag)?;
        let mut hasher = Sha256::new();
        hasher.update(format!("{text:?}|{url:?}|{}", epoch_nanos()));
        let id: String = hasher.finalize()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {}_captures (id, text, url, tag, client) VALUES (?1, ?2, ?3, ?4, ?5)",
                self.prefix
            ),
            params![id, text, url, tag, client],
        )?;
        drop(conn);
        Ok(self.capture(&id)?.ok_or("capture vanished after insert")?)
    }

    pub fn capture(&self, id: &str) -> Result<Option<Capture>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {CAPTURE_COLUMNS} FROM {}_captures WHERE id = ?1", self.prefix),
                params![id],
                row_to_capture,
            )
            .optional()?)
    }

    /// Newest first. `status` of `None` returns every capture.
    pub fn captures(&self, status: Option<&str>) -> Result<Vec<Capture>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {CAPTURE_COLUMNS} FROM {}_captures
             WHERE ?1 IS NULL OR status = ?1 ORDER BY created_at DESC",
            self.prefix
        ))?;
        let rows = stmt.query_map(params![status], row_to_capture)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `Ok(None)` when no capture has this id. An empty `tag` clears it.
    pub fn patch_capture(
        &self,
        id: &str,
        patch: CapturePatch,
    ) -> Result<Option<Capture>, Box<dyn std::error::Error>> {
        let Some(mut c) = self.capture(id)? else {
            return Ok(None);
        };
        if let Some(text) = patch.text {
            c.text = Some(text.trim().to_string()).filter(|t| !t.is_empty());
        }
        if let Some(tag) = patch.tag {
            c.tag = Some(tag).filter(|t| !t.is_empty());
        }
        if let Some(status) = patch.status {
            c.status = status;
        }
        if let Some(routed_to) = patch.routed_to {
            c.routed_to = Some(routed_to).filter(|r| !r.is_empty());
        }
        if c.text.is_none() && c.url.is_none() {
            return Err("a capture needs text or a url".into());
        }
        check_tag(c.tag.as_deref())?;
        if !CAPTURE_STATUSES.contains(&c.status.as_str()) {
            return Err(format!(
                "invalid status '{}', expected one of: {}",
                c.status,
                CAPTURE_STATUSES.join(", ")
            )
            .into());
        }
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "UPDATE {}_captures SET text = ?2, tag = ?3, status = ?4, routed_to = ?5 WHERE id = ?1",
                self.prefix
            ),
            params![id, c.text, c.tag, c.status, c.routed_to],
        )?;
        Ok(Some(c))
    }
}

const CAPTURE_COLUMNS: &str = "id, text, url, tag, status, routed_to, client, created_at";

fn row_to_capture(row: &Row<'_>) -> rusqlite::Result<Capture> {
    Ok(Capture {
        id: row.get(0)?,
        text: row.get(1)?,
        url: row.get(2)?,
        tag: row.get(3)?,
        status: row.get(4)?,
        routed_to: row.get(5)?,
        client: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn epoch_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db_tests::open_test_store;

    #[test]
    fn a_known_hashtag_becomes_the_tag() {
        assert_eq!(split_tag("#buy Proper shoes"), (Some("buy".into()), "Proper shoes".into()));
        assert_eq!(split_tag("shoes #Go"), (Some("go".into()), "shoes".into()));
        assert_eq!(split_tag("C# #rust"), (None, "C# #rust".into()));
    }

    #[test]
    fn a_capture_round_trips_and_patches() {
        let store = open_test_store("captures");
        assert!(store.add_capture(None, Some("  "), None, None).is_err());
        assert!(store.add_capture(Some("x"), None, Some("eat"), None).is_err());

        let c = store
            .add_capture(Some("Proper shoes"), None, Some("buy"), Some("cli"))
            .unwrap();
        assert_eq!(c.status, "new");
        assert_eq!(store.captures(Some("new")).unwrap().len(), 1);

        let patched = store
            .patch_capture(
                &c.id,
                CapturePatch {
                    status: Some("done".into()),
                    routed_to: Some("inventory:shoes".into()),
                    tag: Some(String::new()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(patched.tag, None);
        assert_eq!(store.capture(&c.id).unwrap().unwrap(), patched);
        assert!(store.captures(Some("new")).unwrap().is_empty());
        assert!(store
            .patch_capture(&c.id, CapturePatch { text: Some(" ".into()), ..Default::default() })
            .is_err());
        assert!(store.patch_capture("nope", CapturePatch::default()).unwrap().is_none());
    }
}
