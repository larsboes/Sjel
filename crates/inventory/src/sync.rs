pub(crate) use inventory_api::sync::Envelope;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct ResponseEnvelope {
    pub protocol_version: &'static str,
    pub envelope_id: String,
    pub origin_node_id: String,
    pub target_node_id: String,
    pub actor_device_id: String,
    pub cursor: Option<String>,
    pub mutations: Vec<Value>,
    pub acknowledgements: Vec<Acknowledgement>,
    pub conflicts: Vec<Value>,
    pub sent_at: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct Acknowledgement {
    pub operation_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub processed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn empty_response(envelope: &Envelope, now: i64) -> ResponseEnvelope {
    ResponseEnvelope {
        protocol_version: "sjel-sync/v1",
        envelope_id: envelope.envelope_id.clone(),
        origin_node_id: envelope.origin_node_id.clone(),
        target_node_id: envelope.target_node_id.clone(),
        actor_device_id: envelope.actor_device_id.clone(),
        cursor: envelope.cursor.clone(),
        mutations: Vec::new(),
        acknowledgements: Vec::new(),
        conflicts: Vec::new(),
        sent_at: now,
    }
}
