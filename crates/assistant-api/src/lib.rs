//! Typed API contract, wire models, and route constants for the Sjel assistant capability.
//!
//! Client applications (e.g. `sjel-gpui-core`) and the server capability (`sjel-assistant`)
//! share these definitions to eliminate wire model drift. Zero platform dependencies.

use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const ROUTE_ASK: &str = "/api/ask";
pub const ROUTE_ASK_STREAM: &str = "/api/ask/stream";
pub const ROUTE_ASK_PROVIDERS: &str = "/api/ask/providers";
pub const ROUTE_GENERATE: &str = "/api/generate";
pub const ROUTE_STEER: &str = "/api/ask/{run_id}/steer";
pub const ROUTE_STOP: &str = "/api/ask/{run_id}/stop";

pub const PROXY_ROUTE_ASSISTANT_ASK: &str = "/assistant/api/ask";
pub const PROXY_ROUTE_ASSISTANT_ASK_STREAM: &str = "/assistant/api/ask/stream";
pub const PROXY_ROUTE_ASSISTANT_PROVIDERS: &str = "/assistant/api/ask/providers";

// ============================================================================
// Core Wire Models
// ============================================================================

/// One message in an Ask conversation (`POST /assistant/api/ask`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AskTurn {
    pub role: String,
    pub content: String,
}

/// Request to run one turn in an assistant conversation.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AskRequest {
    pub prompt: String,
    #[serde(default)]
    pub history: Vec<AskTurn>,
    #[serde(default)]
    pub instructions: Option<String>,
    /// Exact configured `cloud_*` role selected by the operator. Absent means local assistant.
    #[serde(default)]
    pub provider_role: Option<String>,
    /// Per-request acknowledgement that this prompt is public (c0).
    #[serde(default)]
    pub confirm_public_send: bool,
}

/// Configured cloud role exposed by the local assistant's privacy-safe discovery route.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AskProvider {
    pub role: String,
    pub provider: String,
    pub model: String,
    pub data_tier: String,
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

pub type CloudProviderOption = AskProvider;

/// Secret-free presence for one supported provider API key.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProviderKeyStatus {
    pub provider: String,
    pub present: bool,
}

/// Reply from `POST /assistant/api/ask` (or deterministic capability fallback).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AskReply {
    pub text: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

pub type AskResponse = AskReply;

/// One streamed event from `POST /assistant/api/ask/stream`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct AskEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub message: String,
}

/// Steer message sent to an active streaming Ask run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SteerRequest {
    pub message: String,
}

/// Request for one bounded generation with the configured local assistant role.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct GenerateRequest {
    pub prompt: String,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Bounded generation response.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct GenerateResponse {
    pub text: String,
    pub model: String,
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ask_turn_roundtrip() {
        let turn = AskTurn {
            role: "user".into(),
            content: "hello".into(),
        };
        let serialized = serde_json::to_string(&turn).unwrap();
        let deserialized: AskTurn = serde_json::from_str(&serialized).unwrap();
        assert_eq!(turn, deserialized);
    }

    #[test]
    fn test_ask_reply_roundtrip() {
        let reply = AskReply {
            text: "answer".into(),
            model: "local".into(),
            source: "assistant".into(),
            tools: vec!["trips".into()],
        };
        let serialized = serde_json::to_string(&reply).unwrap();
        let deserialized: AskReply = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reply, deserialized);
    }

    #[test]
    fn test_ask_event_deserialization() {
        let json = r#"{"type":"chunk","content":"world"}"#;
        let event: AskEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.kind, "chunk");
        assert_eq!(event.content, "world");
    }
}
