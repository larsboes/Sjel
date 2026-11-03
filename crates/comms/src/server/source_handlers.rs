use super::*;

#[derive(Debug, Serialize)]
pub(super) struct FeedSourceOut {
    id: String,
    adapter: String,
    enabled: bool,
    source_url: String,
    query_configured: bool,
    limit: usize,
    last_run_at: Option<String>,
    /// Additive, and the reason a bare `last_run_at` is not enough: a source that has failed
    /// every attempt for a week has a `last_run_at` seconds old. `last_success_at` is when it
    /// last WORKED, and the class plus the streak is why it has not since.
    last_success_at: Option<String>,
    last_error: Option<String>,
    consecutive_failures: i32,
}

pub(super) async fn sources_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<FeedSourceOut>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        cfg.feed_sources
            .into_iter()
            .map(|source| {
                let state = store
                    .get_source_state(&source.id)
                    .map_err(|error| error.to_string())?;
                Ok(FeedSourceOut {
                    id: source.id.clone(),
                    adapter: source.adapter.clone(),
                    enabled: source.enabled,
                    source_url: sources::source_url(&source),
                    query_configured: source
                        .query
                        .as_deref()
                        .is_some_and(|query| !query.trim().is_empty()),
                    limit: source.limit,
                    last_run_at: state.as_ref().map(|state| state.last_run_at.clone()),
                    last_success_at: state
                        .as_ref()
                        .and_then(|state| state.last_success_at.clone()),
                    last_error: state.as_ref().and_then(|state| state.last_error.clone()),
                    consecutive_failures: state
                        .as_ref()
                        .map_or(0, |state| state.consecutive_failures),
                })
            })
            .collect()
    })
    .await;

    match result {
        Ok(Ok(sources)) => (StatusCode::OK, Json(json!({ "sources": sources }))),
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

#[derive(Debug, Deserialize)]
pub(super) struct SourceScanBody {
    source_id: Option<String>,
}

/// A refusal from the scan, with whatever the caller needs to judge it.
///
/// `From<String>` so the ordinary refusals stay one line and the `?` operator keeps working
/// on a `String`; the all-sources-failed case fills `extra` with the per-source rows.
pub(super) struct ScanRefusal {
    message: String,
    extra: Value,
}

impl From<String> for ScanRefusal {
    fn from(message: String) -> Self {
        Self {
            message,
            extra: json!({}),
        }
    }
}

pub(super) async fn source_scan_handler(Json(body): Json<SourceScanBody>) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(Value, Vec<String>), ScanRefusal> {
        let cfg = Config::load();
        let selected = cfg
            .feed_sources
            .into_iter()
            .filter(|source| source.enabled)
            .filter(|source| {
                body.source_id
                    .as_deref()
                    .is_none_or(|requested| requested == source.id)
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("no matching enabled Feed source".to_string().into());
        }
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let mut results = Vec::new();
        let mut ids = HashSet::new();
        let mut source_errors = 0usize;
        let selected_count = selected.len();
        for source in selected {
            // One source failing does not fail the run — the rule the extractor loop below
            // already applies to one URL, applied one level up where it was missing. The `?` that
            // was here meant a single upstream rate-limiting the collector took every other
            // source down with it: arXiv answered 429, and GitHub trending, which was fine, was
            // never asked. Invisible while a human clicked the button and retried; not invisible
            // to any caller that runs this on a timer with nobody watching.
            let found = match sources::fetch(&source) {
                Ok(found) => found,
                Err(error) => {
                    let error = error.to_string();
                    eprintln!("source {}: {error}", source.id);
                    source_errors += 1;
                    // Reported per source rather than only logged, so a surface can say WHICH one
                    // is failing -- and recorded, which this path did not do. `record_run` stays
                    // uncalled because it would write a cursor for a collector that never ran;
                    // `record_sweep_failure` is the call for a source that ran and failed, and it
                    // leaves `last_success_at` alone, so `/__sjel/freshness` still reads this
                    // source as stale while `GET /sources` gains the class and the streak.
                    //
                    // `last_run_at` moves here, which is the convention the inbox sweep's own
                    // failure path already set: it means "last attempt". That is exactly why the
                    // reader needs `last_success_at` beside it.
                    let class = sweep_error_class(&error);
                    store
                        .record_sweep_failure(&source.id, class)
                        .map_err(|error| error.to_string())?;
                    results.push(json!({
                        "source_id": source.id,
                        "adapter": source.adapter,
                        "discovered": 0,
                        "fetched": 0,
                        "new_count": 0,
                        "failed": [],
                        "error": error,
                        // The class beside the reason, so a caller can act on the difference
                        // without parsing prose. It is the same value `record_sweep_failure`
                        // stores, computed once here rather than derived twice.
                        "error_class": class,
                    }));
                    continue;
                }
            };
            let mut new_count = 0;
            let mut failed = Vec::new();

            // The collector discovered URLs; the extractor builds the items, so
            // a repository found here and one pasted by hand land identical
            // (#79). One URL failing to extract does not fail the run.
            // The one place a feed item can become anything other than c1: a
            // declared collector, saying in config what it collects. `media::fetch` leaves every item undeclared, so a
            // source that never reaches this line keeps the fail-closed
            // default -- which is what the ingest and vault-import paths do.
            let declared = content_item::DataClass::declared_by_source(
                &source.data_class,
                &format!("Declared by feed source '{}'.", source.id),
            );

            for discovered in &found {
                let mut item = match media::fetch(&discovered.url) {
                    Ok(item) => item,
                    Err(error) => {
                        eprintln!("source {}: {} -> {error}", source.id, discovered.url);
                        failed.push(discovered.url.clone());
                        continue;
                    }
                };
                item.declare_class(&declared);

                if store
                    .upsert_feed(&item)
                    .map_err(|error| error.to_string())?
                {
                    new_count += 1;
                }
                store
                    .record_feed_origin(
                        &item.id,
                        &source.id,
                        &sources::source_url(&source),
                        discovered.label.as_deref().or(Some(&source.adapter)),
                    )
                    .map_err(|error| error.to_string())?;
                ids.insert(item.id.clone());
            }
            // `record_sweep_success`, not `record_run`. This endpoint is what feed-sweep and the
            // dashboard call, and `record_run` writes only `last_run_at` and the cursor, so a
            // source scanned here could contribute to `GET /__sjel/freshness` neither by success
            // nor by failure. Measured 2026-10-13: `arxiv-ai-recent` and `github-trending-daily`
            // had run every six hours for days with `last_success_at` NULL, which left that
            // contract resting on the inbox sweep alone -- and it went stale the moment gmail's
            // refresh token did, while two healthy producers went unmentioned.
            store
                .record_sweep_success(&source.id, found.len() as i64, new_count as i64)
                .map_err(|error| error.to_string())?;
            let stored = found.len() - failed.len();
            results.push(json!({
                "source_id": source.id,
                "adapter": source.adapter,
                "discovered": found.len(),
                "fetched": stored,
                "new_count": new_count,
                "known_count": stored.saturating_sub(new_count),
                "failed": failed,
            }));
        }
        if source_errors > 0 && source_errors == selected_count {
            // Nothing was collected at all. Reported as the error it is, with the per-source
            // reasons still in the log, rather than as a 200 carrying zeroes.
            //
            // The rows travel with the refusal, because the caller has to tell one outage from
            // another: every upstream refusing at once — a quota, a network that was down — is
            // a collector's ordinary weather and recovers on the next run, while an auth or a
            // config failure is this deployment's to fix. Only the caller can act on that
            // difference, and it cannot act on it from a bare message.
            return Err(ScanRefusal {
                message: format!(
                    "every selected Feed source failed ({source_errors} of {selected_count}) — see the comms log for each"
                ),
                extra: json!({ "sources": results, "source_errors": source_errors }),
            });
        }
        Ok((
            json!({
                "sources": results,
                "fetched": results.iter().map(|result| result["fetched"].as_u64().unwrap_or(0)).sum::<u64>(),
                "new_count": results.iter().map(|result| result["new_count"].as_u64().unwrap_or(0)).sum::<u64>(),
                // Every source failing is still an error, and has to stay one: a caller that
                // treats 200 as "collection happened" would otherwise read a total outage as a
                // quiet day. It is the partial case that is now a success.
                "source_errors": source_errors,
            }),
            ids.into_iter().collect(),
        ))
    })
    .await;

    match result {
        Ok(Ok((value, ids))) => {
            enrich_many_in_background(ids);
            (StatusCode::OK, Json(value))
        }
        Ok(Err(refusal)) => {
            // One body shape whether the scan answered or refused: the message, plus whatever
            // the refusal carried.
            let mut body = json!({ "error": refusal.message });
            if let (Some(fields), Some(extra)) = (body.as_object_mut(), refusal.extra.as_object()) {
                for (name, value) in extra {
                    fields.insert(name.clone(), value.clone());
                }
            }
            (StatusCode::BAD_REQUEST, Json(body))
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

/// `GET /__sjel/freshness` — when this capability last took delivery of data.
///
/// The shared shape behind `freshness_advise_hours` / `freshness_stale_hours` in
/// `schemas/service.toml.example`. One integer, `last_arrival_at`, epoch seconds, or `null` if
/// nothing has ever arrived. Named for arrival rather than for a row: a source that publishes
/// nothing for a week is quiet, not broken, and a contract keyed on the newest ITEM would call
/// that a fault. What the contract is for is a producer that stopped.
///
/// This exists because nine days of an empty Inbox passed with every check on the machine green
/// (PRD D13). `tools/doctor` verifies declarations, health and persistence; none of those is
/// falsified by a collector nobody is running.
///
/// Two producers, and the answer is the older one. Feed sources stand in for each other (the
/// newest of them counts); the inbox sweep does not stand in for them, nor they for it. On
/// 2026-10-23 the sweep had failed on `auth` for 2.5 days while feed arrivals kept this answer
/// at 3.7h, so doctor stayed green over a blind inbox.
pub(super) async fn freshness_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<i64>, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let feed = store
            .newest_source_success(&[INBOX_SWEEP_SOURCE])
            .map_err(|e| e.to_string())?;
        if cfg.inbox_sweep_minutes == 0 {
            return Ok(feed);
        }
        let mail = store
            .get_source_state(INBOX_SWEEP_SOURCE)
            .map_err(|e| e.to_string())?
            .and_then(|s| s.last_success_at)
            .and_then(|at| at.parse::<i64>().ok());
        // Either producer never delivering reads as "never", not as the other one's age.
        Ok(feed.zip(mail).map(|(f, m)| f.min(m)))
    })
    .await;
    match result {
        Ok(Ok(last)) => (StatusCode::OK, Json(json!({ "last_arrival_at": last }))),
        Ok(Err(error)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}
