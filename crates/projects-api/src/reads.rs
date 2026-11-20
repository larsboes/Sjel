//! Classified wire responses for the project list and record readers.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{GlobalNote, ProjectKind, ProjectRecord, ProjectSummary};

/// A project summary with the strictest class of its source records.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ClassifiedProjectSummary {
    #[serde(flatten)]
    pub summary: ProjectSummary,
    pub data_class: String,
}

/// The successful response from the project list route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ProjectListResponse<T = ClassifiedProjectSummary> {
    pub projects: Vec<T>,
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
pub struct RecordReadResponse<T = RecordRead> {
    pub record: T,
}

/// One live record as the task and record lists serve it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct TaskRead {
    pub id: String,
    pub kind: ProjectKind,
    pub title: String,
    pub done: bool,
    pub due: Option<String>,
    pub priority: u8,
    pub summary: Option<String>,
    pub projects: Vec<String>,
    /// The project folder the record lives in.
    pub project: String,
    /// Every frontmatter key, lists flattened to `a, b`.
    pub fields: BTreeMap<String, String>,
    /// The dashboard page that shows the record.
    pub uri: String,
    pub data_class: String,
    pub data_class_rationale: String,
}

/// The successful response from a task write.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct TaskResponse<T = TaskRead> {
    pub task: T,
}

/// The successful response from the task list route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct TaskListResponse<T = TaskRead> {
    pub tasks: Vec<T>,
}

impl From<TaskRead> for GlobalNote {
    fn from(read: TaskRead) -> Self {
        Self {
            url: GlobalNote::web_url(read.fields.get("url").map(String::as_str)),
            path: read.id,
            kind: read.kind.as_str().to_owned(),
            title: read.title,
            project: read.project,
        }
    }
}

/// The successful response from the record-list route, for one kind.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct RecordListResponse {
    pub records: Vec<TaskRead>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_list_client_reads_classified_and_legacy_summaries() {
        let legacy = serde_json::json!({"projects": [{"name": "Example"}]});
        let classified = serde_json::json!({
            "projects": [{"name": "Example", "data_class": "c1"}]
        });
        let legacy: ProjectListResponse<ProjectSummary> = serde_json::from_value(legacy).unwrap();
        let current: ProjectListResponse<ProjectSummary> =
            serde_json::from_value(classified.clone()).unwrap();
        let server: ProjectListResponse = serde_json::from_value(classified).unwrap();
        assert_eq!(legacy, current);
        assert_eq!(server.projects[0].data_class, "c1");
    }

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
        let link: RecordReadResponse<crate::ProjectLinkRecord> =
            serde_json::from_value(wire.clone()).unwrap();
        let mail: RecordReadResponse<crate::ProjectMailTarget> =
            serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(link.record.updated_at, "stamp");
        assert_eq!(link.record.content, "  body\n");
        assert_eq!(mail.record.content, "  body\n");
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

    #[test]
    fn a_record_list_decodes_into_client_notes() {
        let wire = serde_json::to_value(RecordListResponse {
            records: vec![TaskRead {
                id: "Home/Plan.md".into(),
                kind: ProjectKind::Note,
                title: "Plan".into(),
                done: false,
                due: None,
                priority: 0,
                summary: None,
                projects: vec!["Home".into()],
                project: "Home".into(),
                fields: BTreeMap::new(),
                uri: "/projects?project=Home".into(),
                data_class: "c1".into(),
                data_class_rationale: "own".into(),
            }],
        })
        .unwrap();
        let notes: Vec<GlobalNote> = serde_json::from_value::<RecordListResponse>(wire)
            .unwrap()
            .records
            .into_iter()
            .map(GlobalNote::from)
            .collect();
        assert_eq!(
            notes,
            vec![GlobalNote {
                path: "Home/Plan.md".into(),
                kind: "note".into(),
                title: "Plan".into(),
                project: "Home".into(),
                url: None,
            }]
        );
    }

    #[test]
    fn a_note_carries_only_a_web_url_from_its_frontmatter() {
        let note = |url: &str| {
            GlobalNote::from(TaskRead {
                id: "Atlas/Links/Site/Home.md".into(),
                kind: ProjectKind::Note,
                title: "Home".into(),
                done: false,
                due: None,
                priority: 0,
                summary: None,
                projects: Vec::new(),
                project: "Atlas".into(),
                fields: BTreeMap::from([("url".into(), url.into())]),
                uri: String::new(),
                data_class: "c2".into(),
                data_class_rationale: String::new(),
            })
            .url
        };
        assert_eq!(
            note(" https://example.com/a "),
            Some("https://example.com/a".into())
        );
        assert_eq!(
            note("HTTP://example.com"),
            Some("HTTP://example.com".into())
        );
        assert_eq!(note("javascript:alert(1)"), None);
        assert_eq!(note("file:///etc/hosts"), None);
    }
}
