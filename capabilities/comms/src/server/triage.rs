use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct TriageParams {
    status: Option<String>,
    /// Drop every row whose class ranks above this one (`c1` keeps Public and Mine).
    /// The caller chooses it: the one shared token cannot tell an agent from the
    /// dashboard, so this is a policy an agent client applies, not a gate (ISA F8,
    /// ISC-39 — the gate needs the agent identity of ISC-38).
    max_data_class: Option<String>,
}

pub(super) async fn triage_handler(Query(params): Query<TriageParams>) -> Json<Value> {
    // Refused rather than ignored: a typo in the ceiling must not return every class.
    let ceiling = match params.max_data_class.as_deref() {
        None => None,
        Some(class) => match content_item::class_rank(class) {
            Some(rank) => Some(rank),
            None => {
                return Json(json!({
                    "error": format!("max_data_class must be one of c0, c1, c2, c3, not '{class}'")
                }));
            }
        },
    };
    let result = tokio::task::spawn_blocking(move || -> Option<Vec<TriageOut>> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).ok()?;
        let mut items = store.list_triage(params.status.as_deref()).ok()?;
        if let Some(ceiling) = ceiling {
            // An unknown stored class ranks as unreadable, not as Public.
            items.retain(|item| {
                content_item::class_rank(&item.data_class).is_some_and(|rank| rank <= ceiling)
            });
        }
        // One grouped read for the whole list, not one per row. The list already
        // knows the keys; asking per item would be a query per rendered line.
        let scores = store.triage_score_map().unwrap_or_default();
        // One query for every verdict, before the loop. This handler already
        // issues a relevance query per item; a second per-item query would
        // double that over the whole table for no reason.
        let mut verdicts = store.model_verdicts().unwrap_or_default();
        Some(
            items
                .into_iter()
                .map(|item| {
                    let relevance = store.triage_relevance(&item.id).unwrap_or_default();
                    let score = scores.get(&item.id).cloned();
                    let model = verdicts.remove(&item.id);
                    TriageOut::from_store(item, relevance, score, model)
                })
                .collect(),
        )
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(items) => Json(json!(items)),
        None => Json(json!({ "error": "triage query failed" })),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageSweepBody {
    limit: Option<usize>,
    cursor: Option<String>,
}

/// The people registry's state, as a receipt reports it.
///
/// One function because two receipts state it — the manual refresh and the
/// sweep — and a reader comparing the two must not have to work out whether
/// "absent" and "not loaded" are the same word for the same thing.
fn people_registry_receipt() -> (&'static str, usize) {
    match people_registry::state() {
        people_registry::State::Loaded(names) => ("loaded", names),
        people_registry::State::Absent => ("absent", 0),
        people_registry::State::Unreadable => ("unreadable", 0),
    }
}

/// Counts from one sweep pass. No field here can carry mail content — this is
/// what both the HTTP response and the unattended schedule's log are built
/// from, and the schedule writes to a log nobody is watching at the time.
#[derive(Debug, Serialize)]
pub(super) struct SweepOutcome {
    pub(super) fetched: usize,
    pub(super) new_count: usize,
    pub(super) skipped: usize,
    pub(super) redacted: usize,
    /// Whether the named-person rule could run at all, and against how many
    /// names. This is the path that *persists* rows, so a pass run with the
    /// overlay unmounted stores the verbatim subject of every mail that names a
    /// vault-known person and reports the same counts as a pass that simply
    /// found nobody (Q27 ruling 3, `people_registry::State::Absent`). Stated
    /// here, in the same words the refresh receipt uses, so the two runs are
    /// distinguishable afterwards rather than only while somebody is watching.
    pub(super) people_registry: &'static str,
    pub(super) people_registry_names: usize,
    next_cursor: Option<String>,
}

/// The sweep itself, with no HTTP and no scheduling in it. Both the manual
/// route and the timer call this, for the same reason both go through
/// `intake`: two copies of a mail-reading loop is how one of them ends up
/// missing the gate.
pub(super) fn run_inbox_sweep(
    cfg: &Config,
    limit: usize,
    cursor: Option<&str>,
) -> Result<SweepOutcome, String> {
    let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path).map_err(|error| error.to_string())?;
    let page = google::list_inbox_threads_page(&token, limit, cursor)
        .map_err(|error| error.to_string())?;
    let (people_registry, people_registry_names) = people_registry_receipt();
    let mut outcome = SweepOutcome {
        fetched: 0,
        new_count: 0,
        skipped: 0,
        redacted: 0,
        people_registry,
        people_registry_names,
        next_cursor: page.next_page_token.clone(),
    };
    for stub in &page.threads {
        let meta = match google::thread_meta(&token, &stub.id) {
            Ok(meta) => meta,
            Err(_) => {
                outcome.skipped += 1;
                continue;
            }
        };
        let intake = intake::from_thread(meta, &cfg.rules);
        if intake.redaction_count() > 0 {
            outcome.redacted += 1;
        }
        // With the rules verdict, so the rung that decided this thread is
        // stored in the same transaction the row is. The model rung's
        // eligibility query reads it, and no later pass can re-derive it:
        // `rules::classify` reads `List-Unsubscribe`, which no column holds.
        if store
            .upsert_triage_with_rules(&intake.item, &intake.verdict())
            .map_err(|error| error.to_string())?
        {
            outcome.new_count += 1;
        }
        outcome.fetched += 1;
    }
    Ok(outcome)
}

/// Which bucket a failure falls in, for the stored state. Deliberately lossy:
/// the classes drive backoff and are safe to display, while the provider's own
/// message can quote a request URL or a subject line and is only ever logged.
pub(super) fn sweep_error_class(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("401") || lowered.contains("403") || lowered.contains("auth") {
        "auth"
    } else if lowered.contains("429") || lowered.contains("quota") || lowered.contains("rate") {
        "quota"
    } else if lowered.contains("timeout") || lowered.contains("connect") || lowered.contains("dns")
    {
        "network"
    } else {
        "unknown"
    }
}

pub(super) async fn triage_sweep_handler(Json(body): Json<TriageSweepBody>) -> HttpResponse {
    let limit = body.limit.unwrap_or(100).clamp(1, 100);
    let cursor = body.cursor.filter(|value| !value.trim().is_empty());
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let outcome = run_inbox_sweep(&cfg, limit, cursor.as_deref())?;
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let total_stored = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .len();
        Ok(json!({
            "fetched": outcome.fetched,
            "new_count": outcome.new_count,
            "skipped": outcome.skipped,
            "redacted": outcome.redacted,
            "total_stored": total_stored,
            // Same shape and same words as the refresh receipt's, because the
            // question is the same one: did the c2 escalation run blind?
            "people_registry": {
                "state": outcome.people_registry,
                "names": outcome.people_registry_names,
            },
            "next_cursor": outcome.next_cursor,
            "exhausted": outcome.next_cursor.is_none(),
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageRelevanceBody {
    limit: Option<usize>,
}

pub(super) fn loopback_inference_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://127.0.0.1:")
        || value.starts_with("https://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("https://localhost:")
        || value.starts_with("http://[::1]:")
        || value.starts_with("https://[::1]:")
}

pub(super) async fn triage_relevance_handler(
    Json(body): Json<TriageRelevanceBody>,
) -> HttpResponse {
    let limit = body.limit.unwrap_or(200).clamp(1, 500);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let profiles = relevance::load_profiles(&cfg.relevance)?;
        let all_triage = store.list_triage(None).map_err(|error| error.to_string())?;
        // The class gate runs over EVERY stored mail, not only the page this
        // pass would have scored. Rows derived from a refused item are a
        // standing fact about what a model was once shown; leaving them behind
        // because the item's status moved out of the scoring window would
        // repair the gate and keep the evidence.
        let refused_items = all_triage
            .iter()
            .filter(|item| !content_item::local_prompt_allowed(&item.data_class))
            .cloned()
            .collect::<Vec<_>>();
        let refused_ids = refused_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let embedding_role = cfg
            .embedding_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        let reranking_role = cfg
            .reranking_role()
            .filter(|role| loopback_inference_url(&role.backend.base_url));
        // Resolved before the page is picked, because the currency check needs
        // it and the currency check now decides what the page IS.
        let mail_context_revision = mail_evaluation::context_revision(
            &profiles,
            embedding_role
                .as_ref()
                .map(|role| role.cache_key())
                .as_deref(),
            reranking_role
                .as_ref()
                .map(|role| role.cache_key())
                .as_deref(),
        );
        let semantic_available = relevance::embedding_backend_reachable(embedding_role.as_ref());

        // The limit bounds the WORK, not the head of a fixed list. `list_triage`
        // orders by `internal_date DESC`, so taking the first `limit` rows and
        // asking about currency afterwards embedded the same newest 200 mails on
        // every pass and left the 26 oldest scorable mails permanently unscored,
        // while the receipt reported `skipped_current: 200` as if the corpus
        // were finished.
        let mut skipped_current = 0usize;
        let mut stale = Vec::new();
        for item in all_triage
            .into_iter()
            .filter(|item| item.status == "proposed" || item.status == "approved")
            .filter(|item| content_item::local_prompt_allowed(&item.data_class))
        {
            let stored = store.triage_evaluation(&item.id).ok().flatten();
            let item_revision = mail_evaluation::item_revision(&item);
            if mail_evaluation::is_current(
                stored.as_ref(),
                &item_revision,
                &mail_context_revision,
                semantic_available,
            ) {
                skipped_current += 1;
            } else {
                stale.push(item);
            }
        }
        // What this pass could not reach. Reported so an operator can see that
        // the corpus is not finished rather than inferring it from a count that
        // happens to equal the limit.
        let unreached = stale.len().saturating_sub(limit);
        stale.truncate(limit);
        let scorable = stale;
        let items = scorable
            .iter()
            .map(|proposal| {
                let mut item = FeedItem::new(
                    &format!("https://mail.google.com/mail/u/0/#all/{}", proposal.id),
                    "news",
                    "mail",
                );
                item.id = proposal.id.clone();
                item.title = proposal.subject.clone();
                item.author = proposal.from_addr.clone();
                item.transcript = proposal.snippet.clone();
                // The class travels with the item, and this line is the whole
                // repair. `FeedItem::new` fills the class from
                // `DataClass::undeclared()`, which is literally c1 — so every
                // c3 mail read as c1 here and was embedded, sender address and
                // all, by a synthetic item that had simply forgotten what it
                // was. §6.2b: C3 never reaches any model, and the gate blocks
                // the read at the tool boundary rather than filtering the
                // prompt afterwards.
                item.data_class = proposal.data_class.clone();
                item.data_class_rationale = proposal.data_class_rationale.clone();
                item.data_classification_method = proposal.data_classification_method.clone();
                item.data_classification_version = proposal.data_classification_version.clone();
                item
            })
            .collect::<Vec<_>>();
        let outcome = relevance::score_items(
            &items,
            &profiles,
            embedding_role.as_ref(),
            reranking_role.as_ref(),
        );
        let mode = outcome
            .items
            .iter()
            .flat_map(|item| item.matches.first())
            .map(|matched| matched.mode.clone())
            .next();
        let mut scored = 0usize;
        for item in &outcome.items {
            scored += 1;
            store
                .replace_triage_relevance(&item.feed_id, &item.matches)
                .map_err(|error| error.to_string())?;
        }
        // A refused item is written with an EMPTY match set, which deletes
        // whatever a previous ungated pass derived from it: 15 stored rows over
        // 11 c3 mails go on the first gated pass. Written as a deletion rather
        // than left alone, because a stored score is a claim about content the
        // gate says no model may read. `replace_triage_relevance` carries no
        // tier gate, so unlike the feed's an empty set really does delete.
        for id in &refused_ids {
            store
                .replace_triage_relevance(id, &[])
                .map_err(|error| error.to_string())?;
        }
        let refused_class = refused_ids.len();

        // Evaluating mail is the same job as scoring it, with the same owner,
        // so it extends this route rather than earning a second one — the
        // loopback role filter already sits here and a new route would restate
        // it.
        let mut evaluated = 0usize;
        let mut refused_lower_tier = 0usize;
        // One read for the whole pass, not one per item -- the same reason
        // `triage_handler` batches it. The rung writes these on its own
        // schedule, so most passes find an empty map and every urgency factor
        // then carries weight 0.
        let verdicts = store.model_verdicts().unwrap_or_default();
        for (item, scored_item) in scorable.iter().zip(&outcome.items) {
            // The rung's urgency moves the score THROUGH the evaluator rather
            // than competing with it on the wire, so a reader who asks why a
            // mail is at the top gets four bars, one of which is the model's.
            // Gated on `URGENCY_VALIDATED`, which is the same answer the wire's
            // `urgency_validated` reports -- until the frozen corpus measures
            // the band error this is `None` and the factor keeps weight 0.
            let urgency = mail_evaluation::urgency_from_verdict(
                verdicts.get(&item.id),
                mail_evaluation::URGENCY_VALIDATED,
            );
            let evaluation = mail_evaluation::evaluate(
                item,
                scored_item.matches.first(),
                urgency.as_ref(),
                &mail_context_revision,
                false,
            );
            if store
                .replace_triage_evaluation(&evaluation)
                .map_err(|error| error.to_string())?
            {
                evaluated += 1;
            } else {
                refused_lower_tier += 1;
            }
        }
        // A refusal is stored as a row, not as an absence: an evaluation at
        // mode 'unscored' whose interest factor carries weight 0 and the
        // rationale that says why. An item with no evaluation is
        // indistinguishable from one nobody has reached yet.
        for item in &refused_items {
            let item_revision = mail_evaluation::item_revision(item);
            let stored = store.triage_evaluation(&item.id).ok().flatten();
            // A stored refusal at this revision is final. `is_current` cannot
            // say so, because it reads `unscored` as stale whenever an embedder
            // answers -- and no embedder can ever upgrade a refusal.
            if mail_evaluation::refusal_is_current(
                stored.as_ref(),
                &item_revision,
                &mail_context_revision,
            ) {
                skipped_current += 1;
                continue;
            }
            // No urgency on a refusal, whatever the rung once stored: a c3 mail
            // reached no model in this pass, and a refusal exists to withdraw
            // model-derived numbers rather than to carry one forward.
            let evaluation =
                mail_evaluation::evaluate(item, None, None, &mail_context_revision, true);
            // Past the tier gate: a refusal withdraws a model-derived score, and
            // a withdrawal that loses to the score it withdraws leaves the score
            // on the surface forever.
            if store
                .replace_triage_evaluation_refusal(&evaluation)
                .map_err(|error| error.to_string())?
            {
                evaluated += 1;
            } else {
                refused_lower_tier += 1;
            }
        }
        Ok(json!({
            "scored": scored,
            "evaluated": evaluated,
            "skipped_current": skipped_current,
            // Scorable mails this pass could not reach inside `limit`. Non-zero
            // means the corpus is not finished and another pass is owed.
            "unreached": unreached,
            "refused_class": refused_class,
            "refused_lower_tier": refused_lower_tier,
            "profile_count": profiles.len(),
            "mode": mode,
            "evaluator_revision": mail_evaluation::MAIL_EVALUATOR_REVISION,
            "embedding": {
                "mode": outcome.mode,
                "error_class": outcome.error_class,
                "chunks": outcome.chunks,
                "chunks_failed": outcome.chunks_failed,
            },
            "local_only": true,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageBulkBody {
    ids: Vec<String>,
    action: String,
    stream: Option<String>,
    data_class: Option<String>,
    /// One reason for the whole batch. Required when the change lowers a class
    /// for any item in it, which is decided per item: a batch that raises nine
    /// and lowers one refuses exactly that one, by id, in `failures`.
    rationale: Option<String>,
}

pub(super) fn thread_action_for_job(job: &GmailActionJob) -> Result<ThreadAction, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadAction::Archive),
        "trash" => Ok(ThreadAction::Trash),
        "restore" if job.source_status == "archived" => Ok(ThreadAction::RestoreArchive),
        "restore" if job.source_status == "trashed" => Ok(ThreadAction::RestoreTrash),
        _ => Err("stored Gmail action has an invalid source state".into()),
    }
}

pub(super) fn target_location(job: &GmailActionJob) -> Result<ThreadLocation, String> {
    match job.action.as_str() {
        "archive" => Ok(ThreadLocation::Archive),
        "trash" => Ok(ThreadLocation::Trash),
        "restore" => Ok(ThreadLocation::Inbox),
        _ => Err("stored Gmail action is invalid".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GmailActionOutcome {
    Confirmed { changed: bool },
    Missing,
}

/// Execute one durable intent. Reading the current labels first makes replay
/// safe when Gmail succeeded but the process stopped before the local commit.
pub(super) fn execute_gmail_action_job(
    store: &Store,
    token: &str,
    job: &GmailActionJob,
) -> Result<GmailActionOutcome, String> {
    let meta = match google::thread_meta_lookup(token, &job.triage_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return store
                .observe_gmail_missing(&job.triage_id)
                .map_err(|_| "Axon could not record that the Gmail thread is missing".to_string())
                .and_then(|updated| {
                    if updated {
                        Ok(GmailActionOutcome::Missing)
                    } else {
                        Err(
                            "mail proposal disappeared before the missing state was recorded"
                                .into(),
                        )
                    }
                });
        }
        Err(error) => {
            let message = error.to_string();
            let _ = store.fail_gmail_action(job.job_id, &message);
            return Err(message);
        }
    };
    let result = (|| -> Result<GmailActionOutcome, String> {
        let changed = ThreadLocation::from_labels(&meta.label_ids) != target_location(job)?;
        if changed {
            google::apply_thread_action(token, &job.triage_id, thread_action_for_job(job)?)
                .map_err(|error| error.to_string())?;
        }
        match store
            .complete_gmail_action(job.job_id)
            .map_err(|_| "Axon could not commit the confirmed Gmail result".to_string())?
        {
            true => Ok(GmailActionOutcome::Confirmed { changed }),
            false => Err("mail proposal disappeared before local completion".into()),
        }
    })();

    if let Err(error) = &result {
        let _ = store.fail_gmail_action(job.job_id, error);
    }
    result
}

#[derive(Debug, Serialize)]
pub(super) struct GmailMaintenanceCounts {
    retried: usize,
    pub(super) recovered: usize,
    pub(super) retry_failures: usize,
    reconciled: usize,
    pub(super) changed: usize,
    read_failures: usize,
    missing: usize,
    content_fetched: bool,
}

pub(super) fn reconciled_status(current: &str, location: ThreadLocation) -> &str {
    match location {
        ThreadLocation::Trash => "trashed",
        ThreadLocation::Archive => "archived",
        ThreadLocation::Inbox if matches!(current, "archived" | "trashed" | "executed") => {
            "proposed"
        }
        ThreadLocation::Inbox => current,
    }
}

pub(super) fn run_gmail_maintenance(
    cfg: &Config,
    reconcile_limit: i64,
) -> Result<GmailMaintenanceCounts, String> {
    let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
    let token = google::access_token(&cfg.google_env_path)
        .map_err(|_| "Google authorization unavailable".to_string())?;
    let jobs = store
        .pending_gmail_actions(50)
        .map_err(|error| error.to_string())?;
    let mut counts = GmailMaintenanceCounts {
        retried: jobs.len(),
        recovered: 0,
        retry_failures: 0,
        reconciled: 0,
        changed: 0,
        read_failures: 0,
        missing: 0,
        content_fetched: false,
    };
    for job in &jobs {
        match execute_gmail_action_job(&store, &token, job) {
            Ok(GmailActionOutcome::Confirmed { .. }) => counts.recovered += 1,
            Ok(GmailActionOutcome::Missing) => counts.missing += 1,
            Err(_) => counts.retry_failures += 1,
        }
    }

    let candidates = store
        .gmail_reconcile_candidates(reconcile_limit)
        .map_err(|error| error.to_string())?;
    for candidate in candidates {
        match google::thread_meta_lookup(&token, &candidate.triage_id) {
            Ok(Some(meta)) => {
                let location = ThreadLocation::from_labels(&meta.label_ids);
                if reconciled_status(&candidate.status, location) != candidate.status {
                    counts.changed += 1;
                }
                store
                    .observe_gmail_location(&candidate.triage_id, location.as_str())
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
            }
            Ok(None) => {
                store
                    .observe_gmail_missing(&candidate.triage_id)
                    .map_err(|error| error.to_string())?;
                counts.reconciled += 1;
                counts.changed += usize::from(candidate.status != "missing");
                counts.missing += 1;
            }
            Err(_) => counts.read_failures += 1,
        }
    }
    Ok(counts)
}

pub(super) async fn triage_bulk_handler(Json(body): Json<TriageBulkBody>) -> HttpResponse {
    if body.ids.is_empty() || body.ids.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "select between 1 and 100 proposals" })),
        );
    }
    let mut seen = HashSet::new();
    let ids = body
        .ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect::<Vec<_>>();
    let action = body.action;
    let stream = body.stream;
    let selected_data_class = body.data_class;
    let rationale = body.rationale;
    if !matches!(
        action.as_str(),
        "dismiss"
            | "categorize"
            | "set-data-class"
            | "archive"
            | "trash"
            | "waiting"
            | "clear-waiting"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "error": "action must be dismiss, categorize, set-data-class, archive, trash, waiting, or clear-waiting" }),
            ),
        );
    }
    if action == "categorize" && stream.as_deref().is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "categorize requires a stream" })),
        );
    }
    if action == "set-data-class"
        && !selected_data_class
            .as_deref()
            .is_some_and(content_item::valid)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "set-data-class requires one of: {}",
                    content_item::DATA_CLASSES.join(", ")
                )
            })),
        );
    }

    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let gmail_action = ThreadAction::parse(&action);
        let waiting_action = matches!(action.as_str(), "waiting" | "clear-waiting");
        let token = (gmail_action.is_some() || waiting_action)
            .then(|| google::access_token(&cfg.google_env_path));
        // Resolved once for the batch, not once per thread: the label id is the
        // same for every id here, and looking it up a hundred times would spend
        // a hundred requests to learn the same string.
        let waiting_label = match (waiting_action, token.as_ref()) {
            (true, Some(Ok(token))) => Some(google::ensure_waiting_label(token)),
            _ => None,
        };
        let mut succeeded = Vec::new();
        let mut failures = Vec::new();
        // Rows whose stored subject and snippet this batch narrowed, because
        // the class it set on them does not admit what they held. Zero for
        // every other action.
        let mut narrowed = 0usize;
        for id in ids {
            let known = store
                .get_triage_status(&id)
                .map_err(|error| error.to_string())?;
            if known.is_none() {
                failures.push(json!({ "id": id, "error": "not found" }));
                continue;
            }
            let outcome = match action.as_str() {
                "dismiss" => store
                    .set_triage_status(&id, "dismissed")
                    .map_err(|error| error.to_string())
                    .map(|updated| updated.then_some(())),
                "categorize" => {
                    let chosen = stream.as_deref().unwrap_or_default();
                    match class_for_stream(&store, &id, chosen) {
                        Ok(Some(classification)) => store
                            .set_triage_stream(&id, chosen, &classification)
                            .map_err(|error| error.to_string())
                            .map(|write| {
                                if write.narrowed {
                                    narrowed += 1;
                                }
                                write.changed.then_some(())
                            }),
                        Ok(None) => Ok(None),
                        Err(error) => Err(error),
                    }
                }
                "set-data-class" => store
                    .set_triage_data_class(
                        &id,
                        selected_data_class.as_deref().unwrap_or_default(),
                        rationale.as_deref(),
                    )
                    .map_err(|error| error.to_string())
                    .map(|write| {
                        if write.narrowed {
                            narrowed += 1;
                        }
                        write.changed.then_some(())
                    }),
                // Not queued the way archive and trash are, deliberately.
                // The queue exists for a mutation with a restore path and a
                // reconcile story; applying a label has neither. It is
                // idempotent in Gmail, carries no state to drift, and a failure
                // leaves nothing half-done — so a retry is just pressing it
                // again. Gmail first, store second: see `set_triage_waiting`.
                "waiting" | "clear-waiting" => {
                    let want = action == "waiting";
                    match (token.as_ref(), waiting_label.as_ref()) {
                        (Some(Ok(token)), Some(Ok(label))) => {
                            google::set_thread_waiting(token, &id, label, want)
                                .map_err(|error| error.to_string())
                                .and_then(|()| {
                                    store
                                        .set_triage_waiting(&id, want)
                                        .map_err(|error| error.to_string())
                                })
                                .map(|updated| updated.then_some(()))
                        }
                        (Some(Err(_)), _) => Err("Google authorization unavailable".to_string()),
                        (_, Some(Err(error))) => Err(error.to_string()),
                        _ => Err("Google authorization unavailable".to_string()),
                    }
                }
                "archive" | "trash" => store
                    .queue_gmail_action(&id, action.as_str())
                    .map_err(|error| error.to_string())
                    .and_then(|job| match token.as_ref() {
                        Some(Ok(token)) => execute_gmail_action_job(&store, token, &job),
                        Some(Err(_)) => {
                            let _ = store.fail_gmail_action(
                                job.job_id,
                                "Google authorization unavailable",
                            );
                            Err("Google authorization unavailable; the action remains queued for retry".into())
                        }
                        None => unreachable!(),
                    })
                    .and_then(|outcome| match outcome {
                        GmailActionOutcome::Confirmed { .. } => Ok(Some(())),
                        GmailActionOutcome::Missing => {
                            Err("Gmail thread no longer exists; Axon retained its local record".into())
                        }
                    }),
                _ => unreachable!(),
            };
            match outcome {
                Ok(Some(())) => succeeded.push(id),
                Ok(None) => failures.push(json!({ "id": id, "error": "not found" })),
                Err(error) => failures.push(json!({ "id": id, "error": error.to_string() })),
            }
        }
        Ok(json!({
            "succeeded": succeeded,
            "failures": failures,
            "gmail_changed": gmail_action.is_some(),
            "narrowed": narrowed,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_status_handler(
    Path(id): Path<String>,
    Json(body): Json<StatusBody>,
) -> HttpResponse {
    if !matches!(body.status.as_str(), "proposed" | "approved" | "dismissed") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "status must be proposed, approved, or dismissed; Gmail lifecycle states require the Gmail action endpoint"
            })),
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_triage_status(&id, &body.status)
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
pub(super) struct TriageStreamBody {
    stream: String,
}

/// The class a thread would hold once it sits in `stream`.
///
/// The sweep's rule set, registry pass included, applied to the row as it is
/// stored and the category the operator just chose. `set_triage_stream` settles
/// the class in its own transaction and needs the answer as plain data, because
/// the people registry lives above the store.
fn class_for_stream(store: &Store, id: &str, stream: &str) -> Result<Option<DataClass>, String> {
    let Some(item) = store.get_triage(id).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    Ok(Some(intake::classify_mail(
        stream,
        item.from_addr.as_deref().unwrap_or_default(),
        item.subject.as_deref().unwrap_or_default(),
        item.snippet.as_deref().unwrap_or_default(),
    )))
}

pub(super) async fn triage_stream_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageStreamBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<StreamWrite, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        // The class the new category implies, settled in the same transaction
        // as the category itself: `steuern` and `belege` are Others by rule, so
        // a correction into one of them is also a redaction decision, and the
        // dashboard prints "Redacted" from the class alone.
        let Some(classification) = class_for_stream(&store, &id, &body.stream)? else {
            return Ok(StreamWrite::default());
        };
        store
            .set_triage_stream(&id, &body.stream, &classification)
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(write)) if write.changed => (
            StatusCode::OK,
            Json(json!({ "ok": true, "narrowed": write.narrowed })),
        ),
        Ok(Ok(_)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageDataClassBody {
    data_class: String,
    /// Required when the change lowers the class, ignored when it raises it.
    /// Which of the two this is depends on what is stored, so the store
    /// decides and this handler stays out of it.
    rationale: Option<String>,
}

pub(super) async fn triage_data_class_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageDataClassBody>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<ClassWrite, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        store
            .set_triage_data_class(&id, &body.data_class, body.rationale.as_deref())
            .map_err(|error| error.to_string())
    })
    .await;

    match result {
        // `narrowed` rather than an assumption either way: selecting Others or
        // Secret remediates the stored subject and snippet in the same
        // transaction, and the caller is told whether this row had anything
        // left to remove. A row swept after the intake gate existed answers
        // false, and that is a clean row, not a skipped one.
        Ok(Ok(write)) if write.changed => (
            StatusCode::OK,
            Json(json!({ "ok": true, "narrowed": write.narrowed })),
        ),
        Ok(Ok(_)) => error_response(StatusCode::NOT_FOUND, "not found"),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

/// Freshness and failure of the unattended schedule, for a dashboard to read.
/// Unauthenticated with the other reads: it carries counts, timestamps and an
/// error class, and no mail ever reaches it.
pub(super) async fn triage_sweep_status_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let state = store
            .get_source_state(INBOX_SWEEP_SOURCE)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "enabled": cfg.inbox_sweep_minutes > 0,
            "every_minutes": cfg.inbox_sweep_minutes,
            "max_threads": cfg.inbox_sweep_max_threads,
            "quiet_hours": cfg.inbox_sweep_quiet_hours.map(|(s, e)| json!({"start": s, "end": e})),
            "last_run_at": state.as_ref().map(|s| s.last_run_at.clone()),
            "last_success_at": state.as_ref().and_then(|s| s.last_success_at.clone()),
            "last_failure_at": state.as_ref().and_then(|s| s.last_failure_at.clone()),
            "last_error": state.as_ref().and_then(|s| s.last_error.clone()),
            "considered_count": state.as_ref().map(|s| s.considered_count).unwrap_or(0),
            "new_count": state.as_ref().map(|s| s.new_count).unwrap_or(0),
            "consecutive_failures": state.as_ref().map(|s| s.consecutive_failures).unwrap_or(0),
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

#[derive(Debug, Deserialize)]
pub(super) struct TriageRedactBody {
    limit: Option<usize>,
    /// Report what would change without writing. The default is to write:
    /// this endpoint exists because material is already stored, and a preview
    /// that has to be run twice is a preview that gets run once.
    dry_run: Option<bool>,
}

/// Remediate rows persisted before the intake gate existed.
///
/// Bounded, idempotent, and reviewable: it reports how many rows it examined,
/// how many it changed and what kinds of entity it removed — never the removed
/// values, and never the values it left. Running it twice reports zero changes
/// the second time, which is how you know it finished.
pub(super) async fn triage_redact_handler(Json(body): Json<TriageRedactBody>) -> HttpResponse {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let dry_run = body.dry_run.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut in_scope = 0usize;
        let mut changed = 0usize;
        let mut verdicts_narrowed = 0usize;
        let mut entity_types: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut digests = Vec::new();

        for item in items {
            let Some(remediation) = intake::remediate(
                &item.data_class,
                item.subject.as_deref(),
                item.snippet.as_deref(),
            ) else {
                continue;
            };
            in_scope += 1;
            if remediation.changed {
                for finding in &remediation.redactions {
                    *entity_types.entry(finding.entity_type).or_default() += finding.count;
                }
                if let Some(digest) = remediation.audit_digest.clone() {
                    digests.push(json!({ "id": item.id, "digest": digest }));
                }
            }
            if dry_run {
                if remediation.changed {
                    changed += 1;
                }
                continue;
            }
            // Called for every in-scope row, not only for a row whose subject
            // still holds something: a row swept clean can still carry a model
            // verdict written while its class was lower, and that verdict's
            // two sentences are review fields of the same kind (review,
            // 2026-09-05).
            let write = store
                .redact_triage_review_fields(
                    &item.id,
                    &item.data_class,
                    remediation.subject.as_deref(),
                    remediation.snippet.as_deref(),
                )
                .map_err(|error| error.to_string())?;
            if remediation.changed && write.changed {
                changed += 1;
            }
            if write.verdict_narrowed {
                verdicts_narrowed += 1;
            }
        }

        Ok(json!({
            "reviewed": reviewed,
            "in_scope": in_scope,
            "changed": changed,
            // Model verdicts whose stored sentences this run narrowed to the
            // row's current class. Separate from `changed`, which counts the
            // items: the two tables can need remediation independently.
            "verdicts_narrowed": verdicts_narrowed,
            "dry_run": dry_run,
            "entity_types": entity_types,
            "audit": digests,
            "transformation": cloud_derivative::REDACTION_VERSION,
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

#[derive(Debug, Deserialize)]
pub(super) struct TriageDataClassRefreshBody {
    limit: Option<usize>,
}

/// Re-derive one row's class and leave the row in the state that class
/// demands. Returns `(class was rewritten, review fields were narrowed)`.
///
/// Both halves, because they are one decision — and one transaction, because
/// the store owns that. The sweep decides them together in
/// `intake::from_thread`: classify, then redact before the row is written.
/// Splitting them here would mean every row this pass raises to `c2` keeps the
/// verbatim subject `c2` exists to hide until an operator remembers a second
/// endpoint — and the dashboard prints "Redacted" from the class alone, so the
/// gap is invisible in the one place a human would look.
fn refresh_one_triage_class(store: &Store, item: &TriageItem) -> Result<(bool, bool), String> {
    // The sweep's rule set, registry pass included (Q27), so a row re-derived
    // here and a row freshly swept land on the same class.
    let classification = intake::classify_mail(
        &item.stream,
        item.from_addr.as_deref().unwrap_or_default(),
        item.subject.as_deref().unwrap_or_default(),
        item.snippet.as_deref().unwrap_or_default(),
    );
    let write = store
        .refresh_triage_data_class_and_redact(&item.id, &classification)
        .map_err(|error| error.to_string())?;
    Ok((write.changed, write.narrowed))
}

pub(super) async fn triage_data_class_refresh_handler(
    Json(body): Json<TriageDataClassRefreshBody>,
) -> HttpResponse {
    let limit = body.limit.unwrap_or(500).clamp(1, 2_000);
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let items = store
            .list_triage(None)
            .map_err(|error| error.to_string())?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let reviewed = items.len();
        let mut updated = 0usize;
        let mut redacted = 0usize;
        let mut preserved_human = 0usize;
        for item in items {
            if item.data_classification_method == "human" {
                preserved_human += 1;
                continue;
            }
            let (reclassified, narrowed) = refresh_one_triage_class(&store, &item)?;
            if reclassified {
                updated += 1;
            }
            if narrowed {
                redacted += 1;
            }
        }
        let (registry_state, registry_names) = people_registry_receipt();
        Ok(json!({
            "reviewed": reviewed,
            "updated": updated,
            // Rows whose stored subject/snippet this pass narrowed, because the
            // class they now carry requires it. Reported beside `updated`: the
            // two counts differ when a row was already strict but never
            // remediated, and an operator reading only `updated` would see a
            // quiet run and conclude nothing was exposed.
            "redacted": redacted,
            "preserved_human": preserved_human,
            "transformation": cloud_derivative::REDACTION_VERSION,
            "classifier_version": content_item::MAIL_CLASSIFIER_VERSION,
            "content_inputs": ["sender", "subject", "snippet", "category"],
            "people_registry": {
                "state": registry_state,
                "names": registry_names,
            },
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

#[derive(Debug, Deserialize)]
pub(super) struct TriageClassifyRefreshBody {
    mode: Option<String>,
    limit: Option<usize>,
}

/// Run one bounded pass of the local model rung.
///
/// `spawn_blocking` for the reason the data-class refresh uses it:
/// `summarize::complete` speaks HTTP through a blocking client, and a pass over
/// a hundred threads at about two seconds each would hold a tokio worker for
/// minutes.
///
/// Shadow by default, and `apply` is refused with a 409 unless the overlay
/// declares `mail_model.apply`. The refusal is decided by
/// `mail_model::apply_allowed`, a pure function over the config section, so
/// what this handler does is testable without the operator's machine.
pub(super) async fn triage_classify_refresh_handler(
    Json(body): Json<TriageClassifyRefreshBody>,
) -> HttpResponse {
    let requested = match body.mode.as_deref().unwrap_or("shadow").parse::<Mode>() {
        Ok(mode) => mode,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    let requested_limit = body.limit;

    let result = tokio::task::spawn_blocking(move || -> Result<Value, (StatusCode, String)> {
        let cfg = Config::load();
        let mode = mail_model::apply_allowed(cfg.mail_model.as_ref(), requested)
            .map_err(|error| (StatusCode::CONFLICT, error))?;
        let store = Store::open(&cfg.database_path)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        // The overlay's `mail_model.limit` is the default when the request
        // names none. Decided in `mail_model` so the CLI and this route cannot
        // disagree about it.
        let limit = mail_model::pass_limit(cfg.mail_model.as_ref(), requested_limit);
        let min_confidence_bp = cfg
            .mail_model
            .as_ref()
            .map_or(0, |section| i64::from(section.min_confidence_bp));
        let receipt = mail_model::run_pass(&cfg, &store, mode, limit, min_confidence_bp)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let (registry_state, registry_names) = people_registry_receipt();
        Ok(json!({
            "mode": receipt.mode,
            "limit": limit,
            "reviewed": receipt.reviewed,
            "eligible": receipt.eligible,
            // Threads a previous shadow pass already answered, still carrying a
            // disagreement nothing has written. In shadow this is the size of
            // the decision waiting for the operator; in apply it is what this
            // pass acted on without prompting anything.
            "awaiting_apply": receipt.awaiting_apply,
            "prompted": receipt.prompted,
            "refused_c3": receipt.refused_c3,
            "over_window": receipt.over_window,
            "unparseable": receipt.unparseable,
            "invalid_stream": receipt.invalid_stream,
            "errors": receipt.errors,
            // The model matched the rule and nothing was written: re-stamping an
            // unchanged verdict as method='model' would erase the true fact
            // that a rule decided it.
            "agreed_no_write": receipt.agreed_no_write,
            "disagreed": receipt.disagreed,
            "applied": receipt.applied,
            // Proposals apply refused because they would raise the data class.
            // The class UPDATE is escalation-only and the narrowing after it
            // cannot be undone, so a machine may not open that door.
            "held_class_escalation": receipt.held_class_escalation,
            "below_confidence": receipt.below_confidence,
            "redactions": receipt.redactions,
            "model": {
                "producer": receipt.producer,
                "prompt_revision": receipt.prompt_revision,
                "loopback": true,
            },
            "classifier_version": mail_model::MAIL_MODEL_VERSION,
            "rules_version": comms::rules::MAIL_RULES_VERSION,
            "transformation": cloud_derivative::REDACTION_VERSION,
            "content_inputs": ["sender_domain", "subject", "snippet", "stream_vocabulary"],
            "people_registry": {
                "state": registry_state,
                "names": registry_names,
            },
            "cloud_calls": 0,
        }))
    })
    .await;

    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err((status, error))) => error_response(status, error),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageClassifyReportParams {
    mode: Option<String>,
}

/// The agreement between the rules and the model rung, as counts.
///
/// Counts only — no subject, no snippet, no rationale — which is the rule
/// `SweepOutcome` already states for the sweep receipt, and the reason the
/// store method behind it cannot select the text at all. The body is meant to
/// be safe to log and to paste into a decision record.
pub(super) async fn triage_classify_report_handler(
    Query(params): Query<TriageClassifyReportParams>,
) -> HttpResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let summaries = store
            .model_verdict_summaries(params.mode.as_deref())
            .map_err(|error| error.to_string())?;
        // Named `candidates`, not `eligible`: the refresh receipt's `eligible`
        // is how many rows were DUE this run, and two fields of one name
        // meaning two things is the drift a report gets read wrong by. This is
        // the coarse set the rung may look at at all. Counted in SQL, because
        // reading a hundred subjects to call `.len()` on them is the opposite
        // of what this route promises.
        let candidates = store
            .model_rung_candidate_count()
            .map_err(|error| error.to_string())?;
        let by_classification_method = store
            .triage_classification_methods()
            .map_err(|error| error.to_string())?;

        Ok(report_body(
            &summaries,
            candidates,
            &by_classification_method,
            &cfg,
        ))
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

/// The report body, as one pure function over what the two store queries
/// answered.
///
/// A function rather than a `json!` inside the handler, so the test that pins
/// "this body quotes no mail content" can run the thing the route serves. The
/// test used to rebuild the body itself, which meant a new key carrying mail
/// text would have left it green (review, 2026-09-05).
fn report_body(
    summaries: &[ModelVerdictSummary],
    candidates: usize,
    by_classification_method: &[(String, usize)],
    cfg: &Config,
) -> Value {
    let by_rule_stream: Vec<Value> = mail_model::agreement(summaries)
        .into_iter()
        .map(|(rule_stream, n, agree, model_streams)| {
            let mut streams: Vec<(String, usize)> = model_streams.into_iter().collect();
            streams.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
            json!({
                "rule_stream": rule_stream,
                "n": n,
                "agree": agree,
                "agree_percent": percent(agree, n),
                "model_streams": streams
                    .into_iter()
                    .map(|(stream, n)| json!({ "stream": stream, "n": n }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    json!({
        "verdicts": summaries.len(),
        "candidates": candidates,
        "by_rule_stream": by_rule_stream,
        "by_state": tally(summaries.iter().map(|row| row.state.clone()))
            .into_iter()
            .map(|(state, n)| json!({ "state": state, "n": n }))
            .collect::<Vec<_>>(),
        // What actually classified this mailbox, counted where the rows are.
        // The dashboard panel that shows it used to derive it in Svelte, which
        // is arithmetic the frontend does not do.
        "by_classification_method": by_classification_method
            .iter()
            .map(|(method, n)| json!({ "method": method, "n": n }))
            .collect::<Vec<_>>(),
        // What a receipt proves "no Secret mail was prompted" from: the
        // class at prompt time, beside how many of that class produced an
        // answer rather than a refusal.
        "by_data_class": by_data_class(summaries),
        "held": tally(
            summaries
                .iter()
                .filter_map(|row| row.held_reason.clone()),
        )
        .into_iter()
        .map(|(reason, n)| json!({ "reason": reason, "n": n }))
        .collect::<Vec<_>>(),
        "confidence_bp": spread(summaries.iter().filter_map(|row| row.confidence_bp)),
        "urgency_bp": urgency_spread(summaries),
        "apply_enabled": cfg.mail_model.as_ref().is_some_and(|section| section.apply),
        "producer": mail_model::producer(cfg),
        "prompt_revision": mail_model::MAIL_MODEL_PROMPT_REVISION,
        "cloud_calls": 0,
    })
}

/// The urgency numbers, plus how many verdicts carry one and whether anything
/// may rank on them.
///
/// `validated` is hard-coded false rather than read from a key: flipping it is
/// a measurement, and the measurement is the frozen corpus's urgency band
/// error. Until it carries a number, urgency is stored, published and displayed
/// and ranks nothing.
fn urgency_spread(summaries: &[ModelVerdictSummary]) -> Value {
    let scored: Vec<i64> = summaries.iter().filter_map(|row| row.urgency_bp).collect();
    let mut value = spread(scored.iter().copied());
    value["scored"] = json!(scored.len());
    value["validated"] = json!(false);
    value
}

/// One decimal, or `null` where there is nothing to divide by. A zero would
/// read as a measured zero.
fn percent(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| (part as f64 * 1_000.0 / whole as f64).round() / 10.0)
}

/// Count occurrences, largest first, name second. Deterministic order, because
/// a report a human compares against yesterday's must not reshuffle.
fn tally(values: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    rows
}

fn by_data_class(summaries: &[ModelVerdictSummary]) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    for class in content_item::DATA_CLASSES {
        let of_class: Vec<&ModelVerdictSummary> = summaries
            .iter()
            .filter(|row| row.data_class == class)
            .collect();
        if of_class.is_empty() {
            continue;
        }
        rows.push(json!({
            "data_class": class,
            "n": of_class.len(),
            // The same predicate the pass receipt counts `prompted` with. An
            // `unconfigured` row never reached the wire, and counting it here
            // inflated the one number this route calls a prompting receipt
            // (review, 2026-09-05).
            "prompted": of_class
                .iter()
                .filter(|row| mail_model::was_prompted(&row.state))
                .count(),
        }));
    }
    rows
}

/// Min, median and max, or nulls. Not a mean: the question a threshold is set
/// from is where the middle sits, and one confident outlier moves a mean.
fn spread(values: impl Iterator<Item = i64>) -> Value {
    let mut values: Vec<i64> = values.collect();
    values.sort_unstable();
    match values.len() {
        0 => json!({ "min": null, "median": null, "max": null }),
        n => json!({
            "min": values[0],
            "median": values[n / 2],
            "max": values[n - 1],
        }),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageClassifyRevertBody {
    ids: Option<Vec<String>>,
    all: Option<bool>,
}

/// Put the deterministic verdict back on rows the model rung moved.
///
/// The bulk rollback, and it states in its own response what it cannot undo:
/// the class UPDATE is escalation-only and the narrowing that follows it is
/// deliberately not a delete. That asymmetry is exactly why apply never
/// escalates in the first place.
pub(super) async fn triage_classify_revert_handler(
    Json(body): Json<TriageClassifyRevertBody>,
) -> HttpResponse {
    let ids = match (body.ids, body.all) {
        (Some(ids), _) if !ids.is_empty() => Some(ids),
        (_, Some(true)) => None,
        // Neither is a request to revert nothing, and answering 200 to it would
        // read as "nothing had been applied".
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "one of `ids` (non-empty) or `all: true` is required",
            )
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path).map_err(|error| error.to_string())?;
        let (reverted, skipped_not_model, skipped_no_rules_row) = store
            .revert_model_streams(ids.as_deref())
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "reverted": reverted,
            "skipped_not_model": skipped_not_model,
            "skipped_no_rules_row": skipped_no_rules_row,
            "class_unchanged": true,
            "note": "the data class and any narrowing this rung caused are NOT restored",
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

#[derive(Debug, Deserialize)]
pub(super) struct TriageGmailBody {
    action: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TriageGmailJobBody {
    decision: String,
}

pub(super) async fn triage_gmail_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailBody>,
) -> HttpResponse {
    if !matches!(body.action.as_str(), "archive" | "trash" | "restore") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "action must be archive, trash, or restore" })),
        );
    }
    let action_name = body.action;
    let response_action = action_name.clone();
    let result = tokio::task::spawn_blocking(
        move || -> Result<GmailActionOutcome, (StatusCode, String)> {
            let cfg = Config::load();
            let store = Store::open(&cfg.database_path)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            let job = store
                .queue_gmail_action(&id, &action_name)
                .map_err(|error| {
                    let message = error.to_string();
                    let status = if message == "mail proposal not found" {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::CONFLICT
                    };
                    (status, message)
                })?;
            let token = google::access_token(&cfg.google_env_path).map_err(|_| {
                let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
                (
                    StatusCode::BAD_GATEWAY,
                    "Google authorization unavailable; the action remains queued for retry".into(),
                )
            })?;
            execute_gmail_action_job(&store, &token, &job).map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("{error}; the action remains queued for bounded retry"),
                )
            })
        },
    )
    .await;

    match result {
        Ok(Ok(GmailActionOutcome::Confirmed { changed })) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "action": response_action,
                "gmail_changed": changed,
                "gmail_confirmed": true
            })),
        ),
        Ok(Ok(GmailActionOutcome::Missing)) => (
            StatusCode::GONE,
            Json(json!({
                "error": "Gmail thread no longer exists; Axon retained its local record"
            })),
        ),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_gmail_job_handler(
    Path(id): Path<String>,
    Json(body): Json<TriageGmailJobBody>,
) -> HttpResponse {
    if !matches!(body.decision.as_str(), "retry" | "cancel") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "decision must be retry or cancel" })),
        );
    }
    let decision = body.decision;
    let result = tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let store = Store::open(&cfg.database_path)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        if decision == "cancel" {
            return store
                .cancel_abandoned_gmail_action(&id)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                .and_then(|canceled| {
                    if canceled {
                        Ok((StatusCode::OK, json!({ "ok": true, "state": "canceled" })))
                    } else {
                        Err((
                            StatusCode::CONFLICT,
                            "no Gmail action needs operator attention".into(),
                        ))
                    }
                });
        }

        let job = store
            .retry_abandoned_gmail_action(&id)
            .map_err(|error| (StatusCode::CONFLICT, error.to_string()))?;
        let token = google::access_token(&cfg.google_env_path).map_err(|_| {
            let _ = store.fail_gmail_action(job.job_id, "Google authorization unavailable");
            (
                StatusCode::BAD_GATEWAY,
                "Google authorization unavailable; the action remains queued for retry".into(),
            )
        })?;
        match execute_gmail_action_job(&store, &token, &job).map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("{error}; the action remains queued for bounded retry"),
            )
        })? {
            GmailActionOutcome::Confirmed { changed } => Ok((
                StatusCode::OK,
                json!({ "ok": true, "state": "completed", "gmail_changed": changed }),
            )),
            GmailActionOutcome::Missing => Ok((
                StatusCode::GONE,
                json!({
                    "error": "Gmail thread no longer exists; Axon retained its local record"
                }),
            )),
        }
    })
    .await;

    match result {
        Ok(Ok((status, value))) => (status, Json(value)),
        Ok(Err((status, error))) => (status, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

pub(super) async fn triage_reconcile_handler() -> HttpResponse {
    let result = tokio::task::spawn_blocking(|| {
        let cfg = Config::load();
        run_gmail_maintenance(&cfg, 200)
    })
    .await;
    match result {
        Ok(Ok(counts)) => (
            StatusCode::OK,
            Json(serde_json::to_value(counts).unwrap_or_else(|_| json!({}))),
        ),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": error }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "task failed" })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp file this test owns. The live store is never opened here: the
    /// rows below are fixtures, and `refresh_one_triage_class` writes.
    fn test_store(name: &str) -> Store {
        let directory =
            std::env::temp_dir().join(format!("comms-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a writable temp directory");
        let path = directory.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        Store::open(&path).expect("a store on a path this test owns")
    }

    /// A shadow pass writes verdicts and moves no category. Driven through the
    /// store rather than the HTTP handler, because the handler reads
    /// `Config::load()` — the operator's real overlay and their real database
    /// path — and a test that took that route would be a test about this
    /// machine.
    #[test]
    fn the_shadow_route_writes_verdicts_and_changes_no_category() {
        let store = test_store("classify_shadow");
        store
            .upsert_triage_with_rules(
                &stored_row("thread:shadow", "Re: the thing", "A short preview."),
                &comms::rules::Verdict {
                    stream: "aktiv".into(),
                    rationale: "No rule matched; kept active as the conservative default.".into(),
                    decided_by: comms::rules::DecidedBy::Fallback,
                },
            )
            .unwrap();
        // No light role on this config, so no request is made and the state is
        // `unconfigured`. What the test is about is the two invariants either
        // side of the call: a verdict row exists, and the category did not move.
        //
        // `database_path` points at this test's own temp file rather than being
        // empty, because `local_gate::lock_path` falls back to `.` when the
        // path has no parent — an empty one drops an `axon-local-*.lock` in the
        // crate directory, which is how one got committed once.
        let cfg = Config {
            database_path: std::env::temp_dir()
                .join(format!("comms-server-test-{}", std::process::id()))
                .join("classify_shadow.db"),
            inference: sjel_inference::InferenceConfig::default(),
            ..Config::load()
        };
        let receipt = mail_model::run_pass(&cfg, &store, Mode::Shadow, 200, 0)
            .expect("a pass over one candidate");
        assert_eq!(receipt.eligible, 1);
        assert_eq!(receipt.applied, 0);

        let summaries = store.model_verdict_summaries(None).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].mode, "shadow");
        let row = store.get_triage("thread:shadow").unwrap().unwrap();
        assert_eq!(row.stream, "aktiv");
        assert_eq!(
            row.classification_method, "deterministic",
            "a shadow pass must move no category"
        );
    }

    /// The rollout the feature documents: measure in shadow, read the corpus,
    /// set `mail_model.apply`, run apply. It used to move nothing — a stored
    /// `generated` verdict makes the row not due, `run_pass` only ever touched
    /// due rows, so apply found nothing to do (review, 2026-09-05).
    ///
    /// Nothing here prompts: the stored verdict IS the answer, which is also
    /// why this test can run on a machine with no local model.
    #[test]
    fn apply_writes_the_verdict_an_earlier_shadow_pass_stored() {
        let store = test_store("classify_apply_stored");
        store
            .upsert_triage_with_rules(
                &stored_row("thread:apply", "20% off everything", "Shop the sale."),
                &comms::rules::Verdict {
                    stream: "aktiv".into(),
                    rationale: "No rule matched; kept active as the conservative default.".into(),
                    decided_by: comms::rules::DecidedBy::Fallback,
                },
            )
            .unwrap();
        let cfg = Config {
            database_path: std::env::temp_dir()
                .join(format!("comms-server-test-{}", std::process::id()))
                .join("classify_apply_stored.db"),
            ..Config::load()
        };
        // What a shadow pass on this machine leaves behind, written by hand so
        // the test does not need a model: the current producer, the current
        // prompt revision, and the revision of this row.
        let row = store.get_triage("thread:apply").unwrap().unwrap();
        let verdict = ModelVerdict {
            triage_id: "thread:apply".into(),
            mode: "shadow".into(),
            state: "generated".into(),
            rule_decided_by: "fallback".into(),
            rule_stream: "aktiv".into(),
            model_stream: Some("werbung".into()),
            confidence_bp: Some(9_500),
            urgency_bp: Some(0),
            rationale: Some("It advertises a sale.".into()),
            urgency_rationale: Some("Nothing is asked.".into()),
            redactions: 0,
            data_class: row.data_class.clone(),
            redaction_class: row.data_class.clone(),
            producer: mail_model::producer(&cfg),
            item_revision: mail_model::item_revision(
                mail_model::sender_domain(row.from_addr.as_deref().unwrap_or_default()).as_deref(),
                row.subject.as_deref().unwrap_or_default(),
                row.snippet.as_deref().unwrap_or_default(),
                &row.data_class,
            ),
            prompt_revision: mail_model::MAIL_MODEL_PROMPT_REVISION.into(),
            classification_version: mail_model::MAIL_MODEL_VERSION.into(),
            attempts: 0,
            last_error: None,
            next_attempt: None,
            held_reason: None,
            applied_at: None,
        };
        store.upsert_model_verdict(&verdict).unwrap();

        // A second shadow pass still writes nothing, and says how much is
        // waiting.
        let shadow = mail_model::run_pass(&cfg, &store, Mode::Shadow, 200, 9_000)
            .expect("a second shadow pass");
        assert_eq!(shadow.eligible, 0, "the question was already answered");
        assert_eq!(shadow.awaiting_apply, 1);
        assert_eq!(shadow.applied, 0);
        assert_eq!(
            store.get_triage("thread:apply").unwrap().unwrap().stream,
            "aktiv"
        );

        let applied =
            mail_model::run_pass(&cfg, &store, Mode::Apply, 200, 9_000).expect("an apply pass");
        assert_eq!(applied.prompted, 0, "apply re-asks nothing");
        assert_eq!(applied.awaiting_apply, 1);
        assert_eq!(applied.disagreed, 1);
        assert_eq!(applied.applied, 1);

        let moved = store.get_triage("thread:apply").unwrap().unwrap();
        assert_eq!(moved.stream, "werbung");
        assert_eq!(moved.classification_method, "model");
        let stored = store.model_verdicts().unwrap();
        assert_eq!(stored["thread:apply"].mode, "applied");
        assert!(
            stored["thread:apply"].applied_at.is_some(),
            "a machine write onto the category axis leaves a stamp"
        );

        // And it does not run twice: the applied row is no longer a candidate.
        let again = mail_model::run_pass(&cfg, &store, Mode::Apply, 200, 9_000)
            .expect("a third pass changes nothing");
        assert_eq!(again.reviewed, 0);
        assert_eq!(again.applied, 0);
    }

    /// The floor is a policy threshold, and a pass below it writes nothing.
    #[test]
    fn a_stored_verdict_below_the_floor_is_counted_and_not_written() {
        let store = test_store("classify_apply_floor");
        store
            .upsert_triage_with_rules(
                &stored_row("thread:floor", "20% off everything", "Shop the sale."),
                &comms::rules::Verdict {
                    stream: "aktiv".into(),
                    rationale: "No rule matched; kept active as the conservative default.".into(),
                    decided_by: comms::rules::DecidedBy::Fallback,
                },
            )
            .unwrap();
        let cfg = Config {
            database_path: std::env::temp_dir()
                .join(format!("comms-server-test-{}", std::process::id()))
                .join("classify_apply_floor.db"),
            ..Config::load()
        };
        let row = store.get_triage("thread:floor").unwrap().unwrap();
        store
            .upsert_model_verdict(&ModelVerdict {
                triage_id: "thread:floor".into(),
                mode: "shadow".into(),
                state: "generated".into(),
                rule_decided_by: "fallback".into(),
                rule_stream: "aktiv".into(),
                model_stream: Some("werbung".into()),
                confidence_bp: Some(4_000),
                urgency_bp: Some(0),
                rationale: None,
                urgency_rationale: None,
                redactions: 0,
                data_class: row.data_class.clone(),
                redaction_class: row.data_class.clone(),
                producer: mail_model::producer(&cfg),
                item_revision: mail_model::item_revision(
                    mail_model::sender_domain(row.from_addr.as_deref().unwrap_or_default())
                        .as_deref(),
                    row.subject.as_deref().unwrap_or_default(),
                    row.snippet.as_deref().unwrap_or_default(),
                    &row.data_class,
                ),
                prompt_revision: mail_model::MAIL_MODEL_PROMPT_REVISION.into(),
                classification_version: mail_model::MAIL_MODEL_VERSION.into(),
                attempts: 0,
                last_error: None,
                next_attempt: None,
                held_reason: None,
                applied_at: None,
            })
            .unwrap();

        let receipt =
            mail_model::run_pass(&cfg, &store, Mode::Apply, 200, 9_000).expect("an apply pass");
        assert_eq!(receipt.below_confidence, 1);
        assert_eq!(receipt.applied, 0);
        assert_eq!(
            store.get_triage("thread:floor").unwrap().unwrap().stream,
            "aktiv"
        );
    }

    /// The report body is meant to be pasteable into a decision record. The
    /// store method behind it cannot select a rationale, and this pins that the
    /// handler adds no second path to one.
    #[test]
    fn the_report_quotes_no_mail_content() {
        let store = test_store("classify_report_safe");
        store
            .upsert_triage_with_rules(
                &stored_row("thread:report", "ZZSUBJECTTOKEN", "ZZSNIPPETTOKEN"),
                &comms::rules::Verdict {
                    stream: "aktiv".into(),
                    rationale: "No rule matched; kept active as the conservative default.".into(),
                    decided_by: comms::rules::DecidedBy::Fallback,
                },
            )
            .unwrap();
        store
            .upsert_model_verdict(&ModelVerdict {
                triage_id: "thread:report".into(),
                mode: "shadow".into(),
                state: "generated".into(),
                rule_decided_by: "fallback".into(),
                rule_stream: "aktiv".into(),
                model_stream: Some("issue".into()),
                confidence_bp: Some(9_000),
                urgency_bp: Some(7_000),
                rationale: Some("ZZRATIONALETOKEN".into()),
                urgency_rationale: Some("ZZURGENCYTOKEN".into()),
                redactions: 0,
                data_class: "c1".into(),
                redaction_class: "c1".into(),
                producer: "foundation-models/apple:mail-stream-v1-english".into(),
                item_revision: "revision".into(),
                prompt_revision: mail_model::MAIL_MODEL_PROMPT_REVISION.into(),
                classification_version: mail_model::MAIL_MODEL_VERSION.into(),
                attempts: 0,
                last_error: None,
                next_attempt: None,
                held_reason: None,
                applied_at: None,
            })
            .unwrap();

        // The handler's own body, not a rebuild of it: a rebuild would stay
        // green the day the route grows a key that carries mail text, which is
        // the one thing this test claims to pin (review, 2026-09-05).
        let summaries = store.model_verdict_summaries(None).unwrap();
        let body = report_body(
            &summaries,
            store.model_rung_candidate_count().unwrap(),
            &store.triage_classification_methods().unwrap(),
            &Config::load(),
        )
        .to_string();

        for token in [
            "ZZSUBJECTTOKEN",
            "ZZSNIPPETTOKEN",
            "ZZRATIONALETOKEN",
            "ZZURGENCYTOKEN",
        ] {
            assert!(!body.contains(token), "the report quoted {token}: {body}");
        }
        assert!(body.contains("\"agree\":0"), "got {body}");
        assert!(
            body.contains("\"validated\":false"),
            "urgency must not read as ranked"
        );
    }

    fn stored_row(id: &str, subject: &str, snippet: &str) -> TriageItem {
        TriageItem {
            id: id.into(),
            from_addr: Some("security@example.com".into()),
            subject: Some(subject.into()),
            snippet: Some(snippet.into()),
            internal_date_ms: Some(1_754_000_000_000),
            internal_date_text: None,
            stream: "aktiv".into(),
            rationale: "test".into(),
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: comms::rules::MAIL_RULES_VERSION.into(),
            // The state this endpoint has to repair: a row stored before the
            // intake gate existed, holding its subject verbatim.
            data_class: "c1".into(),
            data_class_rationale: "Mail metadata is Mine by default.".into(),
            data_classification_method: content_item::METHOD_DETERMINISTIC.into(),
            data_classification_version: "data-class-rules-v1".into(),
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: String::new(),
            last_seen: String::new(),
        }
    }

    /// The receipt's `redacted` count is not decoration: a class raised to c3
    /// and the narrowing that class demands happen in the same pass, so no
    /// stored row is left labelled Redacted while still holding the code.
    ///
    /// The one-time-code rule is used rather than the named-person rule
    /// deliberately — it reads no registry, so the outcome is a property of the
    /// classifier and not of the machine's contact list.
    #[test]
    fn a_refresh_that_raises_a_class_redacts_in_the_same_pass() {
        let store = test_store("data_class_refresh_redacts");
        let item = stored_row(
            "thread:code",
            "Your verification code is 448215",
            "Use 448215 to sign in",
        );
        store.upsert_triage(&item).expect("the fixture row stores");

        let (reclassified, narrowed) =
            refresh_one_triage_class(&store, &item).expect("the pass completes");
        assert!(reclassified, "the code rule raises this row");
        assert!(narrowed, "and the same pass narrows it");

        let row = store
            .get_triage("thread:code")
            .expect("readable")
            .expect("still there");
        assert_eq!(row.data_class, "c3");
        let subject = row.subject.clone().unwrap();
        let snippet = row.snippet.clone().unwrap();
        assert!(!subject.contains("448215"), "the code survived: {subject}");
        assert!(!snippet.contains("448215"), "the code survived: {snippet}");
        assert!(subject.contains("[number]"));
    }

    /// Running it twice reports zero narrowed rows the second time, which is
    /// how an operator knows the first pass finished rather than half-ran.
    #[test]
    fn a_second_refresh_pass_has_nothing_left_to_narrow() {
        let store = test_store("data_class_refresh_idempotent");
        let item = stored_row(
            "thread:code",
            "Your verification code is 448215",
            "Use 448215 to sign in",
        );
        store.upsert_triage(&item).expect("the fixture row stores");
        refresh_one_triage_class(&store, &item).expect("the first pass completes");

        let stored = store
            .get_triage("thread:code")
            .expect("readable")
            .expect("still there");
        let (_, narrowed) =
            refresh_one_triage_class(&store, &stored).expect("the second pass completes");
        assert!(!narrowed, "a second pass must find nothing to remove");
    }

    /// Redaction stays scoped by class. A c1 row keeps its readable subject, or
    /// the review list stops being reviewable — which is its own safety failure.
    #[test]
    fn a_refresh_leaves_an_ordinary_row_readable() {
        let store = test_store("data_class_refresh_verbatim");
        let item = stored_row("thread:lunch", "Lunch on Tuesday?", "Half twelve as usual");
        store.upsert_triage(&item).expect("the fixture row stores");

        let (_, narrowed) = refresh_one_triage_class(&store, &item).expect("the pass completes");
        assert!(!narrowed);
        let row = store
            .get_triage("thread:lunch")
            .expect("readable")
            .expect("still there");
        assert_eq!(row.subject.as_deref(), Some("Lunch on Tuesday?"));
    }
}
