use super::*;

#[derive(Debug, Serialize)]
pub(super) struct CloudProviderOut {
    role: String,
    name: String,
    model: String,
    provider_label: &'static str,
    location: &'static str,
    data_tier: &'static str,
    billing_mode: &'static str,
    failover_priority: u16,
    max_requests_per_day: u32,
    requests_used_today: Option<u32>,
    requests_remaining_today: Option<u32>,
    max_input_tokens: u32,
    credit_expires_on: Option<String>,
    available: bool,
    unavailable_reason: Option<&'static str>,
}

pub(super) fn cloud_provider_options(
    cfg: &Config,
    store: Option<&Store>,
    utc_date: Option<&str>,
) -> Vec<CloudProviderOut> {
    cfg.inference
        .roles_with_prefix("cloud_")
        .into_iter()
        .filter(|(_, role)| role.has_cloud_policy())
        .map(|(name, role)| {
            let request_limit = role.max_requests_per_day.unwrap();
            let calls = store.and_then(|store| store.cloud_provider_calls_today(&name).ok());
            let unavailable_reason = if !role.credential_ready() {
                Some("missing_credential")
            } else if !utc_date.is_some_and(|date| role.billing_active_on(date)) {
                Some("billing_expired_or_unknown")
            } else if calls.is_none() {
                Some("budget_unavailable")
            } else if calls.is_some_and(|used| used >= request_limit) {
                Some("daily_request_limit_reached")
            } else {
                None
            };
            CloudProviderOut {
                role: name,
                name: role.provider_name.clone().unwrap_or_default(),
                model: role.model.clone(),
                provider_label: role.provider_label(),
                location: "cloud",
                data_tier: role.cloud_data_tier.unwrap().as_str(),
                billing_mode: role.billing_mode.unwrap().as_str(),
                failover_priority: role.failover_priority(),
                max_requests_per_day: request_limit,
                requests_used_today: calls,
                requests_remaining_today: calls.map(|used| request_limit.saturating_sub(used)),
                max_input_tokens: role.max_input_tokens.unwrap(),
                credit_expires_on: role.credit_expires_on.clone(),
                available: unavailable_reason.is_none(),
                unavailable_reason,
            }
        })
        .collect()
}

pub(super) async fn cloud_providers_handler() -> Json<Value> {
    let providers = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).ok();
        let utc_date = store.as_ref().and_then(|store| store.utc_date().ok());
        cloud_provider_options(&cfg, store.as_ref(), utc_date.as_deref())
    })
    .await
    .unwrap_or_default();
    Json(json!(providers))
}

pub(super) async fn cloud_preview_handler(
    Path((source, id)): Path<(String, String)>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativePreview>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let registry = crate::people_registry::entity_registry();
            cloud_derivative::prepare_pseudonymized(&item.cloud_input(), registry)
                .map(|p| p.preview)
                .map(Some)
                .map_err(|refusal| refusal.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(preview))) => (StatusCode::OK, Json(json!(preview))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if error == cloud_derivative::LOCAL_ONLY_REFUSAL => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud preview failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CloudApprovalBody {
    preview_hash: String,
}

pub(super) async fn cloud_approval_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudApprovalBody>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            // The class check that used to sit below this line is gone: there
            // is no preview to compare a hash against in the first place, so
            // the refusal happens one step earlier and cannot be reached with a
            // stale hash instead.
            let registry = crate::people_registry::entity_registry();
            let preview = cloud_derivative::prepare_pseudonymized(&item.cloud_input(), registry)
                .map(|p| p.preview)
                .map_err(|refusal| refusal.to_string())?;
            if preview.preview_hash != body.preview_hash {
                return Err(
                    "preview is stale; prepare and review the current document again".into(),
                );
            }
            let approval = CloudDerivativeApproval {
                source: preview.source,
                item_id: preview.id,
                source_revision: preview.source_revision,
                preview_hash: preview.preview_hash,
                original_data_class: preview.original_data_class,
                derivative_data_class: preview.derivative_data_class,
                transformation: preview.transformation.into(),
                document: preview.document,
                redaction_count: preview.redaction_count as i32,
            };
            store
                .stage_cloud_derivative(&approval)
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if error.starts_with("preview is stale") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error == cloud_derivative::LOCAL_ONLY_REFUSAL => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud approval staging failed" })),
        ),
    }
}
#[derive(Debug, Deserialize)]
pub(super) struct CloudQueueBody {
    preview_hash: String,
    provider_role: String,
}

pub(super) async fn cloud_queue_handler(
    Path((source, id)): Path<(String, String)>,
    Json(body): Json<CloudQueueBody>,
) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<CloudDerivativeState>, String> {
            let cfg = Config::load();
            let role = cfg
                .inference
                .roles_with_prefix("cloud_")
                .into_iter()
                .find_map(|(name, role)| (name == body.provider_role).then_some(role))
                .filter(|role| role.has_cloud_policy())
                .ok_or_else(|| {
                    "provider role is not a configured reviewed HTTPS cloud role".to_string()
                })?;
            if !role.credential_ready() {
                return Err("provider credential is not materialized".into());
            }

            let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
            let Some(item) = load_content_item(&store, &source, &id)? else {
                return Ok(None);
            };
            let registry = crate::people_registry::entity_registry();
            let preview = cloud_derivative::prepare_pseudonymized(&item.cloud_input(), registry)
                .map(|p| p.preview)
                .map_err(|refusal| refusal.to_string())?;
            if preview.preview_hash != body.preview_hash {
                return Err("approved derivative is stale; prepare and review it again".into());
            }
            if !cloud_derivative::tier_allows(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                &preview.original_data_class,
                &preview.derivative_data_class,
                preview.transformation,
            ) {
                return Err("provider role does not allow this reviewed derivative".into());
            }
            let utc_date = store.utc_date().map_err(|error| error.to_string())?;
            let provider_name = role.provider_name.as_deref().unwrap_or("");
            let providers = sjel_inference::ReviewedProvidersList::load();
            providers
                .check_admission(provider_name, &preview.derivative_data_class, &utc_date)
                .map_err(|e| format!("provider review policy: {e}"))?;
            if !role.billing_active_on(&utc_date) {
                return Err("provider billing policy is expired or inactive".into());
            }
            let input_upper_bound = cloud_dispatch::input_token_upper_bound(&preview.document);
            if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
                return Err(
                    "provider input token ceiling is below this reviewed derivative".into(),
                );
            }
            let calls = store
                .cloud_provider_calls_today(&body.provider_role)
                .map_err(|error| error.to_string())?;
            if calls >= role.max_requests_per_day.unwrap_or(0) {
                return Err("provider daily request ceiling is reached".into());
            }
            store
                .queue_cloud_derivative(&CloudQueueRequest {
                    source: preview.source,
                    item_id: preview.id,
                    source_revision: preview.source_revision,
                    preview_hash: preview.preview_hash,
                    provider_role: body.provider_role,
                    // The reviewed queue asks for the structured analysis. The
                    // digest task exists too now and is queued by the drain, not
                    // from here — this endpoint is the human-approval lane.
                    task: cloud_dispatch::TASK_VERSION.into(),
                })
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await;

    match result {
        Ok(Ok(Some(state))) => (StatusCode::OK, Json(json!(state))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error))
            if error.starts_with("provider role") || error.starts_with("provider credential") =>
        {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.contains("stale") || error.starts_with("approved derivative") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error == cloud_derivative::LOCAL_ONLY_REFUSAL => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud queue failed" })),
        ),
    }
}

/// The HTTP shell around `cloud_run::run_job`.
///
/// The roster walk, the budget claim and the tier checks moved into
/// `capabilities/comms/src/cloud_run.rs` when the digest drain became a second
/// caller of the same path. Keeping a copy here would have meant two
/// implementations of the five-call cap and the `preview_hash` pin.
pub(super) async fn cloud_run_handler(Path(job_id): Path<String>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<CloudDerivativeState, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        cloud_run::run_job(&store, &cfg, &job_id)
    })
    .await;

    match result {
        Ok(Ok(state)) => (StatusCode::OK, Json(json!(state))),
        Ok(Err(error)) if error.starts_with("provider role") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.starts_with("provider policy blocked") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        Ok(Err(error)) if error.starts_with("dispatch failed:") => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": error })))
        }
        Ok(Err(error)) if error.starts_with("cloud job") => {
            (StatusCode::CONFLICT, Json(json!({ "error": error })))
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "cloud job execution failed" })),
        ),
    }
}
