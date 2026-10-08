use super::*;

pub(super) async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "comms",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness: whether this capability can actually serve, which liveness does not answer.
///
/// `health_handler` is a literal and cannot observe the database, so during a Postgres outage
/// this capability reported itself up while every query behind it failed (#126). Availability
/// is judged here instead.
///
/// A read route, like `/health`: an availability probe that needed the shared secret would
/// make every consumer hold a credential to ask whether the feed is up.
///
/// 503, not 500: the request was fine, the dependency is not, and a caller that retries should
/// be told to come back rather than to fix its input.
pub(super) async fn ready_handler() -> (StatusCode, Json<Value>) {
    let probe = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        Store::open(&cfg.database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await;
    match probe {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(json!({ "status": "ready", "service": "comms" })),
        ),
        Ok(Err(error)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable", "service": "comms", "error": error })),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unavailable",
                "service": "comms",
                "error": "readiness check failed"
            })),
        ),
    }
}

pub(super) async fn feed_handler(Query(params): Query<FeedParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedListItem>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .list_feed(
                params.stream.as_deref(),
                params.source_id.as_deref(),
                params.days.unwrap_or(7).clamp(1, 3650),
                params.include_dismissed.unwrap_or(false),
            )
            .map_err(|error| error.to_string())?;
        items
            .into_iter()
            .map(|item| -> Result<FeedListItem, String> {
                let relevance = store
                    .feed_relevance(&item.id)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .next();
                let evaluation = store
                    .feed_evaluation(&item.id)
                    .map_err(|error| error.to_string())?;
                // Only fetched for a card that has nothing to show. The list is bounded by the
                // day window, but one extra indexed read per item is still a cost worth not
                // paying for the items that already read fine.
                let digest = if item
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    store
                        .content_digest("feed", &item.id)
                        .map_err(|error| error.to_string())?
                        .and_then(|stored| stored.text)
                } else {
                    None
                };
                Ok(FeedListItem::from_store(
                    item, relevance, evaluation, digest,
                ))
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(items)) => Json(json!(items)),
        _ => Json(json!({ "error": "feed query failed" })),
    }
}

pub(super) async fn feed_origins_handler() -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<OriginSummary>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .list_origin_summaries()
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(summaries)) => Json(json!(summaries)),
        _ => Json(json!({ "error": "feed origins query failed" })),
    }
}

/// Which items arrived together, so the reader can collapse a collector run
/// into one row instead of showing a dozen unrelated-looking ones (#84).
/// Derived per request from `feed_origins`; nothing about grouping is stored.
pub(super) async fn feed_runs_handler(Query(params): Query<FeedParams>) -> Json<Value> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedRun>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .list_feed_runs(params.days.unwrap_or(7))
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(runs)) => Json(json!(runs)),
        _ => Json(json!({ "error": "feed runs query failed" })),
    }
}

pub(super) async fn quality_queue_handler(Query(params): Query<QualityParams>) -> HttpResponse {
    let limit = params.limit.unwrap_or(500).clamp(1, 2_000);
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .feed_quality_review_queue(limit)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(flags)) => (StatusCode::OK, Json(json!(flags))),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn quality_refresh_handler(Json(body): Json<QualityRefreshBody>) -> HttpResponse {
    let days = body.days.unwrap_or(3650);
    if !(1..=3650).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 3650" })),
        );
    }

    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .feed_for_relevance(days, 500, 0)
            .map_err(|error| error.to_string())?;
        let reviewed = items.len();
        let mut flagged_items = 0usize;
        let mut flag_count = 0usize;

        for item in items {
            let raw_content = store
                .get_raw_content(&item.id)
                .map_err(|error| error.to_string())?;
            let stages = store
                .feed_stage_results(&item.id)
                .map_err(|error| error.to_string())?;
            let has_ranking = store
                .feed_evaluation(&item.id)
                .map_err(|error| error.to_string())?
                .is_some();
            let flags = quality::derive(
                &item,
                raw_content.as_deref(),
                &stages,
                has_ranking,
                &cfg.quality_flags,
            );
            if !flags.is_empty() {
                flagged_items += 1;
                flag_count += flags.len();
            }
            store
                .replace_feed_quality_flags(&item.id, &flags)
                .map_err(|error| error.to_string())?;
        }

        Ok(json!({
            "reviewed": reviewed,
            "flagged_items": flagged_items,
            "flag_count": flag_count,
            "bounded_to": 500,
            "days": days,
            "provider_calls": 0,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) fn full_item(store: &Store, item: FeedItem) -> Result<FeedFullItem, String> {
    let relevance = store
        .feed_relevance(&item.id)
        .map_err(|error| error.to_string())?;
    let origins = store
        .feed_origins(&item.id)
        .map_err(|error| error.to_string())?;
    let evaluation = store
        .feed_evaluation(&item.id)
        .map_err(|error| error.to_string())?;
    let processing = store
        .feed_stage_results(&item.id)
        .map_err(|error| error.to_string())?;
    Ok(FeedFullItem::from_store(
        item, relevance, evaluation, processing, origins,
    ))
}

pub(super) async fn feed_item_handler(Path(id): Path<String>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<FeedFullItem>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .get_feed(&id)
            .map_err(|error| error.to_string())?
            .map(|item| full_item(&store, item))
            .transpose()
    })
    .await;

    match result {
        Ok(Ok(Some(item))) => (StatusCode::OK, Json(json!(item))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "feed query failed" })),
        ),
    }
}

pub(super) fn load_content_item(
    store: &Store,
    source: &str,
    id: &str,
) -> Result<Option<ContentItemOut>, String> {
    let item = match source {
        "feed" => store
            .get_feed(id)
            .map_err(|error| error.to_string())?
            .map(|item| -> Result<ContentItemOut, String> {
                let relevance = store
                    .feed_relevance(&item.id)
                    .map_err(|error| error.to_string())?;
                let origins = store
                    .feed_origins(&item.id)
                    .map_err(|error| error.to_string())?;
                let evaluation = store
                    .feed_evaluation(&item.id)
                    .map_err(|error| error.to_string())?;
                let processing = store
                    .feed_stage_results(&item.id)
                    .map_err(|error| error.to_string())?;
                Ok(ContentItemOut::from_feed(
                    item, relevance, evaluation, processing, origins,
                ))
            })
            .transpose()?,
        "mail" => store
            .get_triage(id)
            .map_err(|error| error.to_string())?
            .map(|item| -> Result<ContentItemOut, String> {
                let relevance = store
                    .triage_relevance(&item.id)
                    .map_err(|error| error.to_string())?;
                let evaluation = store.triage_evaluation(&item.id).ok().flatten();
                Ok(ContentItemOut::from_mail(item, relevance, evaluation))
            })
            .transpose()?,
        _ => return Err("source must be 'feed' or 'mail'".into()),
    };
    item.map(|item| {
        item.attach_digest(store)
            .and_then(|item| item.attach_cloud_state(store))
            .map_err(|error| error.to_string())
    })
    .transpose()
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct DigestBody {
    /// `"standard"` or `"detailed"`. Absent means the automatic rung.
    depth: Option<String>,
    /// What the reader wants the digest to pay attention to.
    #[serde(default)]
    focus: Vec<String>,
}

/// Generate or refine one item's digest.
///
/// Synchronous on purpose, unlike `POST /ingest`: this is a button the operator
/// just pressed and is watching, so answering before the model has finished
/// would mean showing them the *old* digest and hoping they refresh.
pub(super) async fn digest_handler(
    Path((source, id)): Path<(String, String)>,
    raw: axum::body::Bytes,
) -> HttpResponse {
    // Raw bytes rather than `Option<Json<DigestBody>>`: that extractor yields
    // `None` for a *malformed* body exactly as it does for an absent one, so a
    // client typo silently produced a standard digest instead of the detailed
    // one it asked for. A button that quietly does something other than what it
    // says is worse than one that reports it could not.
    let body: DigestBody = if raw.is_empty() {
        DigestBody::default()
    } else {
        match serde_json::from_slice(&raw) {
            Ok(body) => body,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("could not read the request body: {error}") })),
                )
            }
        }
    };
    let directive = match digest::parse_directive(body.depth.as_deref(), body.focus) {
        Ok(directive) => directive,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        digest::generate(&store, &cfg, &source, &id, &directive)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "digest failed")
}

/// Draw one item as a Mermaid diagram.
pub(super) async fn diagram_handler(Path((source, id)): Path<(String, String)>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        digest::generate_diagram(&store, &cfg, &source, &id)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "diagram failed")
}

/// Pull a chartable table out of an item.
///
/// Most content has none, and that comes back as `skipped_short` rather than an
/// error: a reader that showed a failure for every ordinary blog post would
/// train the operator to stop reading the state.
pub(super) async fn chart_handler(Path((source, id)): Path<(String, String)>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        digest::generate_chart(&store, &cfg, &source, &id)
            .map(|row| row.as_ref().map(digest::to_contract))
            .map_err(|error| error.to_string())
    })
    .await;
    digest_response(result, "chart failed")
}

pub(super) fn digest_response(
    result: Result<Result<Option<content_item::Digest>, String>, tokio::task::JoinError>,
    failure: &'static str,
) -> HttpResponse {
    match result {
        Ok(Ok(Some(digest))) => (StatusCode::OK, Json(json!(digest))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if sjel_runtime::is_deferred(&error) => (
            StatusCode::CONFLICT,
            Json(json!({ "state": "policy_deferred", "error": error })),
        ),
        Ok(Err(error)) if error.starts_with("unknown digest source") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": failure })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DigestRefreshBody {
    source: String,
    limit: Option<i64>,
}

/// The bounded automatic pass over one source.
///
/// Explicit rather than timer-driven: for mail this reads message bodies, and a
/// background job that quietly pulls every body out of a mailbox is not
/// something a machine should start doing on its own.
pub(super) async fn digest_refresh_handler(Json(body): Json<DigestRefreshBody>) -> HttpResponse {
    let result =
        tokio::task::spawn_blocking(move || -> Result<(String, digest::DrainReport), String> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
            let report =
                digest::refresh_pending(&store, &cfg, &body.source, body.limit.unwrap_or(25))
                    .map_err(|error| error.to_string())?;
            Ok((body.source, report))
        })
        .await;

    match result {
        // `digested` keeps its meaning — rows this pass wrote on-device — and
        // the rest is additive, so a caller reading only that field still reads
        // the same number it did before the quiet lane existed.
        Ok(Ok((source, report))) => (
            StatusCode::OK,
            Json(json!({
                "source": source,
                "digested": report.written,
                "cloud_digested": report.cloud_digested,
                "cloud_failed": report.cloud_failed,
                "over_window": report.over_window,
                "unconfigured": report.unconfigured,
            })),
        ),
        Ok(Err(error)) if error.contains("no digest queue for source") => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "digest refresh failed" })),
        ),
    }
}

pub(super) async fn content_item_handler(
    Path((source, id)): Path<(String, String)>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<ContentItemOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        load_content_item(&store, &source, &id)
    })
    .await;

    match result {
        Ok(Ok(Some(item))) => (StatusCode::OK, Json(json!(item))),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) if error == "source must be 'feed' or 'mail'" => {
            error_response(StatusCode::BAD_REQUEST, error)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "content query failed" })),
        ),
    }
}
