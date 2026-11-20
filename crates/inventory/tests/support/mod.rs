use axum::Router;
use std::{path::Path, sync::Arc};

#[cfg(not(feature = "turso-wal"))]
pub async fn owner_router(path: &Path) -> Router {
    use sjel_inventory::api::{build_router, AppState};

    build_router(Arc::new(AppState {
        device_store: devices::store::DevicesStore::open(path).unwrap(),
        database: path.to_owned(),
    }))
}

#[cfg(feature = "turso-wal")]
pub async fn owner_router(path: &Path) -> Router {
    use sjel_inventory::store::Store;

    if !path.exists() {
        let seed = tempfile::tempdir().unwrap();
        let seed_path = seed.path().join("seed.db");
        let store = Store::open(&seed_path).unwrap();
        store
            .borrow_connection()
            .unwrap()
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        rusqlite::Connection::open(path)
            .unwrap()
            .pragma_update(None, "journal_mode", "wal")
            .unwrap();
    }
    let fixture = tempfile::tempdir().unwrap();
    let registry = fixture.path().join("devices.db");
    devices::store::DevicesStore::open(&registry).unwrap();
    sjel_inventory::api::wal_preview::owner_router(path, &registry, fixture.path())
        .await
        .unwrap()
        .layer(axum::Extension(Arc::new(fixture)))
}
