//! Typed API contract, wire models, and route constants for the Sjel projects capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-projects`)
//! share these definitions to eliminate wire model drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_PROJECTS: &str = "/api/projects";
pub const ROUTE_RECORDS: &str = "/api/records";
pub const ROUTE_RECORD: &str = "/api/record";
pub const ROUTE_RECORD_PROPERTY: &str = "/api/record/property";
pub const ROUTE_RECORD_REMOVE: &str = "/api/record/remove";
pub const ROUTE_RECORD_RENAME: &str = "/api/record/rename";
pub const ROUTE_PROJECT: &str = "/api/project";
pub const ROUTE_FILES: &str = "/api/files";
pub const ROUTE_FILE: &str = "/api/file";
pub const ROUTE_TASKS: &str = "/api/tasks";
pub const ROUTE_TASKS_DONE: &str = "/api/tasks/done";
pub const ROUTE_NOTES: &str = "/api/notes";
pub const ROUTE_MAIL_PROPOSALS: &str = "/api/mail-proposals";
pub const ROUTE_MAIL_PROPOSALS_WITHDRAW: &str = "/api/mail-proposals/withdraw";
pub const ROUTE_MAIL_DECISION: &str = "/api/mail-proposals/{id}/decision";
pub const ROUTE_INVENTORY_LINKS: &str = "/api/inventory-links";
pub const ROUTE_INVENTORY_LINKS_REMOVE: &str = "/api/inventory-links/remove";

// ============================================================================
// Core Enums and Models
// ============================================================================

/// The states a task can be in, in the order a picker shows them. `done:` follows `status:`.
pub const TASK_STATUSES: [&str; 6] = ["open", "doing", "waiting", "blocked", "done", "cancelled"];

/// A status that closes the task, so `done:` is `true`. A cancelled task is closed but not done.
pub fn status_closes(status: &str) -> bool {
    matches!(status, "done" | "cancelled")
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Project,
    Task,
    Note,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Project => "project",
            ProjectKind::Task => "task",
            ProjectKind::Note => "note",
        }
    }
}

pub type Kind = ProjectKind;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct ProjectSummary {
    pub name: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub open: usize,
    #[serde(default)]
    pub done: usize,
    #[serde(default)]
    pub notes: usize,
}

/// One record of `GET /projects/api/project` and `GET /projects/api/record`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct ProjectRecord {
    pub path: String,
    #[serde(default)]
    pub project: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub priority: u8,
}

/// What `POST /projects/api/record/rename` moved, and which records it pointed at the new title.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct RenamedRecord {
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub failed: Vec<String>,
}

/// One task across projects (`GET /projects/api/tasks`, and the reply to every task write).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct GlobalTask {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub done: bool,
    pub due: Option<String>,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub project: String,
    /// Every frontmatter key, lists flattened to `a, b`.
    #[serde(default)]
    pub fields: std::collections::BTreeMap<String, String>,
}

impl GlobalTask {
    /// The task's status: its `status:` key when that is one of [`TASK_STATUSES`] and agrees with
    /// `done:`, else what `done:` implies.
    pub fn status(&self) -> &str {
        match self.fields.get("status").map(String::as_str) {
            Some(status)
                if TASK_STATUSES.contains(&status) && status_closes(status) == self.done =>
            {
                status
            }
            _ if self.done => "done",
            _ => "open",
        }
    }
}

/// One note across projects (`GET /projects/api/records` or `/api/notes`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct GlobalNote {
    pub path: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
}

// ============================================================================
// Mail Proposals
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectMailProposal {
    pub id: i64,
    pub version: i64,
    pub project: String,
    pub title: String,
    pub evidence: String,
    pub source_id: String,
    pub data_class: String,
    #[serde(default)]
    pub task_suggestion: Option<String>,
    pub status: String,
}

impl ProjectMailProposal {
    pub fn gmail_url(&self) -> Option<String> {
        let mut parts = self.source_id.split(':');
        let (Some("mail"), Some("v1"), Some(thread), Some(message), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return None;
        };
        let valid = |id: &str| {
            !id.is_empty()
                && id.len() <= 120
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        };
        (valid(thread) && valid(message))
            .then(|| format!("https://mail.google.com/mail/u/0/#all/{thread}"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectMailPage {
    pub proposals: Vec<ProjectMailProposal>,
    pub limit: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectMailTarget {
    pub path: String,
    pub title: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectMailDecision {
    pub applied: bool,
    pub rendered: bool,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub render_error: Option<String>,
}

// ============================================================================
// Inventory Links & Identities
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectInventoryLink {
    pub record_path: String,
    pub item_ref: String,
    pub verified_revision: i64,
    pub reviewed_at: String,
    #[serde(default)]
    pub provenance: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectLinkRecord {
    pub path: String,
    pub title: String,
    pub content: String,
    pub updated_at: String,
    pub kind: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InventoryIdentity {
    pub typed_ref: String,
    pub raw_id: String,
    pub revision: i64,
    #[serde(default)]
    pub state: Option<String>,
}

pub fn inventory_reference(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 256 {
        return None;
    }
    const HEX: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(4 + id.len() * 2);
    result.push_str("inv:");
    for byte in id.bytes() {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    Some(result)
}

pub fn decode_item_ref(reference: &str) -> Result<String, &'static str> {
    let hex = reference
        .strip_prefix("inv:")
        .ok_or("invalid Inventory reference")?
        .as_bytes();
    if hex.is_empty() || hex.len() > 512 || hex.len() % 2 != 0 {
        return Err("invalid Inventory reference length");
    }
    fn digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        }
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for [hi, lo] in hex.as_chunks::<2>().0 {
        bytes.push(
            digit(*hi).ok_or("Inventory reference must use lowercase hex")? * 16
                + digit(*lo).ok_or("Inventory reference must use lowercase hex")?,
        );
    }
    String::from_utf8(bytes).map_err(|_| "Inventory reference must encode UTF-8")
}

pub fn project_ref(path: &str) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::from("project:");
    for b in path.bytes() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}

// ============================================================================
// Request Schemas
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AddTaskRequest {
    pub project: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub priority: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SetDoneRequest {
    pub id: String,
    pub done: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AddProjectRequest {
    pub name: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SaveRecordRequest {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SetPropertyRequest {
    pub path: String,
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AddNoteRequest {
    pub project: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct RemoveRecordRequest {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ArchiveProjectRequest {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct RenameRecordRequest {
    pub path: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AttachFileRequest {
    pub path: String,
    pub base64: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailDecisionRequest {
    pub version: i64,
    pub decision: String,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InventoryLinkDecisionRequest {
    pub project: String,
    pub path: String,
    pub base: String,
    pub updated_at: String,
    pub item_ref: String,
    pub reviewed: bool,
}
