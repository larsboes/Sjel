//! Purchase claims staged for review, separate from item identity and ownership.

use serde::{Deserialize, Serialize};

/// A purchase event reported by a source; none establishes ownership without review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseEvent {
    Order,
    Shipment,
    Delivered,
    Cancellation,
    Return,
}

impl PurchaseEvent {
    /// The stable wire and storage label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Shipment => "shipment",
            Self::Delivered => "delivered",
            Self::Cancellation => "cancellation",
            Self::Return => "return",
        }
    }
}

/// The body of `POST /api/mail-proposals`, shared by intake and Inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailProposalInput {
    pub source_id: String,
    pub line_id: String,
    pub event: PurchaseEvent,
    pub title: String,
    pub evidence: String,
    pub data_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_ref: Option<String>,
    /// The grounded delivery statement when it is separate from the item excerpt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<String>,
    /// Prompt/parser contract revision; absent on legacy proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_version: Option<String>,
}

impl MailProposalInput {
    /// Check the staging boundary without inferring source truth or item identity.
    ///
    /// # Errors
    /// Returns a field or classification error for an invalid proposal.
    pub fn validate(&self) -> Result<(), &'static str> {
        let bounded = |s: &str, max| !s.trim().is_empty() && s.len() <= max && !s.contains('\0');
        if !bounded(&self.source_id, 512)
            || !bounded(&self.line_id, 256)
            || !bounded(&self.title, 256)
            || !bounded(&self.evidence, 1024)
            || self.order_ref.as_deref().is_some_and(|s| !bounded(s, 256))
            || self
                .extraction_version
                .as_deref()
                .is_some_and(|s| !bounded(s, 128))
        {
            return Err("missing or oversized proposal field");
        }
        if !matches!(self.data_class.as_str(), "c0" | "c1" | "c2") {
            return Err("data_class must be c0, c1 or c2");
        }
        if self
            .claim
            .as_deref()
            .is_some_and(|s| self.event != PurchaseEvent::Delivered || !bounded(s, 1024))
        {
            return Err("claim must be a bounded delivery statement");
        }
        Ok(())
    }
}
