//! Explicitly reviewed, source-scoped names for physical items and product models.

use serde::{Deserialize, Serialize};

/// A physical item alias can establish identity; a product model alias retrieves candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AliasKind {
    PhysicalItem,
    ProductModel,
}

impl AliasKind {
    /// The stored and wire label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhysicalItem => "physical_item",
            Self::ProductModel => "product_model",
        }
    }
}

/// A case-sensitive identifier within an explicit namespace.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AliasKey {
    pub namespace: String,
    pub value: String,
    pub kind: AliasKind,
}

impl AliasKey {
    /// Validate bounds without normalizing an external identifier.
    ///
    /// # Errors
    /// Refuses empty or malformed namespaces and empty, oversized or control-bearing values.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.namespace.is_empty()
            || self.namespace.len() > 128
            || !self.namespace.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'.' | b':' | b'_' | b'-')
            })
            || self.value.trim().is_empty()
            || self.value.trim() != self.value
            || self.value.len() > 256
            || self.value.chars().any(char::is_control)
        {
            return Err("invalid alias namespace or value");
        }
        Ok(())
    }
}

/// An owner-reviewed link to the exact item/state snapshot the reviewer inspected.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AliasReview {
    pub key: AliasKey,
    pub reference: String,
    pub expected_item_revision: i64,
    pub expected_state_id: i64,
    pub source_ref: String,
    pub data_class: String,
}

/// A retained review, including revocation rather than destructive replacement.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ItemAlias {
    pub id: i64,
    pub key: AliasKey,
    pub reference: String,
    pub reviewed_item_revision: i64,
    pub reviewed_state_id: i64,
    pub source_ref: String,
    pub data_class: String,
    pub version: i64,
    pub reviewed_at: String,
    pub revoked_at: Option<String>,
}

/// Revoke one review at a known version.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AliasRevocation {
    pub version: i64,
}
