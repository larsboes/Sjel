use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use inventory_api::aliases::{AliasKey, AliasKind, AliasReview, AliasRevocation, ItemAlias};
use inventory_api::resolution::{CandidateReason, Resolution, ResolutionStatus, ResolveRequest};
use inventory_api::MailMatchSnapshot;
use serde_json::{json, Value};
use turso::transaction::TransactionBehavior;
use turso::{params, Connection, Row};

use super::super::{boom, refused, HttpError};
use super::{items, WalPreview};
use crate::aliases::{conflict, validate_review, COLUMNS};
use crate::identity::{decode_item_ref, encode_item_ref};
use crate::resolution::{candidate, parse_request, TitleRanking, CANDIDATE_LIMIT};
use crate::store::{SNAPSHOT_COLUMNS, SNAPSHOT_FROM};

type PreviewRouter = Router<Arc<WalPreview>>;

pub(super) fn reads(router: PreviewRouter) -> PreviewRouter {
    router
        .route("/api/item-refs/{reference}/aliases", get(list))
        .route("/api/items/resolve", post(resolve))
}

pub(super) fn writes(router: PreviewRouter) -> PreviewRouter {
    router
        .route("/api/item-aliases", post(review))
        .route("/api/item-aliases/{id}/revoke", post(revoke))
}

pub(super) fn snapshot_row(row: &Row) -> Result<MailMatchSnapshot, HttpError> {
    Ok(MailMatchSnapshot {
        item_id: row.get(0).map_err(boom)?,
        title: row.get(1).map_err(boom)?,
        item_revision: row.get(2).map_err(boom)?,
        state: row.get(3).map_err(boom)?,
        state_id: row.get(4).map_err(boom)?,
    })
}

pub(super) async fn item_snapshot(
    connection: &Connection,
    id: &str,
) -> Result<Option<MailMatchSnapshot>, HttpError> {
    let sql = format!("SELECT {SNAPSHOT_COLUMNS} {SNAPSHOT_FROM} WHERE i.id = ?1");
    let mut rows = connection.query(sql, [id]).await.map_err(boom)?;
    rows.next()
        .await
        .map_err(boom)?
        .as_ref()
        .map(snapshot_row)
        .transpose()
}

fn alias_row(row: &Row) -> Result<ItemAlias, HttpError> {
    let kind = serde_json::from_value(Value::String(row.get(3).map_err(boom)?)).map_err(boom)?;
    let id: String = row.get(4).map_err(boom)?;
    Ok(ItemAlias {
        id: row.get(0).map_err(boom)?,
        key: AliasKey {
            namespace: row.get(1).map_err(boom)?,
            value: row.get(2).map_err(boom)?,
            kind,
        },
        reference: encode_item_ref(&id).map_err(boom)?,
        reviewed_item_revision: row.get(5).map_err(boom)?,
        reviewed_state_id: row.get(6).map_err(boom)?,
        source_ref: row.get(7).map_err(boom)?,
        data_class: row.get(8).map_err(boom)?,
        version: row.get(9).map_err(boom)?,
        reviewed_at: row.get(10).map_err(boom)?,
        revoked_at: row.get(11).map_err(boom)?,
    })
}

async fn by_id(conn: &Connection, id: i64) -> Result<ItemAlias, HttpError> {
    let mut rows = conn
        .query(
            format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE id=?1"),
            [id],
        )
        .await
        .map_err(boom)?;
    rows.next()
        .await
        .map_err(boom)?
        .as_ref()
        .map(alias_row)
        .transpose()?
        .ok_or((StatusCode::NOT_FOUND, "alias not found".into()))
}

async fn review_on(conn: &Connection, body: AliasReview, id: &str) -> Result<ItemAlias, HttpError> {
    let mut rows = conn.query(
        format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE namespace=?1 AND value=?2 AND kind=?3 AND item_id=?4 AND revoked_at IS NULL"),
        params![body.key.namespace.as_str(),body.key.value.as_str(),body.key.kind.as_str(),id],
    ).await.map_err(boom)?;
    if let Some(row) = rows.next().await.map_err(boom)? {
        let existing = alias_row(&row)?;
        if existing.reviewed_item_revision != body.expected_item_revision
            || existing.reviewed_state_id != body.expected_state_id
            || existing.source_ref != body.source_ref
            || existing.data_class != body.data_class
        {
            return Err(conflict());
        }
        return Ok(existing);
    }
    drop(rows);
    let item = item_snapshot(conn, id)
        .await?
        .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
    if item.item_revision != body.expected_item_revision
        || item.state_id != Some(body.expected_state_id)
    {
        return Err(conflict());
    }
    conn.execute("INSERT INTO inventory_item_alias (namespace,value,kind,item_id,reviewed_item_revision,reviewed_state_id,source_ref,data_class,reviewed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
      params![body.key.namespace.as_str(),body.key.value.as_str(),body.key.kind.as_str(),id,body.expected_item_revision,body.expected_state_id,body.source_ref.as_str(),body.data_class.as_str()]).await.map_err(boom)?;
    by_id(conn, conn.last_insert_rowid()).await
}

async fn review(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<AliasReview>,
) -> Result<Json<ItemAlias>, HttpError> {
    let id = validate_review(&body)?;
    let mut conn = store.writer().await?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let result = review_on(&tx, body, &id).await;
    Ok(Json(items::finish_write(tx, result).await?))
}

async fn revoke_on(conn: &Connection, id: i64, version: i64) -> Result<ItemAlias, HttpError> {
    let alias = by_id(conn, id).await?;
    if alias.revoked_at.is_some() && alias.version.checked_sub(1) == Some(version) {
        return Ok(alias);
    }
    if alias.revoked_at.is_some() || alias.version != version {
        return Err(conflict());
    }
    if conn.execute("UPDATE inventory_item_alias SET revoked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),version=version+1 WHERE id=?1 AND version=?2 AND revoked_at IS NULL",params![id,version]).await.map_err(boom)?!=1 {return Err(conflict());}
    by_id(conn, id).await
}

async fn revoke(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<i64>,
    Json(body): Json<AliasRevocation>,
) -> Result<Json<ItemAlias>, HttpError> {
    if body.version < 1 {
        return Err(refused("invalid alias version"));
    }
    let mut conn = store.writer().await?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let result = revoke_on(&tx, id, body.version).await;
    Ok(Json(items::finish_write(tx, result).await?))
}

async fn list(
    State(store): State<Arc<WalPreview>>,
    Path(reference): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let id = decode_item_ref(&reference).map_err(refused)?;
    let conn = store.database.connect().map_err(boom)?;
    let mut rows=conn.query(format!("SELECT {COLUMNS},COUNT(*) OVER() FROM inventory_item_alias WHERE item_id=?1 ORDER BY id DESC LIMIT 100"),[id]).await.map_err(boom)?;
    let mut aliases = Vec::new();
    let mut total = 0_i64;
    while let Some(row) = rows.next().await.map_err(boom)? {
        total = row.get(12).map_err(boom)?;
        aliases.push(alias_row(&row)?);
    }
    Ok(Json(
        json!({"aliases":aliases,"total":total,"truncated":total>100}),
    ))
}

async fn resolve(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<ResolveRequest>,
) -> Result<Json<Resolution>, HttpError> {
    let (reference, title, alias) = parse_request(body)?;
    let conn = store.database.connect().map_err(boom)?;
    if let Some(key) = alias {
        let sql=format!("SELECT {SNAPSHOT_COLUMNS}, a.id, COUNT(*) OVER() {SNAPSHOT_FROM} JOIN inventory_item_alias a ON a.item_id=i.id
             WHERE a.namespace=?1 AND a.value=?2 AND a.kind=?3 AND a.revoked_at IS NULL
             AND EXISTS (SELECT 1 FROM inventory_item_state original WHERE original.id=a.reviewed_state_id AND original.item_id=i.id)
             ORDER BY i.id LIMIT {CANDIDATE_LIMIT}");
        let mut rows = conn
            .query(
                sql,
                params![
                    key.namespace.as_str(),
                    key.value.as_str(),
                    key.kind.as_str()
                ],
            )
            .await
            .map_err(boom)?;
        let mut candidates = Vec::with_capacity(CANDIDATE_LIMIT);
        let mut total_candidates = 0;
        while let Some(row) = rows.next().await.map_err(boom)? {
            total_candidates = usize::try_from(row.get::<i64>(6).map_err(boom)?).map_err(boom)?;
            let reason = match key.kind {
                AliasKind::PhysicalItem => CandidateReason::ReviewedItemAlias,
                AliasKind::ProductModel => CandidateReason::ReviewedProductAlias,
            };
            let mut result = candidate(snapshot_row(&row)?, reason, None, vec![])?;
            result.alias_id = Some(row.get(5).map_err(boom)?);
            candidates.push(result);
        }
        let status = match total_candidates {
            0 => ResolutionStatus::Unknown,
            1 if key.kind == AliasKind::PhysicalItem => ResolutionStatus::Exact,
            _ => ResolutionStatus::Ambiguous,
        };
        return Ok(Json(Resolution {
            status,
            candidates,
            total_candidates,
        }));
    }
    if let Some(id) = reference {
        let candidates = item_snapshot(&conn, &id)
            .await?
            .map(|item| candidate(item, CandidateReason::Reference, None, vec![]))
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        let status = if candidates.is_empty() {
            ResolutionStatus::Unknown
        } else {
            ResolutionStatus::Exact
        };
        return Ok(Json(Resolution {
            status,
            total_candidates: candidates.len(),
            candidates,
        }));
    }
    let mut ranking = TitleRanking::new(title.as_deref().unwrap_or_default());
    let mut rows = conn
        .query(format!("SELECT {SNAPSHOT_COLUMNS} {SNAPSHOT_FROM}"), ())
        .await
        .map_err(boom)?;
    while let Some(row) = rows.next().await.map_err(boom)? {
        ranking.add(snapshot_row(&row)?)?;
    }
    Ok(Json(ranking.finish()))
}
