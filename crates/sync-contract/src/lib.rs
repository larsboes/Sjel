//! Wire types shared by capability change feeds and offline replay.

use serde::{Deserialize, Serialize};

pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tombstone {
    pub id: String,
    pub deleted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Changes<T> {
    pub cursor: String,
    pub upserted: Vec<T>,
    pub deleted: Vec<Tombstone>,
}
