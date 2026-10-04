use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct StatusBody {
    pub(super) status: String,
    /// Where the press happened. Absent means `api`, so every caller that
    /// predates the ledger keeps working and still leaves a row.
    pub(super) surface: Option<String>,
}

/// The only writer of the decisive verbs.
///
/// `set_feed_status` commits the UPDATE and one `kept`/`dismissed`/`unkept`
/// ledger row together. `POST /feed/:id/interactions` refuses those three and
/// takes `opened`/`reopened` only, so one decision is one row.
pub(super) async fn feed_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_feed_status(&id, &body.status, body.surface.as_deref().unwrap_or("api"))
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct InteractionBody {
    event: String,
    surface: Option<String>,
}

/// Record that the operator looked at an item.
///
/// Reads only. The decisive verbs are refused here by name, with the route
/// that owns them in the message: a client that becomes a second writer of a
/// keep doubles every count the learned factor is gated on, and the gate's own
/// sample report is the first thing that would lie.
pub(super) async fn feed_interactions_handler(
    Path(id): Path<String>,
    Json(body): Json<InteractionBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(bool, String), String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let event = body.event;
        let recorded = store
            .record_interaction(&id, &event, body.surface.as_deref().unwrap_or("api"))
            .map_err(|error| error.to_string())?;
        Ok((recorded, event))
    })
    .await;

    match result {
        Ok(Ok((true, event))) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "recorded": event })),
        ),
        Ok(Ok((false, _))) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

/// The learned feedback model as it currently stands, with the vocabulary it
/// was fitted against.
///
/// Loaded from the second `comms_feed_context_snapshots` row rather than a table
/// of its own: the table is already keyed on `context_kind`, the travel snapshot
/// is the same shape, and a new table would be a migration for a blob.
pub(super) struct FeedbackContext {
    model: comms::feedback::FeedbackModel,
    space: comms::feedback::FeatureSpace,
    /// `feedback-<hash>` when active, the literal `none` when not.
    pub(super) revision: String,
    /// Which collector source each item arrived from, for the source slot.
    sources: BTreeMap<String, String>,
}

impl FeedbackContext {
    /// The factor for one item. Inert, it renders at weight 0 with the count it
    /// is short by, because a factor nobody can see is a factor nobody can
    /// argue with.
    fn factor(
        &self,
        item: &FeedItem,
        matches: &[RelevanceMatch],
    ) -> comms::evaluation::EvaluationFactor {
        if !self.model.active {
            return self.model.factor(None, comms::evaluation::FEEDBACK_WEIGHT);
        }
        let features = comms::feedback::features(
            &self.space,
            &comms::feedback::FeatureInput {
                item,
                matches,
                source_id: self.sources.get(&item.id).map(String::as_str),
                now: None,
            },
        );
        self.model
            .factor(Some(&features), comms::evaluation::FEEDBACK_WEIGHT)
    }
}

/// Read the stored model, or report an untrained one.
///
/// A stored model whose feature names differ from the ones computed now is stale
/// by definition — a new TELOS lens or a new feed source changes the vocabulary
/// — so it is discarded rather than scored against. That is an invalidation rule
/// rather than a guess about which piece of configuration moved.
pub(super) fn load_feedback(
    store: &Store,
    profiles: &[relevance::InterestProfile],
) -> FeedbackContext {
    let sources = store.feed_origin_sources().unwrap_or_default();
    let declared = {
        let mut ids = sources.values().cloned().collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    };
    let space = comms::feedback::FeatureSpace::new(declared, profiles);
    let stored = store
        .context_snapshot(FEEDBACK_SNAPSHOT_KIND)
        .ok()
        .flatten()
        .and_then(|snapshot| {
            serde_json::from_str::<comms::feedback::FeedbackModel>(&snapshot.payload).ok()
        })
        .filter(|model| comms::feedback::is_usable(model, &space));
    let counts = store
        .training_labels()
        .map(|training| comms::feedback::SampleCounts {
            kept: training.labels.iter().filter(|label| label.kept).count(),
            dismissed: training.labels.iter().filter(|label| !label.kept).count(),
            total: training.labels.len(),
            seeded_from_status: training.seeded_from_status,
            skipped_class: training.skipped_class,
        })
        .unwrap_or_default();
    let model = stored.unwrap_or_else(|| comms::feedback::untrained(&space, counts));
    let revision = model.revision();
    FeedbackContext {
        model,
        space,
        revision,
        sources,
    }
}

/// The `context_kind` the learned model is stored under.
pub(super) const FEEDBACK_SNAPSHOT_KIND: &str = "feedback-model";

#[derive(Debug, Deserialize)]
pub(super) struct TrainBody {
    /// Compute and report the gate without writing the snapshot.
    dry_run: Option<bool>,
}

/// Refit the learned factor from the ledger.
///
/// A full deterministic refit, never an incremental update: ≤372 rows over ~50
/// features is microseconds, and incremental weights depend on arrival order,
/// which cannot be rederived from the ledger — that would make the revision
/// unusable as a cache key, which is the property the snapshot mechanism rests
/// on.
pub(super) async fn model_train_handler(Json(body): Json<TrainBody>) -> HttpResponse {
    let dry_run = body.dry_run.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        let context = load_feedback(&store, &profiles);
        let training = store.training_labels().map_err(|error| error.to_string())?;
        let ids = training
            .labels
            .iter()
            .map(|label| label.feed_id.clone())
            .collect::<Vec<_>>();
        let items = store
            .feed_items_by_ids(&ids)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect::<BTreeMap<_, _>>();
        let matches = store
            .feed_relevance_map(&ids)
            .map_err(|error| error.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let rows = training
            .labels
            .iter()
            .filter_map(|label| {
                let item = items.get(&label.feed_id)?;
                let vector = comms::feedback::features(
                    &context.space,
                    &comms::feedback::FeatureInput {
                        item,
                        matches: matches
                            .get(&label.feed_id)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        source_id: context.sources.get(&label.feed_id).map(String::as_str),
                        now: Some(now),
                    },
                );
                Some(comms::feedback::TrainingRow::from_label(label, vector))
            })
            .collect::<Vec<_>>();
        let trained_at = chrono_free_stamp(now);
        let model = comms::feedback::train(
            &context.space,
            &rows,
            training.skipped_class,
            now,
            &trained_at,
        );
        let revision = model.revision();
        if !dry_run {
            let payload = serde_json::to_string(&model).map_err(|error| error.to_string())?;
            store
                .replace_context_snapshot(FEEDBACK_SNAPSHOT_KIND, &revision, &payload)
                .map_err(|error| error.to_string())?;
        }
        // The strongest signals are reported only for a model that HAS signals.
        // An inert model's weights are all zero, and listing eight zeroes as
        // "top features" would be the endpoint saying something it cannot mean.
        Ok(feedback_status(&model, &revision, model.active))
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

/// An ISO-ish day stamp without a date dependency. comms carries no date crate
/// and the store's own clock writes the durable timestamps; this is a label.
fn chrono_free_stamp(epoch_seconds: i64) -> String {
    let days = epoch_seconds / 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since 1970-01-01 back to a Gregorian date (Howard Hinnant's algorithm,
/// the inverse of the one `evaluation` already carries).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// The one shape both the train route and the status endpoint report the model
/// in, so a reader comparing them is comparing the same fields.
fn feedback_status(
    model: &comms::feedback::FeedbackModel,
    revision: &str,
    include_features: bool,
) -> Value {
    let mut top_features = model
        .feature_names
        .iter()
        .zip(&model.weights)
        .enumerate()
        .filter(|(index, _)| *index != 0)
        .map(|(_, (name, weight))| (name.clone(), *weight))
        .collect::<Vec<_>>();
    top_features.sort_by(|left, right| {
        right
            .1
            .abs()
            .partial_cmp(&left.1.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    json!({
        "revision": revision,
        "active": model.active,
        "gate_reason": model.gate_reason,
        "feature_revision": model.feature_revision,
        "feature_count": model.feature_names.len(),
        "samples": {
            "kept": model.samples.kept,
            "dismissed": model.samples.dismissed,
            "total": model.samples.total,
            "seeded_from_status": model.samples.seeded_from_status,
            "skipped_class": model.samples.skipped_class,
        },
        "holdout": { "n": model.holdout.n, "auc": model.holdout.auc },
        "thresholds": {
            "min_labels": comms::feedback::MIN_LABELS,
            "min_minority": comms::feedback::MIN_MINORITY,
            "min_auc": comms::feedback::MIN_AUC,
            "weight": comms::evaluation::FEEDBACK_WEIGHT,
        },
        "trained_at": model.trained_at,
        // The names include configured source ids and TELOS lens labels, so the
        // payload is c2: it is served to the local dashboard and never reaches
        // the published demo, whose recorded path list does not carry this route.
        "top_features": include_features.then(|| top_features
            .iter()
            .take(8)
            .map(|(name, weight)| json!({ "name": name, "weight": weight }))
            .collect::<Vec<_>>()),
    })
}

#[derive(Debug, Deserialize)]
pub(super) struct FeedDataClassBody {
    data_class: String,
    /// Required when the change lowers the class, ignored when it raises it.
    /// Which of the two this is depends on what is stored, so the store decides
    /// and this handler stays out of it.
    rationale: Option<String>,
}

/// The only path by which a feed item's class goes down, and the reason the
/// endpoint exists at all: everything automatic may escalate, so escalation
/// needs no door — de-escalation needs one that can say no.
pub(super) async fn feed_data_class_handler(
    Path(id): Path<String>,
    Json(body): Json<FeedDataClassBody>,
) -> HttpResponse {
    // Before the connection, not after. A class outside the vocabulary is
    // decidable from the request alone, and answering it here keeps a
    // malformed request from opening a database handle at all — which is also
    // what lets this route be tested without a live store behind it.
    if !content_item::valid(&body.data_class) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "data class must be one of: {}",
                content_item::DATA_CLASSES.join(", ")
            ),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_feed_data_class(&id, &body.data_class, body.rationale.as_deref())
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(true)) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Ok(Ok(false)) => error_response(StatusCode::NOT_FOUND, "not found"),
        // A refused de-escalation and an unknown class are both the caller
        // asking for something that cannot be granted. Both carry the store's
        // own sentence, so the operator reads why rather than just "400".
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct IngestBody {
    url: String,
    content: Option<String>,
    title: Option<String>,
    author: Option<String>,
    /// Who is handing the content over — `axon-clip`, a CLI, a future share
    /// sheet. Recorded as the item's capture provenance; absent means the
    /// server fetched the page itself (#81).
    client: Option<String>,
}

pub(super) fn enrich_many_in_background(ids: Vec<String>) {
    tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = match Store::open(&cfg.database_path) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("ingest: enrichment skipped, store unavailable: {error}");
                return;
            }
        };
        for id in &ids {
            if let Err(error) = media::summarize_item(&store, &cfg, id) {
                eprintln!("ingest: summarize failed for {id}: {error}");
            }
        }
        let mut items = ids
            .iter()
            .filter_map(|id| store.get_feed(id).ok().flatten())
            .collect::<Vec<_>>();
        // Reported rather than failed open. A declared lens directory that has
        // moved used to leave this path scoring against nothing, which stores
        // an `unscored` row over a perfectly good item.
        let profiles = match relevance::load_profiles(&cfg.relevance) {
            Ok(profiles) => profiles,
            Err(error) => {
                eprintln!("ingest: relevance skipped, TELOS profiles unreadable: {error}");
                return;
            }
        };
        // Loopback only; see `relevance_refresh_handler` for why. The producers
        // feed `context_revision`, so a role this pass would refuse must not
        // reach the revision either -- otherwise every stored evaluation reads
        // as stale against a role that is never used.
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        let feedback = load_feedback(&store, &profiles);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
            &feedback.revision,
        );
        let semantic_available = relevance::embedding_backend_reachable(embedding_role.as_ref());
        items.retain(|item| {
            let item_revision = evaluation::item_revision(item);
            let stored = store.feed_evaluation(&item.id).ok().flatten();
            !evaluation::is_current(
                stored.as_ref(),
                &item_revision,
                &context_revision,
                semantic_available,
            )
        });
        if items.is_empty() {
            return;
        }
        let outcome = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, result) in items.iter().zip(outcome.items) {
            if let Err(error) = store.replace_feed_relevance(&item.id, &result.matches) {
                eprintln!("ingest: relevance failed for {}: {error}", item.id);
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                result.matches.first(),
                &context_revision,
                &travel_context.contexts,
                result.refused_class,
                Some(feedback.factor(item, &result.matches)),
            );
            if let Err(error) = store.replace_feed_evaluation(&evaluated) {
                eprintln!("ingest: evaluation failed for {}: {error}", item.id);
            }
        }
    });
}

pub(super) fn enrich_in_background(id: String) {
    enrich_many_in_background(vec![id]);
}

/// Store first, then summarize and score behind the response.
pub(super) async fn ingest_handler(Json(body): Json<IngestBody>) -> HttpResponse {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "url is required" })),
        );
    }
    let content = body.content;
    let title = body.title;
    let author = body.author;
    let client = body.client;

    let stored = tokio::task::spawn_blocking(move || -> Result<FeedFullItem, String> {
        let cfg = Config::load();
        let item = media::fetch_with_content(
            &url,
            content.as_deref(),
            title.as_deref(),
            author.as_deref(),
            client.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .upsert_feed(&item)
            .map_err(|error| error.to_string())?;
        let item = store
            .get_feed(&item.id)
            .map_err(|error| error.to_string())?
            .unwrap_or(item);
        full_item(&store, item)
    })
    .await;

    match stored {
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

#[derive(Debug, Deserialize)]
pub(super) struct RefreshBody {
    days: Option<i32>,
    limit: Option<usize>,
    offset: Option<usize>,
    ids: Option<Vec<String>>,
    force: Option<bool>,
}

/// What the last completed sweep recorded, parsed back out of the receipt's
/// cursor. Three fields in one TEXT column, because `migrations.rs` states why
/// a new column is the expensive option.
///
/// `completed` is the relevance revision of the last sweep that ran from
/// offset 0 through `has_more = false`. `progress` is how far the sweep now
/// running has covered, so a page that arrives out of order cannot mark the
/// corpus done — a partial page advancing the revision would leave the rows it
/// never saw looking current forever.
struct PassCursor {
    mode: String,
    completed: String,
    progress_revision: String,
    /// The `days` window the chain in progress is running with. Pages of a
    /// chain must all ask the same window, or `progress_end` compares offsets
    /// into two different lists.
    progress_days: i32,
    progress_end: usize,
}

impl PassCursor {
    fn parse(cursor: Option<&str>) -> Self {
        let raw = cursor.unwrap_or_default();
        let mut parts = raw.split('|');
        let mode = parts.next().unwrap_or_default().to_string();
        let completed = parts.next().unwrap_or_default().to_string();
        let progress = parts.next().unwrap_or_default();
        let (progress_revision, progress_end) = progress
            .rsplit_once(':')
            .map(|(revision, end)| (revision.to_string(), end.parse().unwrap_or(0)))
            .unwrap_or_default();
        // A cursor written before the window was recorded carries no `@`, and
        // reads as window 0 -- which no pass matches, so its chain simply
        // restarts at offset 0 instead of being extended by a different window.
        let (progress_revision, progress_days) = progress_revision
            .rsplit_once('@')
            .map(|(revision, days)| (revision.to_string(), days.parse().unwrap_or(0)))
            .unwrap_or((progress_revision, 0));
        Self {
            mode,
            completed,
            progress_revision,
            progress_days,
            progress_end,
        }
    }

    fn render(&self) -> String {
        format!(
            "{}|{}|{}@{}:{}",
            self.mode,
            self.completed,
            self.progress_revision,
            self.progress_days,
            self.progress_end
        )
    }

    /// Which page this pass reads.
    ///
    /// An offset the caller NAMED is the page it gets: `comms relevance
    /// backfill` walks the corpus itself and means every number it sends. An
    /// ABSENT offset resumes the chain instead of restarting it, and that is
    /// the whole of the nightly sweep's paging — `tools/feed-sweep` posts
    /// `{days: 3650, limit: 100}` with no offset, so before this it re-read the
    /// newest hundred rows every night for as long as the schedule has existed.
    /// The cursor already recorded how far the sweep had come; nothing read it
    /// back.
    ///
    /// A chain is only resumed at its own window. A page at 90 days cannot
    /// continue a 3650-day chain, because `progress_end` is an offset into a
    /// list and the two lists are not the same list.
    fn page_offset(&self, requested: Option<usize>, relevance_revision: &str, days: i32) -> usize {
        match requested {
            Some(offset) => offset,
            None if self.progress_revision == relevance_revision && self.progress_days == days => {
                self.progress_end
            }
            None => 0,
        }
    }

    /// Record how far the chain has now come.
    ///
    /// A page continues the chain when it starts exactly where the chain
    /// stopped, at the same window and the same vector space. A page at offset
    /// 0 otherwise STARTS one — but a narrower window may not take a wider
    /// chain over. The Feed panel's button asks for 90 days and
    /// `dashboard/src/routes/travel` asks for 365; without that rule either one
    /// would send the nightly 3650-day sweep back to the newest page every time
    /// the operator pressed it, which is the same stall by another door. The
    /// widest window is also the only one that can mark the corpus complete, so
    /// it is the chain worth protecting.
    ///
    /// Reaching the end of the list always resets the progress, whatever the
    /// window. It used to reset only for the widest one, which left a narrower
    /// chain parked one page past its last row: every later page of that chain
    /// selected nothing, forever.
    fn advance(
        &mut self,
        relevance_revision: &str,
        days: i32,
        offset: usize,
        considered: usize,
        has_more: bool,
    ) {
        let continues = self.progress_revision == relevance_revision
            && self.progress_days == days
            && self.progress_end == offset;
        let starts = offset == 0
            && (self.progress_revision.is_empty()
                || self.progress_revision != relevance_revision
                || days >= self.progress_days);
        if !continues && !starts {
            return;
        }
        self.progress_revision = relevance_revision.to_string();
        self.progress_days = days;
        self.progress_end = offset + considered;
        if !has_more {
            if days >= FULL_WINDOW_DAYS {
                self.completed = relevance_revision.to_string();
            }
            self.progress_revision = String::new();
            self.progress_days = 0;
            self.progress_end = 0;
        }
    }
}

/// The widest window the route admits, and the only one that can mark the
/// corpus complete.
///
/// Widened from 365. A backfill has to be able to reach the whole corpus --
/// `comms relevance backfill` and the nightly sweep both ask for ten years --
/// and a window that cannot cover the store makes the oldest rows permanently
/// unreachable, which is the same defect as the ids-after-LIMIT bug.
const FULL_WINDOW_DAYS: i32 = 3650;

/// Re-score and re-evaluate a bounded window of the feed.
///
/// The currency check is split in two, which is the whole point of this
/// rewrite. The old handler retained items by `is_current` and handed the
/// retained set straight to `score_items`, so every term in `context_revision`
/// — including the travel snapshot — was an embedding trigger. Now:
///
/// * an item is RE-SCORED (embedded) when the sweep's relevance revision has
///   moved, when it has no stored matches, when its own content changed, or
///   when its stored matches read `lexical` while an embedding role answers;
/// * every other stale item is RE-EVALUATED from `feed_relevance_map`, one
///   batched read and no model call;
/// * an item the class ladder refuses is scored by neither path, has its
///   stored matches cleared, and gets a refusal evaluation.
pub(super) async fn relevance_refresh_handler(Json(body): Json<RefreshBody>) -> HttpResponse {
    let days = body.days.unwrap_or(90);
    if !(1..=FULL_WINDOW_DAYS).contains(&days) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "days must be between 1 and 3650" })),
        );
    }
    let limit = body.limit.unwrap_or(200);
    if !(1..=500).contains(&limit) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "limit must be between 1 and 500" })),
        );
    }
    let requested_offset = body.offset;
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        // Fallible now: a declared lens directory that has moved is reported
        // rather than scored against, which is what produced 48 `unscored`
        // evaluations over good rows.
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        let requested = body.ids.unwrap_or_default();

        // Loopback only, the same filter `triage_relevance_handler` applies to
        // the same two roles. The class gate here is
        // `content_item::local_prompt_allowed`, which admits c2 -- and c2 is
        // exactly the class PRD §6.2b says may reach a cloud model only after a
        // ladder pass has reduced it to c1. `POST /feed/:id/data-class` accepts
        // any class in the vocabulary, so a c2 feed item is reachable, and
        // `item_document` joins title, author and content. A non-loopback role
        // is therefore not used at all and the pass answers lexical, which the
        // receipt reports.
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let travel_context = travel::load(&store, &cfg.travel_context);
        // The learned model is a term in the RANKING revision only. Under the
        // split currency check a refit therefore re-evaluates from stored
        // matches and re-embeds nothing.
        let feedback = load_feedback(&store, &profiles);
        let context_revision = evaluation::context_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
            &travel_context.revision,
            &feedback.revision,
        );
        let relevance_revision = evaluation::relevance_revision(
            &profiles,
            embedding_producer.as_deref(),
            reranking_producer.as_deref(),
        );
        let semantic_available = relevance::embedding_backend_reachable(embedding_role.as_ref());
        let receipt = store.relevance_pass().map_err(|error| error.to_string())?;
        let mut cursor =
            PassCursor::parse(receipt.as_ref().and_then(|state| state.cursor.as_deref()));
        let vector_space_moved = cursor.completed != relevance_revision;
        // The cursor is read BEFORE the page is selected, because it decides
        // which page that is when the caller named no offset.
        let offset = cursor.page_offset(requested_offset, &relevance_revision, days);

        // The ids filter is applied by the SELECT, not after the LIMIT. The old
        // order silently dropped any named item outside the newest page.
        let items = if requested.is_empty() {
            store
                .feed_for_relevance(days, limit, offset)
                .map_err(|error| error.to_string())?
        } else {
            store
                .feed_items_by_ids(&requested)
                .map_err(|error| error.to_string())?
        };
        let found = items
            .iter()
            .map(|item| item.id.clone())
            .collect::<HashSet<_>>();
        let missing_ids = requested
            .iter()
            .filter(|id| !found.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let has_more = requested.is_empty() && items.len() == limit;

        let considered = items.len();
        let ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        let stored_matches = store
            .feed_relevance_map(&ids)
            .map_err(|error| error.to_string())?;
        let force = body.force.unwrap_or(false);

        let mut to_rescore = Vec::new();
        let mut to_reevaluate = Vec::new();
        let mut refused = Vec::new();
        let mut skipped_current = 0usize;
        for item in items {
            if !content_item::local_prompt_allowed(&item.data_class) {
                refused.push(item);
                continue;
            }
            let stored_evaluation = store.feed_evaluation(&item.id).ok().flatten();
            let item_revision = evaluation::item_revision(&item);
            let matches = stored_matches.get(&item.id);
            let matches_are_lexical = matches
                .and_then(|rows| rows.first())
                .is_some_and(|matched| matched.mode == "lexical");
            let needs_embedding = force
                || vector_space_moved
                || matches.is_none_or(|rows| rows.is_empty())
                || stored_evaluation
                    .as_ref()
                    .is_none_or(|stored| stored.item_revision != item_revision)
                || (matches_are_lexical && semantic_available);
            if needs_embedding {
                to_rescore.push(item);
            } else if !evaluation::is_current(
                stored_evaluation.as_ref(),
                &item_revision,
                &context_revision,
                semantic_available,
            ) {
                to_reevaluate.push(item);
            } else {
                skipped_current += 1;
            }
        }

        let refused_class = refused.len();
        let mut refused_lower_tier = 0usize;
        let mut refused_matches_cleared = 0usize;
        let mut written = 0usize;
        for item in &refused {
            let item_revision = evaluation::item_revision(item);
            let stored_evaluation = store.feed_evaluation(&item.id).ok().flatten();
            // A refusal that is already stored at this revision is left alone.
            // The ordinary currency check cannot say so -- it reads `unscored`
            // as stale whenever an embedding role answers -- which rewrote every
            // c3 row and its factors on every pass.
            if evaluation::refusal_is_current(
                stored_evaluation.as_ref(),
                &item_revision,
                &context_revision,
            ) {
                skipped_current += 1;
                continue;
            }
            // The rows already derived from a refused item are removed, not
            // left to be read as a score nobody may have. `clear_` and not
            // `replace_..(&[])`: the tier gate refuses an empty replacement over
            // a `model` row, so the escalation of an already-scored item left
            // its matches exactly where they were.
            refused_matches_cleared += store
                .clear_feed_relevance(&item.id)
                .map_err(|error| error.to_string())?;
            let evaluated = evaluation::evaluate(
                item,
                None,
                &context_revision,
                &travel_context.contexts,
                true,
                Some(feedback.factor(item, &[])),
            );
            // Past the tier gate: a refusal withdraws a score, and a withdrawal
            // that loses to the score it withdraws is not a withdrawal.
            if store
                .replace_feed_evaluation_refusal(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        let outcome = relevance::score_items(
            &to_rescore,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        for (item, scored) in to_rescore.iter().zip(&outcome.items) {
            if !store
                .replace_feed_relevance(&item.id, &scored.matches)
                .map_err(|error| error.to_string())?
            {
                refused_lower_tier += 1;
                continue;
            }
            let evaluated = evaluation::evaluate(
                item,
                scored.matches.first(),
                &context_revision,
                &travel_context.contexts,
                scored.refused_class,
                Some(feedback.factor(item, &scored.matches)),
            );
            if store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        // The half a refit pays for: an evaluation rewritten from matches that
        // are already stored, with no model call at all.
        let reused_relevance = to_reevaluate.len();
        for item in &to_reevaluate {
            let matches = stored_matches
                .get(&item.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let evaluated = evaluation::evaluate(
                item,
                matches.first(),
                &context_revision,
                &travel_context.contexts,
                false,
                Some(feedback.factor(item, matches)),
            );
            if store
                .replace_feed_evaluation(&evaluated)
                .map_err(|error| error.to_string())?
            {
                written += 1;
            } else {
                refused_lower_tier += 1;
            }
        }

        let mode = if to_rescore.is_empty() {
            cursor.mode.clone()
        } else {
            outcome.mode.to_string()
        };
        if requested.is_empty() {
            // The chain only extends from a page that continues it, at the same
            // window, and only a chain that reached the end of the WIDEST window
            // marks the corpus done.
            //
            // `!has_more` alone was not that test. A pass with `days: 1` reaches
            // the end of its own window after 20 of 372 items and used to stamp
            // the current relevance revision on the whole corpus; the 352 rows
            // whose matches came from the retired vector space then read as
            // current forever. Reproduced, and reachable: `comms relevance
            // backfill` and `tools/feed-sweep` both send a window, and a
            // window that holds fewer rows than `limit` finishes immediately.
            cursor.advance(&relevance_revision, days, offset, considered, has_more);
        }
        cursor.mode = mode.clone();
        store
            .record_relevance_pass(
                &cursor.render(),
                considered as i64,
                written as i64,
                outcome.error_class,
            )
            .map_err(|error| error.to_string())?;

        Ok(json!({
            "scored": outcome.items.len(),
            "evaluated": written,
            "considered": considered,
            "skipped_current": skipped_current,
            "rescored": to_rescore.len(),
            "reused_relevance": reused_relevance,
            "refused_class": refused_class,
            "refused_lower_tier": refused_lower_tier,
            "refused_matches_cleared": refused_matches_cleared,
            "missing_ids": missing_ids,
            "profile_count": profiles.len(),
            "mode": mode,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
            "bounded_to": limit,
            "relevance_revision": relevance_revision,
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "embedding": {
                "mode": outcome.mode,
                "error_class": outcome.error_class,
                "chunks": outcome.chunks,
                "chunks_failed": outcome.chunks_failed,
            },
            "travel_context": {
                "upcoming_count": travel_context.contexts.len(),
                "reachable": travel_context.reachable,
                "from_cache": travel_context.from_cache,
                "refreshed_at": travel_context.refreshed_at,
            },
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

pub(super) async fn evaluation_status_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        // A machine with no declared lens is the ordinary case and answers 200
        // with `profile_count: 0`. A machine whose declared lens directory has
        // moved is a fault, and now says so instead of reporting zero lenses as
        // if that were a configuration.
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        // Loopback only; see `relevance_refresh_handler` for why. The producers
        // feed `context_revision`, so a role this pass would refuse must not
        // reach the revision either -- otherwise every stored evaluation reads
        // as stale against a role that is never used.
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let embedding_producer = embedding_role.as_ref().map(|role| role.cache_key());
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_producer = reranking_role.as_ref().map(|role| role.cache_key());
        let summarization_role = cfg.summarization_role();
        let summary_producer_revision = media::summary_producer_revision(&cfg);
        let travel_context = travel::cached(&store);
        let travel_revision = travel_context
            .as_ref()
            .map(|context| context.revision.as_str())
            .unwrap_or_default();
        let summary = store
            .evaluation_summary()
            .map_err(|error| error.to_string())?;
        let enrichment = store
            .feed_enrichment_counts(summary_producer_revision.as_deref())
            .map_err(|error| error.to_string())?;
        let content_status = store
            .feed_content_status_counts()
            .map_err(|error| error.to_string())?;
        let capacity_state = store
            .get_source_state(comms::capacity::LOCAL_INFERENCE_SOURCE)
            .map_err(|error| error.to_string())?;
        let unattended_role = cfg.light_summarization_role();
        let summarizer_reachable = media::summarizer_reachable(&cfg);
        // Probed rather than assumed: an operator deciding whether to press
        // Regenerate on an over-window item is asking exactly this, and a
        // stopped oMLX is the ordinary state of this machine now.
        let strong_reachable = summarization_role
            .as_ref()
            .is_some_and(|role| role.model_reachable());
        let relevance_reachable =
            relevance::embedding_backend_reachable(embedding_role.as_ref());
        // The receipt of the last pass, and the line that would have made the
        // 2026-08-30 degradation visible the day it happened: 525 rows were
        // written lexical in one pass and nothing on the machine said so.
        let feedback = load_feedback(&store, &profiles);
        // The ledger's own counters. `samples` below counts what the TRAINER
        // could label; this counts what the table holds, which is the only way
        // `opened` and `reopened` -- verbs no label reads -- reach a surface.
        let interactions = store
            .interaction_counts()
            .map_err(|error| error.to_string())?;
        let last_pass = store.relevance_pass().map_err(|error| error.to_string())?;
        let last_pass_mode = last_pass
            .as_ref()
            .and_then(|state| state.cursor.as_deref())
            .and_then(|cursor| cursor.split('|').next())
            .filter(|mode| !mode.is_empty())
            .map(str::to_string);
        let reranking_reachable =
            relevance::embedding_backend_reachable(reranking_role.as_ref());
        Ok(json!({
            "evaluator_revision": evaluation::EVALUATOR_REVISION,
            "context_revision": evaluation::context_revision(
                &profiles,
                embedding_producer.as_deref(),
                reranking_producer.as_deref(),
                travel_revision,
                &feedback.revision,
            ),
            "feedback_model": feedback_status(&feedback.model, &feedback.revision, feedback.model.active),
            "interactions": {
                "opened": interactions.opened,
                "kept": interactions.kept,
                "dismissed": interactions.dismissed,
                "reopened": interactions.reopened,
                "unkept": interactions.unkept,
                "shared": interactions.shared,
                "total": interactions.total,
            },
            "ledger": {
                "evaluated": summary.evaluated,
                "reranked": summary.reranked,
                "semantic": summary.semantic,
                "lexical": summary.lexical,
                "unscored": summary.unscored,
            },
            // Two rungs, named separately, because they are now used by
            // different callers and one number cannot describe both. `model`
            // and `reachable` are the *unattended* rung — the light local role
            // every drain runs on — since that is what the reader is asking
            // about when the feed has no digests. Reporting the strong role's
            // name beside the light role's reachability, which is what this
            // block did for one build, is worse than reporting neither: it
            // said the 9B model was up while its server was stopped.
            "summarizer": {
                "provider": unattended_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No unattended summarization role configured"),
                "model": unattended_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": unattended_role.is_some(),
                "reachable": summarizer_reachable,
                // The rung only a press reaches. Kept in the payload because a
                // reader looking at `skipped_over_window` rows wants to know
                // what pressing Regenerate would actually engage.
                "strong": {
                    "provider": summarization_role
                        .as_ref()
                        .map(|role| role.provider_label())
                        .unwrap_or("No summarization role configured"),
                    "model": summarization_role
                        .as_ref()
                        .map(|role| role.model.as_str())
                        .unwrap_or(""),
                    "configured": summarization_role.is_some(),
                    "reachable": strong_reachable,
                },
                // The durable half of the capacity alert. The drain says it on
                // stderr when the streak crosses the threshold; this is where
                // it can still be read an hour later by someone who was not
                // watching. Same `source_state` row, same shape the inbox
                // sweep's own streak is served in at /triage/sweep/status.
                "capacity": {
                    "alert_after": cfg.capacity_alert_after,
                    "consecutive_aborts": capacity_state
                        .as_ref()
                        .map(|state| state.consecutive_failures)
                        .unwrap_or(0),
                    "alerting": cfg.capacity_alert_after > 0
                        && capacity_state
                            .as_ref()
                            .map(|state| state.consecutive_failures)
                            .unwrap_or(0)
                            >= cfg.capacity_alert_after,
                    "last_abort_at": capacity_state
                        .as_ref()
                        .and_then(|state| state.last_failure_at.clone()),
                    "last_success_at": capacity_state
                        .as_ref()
                        .and_then(|state| state.last_success_at.clone()),
                },
            },
            "enrichment": {
                "pending_summaries": enrichment.pending_summaries,
                "failed_summaries": enrichment.failed_summaries,
                "content_status": {
                    "full": content_status.full,
                    "thin": content_status.thin,
                    "none": content_status.none,
                    "unknown": content_status.unknown,
                },
            },
            "last_pass": last_pass.as_ref().map(|state| json!({
                "mode": last_pass_mode,
                "at": state.last_run_at,
                "considered": state.considered_count,
                "written": state.new_count,
                "error_class": state.last_error,
                "consecutive_fallbacks": state.consecutive_failures,
                "completed_revision": state.cursor
                    .as_deref()
                    .and_then(|cursor| cursor.split('|').nth(1))
                    .unwrap_or_default(),
            })),
            "relevance": {
                "provider": relevance::embedding_provider_label(embedding_role.as_ref()),
                "model": embedding_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": relevance::embedding_backend_configured(embedding_role.as_ref()),
                "reachable": relevance_reachable,
                "profile_count": profiles.len(),
                "active_mode": if relevance_reachable && reranking_reachable {
                    "reranked"
                } else if relevance_reachable {
                    "semantic"
                } else {
                    "lexical"
                },
            },
            "reranker": {
                "provider": reranking_role
                    .as_ref()
                    .map(|role| role.provider_label())
                    .unwrap_or("No reranking role configured"),
                "model": reranking_role
                    .as_ref()
                    .map(|role| role.model.as_str())
                    .unwrap_or(""),
                "configured": reranking_role.is_some(),
                "reachable": reranking_reachable,
            },
            "travel_context": {
                "enabled": cfg.travel_context.enabled,
                "source": cfg.travel_context.base_url,
                "upcoming_count": travel_context.as_ref().map(|context| context.contexts.len()).unwrap_or(0),
                "reachable": travel_context.as_ref().is_some_and(|context| context.reachable),
                "from_cache": travel_context.as_ref().is_some_and(|context| context.from_cache),
                "refreshed_at": travel_context.as_ref().map(|context| context.refreshed_at.clone()).unwrap_or_default(),
                "plans": travel_context
                    .as_ref()
                    .map(|snapshot| snapshot.contexts.iter().map(|plan| json!({
                        "id": plan.id,
                        "label": plan.title,
                        "date_start": plan.date_start,
                        "date_end": plan.date_end,
                    })).collect::<Vec<_>>())
                    .unwrap_or_default(),
            }
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

/// Database-backed; `db_tests` is the one module name CI's test selector splits
/// on (CONTRIBUTING.md, "Validate the changed boundary"). Each test opens a
/// temp SQLite file of its own — never the deployment's.
///
/// WHAT THESE TESTS DO NOT COVER. `run_page` drives `PassCursor` and the real
/// store; it does not drive `relevance_refresh_handler`, which reads its
/// database path from `Config::load()` and so cannot be pointed at a temp file
/// from here. They gate the paging RULES, not the handler's wiring to them.
/// Measured 2026-09-08 by the verifier: replacing the handler's
/// `cursor.page_offset(requested_offset, ..)` with the original
/// `requested_offset.unwrap_or(0)` — the exact defect this module was opened
/// for — leaves all four green. Whoever moves that line has no gate here.
#[cfg(test)]
mod db_tests {
    use super::*;

    /// Stands in for `evaluation::relevance_revision`, which is a hash of the
    /// profiles and the two model producers. The paging rules read it for
    /// equality only, so a literal is the honest fixture.
    const REVISION: &str = "relevance-revision-under-test";

    fn open_store(name: &str) -> (Store, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("comms-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a writable temp directory");
        let path = directory.join(format!("{name}.db"));
        // A recycled pid must not inherit a previous run's rows.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        let store = Store::open(&path)
            .unwrap_or_else(|error| panic!("could not open {}: {error}", path.display()));
        (store, path)
    }

    /// `count` feed items, newest last, with distinct `created_at` stamps.
    ///
    /// The stamps are set explicitly because `upsert_feed` writes
    /// `strftime(...,'now')` and a seeding loop this tight lands several rows in
    /// one millisecond. The pass pages with OFFSET over `ORDER BY created_at
    /// DESC`, so ties would leave the page boundaries this file asserts on
    /// decided by a sort's tie-breaking rather than by the cursor.
    fn seed(store: &Store, path: &std::path::Path, count: usize) {
        let conn = rusqlite::Connection::open(path).expect("a second handle on the test file");
        for n in 0..count {
            let mut item = FeedItem::new(
                &format!("https://example.com/relevance-paging/{n:04}"),
                "news",
                "article",
            );
            item.title = Some(format!("Item {n}"));
            store.upsert_feed(&item).expect("the item is stored");
            conn.execute(
                "UPDATE comms_feed_items SET created_at = ?2 WHERE id = ?1",
                rusqlite::params![&item.id, &format!("2026-01-01 00:00:00.{n:03}+00:00")],
            )
            .expect("the stamp is rewritten");
        }
    }

    /// One page of `POST /feed/relevance/refresh` with the scoring left out:
    /// read the receipt, resolve the offset, select the page, advance the
    /// cursor, write the receipt back. Every step calls the handler's own code.
    ///
    /// Scoring is what needs a model, and it moves no offset — the handler
    /// derives `has_more` from the row count, and the cursor from `has_more`.
    fn run_page(
        store: &Store,
        requested_offset: Option<usize>,
        days: i32,
        limit: usize,
    ) -> (usize, Vec<String>) {
        let receipt = store.relevance_pass().expect("the receipt reads back");
        let mut cursor =
            PassCursor::parse(receipt.as_ref().and_then(|state| state.cursor.as_deref()));
        let offset = cursor.page_offset(requested_offset, REVISION, days);
        let items = store
            .feed_for_relevance(days, limit, offset)
            .expect("the page selects");
        let has_more = items.len() == limit;
        cursor.advance(REVISION, days, offset, items.len(), has_more);
        store
            .record_relevance_pass(&cursor.render(), items.len() as i64, 0, None)
            .expect("the receipt is written");
        (offset, items.into_iter().map(|item| item.id).collect())
    }

    /// The bug this module was opened for. `tools/feed-sweep` posts
    /// `{days: 3650, limit: 100}` every night and names no offset, so every
    /// night re-read the same newest hundred rows and nothing else was ever
    /// re-scored: on 2026-09-08 the deployment's cursor read
    /// `semantic||<revision>@3650:100` with 175 of its 374 scored items still
    /// `lexical`, and every one of those 175 sat at offset 100 or beyond.
    #[test]
    fn the_nightly_page_advances_between_runs() {
        let (store, path) = open_store("relevance_page_advances");
        seed(&store, &path, 250);

        let (first_offset, first) = run_page(&store, None, FULL_WINDOW_DAYS, 100);
        let (second_offset, second) = run_page(&store, None, FULL_WINDOW_DAYS, 100);

        assert_eq!(first_offset, 0, "the first page of a fresh chain");
        assert_eq!(
            second_offset, 100,
            "the second nightly page starts where the first one ended"
        );
        assert_eq!(first.len(), 100);
        assert_eq!(second.len(), 100);
        let overlap = first.iter().filter(|id| second.contains(id)).count();
        assert_eq!(
            overlap, 0,
            "the second run re-read {overlap} of the rows the first had already covered"
        );
    }

    /// A chain that ran off the end of its list starts over rather than asking
    /// for a page past the last row forever.
    ///
    /// The old cursor reset its progress only when the WIDEST window finished,
    /// so a chain at 90 days stopped at the end of the 90-day list and every
    /// later page of that chain selected nothing.
    #[test]
    fn a_chain_that_reached_the_end_of_its_list_starts_over() {
        let (store, path) = open_store("relevance_chain_wraps");
        seed(&store, &path, 250);

        let offsets = (0..4)
            .map(|_| run_page(&store, None, 90, 100).0)
            .collect::<Vec<_>>();

        assert_eq!(
            offsets,
            vec![0, 100, 200, 0],
            "three pages cover 250 rows, and the fourth begins the next sweep"
        );
    }

    /// The Feed panel's button asks for 90 days and `dashboard/src/routes/travel`
    /// asks for 365. Both name no offset, and both would otherwise take the
    /// nightly 3650-day chain over and send it back to the newest page — the
    /// stall this module fixes, arriving through another door.
    #[test]
    fn a_page_at_another_window_leaves_the_chain_where_it_is() {
        let (store, path) = open_store("relevance_window_ownership");
        seed(&store, &path, 250);

        let (nightly, _) = run_page(&store, None, FULL_WINDOW_DAYS, 100);
        let (button, _) = run_page(&store, None, 90, 100);
        let (next_nightly, _) = run_page(&store, None, FULL_WINDOW_DAYS, 100);

        assert_eq!(nightly, 0);
        assert_eq!(
            button, 0,
            "a window with no chain of its own starts at zero"
        );
        assert_eq!(
            next_nightly, 100,
            "the nightly chain resumes where it was, not where the button left off"
        );
    }

    /// `comms relevance backfill` pages explicitly, and an offset a caller
    /// named must still name the page it reads. Only an absent offset resumes.
    #[test]
    fn an_offset_the_caller_named_is_the_page_it_reads() {
        let (store, path) = open_store("relevance_explicit_offset");
        seed(&store, &path, 250);

        let (first, _) = run_page(&store, Some(0), FULL_WINDOW_DAYS, 100);
        let (second, _) = run_page(&store, Some(100), FULL_WINDOW_DAYS, 100);
        let (third, rows) = run_page(&store, Some(200), FULL_WINDOW_DAYS, 100);

        assert_eq!((first, second, third), (0, 100, 200));
        assert_eq!(rows.len(), 50, "the last page of 250 rows is short");
    }
}
