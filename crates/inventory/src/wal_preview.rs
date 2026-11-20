//! Preview routes for validating an owner-only native Turso file.

use std::path::Path;
use std::time::Duration;

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::Response;
use turso::transaction::TransactionBehavior;
use turso::{params_from_iter, Builder, Connection, Database, Row};

use super::{
    apply_sync_envelope, boom, catalogue_entries, expected_revision, health, inventory_page,
    item_reference, merge_patch, search_hits, signed_sync_auth, validate_sync_envelope,
    write_response, Arc, HttpError, Json, Path as AxumPath, Router, SearchQuery, State, StatusCode,
    SyncBackend, AGENT_ROUTES,
};
use crate::client::Catalogue;
use crate::store::{
    catalogue_query, decode_catalogue_row, item_reference_query, search_query, CatalogueRow,
    SyncWriteOutcome, WriteOutcome,
};

#[path = "wal_preview/items.rs"]
mod items;

#[path = "wal_preview/snapshot.rs"]
mod snapshot;

#[path = "wal_preview/wishlist.rs"]
mod wishlist;

#[path = "wal_preview/aliases.rs"]
mod aliases;

#[path = "wal_preview/mail.rs"]
mod mail;

type Error = Box<dyn std::error::Error>;

pub(super) struct WalPreview {
    database: Database,
}

impl WalPreview {
    async fn open(path: &Path, allow_writes: bool) -> Result<Self, Error> {
        if !path.is_file() {
            return Err("provide an existing Inventory owner file".into());
        }
        let path = path.to_str().ok_or("Inventory path is not UTF-8")?;
        let database = Builder::new_local(path)
            .read_only(!allow_writes)
            .build()
            .await?;
        let connection = database.connect()?;
        let mut rows = connection
            .query(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*'",
                (),
            )
            .await?;
        let mut names = std::collections::BTreeSet::new();
        while let Some(row) = rows.next().await? {
            names.insert(row.get::<String>(0)?);
        }
        if !names.iter().all(|name| {
            name.starts_with("inventory_")
                || name
                    .strip_prefix("__turso_internal_seq___turso_internal_autoincrement_")
                    .is_some_and(|table| table.starts_with("inventory_") && names.contains(table))
        }) || ![
            "inventory_item",
            "inventory_item_alias",
            "inventory_item_state",
            "inventory_mail_proposal",
            "inventory_price_observation",
            "inventory_sync_operation",
            "inventory_wish_group",
            "inventory_wish_group_item",
            "inventory_wish_group_need",
            "inventory_wish_spec",
        ]
        .iter()
        .all(|name| names.contains(*name))
        {
            return Err("the file must contain only the Inventory owner schema".into());
        }
        if allow_writes {
            let mut modes = connection.query("PRAGMA journal_mode", ()).await?;
            let mode = modes
                .next()
                .await?
                .ok_or("missing journal mode")?
                .get::<String>(0)?;
            if mode != "wal" {
                return Err("the writable Inventory owner requires a WAL file".into());
            }
        }
        Ok(Self { database })
    }

    async fn catalogue(&self) -> Result<Catalogue, Error> {
        let connection = self.database.connect()?;
        let mut rows = connection.query(catalogue_query("inventory"), ()).await?;
        let mut items = Catalogue::new();
        while let Some(row) = rows.next().await? {
            let (item, state) = decode_catalogue_row(&row)?;
            items.insert(item.id.clone(), (item, state));
        }
        Ok(items)
    }

    async fn search(&self, query: &str) -> Result<Vec<(String, String)>, Error> {
        let Some((sql, parameters)) = search_query("inventory", query) else {
            return Ok(Vec::new());
        };
        let connection = self.database.connect()?;
        let mut rows = connection.query(sql, params_from_iter(parameters)).await?;
        let mut hits = Vec::new();
        while let Some(row) = rows.next().await? {
            hits.push((row.get(0)?, row.get(1)?));
        }
        Ok(hits)
    }

    async fn reference(&self, id: &str) -> Result<Option<(i64, Option<String>)>, Error> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(item_reference_query("inventory"), [id])
            .await?;
        let result = match rows.next().await? {
            Some(row) => Some((row.get(0)?, row.get(1)?)),
            None => None,
        };
        Ok(result)
    }
    pub(super) async fn sync_receipt(&self, operation_id: &str) -> Result<Option<i64>, HttpError> {
        let connection = self.database.connect().map_err(boom)?;
        read_sync_receipt(&connection, operation_id).await
    }

    pub(super) async fn apply_sync_create(
        &self,
        operation_id: &str,
        body: &super::NewItem,
    ) -> Result<Option<SyncWriteOutcome>, HttpError> {
        let mut connection = self.writer().await?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(boom)?;
        let outcome = async {
            if let Some(revision) = read_sync_receipt(&tx, operation_id).await? {
                return Ok(Some(SyncWriteOutcome::Duplicate(revision)));
            }
            let Some(revision) = items::create_on(&tx, body, body.note.as_deref()).await? else {
                return Ok(None);
            };
            record_sync_receipt(&tx, operation_id, revision).await?;
            Ok(Some(SyncWriteOutcome::Edit(WriteOutcome::Written(
                revision,
            ))))
        }
        .await;
        items::finish_write(tx, outcome).await
    }

    pub(super) async fn apply_sync(
        &self,
        operation_id: &str,
        item_id: &str,
        expected: i64,
        patch: serde_json::Value,
    ) -> Result<SyncWriteOutcome, HttpError> {
        let mut connection = self.writer().await?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(boom)?;
        let outcome = sync_on(&transaction, operation_id, item_id, expected, patch).await;
        if matches!(
            outcome,
            Ok(SyncWriteOutcome::Edit(WriteOutcome::Written(_)))
        ) {
            transaction.commit().await.map_err(boom)?;
        } else {
            transaction.rollback().await.map_err(boom)?;
        }
        outcome
    }

    async fn writer(&self) -> Result<Connection, HttpError> {
        let connection = self.database.connect().map_err(boom)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(boom)?;
        connection
            .pragma_update("foreign_keys", "ON")
            .await
            .map_err(boom)?;
        Ok(connection)
    }
}

async fn patch_on(
    connection: &Connection,
    id: &str,
    expected: Option<i64>,
    patch: serde_json::Value,
) -> Result<WriteOutcome, HttpError> {
    let Some((current, state)) = items::item_on(connection, id).await? else {
        return Ok(WriteOutcome::Missing);
    };
    let item = merge_patch(&current, patch)?;
    if expected.is_some_and(|expected| expected != current.revision) {
        return Ok(WriteOutcome::Stale(Box::new(current), state));
    }
    let revision = items::update_revision(connection, &item, current.revision)
        .await?
        .ok_or_else(|| boom("Inventory edit did not update the selected revision"))?;
    Ok(WriteOutcome::Written(revision))
}

async fn read_sync_receipt(connection: &Connection, id: &str) -> Result<Option<i64>, HttpError> {
    let mut rows = connection
        .query(
            "SELECT revision FROM inventory_sync_operation WHERE operation_id = ?1",
            [id],
        )
        .await
        .map_err(boom)?;
    rows.next()
        .await
        .map_err(boom)?
        .map(|row| row.get(0).map_err(boom))
        .transpose()
}

async fn sync_on(
    connection: &Connection,
    operation_id: &str,
    item_id: &str,
    expected: i64,
    patch: serde_json::Value,
) -> Result<SyncWriteOutcome, HttpError> {
    if let Some(revision) = read_sync_receipt(connection, operation_id).await? {
        return Ok(SyncWriteOutcome::Duplicate(revision));
    }
    let outcome = patch_on(connection, item_id, Some(expected), patch).await?;
    match outcome {
        WriteOutcome::Written(revision) => {
            record_sync_receipt(connection, operation_id, revision).await?;
        }
        WriteOutcome::Missing => {
            return Err((StatusCode::NOT_FOUND, format!("no item `{item_id}`")));
        }
        WriteOutcome::Stale(_, _) => {}
    }
    Ok(SyncWriteOutcome::Edit(outcome))
}

async fn record_sync_receipt(
    connection: &Connection,
    operation_id: &str,
    revision: i64,
) -> Result<(), HttpError> {
    connection
        .execute(
            "INSERT INTO inventory_sync_operation (operation_id, revision, processed_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            turso::params![operation_id, revision],
        )
        .await
        .map_err(boom)?;
    Ok(())
}

async fn api_sync(
    State(store): State<Arc<WalPreview>>,
    axum::Extension(device): axum::Extension<devices::store::Device>,
    Json(envelope): Json<super::sync::Envelope>,
) -> Result<Json<super::sync::ResponseEnvelope>, HttpError> {
    validate_sync_envelope(&device, &envelope)?;
    Ok(Json(
        apply_sync_envelope(SyncBackend::Wal(store), device, envelope).await?,
    ))
}

async fn patch_item(
    State(store): State<Arc<WalPreview>>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(mut patch): Json<serde_json::Value>,
) -> Result<Response, HttpError> {
    let expected = expected_revision(&headers, &mut patch)?;
    let mut connection = store.writer().await?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = patch_on(&transaction, &id, expected, patch).await;
    if matches!(outcome, Ok(WriteOutcome::Written(_))) {
        transaction.commit().await.map_err(boom)?;
    } else {
        transaction.rollback().await.map_err(boom)?;
    }
    write_response(&id, expected.unwrap_or_default(), outcome?)
}

impl CatalogueRow for Row {
    fn text(&self, index: usize) -> Result<String, Error> {
        Ok(self.get(index)?)
    }

    fn optional_text(&self, index: usize) -> Result<Option<String>, Error> {
        Ok(self.get(index)?)
    }

    fn integer(&self, index: usize) -> Result<i64, Error> {
        Ok(self.get(index)?)
    }

    fn optional_integer(&self, index: usize) -> Result<Option<i64>, Error> {
        Ok(self.get(index)?)
    }
}

async fn index(
    State(store): State<Arc<WalPreview>>,
) -> Result<axum::response::Html<String>, HttpError> {
    let rows = store.catalogue().await.map_err(boom)?;
    Ok(inventory_page(&rows))
}

async fn catalogue(
    State(store): State<Arc<WalPreview>>,
) -> Result<Json<Vec<serde_json::Value>>, HttpError> {
    let rows = store.catalogue().await.map_err(boom)?;
    Ok(Json(catalogue_entries(rows)))
}

async fn search(
    State(store): State<Arc<WalPreview>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, HttpError> {
    Ok(search_hits(store.search(&query.q).await.map_err(boom)?))
}

async fn reference(
    State(store): State<Arc<WalPreview>>,
    AxumPath(reference): AxumPath<String>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let id = crate::identity::decode_item_ref(&reference)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let result = store.reference(&id).await.map_err(boom)?;
    item_reference(reference, id, result)
}

fn preview_router(
    store: Arc<WalPreview>,
    allow_writes: bool,
    device_store: Option<Arc<devices::store::DevicesStore>>,
    snapshot: Option<Arc<snapshot::SnapshotDirectory>>,
) -> Router {
    let mut router = Router::new()
        .route("/", axum::routing::get(index))
        .route("/routes", axum::routing::get(super::routes))
        .route("/health", axum::routing::get(health))
        .route("/api/inventory", axum::routing::get(catalogue))
        .route("/api/search", axum::routing::get(search))
        .route("/api/item-refs/{reference}", axum::routing::get(reference))
        .route("/api/items/{id}/state", axum::routing::get(items::history));
    router = mail::reads(aliases::reads(wishlist::reads(router)));
    if allow_writes {
        router = mail::writes(aliases::writes(wishlist::writes(router)));
        router = router
            .route("/api/items", axum::routing::post(items::create))
            .route(
                inventory_api::ROUTE_IMPORT_OVERLAY,
                axum::routing::post(items::import),
            )
            .route(
                "/api/items/{id}",
                axum::routing::put(items::replace).patch(patch_item),
            )
            .route(
                "/api/items/{id}/state",
                axum::routing::post(items::post_state),
            );
        if let Some(device_store) = device_store {
            router = router.merge(
                Router::new()
                    .route("/api/sync", axum::routing::post(api_sync))
                    .layer(axum::middleware::from_fn_with_state(
                        device_store,
                        signed_sync_auth,
                    )),
            );
        }
    }
    if let Some(snapshot) = snapshot {
        router = router.route(super::SNAPSHOT_ROUTE, snapshot::export(snapshot));
    }
    router
        .layer(axum::middleware::from_fn_with_state(
            "inventory",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(store)
}

/// Serve an existing native WAL owner with a separate Devices registry and snapshot directory.
pub async fn owner_router(
    path: &Path,
    registry: &Path,
    snapshot_directory: &Path,
) -> Result<Router, Error> {
    let registry = registry.canonicalize()?;
    if registry == path.canonicalize()? || !registry.is_file() {
        return Err("Turso Inventory requires a separate, existing Devices registry".into());
    }
    let store = Arc::new(WalPreview::open(path, true).await?);
    let snapshot = Arc::new(snapshot::SnapshotDirectory::open(
        snapshot_directory,
        &[path, &registry],
    )?);
    let devices = Arc::new(devices::store::DevicesStore::open(&registry)?);
    Ok(preview_router(store, true, Some(devices), Some(snapshot)))
}

pub async fn serve(path: &Path, port: u16, allow_writes: bool) -> Result<(), Error> {
    let store = Arc::new(WalPreview::open(path, allow_writes).await?);
    let device_store = if let Ok(registry) = sjel_config::env_var("SJEL_WAL_PREVIEW_DEVICE_DB") {
        if !allow_writes {
            return Err("a preview device registry requires --allow-writes".into());
        }
        let registry = Path::new(&registry).canonicalize()?;
        if registry == path.canonicalize()? || !registry.is_file() {
            return Err("provide a separate, existing Devices registry copy".into());
        }
        Some(Arc::new(devices::store::DevicesStore::open(&registry)?))
    } else {
        None
    };
    let snapshot = if let Ok(directory) = sjel_config::env_var("SJEL_WAL_PREVIEW_SNAPSHOT_DIR") {
        if !allow_writes {
            return Err("a preview snapshot directory requires --allow-writes".into());
        }
        Some(Arc::new(snapshot::SnapshotDirectory::open(
            Path::new(&directory),
            &[path],
        )?))
    } else {
        None
    };
    let router = preview_router(store, allow_writes, device_store, snapshot);
    sjel_server::serve_local_with("inventory", port, router, AGENT_ROUTES).await;
    Ok(())
}

#[cfg(test)]
#[path = "wal_preview/tests.rs"]
mod tests;
