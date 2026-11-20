use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use inventory_api::{
    Candidate, GroupComparison, PriceObservation, PriceRecorded, RecordPriceRequest,
    SaveWishGroupRequest, SetAlternativesRequest, SetNeedsRequest, SetSpecsRequest,
    SetWishGroupItemsRequest, Spec,
};
use serde_json::{json, Value};
use turso::{params, Connection};

use super::super::{boom, refused, wishlist_response, HttpError};
use super::{items, WalPreview};
use crate::store::{compare, wish_group_view, WishGroup};

type PreviewRouter = Router<Arc<WalPreview>>;

pub(super) fn reads(router: PreviewRouter) -> PreviewRouter {
    router
        .route("/api/wishlist", get(wishlist))
        .route("/api/wish-groups", get(groups))
        .route("/api/wish-comparison", get(comparisons))
}

pub(super) fn writes(router: PreviewRouter) -> PreviewRouter {
    router
        .route("/api/wish-groups", post(save_group))
        .route("/api/wish-groups/{id}", delete(delete_group))
        .route("/api/wish-groups/{id}/items", put(set_members))
        .route("/api/wish-groups/{id}/needs", put(set_needs))
        .route("/api/wish-groups/{id}/alternatives", put(set_alternatives))
        .route("/api/items/{id}/specs", put(set_specs))
        .route("/api/items/{id}/prices", post(record_price))
}

async fn read_groups(connection: &Connection) -> Result<Vec<WishGroup>, HttpError> {
    let mut rows = connection
        .query(
            "SELECT id,title,constraint_text,created_at,updated_at,alternatives
             FROM inventory_wish_group ORDER BY title",
            (),
        )
        .await
        .map_err(boom)?;
    let mut groups = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        groups.push(WishGroup {
            id: row.get(0).map_err(boom)?,
            title: row.get(1).map_err(boom)?,
            constraint_text: row.get(2).map_err(boom)?,
            created_at: row.get(3).map_err(boom)?,
            updated_at: row.get(4).map_err(boom)?,
            alternatives: row.get::<i64>(5).map_err(boom)? == 1,
        });
    }
    Ok(groups)
}

async fn members(connection: &Connection, id: &str) -> Result<Vec<String>, HttpError> {
    let mut rows = connection
        .query(
            "SELECT item_id FROM inventory_wish_group_item WHERE group_id=?1 ORDER BY item_id",
            [id],
        )
        .await
        .map_err(boom)?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        ids.push(row.get(0).map_err(boom)?);
    }
    Ok(ids)
}

async fn wishlist(State(store): State<Arc<WalPreview>>) -> Result<Json<Value>, HttpError> {
    let catalogue = store.catalogue().await.map_err(boom)?;
    let connection = store.database.connect().map_err(boom)?;
    let mut groups = read_groups(&connection).await?;
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    let mut alternatives = Vec::new();
    for group in groups.into_iter().filter(|group| group.alternatives) {
        alternatives.push(members(&connection, &group.id).await?);
    }
    wishlist_response(catalogue, alternatives).await
}

async fn groups(State(store): State<Arc<WalPreview>>) -> Result<Json<Value>, HttpError> {
    let catalogue = store.catalogue().await.map_err(boom)?;
    let connection = store.database.connect().map_err(boom)?;
    let mut views = Vec::new();
    for group in read_groups(&connection).await? {
        let ids = members(&connection, &group.id).await?;
        let items = ids
            .iter()
            .filter_map(|id| catalogue.get(id).map(|(item, _)| item.clone()))
            .collect();
        views.push(wish_group_view(group, ids, items));
    }
    Ok(Json(json!({"wish_groups":views})))
}

#[derive(Clone, Copy)]
enum Numbers {
    Specs,
    Needs,
}
impl Numbers {
    fn relation(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Specs => ("inventory_wish_spec", "item_id", "inventory_item", "entry"),
            Self::Needs => (
                "inventory_wish_group_need",
                "group_id",
                "inventory_wish_group",
                "wish group",
            ),
        }
    }
}

async fn numbers(
    connection: &Connection,
    scope: Numbers,
    id: &str,
) -> Result<Vec<Spec>, HttpError> {
    let (table, column, _, _) = scope.relation();
    let mut rows = connection
        .query(
            format!("SELECT key,value,unit FROM {table} WHERE {column}=?1 ORDER BY key"),
            [id],
        )
        .await
        .map_err(boom)?;
    let mut specs = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        specs.push(Spec {
            key: row.get(0).map_err(boom)?,
            value: row.get(1).map_err(boom)?,
            unit: row.get(2).map_err(boom)?,
        });
    }
    Ok(specs)
}

async fn observations(
    connection: &Connection,
    id: &str,
) -> Result<Vec<PriceObservation>, HttpError> {
    let mut rows = connection.query(
        "SELECT price_cents,source,configuration,observed_at,alert FROM inventory_price_observation
         WHERE item_id=?1 ORDER BY observed_at,id", [id],
    ).await.map_err(boom)?;
    let mut prices = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        prices.push(PriceObservation {
            price_cents: row.get(0).map_err(boom)?,
            source: row.get(1).map_err(boom)?,
            configuration: row.get(2).map_err(boom)?,
            observed_at: row.get(3).map_err(boom)?,
            alert: row.get::<i64>(4).map_err(boom)? == 1,
        });
    }
    Ok(prices)
}

async fn comparisons(State(store): State<Arc<WalPreview>>) -> Result<Json<Value>, HttpError> {
    let catalogue = store.catalogue().await.map_err(boom)?;
    let connection = store.database.connect().map_err(boom)?;
    let mut out = Vec::new();
    for group in read_groups(&connection).await? {
        let mut candidates = Vec::new();
        for id in members(&connection, &group.id).await? {
            if let Some((item, _)) = catalogue.get(&id) {
                candidates.push(Candidate {
                    id: id.clone(),
                    label: item.label.clone(),
                    link: item.link.clone(),
                    cost_max_cents: item.cost_max_cents,
                    specs: numbers(&connection, Numbers::Specs, &id).await?,
                    observations: observations(&connection, &id).await?,
                });
            }
        }
        out.push(GroupComparison {
            needs: numbers(&connection, Numbers::Needs, &group.id).await?,
            id: group.id,
            title: group.title,
            constraint_text: group.constraint_text,
            candidates,
        });
    }
    Ok(Json(json!({"groups":out})))
}

async fn replace_members(
    connection: &Connection,
    id: &str,
    members: &[String],
) -> Result<(), HttpError> {
    connection
        .execute(
            "DELETE FROM inventory_wish_group_item WHERE group_id=?1",
            [id],
        )
        .await
        .map_err(boom)?;
    for item in members {
        connection
            .execute(
                "INSERT OR IGNORE INTO inventory_wish_group_item (group_id,item_id) VALUES (?1,?2)",
                params![id, item.as_str()],
            )
            .await
            .map_err(boom)?;
    }
    Ok(())
}

async fn update_alternatives(
    connection: &Connection,
    id: &str,
    value: bool,
) -> Result<(), HttpError> {
    let changed = connection
        .execute(
            "UPDATE inventory_wish_group SET alternatives=?2 WHERE id=?1",
            params![id, i64::from(value)],
        )
        .await
        .map_err(boom)?;
    if changed == 0 {
        return Err(refused(format!("no wish group `{id}`")));
    }
    Ok(())
}

async fn save_group(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<SaveWishGroupRequest>,
) -> Result<Json<WishGroup>, HttpError> {
    let mut connection = store.writer().await?;
    let tx = begin(&mut connection).await?;
    let result = async {
        let now = sjel_store::now_offset("'+0 seconds'");
        tx.execute(
            format!(
                "INSERT INTO inventory_wish_group (id,title,constraint_text,created_at,updated_at)
             VALUES (?1,?2,?3,{now},{now}) ON CONFLICT(id) DO UPDATE SET
             title=excluded.title,constraint_text=excluded.constraint_text,updated_at={now}"
            ),
            params![
                body.id.as_str(),
                body.title.as_str(),
                body.constraint_text.as_str()
            ],
        )
        .await
        .map_err(boom)?;
        if let Some(members) = body.item_ids.as_deref() {
            replace_members(&tx, &body.id, members).await?;
        }
        if let Some(value) = body.alternatives {
            update_alternatives(&tx, &body.id, value).await?;
        }
        Ok(WishGroup {
            id: body.id,
            title: body.title,
            constraint_text: body.constraint_text,
            alternatives: body.alternatives.unwrap_or(false),
            created_at: String::new(),
            updated_at: String::new(),
        })
    }
    .await;
    Ok(Json(items::finish_write(tx, result).await?))
}

async fn begin(
    connection: &mut Connection,
) -> Result<turso::transaction::Transaction<'_>, HttpError> {
    connection
        .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
        .await
        .map_err(boom)
}

async fn delete_group(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let connection = store.writer().await?;
    let count = connection
        .execute("DELETE FROM inventory_wish_group WHERE id=?1", [id])
        .await
        .map_err(boom)?;
    if count == 0 {
        return Err((StatusCode::NOT_FOUND, "wish group not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_members(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<SetWishGroupItemsRequest>,
) -> Result<StatusCode, HttpError> {
    let mut connection = store.writer().await?;
    let tx = begin(&mut connection).await?;
    let result = replace_members(&tx, &id, &body.item_ids).await;
    items::finish_write(tx, result).await?;
    Ok(StatusCode::OK)
}

async fn set_alternatives(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<SetAlternativesRequest>,
) -> Result<StatusCode, HttpError> {
    update_alternatives(&store.writer().await?, &id, body.alternatives).await?;
    Ok(StatusCode::OK)
}

async fn write_numbers(
    store: &WalPreview,
    scope: Numbers,
    id: &str,
    values: &[Spec],
) -> Result<StatusCode, HttpError> {
    compare::check_specs(values).map_err(refused)?;
    let mut connection = store.writer().await?;
    let tx = begin(&mut connection).await?;
    let (table, column, parent, label) = scope.relation();
    let result = async {
        let mut rows = tx
            .query(format!("SELECT 1 FROM {parent} WHERE id=?1"), [id])
            .await
            .map_err(refused)?;
        let exists = rows.next().await.map_err(refused)?.is_some();
        drop(rows);
        if !exists {
            return Err(refused(format!("no {label} `{id}`")));
        }
        tx.execute(format!("DELETE FROM {table} WHERE {column}=?1"), [id])
            .await
            .map_err(refused)?;
        for spec in values {
            tx.execute(
                format!("INSERT INTO {table} ({column},key,value,unit) VALUES (?1,?2,?3,?4)"),
                params![id, spec.key.as_str(), spec.value, spec.unit.as_str()],
            )
            .await
            .map_err(refused)?;
        }
        Ok(StatusCode::OK)
    }
    .await;
    items::finish_write(tx, result).await
}

async fn set_specs(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<SetSpecsRequest>,
) -> Result<StatusCode, HttpError> {
    write_numbers(&store, Numbers::Specs, &id, &body.specs).await
}
async fn set_needs(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<SetNeedsRequest>,
) -> Result<StatusCode, HttpError> {
    write_numbers(&store, Numbers::Needs, &id, &body.needs).await
}

async fn price_on(
    connection: &Connection,
    id: &str,
    body: &RecordPriceRequest,
) -> Result<PriceRecorded, HttpError> {
    if body.price_cents <= 0 {
        return Err(refused("a price must be above zero"));
    }
    let source = body.source.trim();
    if source.is_empty() {
        return Err(refused("a price needs a source"));
    }
    let (item, _) = items::item_on(connection, id)
        .await?
        .ok_or_else(|| refused(format!("no entry `{id}`")))?;
    let at = match body
        .observed_at
        .as_deref()
        .map(str::trim)
        .filter(|at| !at.is_empty())
    {
        Some(at) => {
            compare::check_date(at).map_err(refused)?;
            at.to_owned()
        }
        None => {
            let mut now = connection
                .query(
                    format!("SELECT {}", sjel_store::now_offset("'+0 seconds'")),
                    (),
                )
                .await
                .map_err(boom)?;
            now.next()
                .await
                .map_err(boom)?
                .ok_or_else(|| boom("missing observation time"))?
                .get::<String>(0)
                .map_err(boom)?
        }
    };
    let on = at
        .get(..10)
        .ok_or_else(|| boom("invalid observation time"))?;
    let mut rows = connection
        .query(
            "SELECT 1 FROM inventory_price_observation WHERE item_id=?1 AND alert=1 LIMIT 1",
            [id],
        )
        .await
        .map_err(boom)?;
    let alerted = rows.next().await.map_err(boom)?.is_some();
    drop(rows);
    let alert = !alerted
        && item
            .cost_max_cents
            .is_some_and(|limit| body.price_cents <= limit);
    let added=connection.execute(
        "INSERT OR IGNORE INTO inventory_price_observation
         (item_id,source,price_cents,configuration,observed_at,observed_on,alert) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id,source,body.price_cents,body.configuration.as_str(),at.as_str(),on,i64::from(alert)],
    ).await.map_err(refused)?==1;
    Ok(PriceRecorded {
        added,
        alert: added && alert,
    })
}

async fn record_price(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
    Json(body): Json<RecordPriceRequest>,
) -> Result<Json<PriceRecorded>, HttpError> {
    let mut connection = store.writer().await?;
    let tx = begin(&mut connection).await?;
    let result = price_on(&tx, &id, &body).await;
    Ok(Json(items::finish_write(tx, result).await?))
}
