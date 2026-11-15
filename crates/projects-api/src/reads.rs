//! Classified wire responses for the project list and record readers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ProjectKind, ProjectSummary};

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
