use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 100;
pub const MAX_TEXT_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ProfileCategory {
    #[serde(rename = "preference")]
    Preference,
    #[serde(rename = "principle")]
    Principle,
    #[serde(rename = "boundary")]
    Boundary,
    #[serde(rename = "interaction_style")]
    InteractionStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Classification {
    #[serde(rename = "c0")]
    C0,
    #[serde(rename = "c1")]
    C1,
    #[serde(rename = "c2")]
    C2,
    #[serde(rename = "c3")]
    C3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Pi,
    Opencode,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Opencode => "opencode",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    Text(String),
    TextList(Vec<String>),
    Boolean(bool),
    Integer(i64),
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldInput {
    pub category: ProfileCategory,
    pub value: FieldValue,
    pub classification: Classification,
    #[serde(default)]
    pub export_to: Vec<Harness>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatementInput {
    pub category: ProfileCategory,
    pub text: String,
    pub classification: Classification,
    #[serde(default)]
    pub export_to: Vec<Harness>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileInput {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldInput>,
    #[serde(default)]
    pub statements: BTreeMap<String, StatementInput>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileStatement {
    pub category: ProfileCategory,
    pub text: String,
    pub classification: Classification,
    pub export_to: Vec<Harness>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoredProfile {
    pub schema_version: u32,
    pub revision: u64,
    pub fields: BTreeMap<String, FieldInput>,
    pub statements: BTreeMap<String, ProfileStatement>,
    pub updated_at: String,
}

/// The canonical profile, with no personal values in its Debug representation.
pub struct Profile {
    pub revision: u64,
    pub input: ProfileInput,
    pub updated_at: String,
}

impl std::fmt::Debug for ProfileInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileInput")
            .field("schema_version", &self.schema_version)
            .field("field_count", &self.fields.len())
            .field("statement_count", &self.statements.len())
            .finish()
    }
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("revision", &self.revision)
            .field("field_count", &self.input.fields.len())
            .field("statement_count", &self.input.statements.len())
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl std::fmt::Debug for FieldValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Text(_) => "text",
            Self::TextList(_) => "text_list",
            Self::Boolean(_) => "boolean",
            Self::Integer(_) => "integer",
        };
        f.debug_struct("FieldValue")
            .field("kind", &kind)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl std::fmt::Debug for FieldInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldInput")
            .field("category", &self.category)
            .field("value", &"[REDACTED]")
            .field("classification", &self.classification)
            .field("export_to", &self.export_to)
            .finish()
    }
}

impl std::fmt::Debug for StatementInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatementInput")
            .field("category", &self.category)
            .field("text", &"[REDACTED]")
            .field("classification", &self.classification)
            .field("export_to", &self.export_to)
            .finish()
    }
}

impl std::fmt::Debug for ProfileStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileStatement")
            .field("category", &self.category)
            .field("text", &"[REDACTED]")
            .field("classification", &self.classification)
            .field("export_to", &self.export_to)
            .finish()
    }
}

impl std::fmt::Debug for StoredProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredProfile")
            .field("schema_version", &self.schema_version)
            .field("revision", &self.revision)
            .field("field_count", &self.fields.len())
            .field("statement_count", &self.statements.len())
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl ProfileInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported profile schema_version {}",
                self.schema_version
            ));
        }
        if self.fields.len() + self.statements.len() > MAX_ENTRIES {
            return Err(format!("profile exceeds {MAX_ENTRIES} total entries"));
        }
        for (id, field) in &self.fields {
            validate_id(id)?;
            validate_value(&field.value)?;
            validate_export(field.classification, &field.export_to, id)?;
        }
        for (id, statement) in &self.statements {
            validate_id(id)?;
            validate_text(&statement.text)?;
            validate_export(statement.classification, &statement.export_to, id)?;
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        || !id.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(format!(
            "entry id {id:?} must start with a lowercase letter and contain only lowercase letters, digits, '.', '_' or '-' (max 80 bytes)"
        ));
    }
    Ok(())
}

fn validate_value(value: &FieldValue) -> Result<(), String> {
    match value {
        FieldValue::Text(text) => validate_text(text),
        FieldValue::TextList(items) => {
            if items.len() > 50 {
                return Err("a text-list field may contain at most 50 items".into());
            }
            for item in items {
                validate_text(item)?;
            }
            Ok(())
        }
        FieldValue::Boolean(_) | FieldValue::Integer(_) => Ok(()),
    }
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("text values must not be empty".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!(
            "text values may contain at most {MAX_TEXT_CHARS} characters"
        ));
    }
    if text.chars().any(char::is_control) {
        return Err("text values must not contain control characters".into());
    }
    if text.contains("<!--") || text.contains("-->") || text.contains('\n') || text.contains('\r') {
        return Err("text values must not contain newlines or HTML comment markers".into());
    }
    Ok(())
}

fn validate_export(
    classification: Classification,
    targets: &[Harness],
    id: &str,
) -> Result<(), String> {
    let mut seen = Vec::new();
    for target in targets {
        if seen.contains(target) {
            return Err(format!(
                "entry {id:?} lists export target '{}' more than once",
                target.as_str()
            ));
        }
        seen.push(*target);
    }
    if !targets.is_empty() && matches!(classification, Classification::C2 | Classification::C3) {
        return Err(format!(
            "entry {id:?} is classified C2/C3 and cannot be exported to assistant instructions"
        ));
    }
    Ok(())
}

impl From<ProfileInput> for StoredProfile {
    fn from(input: ProfileInput) -> Self {
        let statements = input
            .statements
            .into_iter()
            .map(|(id, statement)| {
                (
                    id,
                    ProfileStatement {
                        category: statement.category,
                        text: statement.text,
                        classification: statement.classification,
                        export_to: statement.export_to,
                    },
                )
            })
            .collect();
        Self {
            schema_version: input.schema_version,
            revision: 0,
            fields: input.fields,
            statements,
            updated_at: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProfileInput {
        ProfileInput {
            schema_version: SCHEMA_VERSION,
            fields: BTreeMap::from([(
                "interaction.verbosity".into(),
                FieldInput {
                    category: ProfileCategory::InteractionStyle,
                    value: FieldValue::Text("concise".into()),
                    classification: Classification::C1,
                    export_to: vec![Harness::Claude],
                },
            )]),
            statements: BTreeMap::from([(
                "boundaries.external_actions".into(),
                StatementInput {
                    category: ProfileCategory::Boundary,
                    text: "Ask before external side effects.".into(),
                    classification: Classification::C1,
                    export_to: vec![Harness::Claude],
                },
            )]),
        }
    }

    #[test]
    fn accepts_hybrid_profile_and_allowlists() {
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn refuses_export_of_c2_or_c3_values() {
        let mut profile = sample();
        profile
            .fields
            .get_mut("interaction.verbosity")
            .unwrap()
            .classification = Classification::C2;
        assert!(profile.validate().unwrap_err().contains("C2/C3"));
    }

    #[test]
    fn rejects_duplicate_targets_and_untrusted_ids() {
        let mut profile = sample();
        profile
            .fields
            .get_mut("interaction.verbosity")
            .unwrap()
            .export_to = vec![Harness::Claude, Harness::Claude];
        assert!(profile.validate().unwrap_err().contains("more than once"));
        profile.fields.clear();
        profile.statements.insert(
            "../bad".into(),
            sample().statements.into_values().next().unwrap(),
        );
        assert!(profile.validate().unwrap_err().contains("entry id"));
    }

    #[test]
    fn debug_output_does_not_reveal_values() {
        let profile = sample();
        let rendered = format!("{profile:?}");
        assert!(!rendered.contains("Ask before"));
        assert!(!rendered.contains("concise"));
        assert!(rendered.contains("REDACTED") || rendered.contains("field_count"));
        assert!(!format!("{:?}", FieldValue::Text("concise".into())).contains("concise"));
    }
}
