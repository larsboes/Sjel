use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const PSEUDONYMIZE_VERSION: &str = "reversible-pseudonymize-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityType {
    Person,
    /// A whole sender or author field, replaced as one unit. The field is an identity even
    /// when no rule recognises a part of it: `Alice <alice@example.com>` must not become
    /// `Alice <EMAIL_01>`. Reads as `identity` in receipts, the kind the destructive path
    /// already records for the same field (`cloud_derivative.rs`, `[identity removed]`).
    Identity,
    Place,
    Email,
    Phone,
    Iban,
    Secret,
    Number,
    Link,
}

impl EntityType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Identity => "identity",
            Self::Place => "place",
            Self::Email => "email",
            Self::Phone => "phone_number",
            Self::Iban => "financial_identifier",
            Self::Secret => "secret_token",
            Self::Number => "long_number",
            Self::Link => "link",
        }
    }

    pub fn token_prefix(self) -> &'static str {
        match self {
            Self::Person => "TRAVELER",
            Self::Identity => "SENDER",
            Self::Place => "PLACE",
            Self::Email => "EMAIL",
            Self::Phone => "PHONE",
            Self::Iban => "ACCOUNT",
            Self::Secret => "TOKEN",
            Self::Number => "NUM",
            Self::Link => "LINK",
        }
    }

    /// The token family as a fixed string, for a receipt row that names its marker without
    /// naming one issued token: `<TRAVELER_nn>`.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Person => "<TRAVELER_nn>",
            Self::Identity => "<SENDER_nn>",
            Self::Place => "<PLACE_nn>",
            Self::Email => "<EMAIL_nn>",
            Self::Phone => "<PHONE_nn>",
            Self::Iban => "<ACCOUNT_nn>",
            Self::Secret => "<TOKEN_nn>",
            Self::Number => "<NUM_nn>",
            Self::Link => "<LINK_nn>",
        }
    }

    pub fn one_label(self) -> &'static str {
        match self {
            Self::Person => "mention of a person",
            Self::Identity => "identity",
            Self::Place => "mention of a place",
            Self::Email => "email address",
            Self::Phone => "phone number",
            Self::Iban => "account number",
            Self::Secret => "token-like secret",
            Self::Number => "long number",
            Self::Link => "link",
        }
    }

    pub fn many_label(self) -> &'static str {
        match self {
            Self::Person => "mentions of people",
            Self::Identity => "identities",
            Self::Place => "mentions of places",
            Self::Email => "email addresses",
            Self::Phone => "phone numbers",
            Self::Iban => "account numbers",
            Self::Secret => "token-like secrets",
            Self::Number => "long numbers",
            Self::Link => "links",
        }
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// How many occurrences of one kind a call replaced. Occurrences, not distinct entities:
/// `Lars … Lars` is two. The receipt says "mentions", and a count of distinct values would
/// need the values (PRD Q9b; `cloud_derivative::redaction_receipt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionFinding {
    pub entity_type: EntityType,
    pub count: usize,
}

pub fn format_receipt(findings: &[RedactionFinding]) -> Option<String> {
    let total: usize = findings.iter().map(|f| f.count).sum();
    if total == 0 {
        return None;
    }

    const ORDER: [EntityType; 9] = [
        EntityType::Person,
        EntityType::Identity,
        EntityType::Place,
        EntityType::Email,
        EntityType::Phone,
        EntityType::Iban,
        EntityType::Secret,
        EntityType::Number,
        EntityType::Link,
    ];

    let mut parts: Vec<String> = Vec::new();
    for kind in ORDER {
        let count: usize = findings
            .iter()
            .filter(|f| f.entity_type == kind)
            .map(|f| f.count)
            .sum();
        match count {
            0 => {}
            1 => parts.push(format!("1 {}", kind.one_label())),
            n => parts.push(format!("{n} {}", kind.many_label())),
        }
    }

    let detail = match parts.len() {
        0 => return None,
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{} and {last}", parts.join(", "))
        }
    };
    let noun = if total == 1 { "detail" } else { "details" };
    Some(format!(
        "Reduced {total} {noun} before this call: {detail}."
    ))
}

pub fn compute_digest(findings: &[RedactionFinding]) -> String {
    let mut parts: Vec<String> = findings
        .iter()
        .map(|f| format!("{}:{}", f.entity_type.as_str(), f.count))
        .collect();
    parts.sort();
    let joined = parts.join(",");
    let mut hasher = Sha256::new();
    hasher.update(PSEUDONYMIZE_VERSION.as_bytes());
    hasher.update(b"\0");
    hasher.update(joined.as_bytes());
    // sha2 0.11's `finalize()` returns `Array<u8, U32>`, which no longer implements `LowerHex`
    // the way 0.10's `GenericArray` did. The workspace's own idiom (devices::store, finance,
    // places) is a byte-wise `02x` write, so this matches it rather than pulling in a hex crate.
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
