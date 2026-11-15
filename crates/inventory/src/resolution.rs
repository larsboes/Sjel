//! Bounded candidate retrieval over existing item labels, without learned state.

use std::{collections::BTreeSet, sync::Arc};

use axum::{extract::State, http::StatusCode, Json};
use inventory_api::aliases::AliasKind;
use inventory_api::resolution::{
    CandidateReason, Resolution, ResolutionCandidate, ResolutionStatus, ResolveRequest,
};
use rusqlite::params;

use crate::{
    api::AppState,
    identity,
    store::{self, Store},
};

const CANDIDATE_LIMIT: usize = 10;
type HttpError = (StatusCode, String);

fn internal(e: impl std::fmt::Display) -> HttpError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn title_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn candidate(
    item: inventory_api::MailMatchSnapshot,
    reason: CandidateReason,
    overlap_bp: Option<u32>,
    shared_tokens: Vec<String>,
) -> Result<ResolutionCandidate, HttpError> {
    let typed_ref = identity::encode_item_ref(&item.item_id).map_err(internal)?;
    Ok(ResolutionCandidate {
        alias_id: None,
        item,
        typed_ref,
        reason,
        overlap_bp,
        shared_tokens,
    })
}

pub(crate) async fn resolve(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResolveRequest>,
) -> Result<Json<Resolution>, HttpError> {
    let invalid = || {
        (
            StatusCode::BAD_REQUEST,
            "provide exactly one bounded reference, title or scoped alias".into(),
        )
    };
    let (reference, title, alias) = match (request.reference, request.title, request.alias) {
        (Some(reference), None, None) => (
            Some(
                identity::decode_item_ref(&reference)
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.into()))?,
            ),
            None,
            None,
        ),
        (None, Some(title), None)
            if !title.trim().is_empty()
                && title.len() <= 256
                && !title.chars().any(char::is_control)
                && !tokens(&title).is_empty() =>
        {
            (None, Some(title), None)
        }
        (None, None, Some(key)) => {
            key.validate()
                .map_err(|e| (StatusCode::BAD_REQUEST, e.into()))?;
            (None, None, Some(key))
        }
        _ => return Err(invalid()),
    };
    let store = Store::open(&state.database).map_err(internal)?;
    let conn = store.borrow_connection().map_err(internal)?;
    if let Some(key) = alias {
        let sql = format!("SELECT {}, a.id, COUNT(*) OVER() {} JOIN inventory_item_alias a ON a.item_id=i.id
            WHERE a.namespace=?1 AND a.value=?2 AND a.kind=?3 AND a.revoked_at IS NULL
            AND EXISTS (SELECT 1 FROM inventory_item_state original WHERE original.id=a.reviewed_state_id AND original.item_id=i.id)
            ORDER BY i.id LIMIT {CANDIDATE_LIMIT}", store::SNAPSHOT_COLUMNS, store::SNAPSHOT_FROM);
        let mut stmt = conn.prepare(&sql).map_err(internal)?;
        let mut rows = stmt
            .query(params![key.namespace, key.value, key.kind.as_str()])
            .map_err(internal)?;
        let mut candidates = Vec::with_capacity(CANDIDATE_LIMIT);
        let mut total_candidates = 0;
        while let Some(row) = rows.next().map_err(internal)? {
            total_candidates =
                usize::try_from(row.get::<_, i64>(6).map_err(internal)?).map_err(internal)?;
            let reason = match key.kind {
                AliasKind::PhysicalItem => CandidateReason::ReviewedItemAlias,
                AliasKind::ProductModel => CandidateReason::ReviewedProductAlias,
            };
            let mut c = candidate(
                store::read_snapshot(row).map_err(internal)?,
                reason,
                None,
                vec![],
            )?;
            c.alias_id = Some(row.get(5).map_err(internal)?);
            candidates.push(c);
        }
        let status = if total_candidates == 0 {
            ResolutionStatus::Unknown
        } else if total_candidates == 1 && key.kind == AliasKind::PhysicalItem {
            ResolutionStatus::Exact
        } else {
            ResolutionStatus::Ambiguous
        };
        return Ok(Json(Resolution {
            status,
            candidates,
            total_candidates,
        }));
    }
    if let Some(id) = reference {
        let candidates = store::item_snapshot(&conn, &id)
            .map_err(internal)?
            .map(|item| candidate(item, CandidateReason::Reference, None, vec![]))
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        return Ok(Json(Resolution {
            status: if candidates.is_empty() {
                ResolutionStatus::Unknown
            } else {
                ResolutionStatus::Exact
            },
            total_candidates: candidates.len(),
            candidates,
        }));
    }
    let title = title.unwrap_or_default();
    let query = tokens(&title);
    let normalized_title = title_key(&title);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {} {}",
            store::SNAPSHOT_COLUMNS,
            store::SNAPSHOT_FROM
        ))
        .map_err(internal)?;
    let rows = stmt.query_map([], store::read_snapshot).map_err(internal)?;
    let mut candidates = Vec::with_capacity(CANDIDATE_LIMIT + 1);
    let mut total_candidates = 0;
    for row in rows {
        let item = row.map_err(internal)?;
        let item_tokens = tokens(&item.title);
        let shared: Vec<_> = query.intersection(&item_tokens).cloned().collect();
        if shared.is_empty() {
            continue;
        }
        let union = query.len() + item_tokens.len() - shared.len();
        let overlap_bp = (shared.len() * 10_000 / union) as u32;
        let reason = if normalized_title == title_key(&item.title) {
            CandidateReason::TitleEqual
        } else {
            CandidateReason::TokenOverlap
        };
        total_candidates += 1;
        candidates.push(candidate(item, reason, Some(overlap_bp), shared)?);
        candidates.sort_by(|a, b| {
            b.overlap_bp
                .cmp(&a.overlap_bp)
                .then_with(|| a.item.item_id.cmp(&b.item.item_id))
        });
        candidates.truncate(CANDIDATE_LIMIT);
    }
    Ok(Json(Resolution {
        status: if total_candidates == 0 {
            ResolutionStatus::Unknown
        } else {
            ResolutionStatus::Ambiguous
        },
        candidates,
        total_candidates,
    }))
}
