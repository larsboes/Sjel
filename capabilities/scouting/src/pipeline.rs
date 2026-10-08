use std::collections::HashMap;
use std::path::Path;

use crate::score::{embed_opportunities, score_labelled, ScoredOpportunity, TelosProfile};
use crate::source::{SearchQuery, SourceAdapter};
use crate::store::{RankedRow, Store};
use crate::vault_linker;

#[derive(Debug)]
pub struct PipelineReport {
    pub scored: Vec<ScoredOpportunity>,
    pub new_count: usize,
    pub vault_links: usize,
    pub store_total: i64,
}

pub fn run(
    adapter: &dyn SourceAdapter,
    query: &SearchQuery,
    telos: &[TelosProfile],
    opp_embeddings: Option<&HashMap<String, Vec<f32>>>,
    mut store: Option<&mut Store>,
    events_dir: Option<&Path>,
) -> Result<PipelineReport, Box<dyn std::error::Error>> {
    sjel_runtime::require(sjel_runtime::Category::BulkIndexing)?;
    let role = crate::embed::embedding_role();
    if opp_embeddings.is_none() {
        if let Some(role) = &role { role.runtime_admission()?; }
    }
    let opportunities = adapter.search(query)?;

    // A pre-computed file, if one was handed in, is authoritative. Otherwise
    // embed the opportunity side live: without this only the interest profile
    // ever carried a real vector and every cosine was half hash, whichever
    // backend was configured.
    let live: Option<HashMap<String, Vec<f32>>> = match (opp_embeddings, role.as_ref()) {
        (Some(_), _) => None,
        (None, Some(role)) => embed_opportunities(&opportunities, role),
        (None, None) => None,
    };
    if opp_embeddings.is_none() {
        if let Some(role) = &role { role.runtime_admission()?; }
    }
    let vectors = opp_embeddings.or(live.as_ref());
    // Labelled by what happened, not by what was configured. Deriving this
    // from `role` alone printed the model's name on a run that had already
    // fallen back to hashing, because the backend was down -- the same
    // intent-reported-as-outcome bug the banner had one layer up.
    let label = match (opp_embeddings, live.as_ref()) {
        (Some(_), _) => "precomputed".to_string(),
        (None, Some(_)) => role
            .as_ref()
            .map(|role| role.cache_key())
            .unwrap_or_else(|| "unknown".to_string()),
        (None, None) => "hash-fallback".to_string(),
    };
    let mut scored = score_labelled(&opportunities, telos, vectors, &label);

    let mut new_count = 0;
    let mut vault_links = 0;

    for s in &mut scored {
        sjel_runtime::require(sjel_runtime::Category::BulkIndexing)?;
        let vault_link =
            events_dir.and_then(|dir| vault_linker::link_to_vault(&s.opportunity, dir));
        if let Some(ref vl) = vault_link {
            vault_links += 1;
            s.rationale = format!("{}\n     vault link: {vl}", s.rationale);
        }

        if let Some(st) = store.as_mut() {
            let is_new = st.upsert(
                &s.opportunity,
                s.score,
                s.matched_focus.as_deref(),
                &s.rationale,
                vault_link.as_deref(),
            )?;
            if is_new {
                new_count += 1;
            }
        }
    }

    let store_total = match store.as_ref() {
        Some(st) => st.count().unwrap_or(0),
        None => 0,
    };

    Ok(PipelineReport {
        scored,
        new_count,
        vault_links,
        store_total,
    })
}

pub fn fetch_json(
    adapter: &dyn SourceAdapter,
    query: &SearchQuery,
) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let opportunities = adapter.search(query)?;
    Ok(opportunities
        .into_iter()
        .map(|o| serde_json::to_value(o).unwrap_or(serde_json::Value::Null))
        .collect())
}

pub fn backlog_from_store(
    store: &Store,
    limit: usize,
    include_dismissed: bool,
) -> Result<Vec<RankedRow>, Box<dyn std::error::Error>> {
    store.list_top(limit, include_dismissed)
}
