//! Classified wire responses for the project list and record readers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ProjectKind, ProjectRecord, ProjectSummary};

/// A project summary with the strictest class of its source records.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ClassifiedProjectSummary {
    #[serde(flatten)]
    pub summary: ProjectSummary,
    pub data_class: String,
}

/// The successful response from the project list route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ProjectListResponse {
    pub projects: Vec<ClassifiedProjectSummary>,
}

/// A record's classified read projection; omitted content is not an empty record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RecordRead {
    pub path: String,
    pub kind: ProjectKind,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub archived: bool,
    pub data_class: String,
    pub data_class_rationale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// A list read omits content, so the client record holds it empty. The wire carries no
/// project name; readers take it from the path.
impl From<RecordRead> for ProjectRecord {
    fn from(read: RecordRead) -> Self {
        Self {
            path: read.path,
            project: String::new(),
            kind: read.kind.as_str().to_owned(),
            title: read.title,
            content: read.content.unwrap_or_default(),
            done: read.done,
            due: read.due,
            priority: read.priority,
        }
    }
}

/// The successful response from a project's record-list route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ProjectReadResponse {
    pub records: Vec<RecordRead>,
}

/// The successful response from the single-record route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RecordReadResponse {
    pub record: RecordRead,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(content: Option<&str>) -> RecordRead {
        RecordRead {
            path: "P/a.md".into(),
            kind: ProjectKind::Task,
            title: "A".into(),
            done: true,
            due: Some("2026-11-20".into()),
            priority: 3,
            archived: false,
            data_class: "c1".into(),
            data_class_rationale: "own".into(),
            content: content.map(str::to_owned),
            updated_at: Some("stamp".into()),
        }
    }

    #[test]
    fn a_server_read_decodes_into_the_client_record() {
        let wire = serde_json::to_value(RecordReadResponse {
            record: read(Some("  body\n")),
        })
        .unwrap();
        let record: ProjectRecord = serde_json::from_value::<RecordReadResponse>(wire)
            .unwrap()
            .record
            .into();
        assert_eq!(record.path, "P/a.md");
        assert_eq!(record.kind, "task");
        assert_eq!(record.content, "  body\n");
        assert!(record.done);
        assert_eq!(record.due.as_deref(), Some("2026-11-20"));
        assert_eq!(record.priority, 3);
    }

    #[test]
    fn a_list_read_without_content_keeps_the_record() {
        let wire = serde_json::to_value(ProjectReadResponse {
            records: vec![read(None)],
        })
        .unwrap();
        assert!(wire["records"][0].get("content").is_none());
        let records = serde_json::from_value::<ProjectReadResponse>(wire)
            .unwrap()
            .records;
        let record = ProjectRecord::from(records.into_iter().next().unwrap());
        assert_eq!(record.content, "");
        assert_eq!(record.title, "A");
    }
}
