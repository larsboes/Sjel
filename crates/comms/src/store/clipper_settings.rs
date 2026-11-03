use super::*;

const MAX_TEMPLATES_BYTES: usize = 1024 * 1024;
const BEHAVIORS: [&str; 6] = [
    "create",
    "overwrite",
    "append-specific",
    "prepend-specific",
    "append-daily",
    "prepend-daily",
];

/// The Sjel Clipper's templates as Sjel owns them. Revision 0 means Sjel holds none yet.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClipperSettings {
    pub revision: i64,
    pub templates: Option<serde_json::Value>,
}

/// Why a settings write was refused.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SettingsError {
    #[error("templates must be a list of templates with an id, a name and a known behavior")]
    Invalid,
    #[error("templates exceed the size limit")]
    TooLarge,
    #[error("the settings changed since they were read")]
    Stale,
    #[error("Sjel already holds templates")]
    AlreadySeeded,
    #[error("settings storage unavailable")]
    Storage,
}

/// Upstream's template shape, checked only where Sjel depends on it. Everything else in a
/// template is the extension's and is stored as sent.
fn validate(templates: &serde_json::Value) -> Result<String, SettingsError> {
    let list = templates.as_array().ok_or(SettingsError::Invalid)?;
    let mut ids = std::collections::HashSet::new();
    for template in list {
        let text = |key: &str| template.get(key).and_then(serde_json::Value::as_str);
        let valid = text("id").is_some_and(|id| !id.is_empty() && ids.insert(id.to_string()))
            && text("name").is_some_and(|name| !name.trim().is_empty())
            && text("behavior").is_some_and(|b| BEHAVIORS.contains(&b));
        if !valid {
            return Err(SettingsError::Invalid);
        }
    }
    let body = serde_json::to_string(templates).map_err(|_| SettingsError::Invalid)?;
    if body.len() > MAX_TEMPLATES_BYTES {
        return Err(SettingsError::TooLarge);
    }
    Ok(body)
}

impl Store {
    /// The current clipper settings.
    ///
    /// # Errors
    /// Storage is unavailable or the stored templates are not JSON.
    pub fn clipper_settings(&self) -> Result<ClipperSettings, SettingsError> {
        let conn = self.conn().map_err(|_| SettingsError::Storage)?;
        let row = conn
            .query_row(
                &format!(
                    "SELECT revision, templates FROM {}_clipper_settings WHERE id = 1",
                    self.prefix
                ),
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| SettingsError::Storage)?;
        match row {
            None => Ok(ClipperSettings {
                revision: 0,
                templates: None,
            }),
            Some((revision, templates)) => Ok(ClipperSettings {
                revision,
                templates: Some(
                    serde_json::from_str(&templates).map_err(|_| SettingsError::Storage)?,
                ),
            }),
        }
    }

    /// Accept the extension's templates once, while Sjel holds none. Afterwards Sjel is the
    /// source and the extension only reads.
    ///
    /// # Errors
    /// The templates are invalid or too large, Sjel already holds templates, or storage fails.
    pub fn seed_clipper_templates(
        &self,
        templates: &serde_json::Value,
    ) -> Result<ClipperSettings, SettingsError> {
        let body = validate(templates)?;
        let conn = self.conn().map_err(|_| SettingsError::Storage)?;
        let inserted = conn
            .execute(
                &format!(
                    "INSERT INTO {}_clipper_settings (id, templates, revision) VALUES (1, ?1, 1)
                     ON CONFLICT (id) DO NOTHING",
                    self.prefix
                ),
                params![body],
            )
            .map_err(|_| SettingsError::Storage)?;
        if inserted == 0 {
            return Err(SettingsError::AlreadySeeded);
        }
        self.clipper_settings()
    }

    /// Replace the templates when `base_revision` is still current.
    ///
    /// # Errors
    /// The templates are invalid or too large, the base revision is stale, or storage fails.
    pub fn put_clipper_templates(
        &self,
        templates: &serde_json::Value,
        base_revision: i64,
    ) -> Result<ClipperSettings, SettingsError> {
        let body = validate(templates)?;
        let conn = self.conn().map_err(|_| SettingsError::Storage)?;
        let changed = if base_revision == 0 {
            conn.execute(
                &format!(
                    "INSERT INTO {}_clipper_settings (id, templates, revision) VALUES (1, ?1, 1)
                     ON CONFLICT (id) DO NOTHING",
                    self.prefix
                ),
                params![body],
            )
        } else {
            conn.execute(
                &format!(
                    "UPDATE {}_clipper_settings SET templates = ?1, revision = revision + 1,
                     updated_at = {} WHERE id = 1 AND revision = ?2",
                    self.prefix,
                    sjel_store::NOW
                ),
                params![body, base_revision],
            )
        }
        .map_err(|_| SettingsError::Storage)?;
        if changed == 0 {
            return Err(SettingsError::Stale);
        }
        self.clipper_settings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db_tests::open_test_store;

    fn template(id: &str, behavior: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "name": format!("T {id}"), "behavior": behavior, "noteContentFormat": "{{content}}"})
    }

    #[test]
    fn the_extension_seeds_once_and_then_sjel_owns_the_templates() {
        let store = open_test_store("clipper_settings_seed");
        assert_eq!(
            store.clipper_settings().unwrap(),
            ClipperSettings {
                revision: 0,
                templates: None
            }
        );
        let first = serde_json::json!([template("a", "create")]);
        assert_eq!(store.seed_clipper_templates(&first).unwrap().revision, 1);
        let other = serde_json::json!([template("b", "create")]);
        assert_eq!(
            store.seed_clipper_templates(&other),
            Err(SettingsError::AlreadySeeded)
        );
        assert_eq!(store.clipper_settings().unwrap().templates, Some(first));
    }

    #[test]
    fn an_edit_needs_the_current_revision() {
        let store = open_test_store("clipper_settings_put");
        let one = serde_json::json!([template("a", "create")]);
        assert_eq!(store.put_clipper_templates(&one, 0).unwrap().revision, 1);
        assert_eq!(
            store.put_clipper_templates(&one, 0),
            Err(SettingsError::Stale)
        );
        let two = serde_json::json!([template("a", "overwrite"), template("b", "append-daily")]);
        assert_eq!(store.put_clipper_templates(&two, 1).unwrap().revision, 2);
        assert_eq!(
            store.put_clipper_templates(&one, 1),
            Err(SettingsError::Stale)
        );
        assert_eq!(store.clipper_settings().unwrap().templates, Some(two));
    }

    #[test]
    fn templates_sjel_depends_on_are_validated() {
        let store = open_test_store("clipper_settings_invalid");
        for bad in [
            serde_json::json!({"not": "a list"}),
            serde_json::json!([{"id": "a", "name": "x"}]),
            serde_json::json!([template("a", "delete")]),
            serde_json::json!([template("a", "create"), template("a", "create")]),
            serde_json::json!([{"id": "", "name": "x", "behavior": "create"}]),
        ] {
            assert_eq!(
                store.put_clipper_templates(&bad, 0),
                Err(SettingsError::Invalid),
                "{bad}"
            );
        }
        let huge = serde_json::json!([{"id": "a", "name": "x", "behavior": "create", "noteContentFormat": "x".repeat(MAX_TEMPLATES_BYTES)}]);
        assert_eq!(
            store.put_clipper_templates(&huge, 0),
            Err(SettingsError::TooLarge)
        );
    }
}
