//! Owner-reviewed aliases with retained revocations and item-lifetime checks.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use inventory_api::aliases::{AliasReview, AliasRevocation, ItemAlias};
use rusqlite::{params, OptionalExtension};

use crate::{
    api::AppState,
    identity,
    store::{self, Store},
};

type HttpError = (StatusCode, String);
const COLUMNS: &str = "id, namespace, value, kind, item_id, reviewed_item_revision, reviewed_state_id, source_ref, data_class, version, reviewed_at, revoked_at";

fn internal(e: impl std::fmt::Display) -> HttpError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn invalid(e: impl ToString) -> HttpError {
    (StatusCode::BAD_REQUEST, e.to_string())
}
fn conflict() -> HttpError {
    (
        StatusCode::CONFLICT,
        "alias or reviewed item/state changed".into(),
    )
}

fn read_alias(row: &rusqlite::Row<'_>) -> rusqlite::Result<ItemAlias> {
    let kind: String = row.get(3)?;
    let kind = serde_json::from_value(serde_json::Value::String(kind)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let item: String = row.get(4)?;
    let reference = identity::encode_item_ref(&item).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e)),
        )
    })?;
    Ok(ItemAlias {
        id: row.get(0)?,
        key: inventory_api::aliases::AliasKey {
            namespace: row.get(1)?,
            value: row.get(2)?,
            kind,
        },
        reference,
        reviewed_item_revision: row.get(5)?,
        reviewed_state_id: row.get(6)?,
        source_ref: row.get(7)?,
        data_class: row.get(8)?,
        version: row.get(9)?,
        reviewed_at: row.get(10)?,
        revoked_at: row.get(11)?,
    })
}

pub(crate) async fn review(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AliasReview>,
) -> Result<Json<ItemAlias>, HttpError> {
    body.key.validate().map_err(invalid)?;
    let item_id = identity::decode_item_ref(&body.reference).map_err(invalid)?;
    if body.expected_item_revision < 1
        || body.expected_state_id < 1
        || !matches!(body.data_class.as_str(), "c0" | "c1")
        || body.source_ref.trim().is_empty()
        || body.source_ref.trim() != body.source_ref
        || body.source_ref.len() > 512
        || body.source_ref.chars().any(char::is_control)
    {
        return Err(invalid("invalid review snapshot, provenance or data class"));
    }
    let store = Store::open(&state.database).map_err(internal)?;
    let mut conn = store.borrow_connection().map_err(internal)?;
    let tx = sjel_store::write_transaction(&mut conn).map_err(internal)?;
    let existing = tx.query_row(&format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE namespace=?1 AND value=?2 AND kind=?3 AND item_id=?4 AND revoked_at IS NULL"), params![body.key.namespace, body.key.value, body.key.kind.as_str(), item_id], read_alias).optional().map_err(internal)?;
    if let Some(existing) = existing {
        if existing.reviewed_item_revision != body.expected_item_revision
            || existing.reviewed_state_id != body.expected_state_id
            || existing.source_ref != body.source_ref
            || existing.data_class != body.data_class
        {
            return Err(conflict());
        }
        tx.commit().map_err(internal)?;
        return Ok(Json(existing));
    }
    let item = store::item_snapshot(&tx, &item_id)
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
    if item.item_revision != body.expected_item_revision
        || item.state_id != Some(body.expected_state_id)
    {
        return Err(conflict());
    }
    tx.execute("INSERT INTO inventory_item_alias (namespace,value,kind,item_id,reviewed_item_revision,reviewed_state_id,source_ref,data_class,reviewed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![body.key.namespace,body.key.value,body.key.kind.as_str(),item_id,body.expected_item_revision,body.expected_state_id,body.source_ref,body.data_class]).map_err(internal)?;
    let alias = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE id=?1"),
            [tx.last_insert_rowid()],
            read_alias,
        )
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(Json(alias))
}

pub(crate) async fn revoke(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<AliasRevocation>,
) -> Result<Json<ItemAlias>, HttpError> {
    if body.version < 1 {
        return Err(invalid("invalid alias version"));
    }
    let store = Store::open(&state.database).map_err(internal)?;
    let mut conn = store.borrow_connection().map_err(internal)?;
    let tx = sjel_store::write_transaction(&mut conn).map_err(internal)?;
    let alias = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE id=?1"),
            [id],
            read_alias,
        )
        .optional()
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "alias not found".into()))?;
    if alias.revoked_at.is_some() && alias.version.checked_sub(1) == Some(body.version) {
        tx.commit().map_err(internal)?;
        return Ok(Json(alias));
    }
    if alias.revoked_at.is_some() || alias.version != body.version {
        return Err(conflict());
    }
    let changed = tx.execute("UPDATE inventory_item_alias SET revoked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), version=version+1 WHERE id=?1 AND version=?2 AND revoked_at IS NULL", params![id,body.version]).map_err(internal)?;
    if changed != 1 {
        return Err(conflict());
    }
    let alias = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM inventory_item_alias WHERE id=?1"),
            [id],
            read_alias,
        )
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(Json(alias))
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    Path(reference): Path<String>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let item = identity::decode_item_ref(&reference).map_err(invalid)?;
    let store = Store::open(&state.database).map_err(internal)?;
    let conn = store.borrow_connection().map_err(internal)?;
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS}, COUNT(*) OVER() FROM inventory_item_alias WHERE item_id=?1 ORDER BY id DESC LIMIT 100")).map_err(internal)?;
    let mut rows = stmt.query([item]).map_err(internal)?;
    let mut aliases = Vec::new();
    let mut total = 0_i64;
    while let Some(row) = rows.next().map_err(internal)? {
        total = row.get(12).map_err(internal)?;
        aliases.push(read_alias(row).map_err(internal)?);
    }
    Ok(Json(
        serde_json::json!({"aliases":aliases,"total":total,"truncated":total > 100}),
    ))
}
