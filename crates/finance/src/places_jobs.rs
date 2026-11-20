//! Private Finance projections for Places and its maintenance commands.

use std::{path::PathBuf, sync::Arc};

use axum::{extract::Query, http::StatusCode, routing::get, Json, Router};
use finance::store::FinanceStore;
use finance_api::places;
use serde::Deserialize;

#[derive(Deserialize)]
struct ExpensesQuery {
    description: Option<String>,
}

#[derive(Deserialize)]
struct CandidatesQuery {
    source_account: String,
}

async fn read<T: Send + 'static>(
    database: Arc<PathBuf>,
    project: impl FnOnce(FinanceStore) -> Result<T, Box<dyn std::error::Error>> + Send + 'static,
) -> Result<Json<T>, StatusCode> {
    tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database)
            .and_then(project)
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

pub(super) fn router(database: Arc<PathBuf>) -> Router {
    let candidates = Arc::clone(&database);
    Router::new()
        .route(
            places::EXPENSES,
            get(move |Query(query): Query<ExpensesQuery>| async move {
                read(database, move |store| {
                    store.place_expenses(query.description.as_deref())
                })
                .await
            }),
        )
        .route(
            places::CANDIDATES,
            get(move |Query(query): Query<CandidatesQuery>| async move {
                read(candidates, move |store| {
                    store.place_candidates(&query.source_account)
                })
                .await
            }),
        )
}
