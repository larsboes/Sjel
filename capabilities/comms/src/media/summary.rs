//! The summarization pipeline: prompt, dispatch, and the drain that walks the
//! store.
//!
//! Separate from the fetch half because it is the slow half -- a local model, up to
//! two minutes -- and the server answers after the fetch and summarizes behind the
//! response.

use super::*;

/// Max characters of article/transcript text fed to the summarizer prompt.
pub(super) const SUMMARY_INPUT_CAP: usize = 15_000;

/// Room the summary prompt asks the model to answer in. A named constant
/// because the window check in [`crate::quiet`] has to count the same number
/// this request sends: a fit computed against the prompt alone offers a
/// 4,096-token model a job it has no room to finish.
pub(super) const SUMMARY_REPLY_TOKENS: u32 = 800;

pub const SUMMARY_PROMPT_REVISION: &str = "feed-summary-v2-english";

/// Typed summarization outcome -- replaces the old `Option<String>` that
/// collapsed every failure into `None`, making them indistinguishable from
/// "not configured" and unretryable.
pub enum SummarizeOutcome {
    Ok(String),
    Unconfigured,
    HttpError(String),
    ModelError(String),
    /// The server took the request and then ran out of room for it. Same
    /// distinction `summarize::Outcome` draws, and for the same reason: this
    /// one is about the machine, not the request.
    CapacityAborted(String),
    EmptyResponse,
    Timeout,
    /// The configured summarization role is not loopback, and the item's stored
    /// data class does not clear it for that endpoint. Same verdict and same
    /// spelling as `summarize::Outcome::RemoteRefused`, because it is the same
    /// refusal about the same item.
    RemoteRefused,
    /// The item's stored class enters no prompt at all — `c3`, and any value
    /// from outside the vocabulary (T3). One step stricter than
    /// [`SummarizeOutcome::RemoteRefused`]: that one asks where the endpoint is,
    /// this one does not care, because there is no model this text may be shown
    /// to. Same spelling as `digest::LOCAL_REFUSED`, the same refusal on the
    /// other prefill path.
    LocalRefused,
    /// The source does not fit the light local rung, and an unattended pass may
    /// not reach past it (`crate::quiet`). Not a failure and not an attempt:
    /// nothing was sent anywhere, so nothing is counted against the row's retry
    /// ledger or the capacity-alert streak.
    OverWindow,
    PolicyDeferred(String),
}

impl SummarizeOutcome {
    /// Short, loggable error class for the retry ledger.
    pub fn error_class(&self) -> &'static str {
        match self {
            SummarizeOutcome::Ok(_) => "ok",
            SummarizeOutcome::Unconfigured => "unconfigured",
            SummarizeOutcome::HttpError(_) => "http_error",
            SummarizeOutcome::ModelError(_) => "model_error",
            SummarizeOutcome::CapacityAborted(_) => "capacity_aborted",
            SummarizeOutcome::EmptyResponse => "empty_response",
            SummarizeOutcome::Timeout => "timeout",
            SummarizeOutcome::RemoteRefused => "remote_refused",
            SummarizeOutcome::LocalRefused => crate::digest::LOCAL_REFUSED,
            SummarizeOutcome::OverWindow => "over_window",
            SummarizeOutcome::PolicyDeferred(_) => "policy_deferred",
        }
    }
}

/// Cap the text handed to the summarizer (a 1h transcript is 100k+ chars; the
/// local model's context is finite). Appends a `…[truncated]` marker when cut.
/// The full transcript is still stored in the DB unchanged.
pub(super) fn truncate_for_summary(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        text.to_string()
    } else {
        let head: String = text.chars().take(cap).collect();
        format!("{head}…[truncated]")
    }
}

pub(super) fn summary_prompt(input: &str) -> String {
    format!(
        "Summarize the following content as a compact digest. Start with the key points as short \
         bullet points, then add exactly one sentence of context. Write in English, even when the \
         source is in another language. Do not add a preamble.\n\nContent:\n{input}"
    )
}

/// The producer string this machine's unattended summary pass writes.
///
/// The **light** role, because that is the only rung an unattended pass may use
/// now (`crate::quiet`). It moved here from `summarization`: leaving it on the
/// strong role would have every summary the drain writes labelled as the big
/// model's work, and the staleness query would then hand every one of them
/// straight back on the next pass.
pub fn summary_producer_revision(cfg: &Config) -> Option<String> {
    cfg.light_summarization_role()
        .map(|role| format!("{}:{SUMMARY_PROMPT_REVISION}", role.cache_key()))
}

/// Summarize text into a compact English digest via an OpenAI-compatible
/// chat-completions endpoint. Returns a typed outcome so the caller can
/// distinguish "not configured" from "server down" from "empty response" and
/// record the failure class for bounded retry.
///
/// ## Why this path takes the gate too
///
/// This is the *other* thing that prefills a feed item's transcript on the
/// local server: `feed_items.summary`, where `digest::generate` writes
/// `content_digests`. Until 2026-08-13 only the digest path went through
/// [`local_gate::AdvisoryGate`], so the two drains — which both defaulted to 15
/// minutes and both started their tickers at spawn — sent two prefills of the
/// same transcript at the same backend on the same tick, one of them holding a
/// lock the other had never heard of. The lock is only admission control if
/// everything that prefills asks for it.
///
/// ## And why it takes the class too
///
/// This function speaks HTTP itself rather than going through `libs/summarize`,
/// so it inherited none of that lib's remote refusal. It asked no question about
/// the item at all: point the summarization role at an https endpoint and every
/// feed transcript the enrichment drain touched went there, whatever its class.
/// `data_class` is the item's stored value, and the verdict is
/// `cloud_derivative::tier_allows` asked about the passthrough representation —
/// the same question the digest path asks, because this sends the same text in
/// the same unredacted form.
///
/// ## And why the class is asked twice
///
/// [`SummarizeOutcome::LocalRefused`] is asked first and about the class alone,
/// before a rung is even resolved: `c3` enters no prompt on this machine either
/// (T3), and the loopback POST below is a prompt. `content_item::
/// local_prompt_allowed` is the gate, the same one `digest.rs` asks and the same
/// one `processing_policy(..).local_processing` is derived from.
pub fn summarize(text: &str, cfg: &Config, data_class: &str) -> SummarizeOutcome {
    if !crate::content_item::local_prompt_allowed(data_class) {
        return SummarizeOutcome::LocalRefused;
    }
    // The quiet lane, and only the quiet lane. Every caller of this function is
    // an unattended pass — the enrichment drain, the prefill behind
    // `POST /ingest`, `comms summarize --pending` — and none of them is an
    // operator watching an item. Resolving `summarization` here is what fed 182
    // transcripts through a 9B model on the GPU on 2026-08-13.
    let role = match crate::quiet::rung(&cfg.inference, text.chars().count(), SUMMARY_REPLY_TOKENS)
    {
        crate::quiet::Rung::Light(role) => *role,
        crate::quiet::Rung::OverWindow => return SummarizeOutcome::OverWindow,
        crate::quiet::Rung::Unconfigured => return SummarizeOutcome::Unconfigured,
    };
    if let Err(reason) =
        crate::quiet::runtime_admission(&role, text.chars().count(), SUMMARY_REPLY_TOKENS)
    {
        return SummarizeOutcome::PolicyDeferred(reason.to_string());
    }
    // The class gate asks whether this endpoint may see the content at all, so
    // it asks about hardware the operator controls (Q39), not about loopback.
    // The admission gate below still asks `is_loopback`: a trusted peer does not
    // share this machine's GPU.
    if !role.trusted_for_every_class()
        && !crate::cloud_derivative::verbatim_send_allowed(
            role.cloud_data_tier.map(|tier| tier.as_str()),
            data_class,
        )
    {
        return SummarizeOutcome::RemoteRefused;
    }
    let input = if role.is_afm() {
        text.to_string()
    } else {
        truncate_for_summary(text, SUMMARY_INPUT_CAP)
    };
    let prompt = summary_prompt(&input);

    // Held to the end of the function by drop, on every return path below.
    // Loopback only, for the reason `summarize::complete` gives: a hosted
    // provider queues for itself and shares no GPU with anything here.
    let _admission = if role.is_loopback() {
        let gate = crate::local_gate::AdvisoryGate::new(&cfg.database_path, &role.backend_name);
        match crate::summarize::LocalGate::acquire(&gate) {
            Ok(admission) => Some(admission),
            // Not a failure of the request: the same text succeeds later, and
            // the drain brings the row back.
            Err(reason) => return SummarizeOutcome::CapacityAborted(reason),
        }
    } else {
        None
    };

    let http = match sjel_http::client(
        sjel_http::Purpose::new("comms-summarize"),
        std::time::Duration::from_secs(120),
    ) {
        Ok(c) => c,
        Err(e) => return SummarizeOutcome::HttpError(e.to_string()),
    };
    let body = serde_json::json!({
        "model": role.model,
        "messages": [{ "role": "user", "content": prompt }],
        "max_tokens": SUMMARY_REPLY_TOKENS,
        "stream": false,
    });
    let mut req = http.post(role.chat_completions_endpoint()).json(&body);
    if let Some(key) = role.bearer_key() {
        req = req.bearer_auth(key);
    }
    if let Err(reason) = role.admit_chat_request(&body) {
        return if sjel_runtime::is_deferred(&reason) {
            SummarizeOutcome::PolicyDeferred(reason)
        } else {
            // No request was sent. In particular, AFM's context refusal must not
            // burn retries or replace a prior summary.
            SummarizeOutcome::OverWindow
        };
    }
    let resp = match req.send() {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return SummarizeOutcome::Timeout,
        Err(e) => return SummarizeOutcome::HttpError(e.to_string()),
    };
    if !resp.status().is_success() {
        return SummarizeOutcome::ModelError(format!("status {}", resp.status()));
    }
    let body: serde_json::Value = match resp.json() {
        Ok(b) => b,
        Err(e) => return SummarizeOutcome::ModelError(e.to_string()),
    };
    // Before `choices`, for the reason `summarize::server_error` documents: a
    // 200 whose body is an error envelope was reaching the ledger as
    // `empty_response`, which is both the wrong cause and the wrong advice.
    // This path is the 15-minute enrichment drain, so it hits the same busy
    // server the digest path does.
    match crate::summarize::server_error(&body) {
        Some(crate::summarize::ServerError::Capacity(message)) => {
            return SummarizeOutcome::CapacityAborted(message)
        }
        Some(crate::summarize::ServerError::Other(message)) => {
            return SummarizeOutcome::ModelError(message)
        }
        None => {}
    }
    let out = body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::trim)
        .unwrap_or("");
    if out.is_empty() {
        SummarizeOutcome::EmptyResponse
    } else {
        SummarizeOutcome::Ok(out.to_string())
    }
}

/// Cheap readiness probe for the configured OpenAI-compatible summarizer.
/// `/models` does not trigger a generation or load a model into memory.
/// The rung the unattended passes actually use, so a health probe answers about
/// the model that is going to be asked rather than about one that is not.
pub fn summarizer_reachable(cfg: &Config) -> bool {
    cfg.light_summarization_role()
        .as_ref()
        .is_some_and(sjel_inference::ResolvedRole::model_reachable)
}

/// Summarize one stored item, if it has a transcript and no summary yet.
/// Returns whether a summary was written. Records attempt count and error
/// class on failure so the retry ledger is honest. Used by the server after a
/// `POST /ingest` has already answered — deliberately scoped to the single id
/// rather than reusing `summarize_pending`, so two concurrent ingests don't
/// each pick up the other's row.
pub fn summarize_item(store: &Store, cfg: &Config, id: &str) -> Result<bool> {
    let item = match store
        .get_feed(id)
        .map_err(|e| CommsError::Other(e.to_string()))?
    {
        Some(i) => i,
        None => return Ok(false),
    };
    let Some(producer_revision) = summary_producer_revision(cfg) else {
        return Ok(false);
    };
    if !store
        .feed_summary_needs_revision(id, &producer_revision)
        .map_err(|e| CommsError::Other(e.to_string()))?
    {
        return Ok(false);
    }
    let text = match &item.transcript {
        Some(t) => t,
        None => return Ok(false),
    };
    match summarize(text, cfg, &item.data_class) {
        SummarizeOutcome::Ok(summary) => {
            store
                .update_feed_summary(id, &summary, &producer_revision)
                .map_err(|e| CommsError::Other(e.to_string()))?;
            Ok(true)
        }
        // None of these is an attempt. One says this machine has no unattended
        // rung at all, one says this source is past it, and the third says this
        // class enters no prompt on this machine — a verdict, not a failure.
        // Writing any of them to the retry ledger would burn the row's three
        // attempts on a decision no retry can change, and for `LocalRefused`
        // that is worse than useless: after three drain ticks the row is parked
        // until the producer revision moves, so a c3 item later downgraded to c1
        // would stay un-summarized. `digest.rs` decides the identical refusal
        // the same way — `write_digest` forces `attempts` to 0 for it.
        SummarizeOutcome::Unconfigured
        | SummarizeOutcome::OverWindow
        | SummarizeOutcome::LocalRefused => Ok(false),
        SummarizeOutcome::PolicyDeferred(reason) => Err(CommsError::Other(reason)),
        outcome => {
            let _ = store.record_summary_attempt(id, outcome.error_class(), &producer_revision);
            Ok(false)
        }
    }
}

/// What one unattended enrichment pass did.
///
/// Two numbers rather than one, for the reason `digest::DrainReport` gives:
/// most of this machine's backlog is longer than the on-device window, so
/// "summarized 0" on its own reads as a broken model server when it is in fact
/// the quiet policy working exactly as ratified.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnrichmentPass {
    /// Summaries written by the light local rung.
    pub summarized: usize,
    /// Items it could not hold, skipped without a request and without a mark on
    /// their retry ledger.
    pub over_window: usize,
}

/// Retry summarization for eligible feed items (bounded by attempt cap and
/// exponential backoff).
pub fn summarize_pending(store: &Store, cfg: &Config) -> Result<EnrichmentPass> {
    let Some(producer_revision) = summary_producer_revision(cfg) else {
        return Ok(EnrichmentPass::default());
    };
    let pending = store
        .feed_pending_summaries(Some(&producer_revision))
        .map_err(|e| CommsError::Other(e.to_string()))?;
    let mut pass = EnrichmentPass::default();
    for item in pending {
        if let Some(text) = &item.transcript {
            // Re-read, because the batch above is a snapshot and the pass takes
            // minutes. `POST /ingest` summarizes its own row inline, and a
            // drain tick that overlapped an ingest prefilled the same
            // transcript twice on the same backend — the second call paid for
            // an answer the first had already written. Cheap indexed read
            // against a 12-20s model call; skipping it is what cost the double.
            match store.feed_summary_needs_revision(&item.id, &producer_revision) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => return Err(CommsError::Other(e.to_string())),
            }
            match summarize(text, cfg, &item.data_class) {
                SummarizeOutcome::Ok(summary) => {
                    store
                        .update_feed_summary(&item.id, &summary, &producer_revision)
                        .map_err(|e| CommsError::Other(e.to_string()))?;
                    crate::capacity::record_success(store);
                    pass.summarized += 1;
                }
                // Left pending by design. No request was made, so there is
                // nothing to count — not against this row's three attempts,
                // and not against the capacity streak, which exists to say the
                // local server is failing requests it accepted.
                SummarizeOutcome::OverWindow => pass.over_window += 1,
                // The same no-attempt arm, and not counted as over-window: the
                // source fits the rung, the class is what refuses. A verdict is
                // not a failed attempt, so the retry ledger must not move —
                // `summarize_item` and `digest::write_digest` decide this
                // refusal the same way.
                SummarizeOutcome::LocalRefused => {}
                SummarizeOutcome::PolicyDeferred(reason) => return Err(CommsError::Other(reason)),
                SummarizeOutcome::Unconfigured => break, // no point continuing
                outcome => {
                    // Same streak the digest drain counts, because it is the
                    // same server running out of the same room. Two counters
                    // would each sit below the threshold while the machine was
                    // plainly broken.
                    if let SummarizeOutcome::CapacityAborted(_) = outcome {
                        if let Some(streak) =
                            crate::capacity::record_failure(store, cfg.capacity_alert_after)
                        {
                            eprintln!(
                                "enrichment drain: ALERT — {streak} consecutive capacity aborts \
                                 from the local inference server; summaries are not being written"
                            );
                        }
                    }
                    let _ = store.record_summary_attempt(
                        &item.id,
                        outcome.error_class(),
                        &producer_revision,
                    );
                }
            }
        }
    }
    Ok(pass)
}
