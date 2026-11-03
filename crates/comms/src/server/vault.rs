use super::*;

#[derive(Debug, Serialize)]
pub(super) struct VaultCandidateOut {
    id: String,
    source_id: String,
    source_ref: String,
    label: Option<String>,
    url: String,
    imported: bool,
}

pub(super) async fn vault_scan_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<VaultCandidateOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        vault_links::scan(&cfg.vault_link_sources)
            .into_iter()
            .map(|candidate| {
                let imported = store
                    .get_feed_status(&candidate.id)
                    .map_err(|error| error.to_string())?
                    .is_some();
                Ok(VaultCandidateOut {
                    id: candidate.id,
                    source_id: candidate.source_id,
                    source_ref: candidate.source_ref,
                    label: candidate.label,
                    url: candidate.url,
                    imported,
                })
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(candidates)) => (StatusCode::OK, Json(json!(candidates))),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "vault link scan failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct VaultImportBody {
    source_id: String,
    url: String,
}

pub(super) async fn vault_import_handler(Json(body): Json<VaultImportBody>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<FeedFullItem, String> {
        let cfg = Config::load();
        let candidate = vault_links::scan(&cfg.vault_link_sources)
            .into_iter()
            .find(|candidate| candidate.source_id == body.source_id && candidate.url == body.url)
            .ok_or_else(|| "link is not in a configured Vault source".to_string())?;
        let mut item = media::fetch(&candidate.url).map_err(|error| error.to_string())?;
        if item.title.is_none() {
            item.title = candidate.label.clone();
        }
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .upsert_feed(&item)
            .map_err(|error| error.to_string())?;
        store
            .record_feed_origin(
                &item.id,
                &candidate.source_id,
                &candidate.source_ref,
                candidate.label.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        let item = store
            .get_feed(&item.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(item);
        full_item(&store, item)
    })
    .await;

    match result {
        Ok(Ok(item)) => {
            enrich_in_background(item.id.clone());
            (StatusCode::CREATED, Json(json!(item)))
        }
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}
