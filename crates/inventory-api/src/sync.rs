//! Signed write envelope shared by Inventory and paired clients.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Deserialize, Serialize)]
pub struct Envelope {
    pub protocol_version: String,
    pub envelope_id: String,
    pub origin_node_id: String,
    pub target_node_id: String,
    pub actor_device_id: String,
    pub cursor: Option<String>,
    #[serde(default)]
    pub mutations: Vec<Mutation>,
    pub sent_at: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Mutation {
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub action: String,
    pub base_revision: Option<String>,
    pub fields: Map<String, Value>,
    pub actor_device_id: String,
    pub created_at: i64,
}
