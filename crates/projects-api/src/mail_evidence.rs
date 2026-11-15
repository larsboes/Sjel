//! Source claims assigned to an existing project, without task mutation authority.

use serde::{Deserialize, Serialize};

/// The only event accepted by the project evidence queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ProjectEvidenceEvent {
    #[serde(rename = "project_evidence")]
    Evidence,
}

impl ProjectEvidenceEvent {
    /// The stable wire and storage label.
    pub fn as_str(self) -> &'static str {
        "project_evidence"
    }
}

/// The body of `POST /api/mail-proposals`, shared by intake and Projects.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MailProposalInput {
    pub source_id: String,
    pub line_id: String,
    pub event: ProjectEvidenceEvent,
    pub project: String,
    pub title: String,
    pub evidence: String,
    pub data_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_suggestion: Option<String>,
    /// Prompt/parser contract revision; absent on legacy proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_version: Option<String>,
}

impl MailProposalInput {
    /// Check fields and classification; the owner separately verifies the target project.
    ///
    /// # Errors
    /// Returns an error for empty, oversized or unsupported fields.
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, max) in [
            (&self.source_id, 512),
            (&self.line_id, 512),
            (&self.project, 255),
            (&self.title, 255),
            (&self.evidence, 2000),
        ] {
            if value.trim().is_empty() || value.len() > max || value.contains('\0') {
                return Err("proposal field is empty or exceeds its limit");
            }
        }
        if !matches!(self.data_class.as_str(), "c0" | "c1" | "c2") {
            return Err("invalid data class");
        }
        for (value, max) in [
            (&self.task_suggestion, 1000),
            (&self.extraction_version, 128),
        ] {
            if value
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > max || s.contains('\0'))
            {
                return Err("invalid task suggestion or extraction version");
            }
        }
        Ok(())
    }
}
