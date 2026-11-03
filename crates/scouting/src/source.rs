use serde::{Deserialize, Serialize};

use crate::opportunity::{Opportunity, OpportunityType};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("fetch: {0}")]
    Fetch(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("rate limited")]
    RateLimited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub location: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub limit: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            location: None,
            date_from: None,
            date_to: None,
            limit: 50,
        }
    }
}

pub trait SourceAdapter: Send + Sync {
    /// This source's identity, not its adapter type. It is the key
    /// `store::record_run` writes `source_state` under, so two sources built
    /// from the same adapter must not answer the same thing: while this was
    /// `&'static str`, every configured `rss` entry shared one cursor row.
    /// A config-built adapter returns its `sources[].id`; a hardcoded one
    /// returns its own literal, which is already unique.
    fn name(&self) -> &str;

    fn opportunity_type(&self) -> OpportunityType;

    fn rate_limit_per_min(&self) -> u32;

    fn search(&self, query: &SearchQuery) -> Result<Vec<Opportunity>, SourceError>;
}
