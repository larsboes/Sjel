use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::types::Value as SqliteValue;
use std::sync::Arc;
use turso::transaction::{Transaction, TransactionBehavior};
use turso::{params, params_from_iter, Connection, Value};

use super::super::{
    boom, created_item, expected_revision, import_response, not_an_entry, overlay_import_items,
    state_history_response, state_response, write_response, HttpError, NewItem, StateBody,
};
use super::WalPreview;
use crate::store::{
    catalogue_select, decode_catalogue_row, item_insert_query, item_revision_update,
    state_history_query, state_insert_query, write_params, Item, State as ItemState, WriteOutcome,
};

async fn returning_revision(
    connection: &Connection,
    sql: String,
    values: Vec<Value>,
) -> Result<Option<i64>, HttpError> {
    let mut rows = connection
        .query(sql, params_from_iter(values))
        .await
        .map_err(boom)?;
    let revision = rows
        .next()
        .await
        .map_err(boom)?
        .map(|row| row.get(0).map_err(boom))
        .transpose()?;
    while rows.next().await.map_err(boom)?.is_some() {}
    Ok(revision)
}

fn bind_values(item: &Item) -> Result<Vec<Value>, HttpError> {
    Ok(write_params(item)
        .map_err(boom)?
        .into_iter()
        .map(|value| match value {
            SqliteValue::Null => Value::Null,
            SqliteValue::Integer(value) => Value::Integer(value),
            SqliteValue::Real(value) => Value::Real(value),
            SqliteValue::Text(value) => Value::Text(value),
            SqliteValue::Blob(value) => Value::Blob(value),
        })
        .collect())
}

pub(super) async fn item_on(
    connection: &Connection,
    id: &str,
) -> Result<Option<(Item, Option<ItemState>)>, HttpError> {
    let mut rows = connection
        .query(
            format!("{} WHERE i.id = ?1", catalogue_select("inventory")),
            [id],
        )
        .await
        .map_err(boom)?;
    rows.next()
        .await
        .map_err(boom)?
        .map(|row| decode_catalogue_row(&row).map_err(boom))
        .transpose()
}

pub(super) async fn update_revision(
    connection: &Connection,
    item: &Item,
    expected: i64,
) -> Result<Option<i64>, HttpError> {
    let mut values = bind_values(item)?;
    values.push(Value::Integer(expected));
    returning_revision(connection, item_revision_update("inventory"), values).await
}

pub(super) async fn finish_write<T>(
    transaction: Transaction<'_>,
    outcome: Result<T, HttpError>,
) -> Result<T, HttpError> {
    match outcome {
        Ok(value) => {
            transaction.commit().await.map_err(boom)?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await.map_err(boom)?;
            Err(error)
        }
    }
}

pub(super) async fn append_state(
    connection: &Connection,
    id: &str,
    state: ItemState,
    note: Option<&str>,
) -> Result<bool, HttpError> {
    let changes = connection
        .execute(
            state_insert_query("inventory"),
            params![id, state.as_str(), note],
        )
        .await
        .map_err(boom)?;
    Ok(changes != 0)
}

pub(super) async fn create_on(
    connection: &Connection,
    body: &NewItem,
    note: Option<&str>,
) -> Result<Option<i64>, HttpError> {
    let revision = returning_revision(
        connection,
        item_insert_query("inventory", false),
        bind_values(&body.item)?,
    )
    .await?;
    if revision.is_some() && !append_state(connection, &body.item.id, body.state, note).await? {
        return Err(boom("initial item state was not recorded"));
    }
    Ok(revision)
}

pub(super) async fn create(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<NewItem>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    if body.item.id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "`id` is missing".into()));
    }
    let mut connection = store.writer().await?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = create_on(
        &transaction,
        &body,
        body.note.as_deref().or(Some("created in the dashboard")),
    )
    .await;
    created_item(body, finish_write(transaction, outcome).await?)
}

async fn replace_on(
    connection: &Connection,
    item: &Item,
    expected: Option<i64>,
) -> Result<WriteOutcome, HttpError> {
    let revision = match expected {
        Some(expected) => update_revision(connection, item, expected).await?,
        None => {
            returning_revision(
                connection,
                item_insert_query("inventory", true),
                bind_values(item)?,
            )
            .await?
        }
    };
    if let Some(revision) = revision {
        return Ok(WriteOutcome::Written(revision));
    }
    Ok(match item_on(connection, &item.id).await? {
        Some((item, state)) => WriteOutcome::Stale(Box::new(item), state),
        None => WriteOutcome::Missing,
    })
}

pub(super) async fn replace(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Response, HttpError> {
    let expected = expected_revision(&headers, &mut body)?;
    let mut item: Item = serde_json::from_value(body).map_err(not_an_entry)?;
    item.id = id.clone();
    let mut connection = store.writer().await?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = replace_on(&transaction, &item, expected).await;
    let outcome = finish_write(transaction, outcome).await?;
    write_response(&id, expected.unwrap_or_default(), outcome)
}

pub(super) async fn post_state(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<StateBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let mut connection = store.writer().await?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = async {
        if item_on(&transaction, &id).await?.is_none() {
            return Err((StatusCode::NOT_FOUND, format!("no entry `{id}`")));
        }
        append_state(&transaction, &id, body.state, body.note.as_deref()).await
    }
    .await;
    Ok(state_response(
        id,
        body.state,
        finish_write(transaction, outcome).await?,
    ))
}

pub(super) async fn history(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, HttpError> {
    let connection = store.database.connect().map_err(boom)?;
    let mut rows = connection
        .query(state_history_query("inventory"), [id])
        .await
        .map_err(boom)?;
    let mut states = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        let raw: String = row.get(0).map_err(boom)?;
        if let Some(state) = ItemState::parse(&raw) {
            states.push((state, row.get(1).map_err(boom)?, row.get(2).map_err(boom)?));
        }
    }
    Ok(state_history_response(states))
}

async fn import_on(
    connection: &Connection,
    items: Vec<(Item, ItemState)>,
    force: bool,
) -> Result<Option<crate::import::Report>, HttpError> {
    if !force {
        let mut rows = connection
            .query("SELECT 1 FROM inventory_item LIMIT 1", ())
            .await
            .map_err(boom)?;
        if rows.next().await.map_err(boom)?.is_some() {
            return Ok(None);
        }
    }
    let mut report = crate::import::Report::default();
    for (item, state) in items {
        returning_revision(
            connection,
            item_insert_query("inventory", true),
            bind_values(&item)?,
        )
        .await?
        .ok_or_else(|| boom("Inventory import did not write the item"))?;
        let changed = append_state(
            connection,
            &item.id,
            state,
            Some("imported from inventory/*.toml"),
        )
        .await?;
        report.record(item.kind, changed);
    }
    Ok(Some(report))
}

pub(super) async fn import(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<inventory_api::ImportOverlayRequest>,
) -> Result<Json<crate::import::Report>, HttpError> {
    let items = tokio::task::spawn_blocking(overlay_import_items)
        .await
        .map_err(boom)??;
    let mut connection = store.writer().await?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = import_on(&transaction, items, body.force).await;
    import_response(finish_write(transaction, outcome).await?)
}
