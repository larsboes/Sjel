//! Producing a digest for one stored item, whatever source it came from.
//!
//! `libs/summarize` owns the ladder, the prompt and the Mermaid gate. This
//! module owns the three things that are specific to Axon: **where the source
//! text comes from**, **whether a remote target is allowed to see it**, and
//! **what has to be removed before the result is written down**.
//!
//! ## The mail body never lands
//!
//! A mail digest reads the message body with `format=full`, hands it to the
//! model and drops it. Nothing writes it. The operator's mail doctrine is that
//! a message is distilled into an outcome and never retained as a local copy, so
//! the digest row is the only thing that survives [`generate`]. The sweep is
//! untouched and still reads
//! `format=metadata` — reading a body is a separate, bounded, explicit act.
//!
//! ## c1, c2 and c3 content never leaves the machine
//!
//! The stored data class decides, and it decides through the same gate the
//! reviewed-derivative queue uses: [`cloud_derivative::tier_allows`], asked
//! about the passthrough representation, because a digest hands the model the
//! source text as it stands. Anything it does not admit becomes
//! `Reach::LoopbackOnly`, and `libs/summarize` refuses a non-loopback endpoint
//! outright rather than downgrading it. Mail is never `c0` by construction
//! (`content_item::DataClass::classify_mail`), so a mail digest is loopback-only
//! by the same rule that already governs mail relevance.
//!
//! This used to be its own class comparison here — a second copy of a policy
//! that already had a home, and a laxer one: it admitted any non-loopback
//! target for a c0 item, including an https endpoint with no reviewed cloud
//! policy on it at all.
//!
//! ## A c2 or c3 digest is redacted before it is stored
//!
//! For those classes the metadata *is* the payload — a one-time code arrives in
//! the subject line, and a model asked to summarize that mail will quote the
//! code back. The produced text therefore goes through the same deterministic
//! detector the sweep uses on subject and snippet, and the count is recorded, so
//! a digest cannot republish what the sweep redacted.

use crate::cloud_derivative::{self, redact_review_field, RedactionFinding};
use crate::config::Config;
use crate::content_item;
use crate::google;
use crate::store::{Store, StoredDigest};
use crate::summarize::{self, Depth, Directive, Outcome, Reach, Target};
use crate::Result;

/// How many retryable failures a row accumulates before the automatic pass
/// leaves it alone. An explicit press always runs regardless — the operator can
/// see the model is back up.
pub const MAX_ATTEMPTS: i32 = 3;

/// The target for one specific piece of work.
///
/// `Depth::Detailed` always takes the strongest configured role. That is the
/// press meaning "give me more", and answering it on the small model would be
/// the one case where the ladder's promise — that a rung is a step up rather
/// than a longer version of the same guess — stops being true.
///
/// Otherwise a light role is used when the source demonstrably fits its context
/// window, input and output together. Everything else falls through to the
/// strong role, so a machine with no light role configured, or a source too big
/// for it, behaves exactly as it did before this existed.
pub fn role_for(
    cfg: &Config,
    directive: &Directive,
    source_chars: usize,
) -> Option<sjel_inference::ResolvedRole> {
    let shape = directive.shape_for(source_chars);
    if directive.depth == Depth::Standard {
        if let Some(light) = cfg.light_summarization_role() {
            let window = light.max_input_tokens.unwrap_or_default();
            if summarize::fits_context(source_chars, shape, window) {
                return Some(light);
            }
        }
    }
    cfg.summarization_role()
}

/// Every producer string this machine could currently write for a digest.
///
/// A list rather than one string because the role is chosen per item: a short
/// source may be digested by the light model, a long one by the strong model on
/// the same pass, and a long `c0` one by a cloud provider, and all three are
/// current. Checking staleness against a single producer would mark the other
/// two stale on the next sweep and re-digest them forever, which is worse than
/// the gap it was meant to close — and for the cloud rung it would mean paying
/// for the same digest every fifteen minutes.
pub fn producer_revisions(cfg: &Config) -> Vec<String> {
    cfg.summarization_role()
        .into_iter()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
        .chain(unattended_producer_revisions(cfg))
        .collect()
}

/// The producer strings an **unattended** pass can write: the light local rung
/// and the cloud rungs, and deliberately not the strong local one, which only a
/// press reaches.
///
/// Used as the scope of the retry-attempt cap. Attempts spent by a model this
/// pass will not use are not this pass's attempts.
pub fn unattended_producer_revisions(cfg: &Config) -> Vec<String> {
    cfg.light_summarization_role()
        .into_iter()
        .chain(
            cfg.inference
                .roles_with_prefix("cloud_")
                .into_iter()
                .filter(|(_, role)| role.has_cloud_policy())
                .map(|(_, role)| role),
        )
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
        .collect()
}

/// Where one resolved role's requests go.
///
/// The `Target` shape is deliberately plain data: `libs/summarize` never learns
/// what an `InferenceConfig` is, so a capability with no inference dependency
/// can still call it.
pub(crate) fn to_target(cfg: &Config, role: &sjel_inference::ResolvedRole) -> Target {
    // Two questions, and Q39 is where they stopped having one answer. A trusted
    // peer may see any class (`trusted_for_every_class`) and must not queue
    // behind this machine's GPU gate (`is_loopback`), because it has its own.
    let loopback = role.is_loopback();
    Target {
        endpoint: role.chat_completions_endpoint(),
        model: role.model.clone(),
        backend_name: role.backend_name.clone(),
        max_input_tokens: role.max_input_tokens,
        api_key: role.bearer_key(),
        loopback,
        operator_owned: role.trusted_for_every_class(),
        // Only a local target gets a gate. A hosted provider queues for itself
        // and shares no GPU with anything here, so serialising against it would
        // cost latency and buy nothing.
        //
        // Keyed by backend, not by machine: AFM and oMLX are both loopback and
        // both local, and they share no memory pool. One key for both meant a
        // two-second AFM digest waited out a twenty-second oMLX prefill and
        // then reported the machine busy.
        gate: loopback.then(|| {
            crate::local_gate::AdvisoryGate::shared(&cfg.database_path, &role.backend_name)
        }),
    }
}

/// The stored producer string for the current target, or `None` when no
/// summarization role is configured.
pub fn producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
}

pub fn diagram_producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIAGRAM_PROMPT_REVISION))
}

pub fn chart_producer_revision(cfg: &Config) -> Option<String> {
    cfg.summarization_role()
        .map(|role| summarize::producer(&role.cache_key(), summarize::chart::CHART_PROMPT_REVISION))
}

/// What a source hands the model, and under which policy.
struct SourceText {
    text: String,
    data_class: String,
}

impl SourceText {
    /// How far this text may travel to reach `cloud_data_tier`.
    ///
    /// A digest sends the source text as it stands — nothing is redacted on the
    /// way out — which is exactly what `verbatim_send_allowed` answers. Handing
    /// the question to the reviewed-derivative gate is the point: the answer
    /// used to be an independent class comparison here, which is a
    /// second copy of a policy that already had a home and, being a copy, was
    /// already laxer than it.
    ///
    /// `None` — every loopback role, and any endpoint nobody gave a reviewed
    /// cloud policy — admits nothing. That is not a restriction on local models:
    /// `libs/summarize` only consults the verdict for a non-loopback target.
    fn reach(&self, cloud_data_tier: Option<&str>) -> Reach {
        if cloud_derivative::verbatim_send_allowed(cloud_data_tier, &self.data_class) {
            Reach::CloudCleared
        } else {
            Reach::LoopbackOnly
        }
    }

    /// Whether this text may enter a prompt on this machine at all.
    ///
    /// `false` for c3 and for any value outside the vocabulary (T3). The whole
    /// gate is `content_item::local_prompt_allowed`, which is also where
    /// `processing_policy(..).local_processing` comes from — so the field the
    /// dashboard prints and the answer this path gets are one function. They
    /// were two: the field said `blocked` for c3 and no call site read it, so a
    /// credential mail reached the loopback model through here.
    ///
    /// Refusing is not a downgrade to a smaller model. There is no prompt a
    /// credential belongs in, and a local model is still a log, a context
    /// window and a cache.
    fn local_prompt_allowed(&self) -> bool {
        content_item::local_prompt_allowed(&self.data_class)
    }

    fn redact_before_persistence(&self) -> bool {
        content_item::redact_before_persistence(&self.data_class)
    }
}

/// The verdict for one resolved role, or `LoopbackOnly` when no role resolved.
///
/// An unresolved role produces `Outcome::Unconfigured` before the verdict is
/// consulted at all, so the value is unobservable — but defaulting it closed
/// keeps that true if the order ever changes.
fn reach_for(gathered: &SourceText, role: Option<&sjel_inference::ResolvedRole>) -> Reach {
    match role {
        Some(role) => gathered.reach(role.cloud_data_tier.map(|tier| tier.as_str())),
        None => Reach::LoopbackOnly,
    }
}

/// Gather the text worth digesting for one stored item.
///
/// `None` means the item does not exist. An item that exists but has no usable
/// text comes back with an empty string, which the ladder correctly reads as
/// nothing worth digesting.
fn source_text(store: &Store, cfg: &Config, source: &str, id: &str) -> Result<Option<SourceText>> {
    match source {
        "feed" => Ok(store
            .get_feed(id)
            .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
            .map(|item| SourceText {
                // The stored class, not a literal. A hardcoded c0 here meant
                // every feed digest reported itself remotely eligible and
                // skipped redaction, on an item nobody had classified.
                data_class: item.data_class,
                // The transcript is the source; the stored summary is a
                // previous answer, and digesting an answer compounds whatever
                // it got wrong.
                text: item.transcript.unwrap_or_default(),
            })),
        "mail" => {
            let Some(item) = store
                .get_triage(id)
                .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
            else {
                return Ok(None);
            };
            // The body is fetched here and dropped when this function returns
            // its digest. The snippet is the fallback for a thread Gmail no
            // longer has, or a token that cannot be refreshed right now.
            let body = match google::access_token(&cfg.google_env_path) {
                Ok(token) => google::thread_body_text(&token, id).unwrap_or(None),
                Err(_) => None,
            };
            let text = body
                .filter(|body| !body.trim().is_empty())
                .or_else(|| item.snippet.clone())
                .unwrap_or_default();
            Ok(Some(SourceText {
                text,
                data_class: item.data_class,
            }))
        }
        "calendar" => Ok(calendar_entry_text(cfg, id)?),
        other => Err(crate::CommsError::Other(format!(
            "unknown digest source {other:?}"
        ))),
    }
}

/// One calendar entry, read over Calendar's own HTTP contract.
///
/// Comms already reads Trips this way for its evaluation context
/// (`travel.rs`); one more bounded cross-capability read follows that precedent
/// instead of opening a second capability's database schema. The entry's own
/// description and notes are the source — a calendar entry is c1 by
/// construction, so this never reaches a remote target.
fn calendar_entry_text(cfg: &Config, id: &str) -> Result<Option<SourceText>> {
    let base = cfg.calendar_context.base_url.trim_end_matches('/');
    let http = sjel_http::client(
        sjel_http::Purpose::new("comms-digest"),
        std::time::Duration::from_millis(cfg.calendar_context.timeout_ms),
    )?;
    let url = format!("{base}/api/content/calendar/{id}");
    let request = sjel_server::InboundAuth::with_loopback_auth(http.get(&url), &url);
    let response = request.send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(crate::CommsError::Other(format!(
            "calendar read failed with HTTP {}",
            response.status()
        )));
    }
    let item: serde_json::Value = response.json()?;
    let field = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mut text = field("title");
    for key in ["summary", "content"] {
        let value = field(key);
        if !value.is_empty() {
            text.push_str("\n\n");
            text.push_str(&value);
        }
    }
    Ok(Some(SourceText {
        text,
        data_class: item
            .get("data_class")
            .and_then(|class| class.get("value"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("c1")
            .to_string(),
    }))
}

/// Produce and store the digest for one item.
///
/// Returns the stored row, or `None` when the item does not exist. A failure
/// class is stored rather than raised: a model that was down is a fact about the
/// row, and the reader shows it.
pub fn generate(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    directive: &Directive,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    // The press ladder: light rung when the source fits it, strong local rung
    // otherwise. Unattended passes do not come through here — see
    // [`refresh_pending`] and `crate::quiet`.
    let role = role_for(cfg, directive, gathered.text.chars().count());
    write_digest(store, cfg, source, id, directive, &gathered, role).map(Some)
}

/// Run one resolved role over gathered text and store what came back.
///
/// Split from [`generate`] so the unattended pass can hand in the rung it is
/// allowed to use instead of re-deriving it. The role arrives resolved rather
/// than as a name: this function must not be able to pick a different one than
/// the caller's policy decided.
fn write_digest(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    directive: &Directive,
    gathered: &SourceText,
    role: Option<sjel_inference::ResolvedRole>,
) -> Result<StoredDigest> {
    let previous = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let chars = gathered.text.chars().count();
    let source_chars = chars as i64;
    let shape = directive.shape_for(chars);
    // The stored producer names the role that actually ran. Deriving it
    // separately from `summarization_role` would label a light-model digest as
    // the strong model's work, and provenance that lies is worse than
    // provenance that is missing.
    let producer = role
        .as_ref()
        .map(|role| summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION))
        .unwrap_or_else(|| "unconfigured".into());

    // `None` is the class refusal, and it is checked before the target is
    // built: no prompt is assembled, no local model is woken, no request is
    // made. `libs/summarize`'s own `Reach` cannot express this — it decides how
    // far a payload may travel, and the answer here is "nowhere at all".
    let outcome = gathered.local_prompt_allowed().then(|| {
        if let Some(role) = &role {
            if let Err(reason) = crate::quiet::runtime_admission(role, chars, shape.max_tokens()) {
                return Outcome::PolicyDeferred(reason.to_string());
            }
        }
        summarize::digest(
            role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
            &gathered.text,
            directive,
            reach_for(gathered, role.as_ref()),
        )
    });

    if let Some(Outcome::PolicyDeferred(reason)) = &outcome {
        return defer_digest(store, source, id, directive, gathered, &producer, reason);
    }

    let mut redactions: Vec<RedactionFinding> = Vec::new();
    let text = match &outcome {
        Some(Outcome::Ok(text)) if gathered.redact_before_persistence() => {
            redact_review_field(Some(text), &mut redactions)
        }
        Some(Outcome::Ok(text)) => Some(text.clone()),
        _ => None,
    };

    // A retryable failure accumulates; anything else starts the count over,
    // because a success or a verdict says the previous failures are no longer
    // the state of this row. A class refusal is a verdict: waiting changes
    // nothing about it.
    let attempts = match (&outcome, &previous) {
        (Some(outcome), Some(previous)) if outcome.retryable() => {
            previous.attempts.saturating_add(1)
        }
        (Some(outcome), None) if outcome.retryable() => 1,
        _ => 0,
    };

    // The diagram and the chart are separate presses and survive a regenerated
    // digest — but not a class refusal. `clear_derived_output` below is what
    // actually clears them, because `upsert_content_digest` writes neither
    // column. These four fields are therefore ignored on write; they are set to
    // the same answer so the struct handed to the store does not describe a row
    // the store is about to contradict.
    let carried = match &outcome {
        Some(_) => previous.as_ref(),
        None => None,
    };

    let stored = StoredDigest {
        source: source.to_string(),
        item_id: id.to_string(),
        text,
        state: outcome
            .as_ref()
            .map_or(LOCAL_REFUSED, Outcome::state)
            .to_string(),
        shape: shape.as_str().to_string(),
        depth: directive.depth.as_str().to_string(),
        focus: directive.focus_text(),
        producer,
        source_chars,
        redactions: redactions
            .iter()
            .map(|finding| finding.count)
            .sum::<usize>() as i32,
        attempts,
        last_error: outcome
            .as_ref()
            .and_then(Outcome::error_detail)
            .map(str::to_string),
        diagram: carried.and_then(|row| row.diagram.clone()),
        diagram_state: carried.and_then(|row| row.diagram_state.clone()),
        diagram_error: carried.and_then(|row| row.diagram_error.clone()),
        chart: carried.and_then(|row| row.chart.clone()),
        chart_state: carried.and_then(|row| row.chart_state.clone()),
        chart_error: carried.and_then(|row| row.chart_error.clone()),
        generated_at: String::new(),
    };
    store
        .upsert_content_digest(&stored)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    if outcome.is_none() {
        clear_derived_output(store, cfg, source, id)?;
    }
    read_back(store, source, id)
}

/// A temporary policy never replaces existing work or its retry ledger.
/// A first-time deferral stays selectable by the ordinary pending drain.
fn defer_digest(
    store: &Store,
    source: &str,
    id: &str,
    directive: &Directive,
    gathered: &SourceText,
    producer: &str,
    reason: &str,
) -> Result<StoredDigest> {
    if store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
        .is_none()
    {
        store
            .upsert_content_digest(&StoredDigest {
                source: source.into(),
                item_id: id.into(),
                text: None,
                state: "policy_deferred".into(),
                shape: directive
                    .shape_for(gathered.text.chars().count())
                    .as_str()
                    .into(),
                depth: directive.depth.as_str().into(),
                focus: directive.focus_text(),
                producer: producer.into(),
                source_chars: gathered.text.chars().count() as i64,
                redactions: 0,
                attempts: 0,
                last_error: Some(reason.into()),
                diagram: None,
                diagram_state: None,
                diagram_error: None,
                chart: None,
                chart_state: None,
                chart_error: None,
                generated_at: String::new(),
            })
            .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    }
    Err(crate::CommsError::Other(reason.into()))
}

/// Drop the diagram and the chart beside a digest that was refused for its
/// class.
///
/// A separate pair of statements because [`Store::upsert_content_digest`] does
/// not write those columns at all — that is how a diagram survives a
/// regenerated digest, and it is why the fields on [`StoredDigest`] cannot
/// clear them. Both were written by a model from this item's own text, so an
/// item escalated to `c3` after it was diagrammed must not keep a Mermaid
/// diagram and an extracted table on its page under a digest row that says
/// nothing was sent to any model.
///
/// Writes exactly what [`generate_diagram`] and [`generate_chart`] write when
/// they refuse: the field cleared, the state `local_refused`, no error. They
/// already got this right for their own press; this is the path an escalation
/// actually travels.
fn clear_derived_output(store: &Store, cfg: &Config, source: &str, id: &str) -> Result<()> {
    store
        .update_content_diagram(
            source,
            id,
            None,
            LOCAL_REFUSED,
            None,
            &diagram_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into()),
        )
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    store
        .update_content_chart(
            source,
            id,
            None,
            LOCAL_REFUSED,
            None,
            &chart_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into()),
        )
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    Ok(())
}

/// The row that was just written, read back so the caller sees the stored
/// `generated_at` rather than the empty placeholder above.
fn read_back(store: &Store, source: &str, id: &str) -> Result<StoredDigest> {
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
        .ok_or_else(|| {
            crate::CommsError::Other(
                "the digest row was not there immediately after being written".into(),
            )
        })
}

/// Produce and store a Mermaid diagram beside an existing digest.
///
/// The digest is the input when there is one — it is the model's own compressed
/// reading of the item, and a diagram of a forty-page paper drawn from the
/// paper's first 15,000 characters is a diagram of its introduction. Falls back
/// to the source text when no digest exists yet.
pub fn generate_diagram(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    let existing = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let input = existing
        .as_ref()
        .and_then(|row| row.text.clone())
        .unwrap_or_else(|| gathered.text.clone());

    // Resolved once, so the verdict is asked about the role that will actually
    // serve the request. Deriving the target and the verdict from two separate
    // lookups is how a gate ends up answering about a different endpoint than
    // the one the payload goes to.
    let role = cfg.summarization_role();
    // Same refusal as `write_digest`, because this is the same stored text
    // asked of the same model (T3). The input is the digest when one exists,
    // and for a refused class there is none — so this would send the c3 source
    // itself.
    if gathered.local_prompt_allowed() {
        if let Some(role) = &role {
            crate::quiet::runtime_admission(role, input.chars().count(), 700)
                .map_err(|e| crate::CommsError::Other(e.to_string()))?;
        }
    }
    let outcome = gathered.local_prompt_allowed().then(|| {
        summarize::diagram(
            role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
            &input,
            reach_for(&gathered, role.as_ref()),
        )
    });
    if let Some(Outcome::PolicyDeferred(reason)) = &outcome {
        return Err(crate::CommsError::Other(reason.clone()));
    }
    let producer = diagram_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into());

    // A diagram hangs off a digest row, so an item digested for the first time
    // by this press needs one to exist. Generating the digest first is also the
    // better diagram: see the note above.
    if existing.is_none() {
        generate(store, cfg, source, id, &Directive::default())?;
    }

    let (diagram, error) = match &outcome {
        Some(Outcome::Ok(diagram)) => (Some(diagram.as_str()), None),
        Some(other) => (None, other.error_detail()),
        None => (None, None),
    };
    let updated = store
        .update_content_diagram(
            source,
            id,
            diagram,
            outcome.as_ref().map_or(LOCAL_REFUSED, Outcome::state),
            error,
            &producer,
        )
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    if updated == 0 {
        return Ok(None);
    }
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// Extract and store a chartable table for one item.
///
/// The **source text** is the input, never the digest: the digest is prose the
/// model already wrote, so verifying numbers against it would only prove the
/// model agrees with itself. Verification has to run against what the source
/// actually said.
pub fn generate_chart(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
) -> Result<Option<StoredDigest>> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(None);
    };
    let existing = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;

    let role = cfg.summarization_role();
    // The source text is this path's input by design, so a refused class is
    // refused here most plainly of all (T3).
    if gathered.local_prompt_allowed() {
        if let Some(role) = &role {
            crate::quiet::runtime_admission(role, gathered.text.chars().count(), 900)
                .map_err(|e| crate::CommsError::Other(e.to_string()))?;
        }
    }
    let outcome = gathered.local_prompt_allowed().then(|| {
        summarize::chart::chart(
            role.as_ref().map(|role| to_target(cfg, role)).as_ref(),
            &gathered.text,
            reach_for(&gathered, role.as_ref()),
        )
    });
    if let Some(Outcome::PolicyDeferred(reason)) = &outcome {
        return Err(crate::CommsError::Other(reason.clone()));
    }
    let producer = chart_producer_revision(cfg).unwrap_or_else(|| "unconfigured".into());

    // A chart hangs off a digest row, so an item charted before it was digested
    // needs one to exist.
    if existing.is_none() {
        generate(store, cfg, source, id, &Directive::default())?;
    }

    let (chart, error) = match &outcome {
        Some(Outcome::Ok(chart)) => (Some(chart.as_str()), None),
        Some(other) => (None, other.error_detail()),
        None => (None, None),
    };
    let updated = store
        .update_content_chart(
            source,
            id,
            chart,
            outcome.as_ref().map_or(LOCAL_REFUSED, Outcome::state),
            error,
            &producer,
        )
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    if updated == 0 {
        return Ok(None);
    }
    store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// The bounded automatic pass: digest the items of one source that still need
/// one. Returns how many rows were written.
///
/// Bounded and explicit rather than timer-driven. For mail this reads message
/// bodies, and a background job that quietly pulls every body out of a mailbox
/// is not something a machine should start doing on its own.
/// An error plus everything underneath it.
///
/// `postgres::Error` Displays as the bare string "db error" and keeps the
/// statement, the SQLSTATE and the message on its `source()` chain, so
/// `error.to_string()` at an error-type boundary throws away the only part
/// worth reading. A drain that logs "digest drain: db error" every fifteen
/// minutes reports that something is wrong and nothing about what.
fn detail(error: &(dyn std::error::Error + 'static)) -> String {
    let mut out = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        out.push_str(": ");
        out.push_str(&next.to_string());
        cause = next.source();
    }
    out
}

pub fn refresh_pending(
    store: &Store,
    cfg: &Config,
    source: &str,
    limit: i64,
) -> Result<DrainReport> {
    let producers = producer_revisions(cfg);
    if producers.is_empty() {
        return Ok(DrainReport::default());
    }
    let ids = store
        .items_needing_digest(
            source,
            &producers,
            &unattended_producer_revisions(cfg),
            MAX_ATTEMPTS,
            limit.clamp(1, 500),
        )
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let directive = Directive::new(Depth::Standard, []);
    let mut report = DrainReport::default();
    for id in ids {
        match refresh_one(store, cfg, source, &id, &directive) {
            Ok(Pass::Written(state)) => {
                report.written += 1;
                // The streak the alert threshold counts. Recorded here rather
                // than in `write_digest` because this is the *unattended* pass:
                // an operator pressing Regenerate on a busy machine is told so
                // on the spot and is not an alert condition. `OverWindow` never
                // reaches this arm, which is the point — a rung this pass
                // declined to use is not the local server failing.
                match state.as_str() {
                    "capacity_aborted" => {
                        if let Some(streak) =
                            crate::capacity::record_failure(store, cfg.capacity_alert_after)
                        {
                            eprintln!(
                                "digest drain: ALERT — {streak} consecutive capacity aborts from \
                                 the local inference server; digests are not being written"
                            );
                        }
                    }
                    "generated" => crate::capacity::record_success(store),
                    _ => {}
                }
            }
            // No light rung on this machine, so no unattended pass will get
            // anywhere for the next hundred items either.
            Ok(Pass::Unconfigured) => {
                report.unconfigured = true;
                break;
            }
            Ok(Pass::OverWindow) => report.over_window += 1,
            Ok(Pass::CloudDigested) => report.cloud_digested += 1,
            Ok(Pass::CloudFailed) => report.cloud_failed += 1,
            Ok(Pass::Missing) => {}
            Err(_) => {}
        }
    }
    Ok(report)
}

/// What one unattended pass did across the items it looked at.
///
/// Counted rather than summed into "wrote N": on this machine most of the
/// backlog is over the on-device window, so a pass that writes nothing and one
/// where nothing needed doing look identical from a single number — which is
/// exactly the silence the drain logging exists to end.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DrainReport {
    /// Digest rows written by the light local rung.
    pub written: usize,
    /// Items too long for it, recorded as such and left for a press.
    pub over_window: usize,
    /// Long `c0` items digested by a cloud provider on this pass.
    pub cloud_digested: usize,
    /// Long `c0` items whose cloud attempt failed. The honest count, kept
    /// separate from `over_window` so a provider outage is not read as a
    /// windowing decision.
    pub cloud_failed: usize,
    /// This machine has no light role, so the pass stopped early.
    pub unconfigured: bool,
}

/// What happened to one item on the unattended pass.
enum Pass {
    /// A digest row was written by the light rung, carrying this state.
    Written(String),
    /// Over the light window, recorded and left alone. No model was called.
    OverWindow,
    CloudDigested,
    CloudFailed,
    Unconfigured,
    Missing,
}

/// One item, through the quiet lane.
///
/// The whole of C20 and C21 as a single decision, in the one place that makes
/// it: the light rung when the source fits it, the cloud door when it does not
/// and the item is positively `c0`, and a recorded skip otherwise. Nothing
/// here can reach the strong local model — `crate::quiet::rung` has no branch
/// that returns it.
fn refresh_one(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    directive: &Directive,
) -> Result<Pass> {
    let Some(gathered) = source_text(store, cfg, source, id)? else {
        return Ok(Pass::Missing);
    };
    let chars = gathered.text.chars().count();
    let shape = directive.shape_for(chars);
    if gathered.local_prompt_allowed() {
        if let Some(role) = cfg.light_summarization_role() {
            role.runtime_admission()
                .map_err(|e| crate::CommsError::Other(e.to_string()))?;
        }
    }
    match crate::quiet::rung(&cfg.inference, chars, shape.max_tokens()) {
        crate::quiet::Rung::Unconfigured => Ok(Pass::Unconfigured),
        crate::quiet::Rung::Light(role) => {
            let row = write_digest(store, cfg, source, id, directive, &gathered, Some(*role))?;
            Ok(Pass::Written(row.state))
        }
        // A class verdict outranks a window verdict, so it is asked first.
        // `over_window` below writes `skipped_over_window` for every source that
        // is not the feed — every mail thread, which is where `c3` almost
        // entirely lives — and that state's dashboard text tells the reader the
        // skip is theirs to override by pressing for more detail. The press
        // lands in `generate`, which refuses: nothing leaks, but the stored
        // reason names the wrong cause and the button cannot work. `write_digest`
        // records the refusal the item actually got (T3).
        crate::quiet::Rung::OverWindow if !gathered.local_prompt_allowed() => {
            // The light rung's role, the same one `skip_over_window` names, so
            // the row counts as current and the queue stops returning it. No
            // prompt is built from it: `write_digest` asks the class before it
            // resolves anything.
            let row = write_digest(
                store,
                cfg,
                source,
                id,
                directive,
                &gathered,
                cfg.light_summarization_role(),
            )?;
            Ok(Pass::Written(row.state))
        }
        crate::quiet::Rung::OverWindow => over_window(store, cfg, source, id, shape),
    }
}

/// A source no unattended local rung can hold.
///
/// For a `c0` feed item this is the cloud door: one job on the existing
/// ledger, dispatched immediately, budget and retry cap unchanged. For anything
/// else — every `c1` and `c2` item, every source that is not the feed — it is a
/// recorded verdict and nothing else.
///
/// `c3` and every class from outside the vocabulary never arrive here:
/// [`refresh_one`] answers them with the class refusal first, because "no local
/// rung is big enough" is not why that item got no digest.
///
/// The verdict is written down rather than left implicit because the queue is
/// `ORDER BY created_at DESC LIMIT 25`: on this machine 120 of 190 items with a
/// transcript are over the on-device window, so a pass that silently skipped
/// them would hand the same twenty-five back every fifteen minutes and never
/// reach anything it could actually digest.
fn over_window(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    shape: summarize::Shape,
) -> Result<Pass> {
    if source != "feed" {
        sjel_runtime::require(sjel_runtime::Category::OtherLocalModels)
            .map_err(|e| crate::CommsError::Other(e.to_string()))?;
        skip_over_window(store, cfg, source, id, shape)?;
        return Ok(Pass::OverWindow);
    }
    let Some(item) = store
        .get_feed(id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?
    else {
        return Ok(Pass::Missing);
    };
    match crate::cloud_run::enqueue_digest_job(store, cfg, &item) {
        Ok(queued) => match crate::cloud_run::run_job(store, cfg, &queued.job_id) {
            Ok(_) => {
                eprintln!(
                    "digest drain: cloud digest for {id} via {}",
                    queued.provider_role
                );
                Ok(Pass::CloudDigested)
            }
            // Every way the dispatch can fail to produce a digest, not only
            // the one where the roster ran out: a job already at its five-call
            // cap, a role that stopped being reviewed, a claim another request
            // won. Recorded on the digest row rather than left to the job
            // table, because the reader asks the digest row — and because
            // without it this item is selected again on every single pass,
            // forever, ahead of items the drain could actually digest.
            Err(error) => {
                eprintln!("digest drain: cloud digest for {id} failed: {error}");
                store_cloud_failure(
                    store,
                    source,
                    id,
                    &cloud_producer(cfg, &queued.provider_role),
                    &error,
                );
                Ok(Pass::CloudFailed)
            }
        },
        // The class or the machine says there is no cloud lane for this item at
        // all, which will still be true in fifteen minutes. Record the skip.
        Err(
            crate::cloud_run::DigestNotQueued::ClassNotCleared { .. }
            | crate::cloud_run::DigestNotQueued::LocalOnlyRefused,
        ) => {
            skip_over_window(store, cfg, source, id, shape)?;
            Ok(Pass::OverWindow)
        }
        // Budget spent, credential missing, billing lapsed, store unreachable:
        // all transient. Left untouched so the next pass, or tomorrow's budget,
        // picks it up. Public items are a small, bounded set, so leaving them in
        // the queue does not starve it.
        Err(_) => Ok(Pass::OverWindow),
    }
}

/// Record that no unattended rung could hold this source.
///
/// Producer is the light rung's, so the row counts as current and the queue
/// stops returning it. A press writes over it with the full ladder.
fn skip_over_window(
    store: &Store,
    cfg: &Config,
    source: &str,
    id: &str,
    shape: summarize::Shape,
) -> Result<()> {
    let Some(role) = cfg.light_summarization_role() else {
        return Ok(());
    };
    let previous = store
        .content_digest(source, id)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))?;
    let stored = StoredDigest {
        source: source.to_string(),
        item_id: id.to_string(),
        text: None,
        state: SKIPPED_OVER_WINDOW.to_string(),
        shape: shape.as_str().to_string(),
        depth: Depth::Standard.as_str().to_string(),
        focus: String::new(),
        producer: summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION),
        source_chars: previous.as_ref().map(|row| row.source_chars).unwrap_or(0),
        redactions: 0,
        attempts: 0,
        last_error: None,
        diagram: previous.as_ref().and_then(|row| row.diagram.clone()),
        diagram_state: previous.as_ref().and_then(|row| row.diagram_state.clone()),
        diagram_error: previous.as_ref().and_then(|row| row.diagram_error.clone()),
        chart: previous.as_ref().and_then(|row| row.chart.clone()),
        chart_state: previous.as_ref().and_then(|row| row.chart_state.clone()),
        chart_error: previous.as_ref().and_then(|row| row.chart_error.clone()),
        generated_at: String::new(),
    };
    store
        .upsert_content_digest(&stored)
        .map_err(|error| crate::CommsError::Other(detail(error.as_ref())))
}

/// The verdict for a source no unattended rung can hold. Terminal: a later pass
/// on the same machine would reach the same conclusion, and only a press or a
/// different light role changes the answer.
pub const SKIPPED_OVER_WINDOW: &str = "skipped_over_window";

/// A cloud digest attempt that reached no provider or came back an error.
/// Retryable — see `store::RETRYABLE_DIGEST_STATES` — so the existing backoff
/// and three-attempt cap apply without a second ledger.
pub const CLOUD_ERROR: &str = "cloud_error";

/// The class refuses every prompt, local model included: `c3`, and any value
/// from outside the vocabulary (T3). The row exists and says why, rather than
/// being absent — an item with no digest row reads as "not digested yet", and
/// this one never will be.
///
/// Terminal, and deliberately not in `store::RETRYABLE_DIGEST_STATES`: it is a
/// verdict about the item, the same way `remote_refused` is, and no later pass
/// reaches a different answer. Named as the local counterpart of that state,
/// because it is the same refusal one step closer in.
pub const LOCAL_REFUSED: &str = "local_refused";

/// The producer string a cloud-written digest carries.
///
/// Same shape as every other producer — backend, model, prompt revision — so
/// `producer_revisions` can hold it and the reader can see at a glance which
/// provider wrote what. "Honest" here means it names Cloudflare and the Llama
/// build, not "cloud".
pub fn cloud_producer_revision(role: &sjel_inference::ResolvedRole) -> String {
    summarize::producer(&role.cache_key(), summarize::DIGEST_PROMPT_REVISION)
}

/// Store the digest a cloud provider produced for a queued job.
///
/// Called from `cloud_run::run_job` before the attempt is marked succeeded: a
/// completed attempt with no digest row behind it is a call that was paid for
/// and cannot be read.
pub fn store_cloud_digest(
    store: &Store,
    job: &crate::store::CloudDispatchJob,
    role: &sjel_inference::ResolvedRole,
    text: &str,
    shape: summarize::Shape,
) -> std::result::Result<(), String> {
    let previous = store
        .content_digest(&job.source, &job.item_id)
        .map_err(|error| detail(error.as_ref()))?;
    let stored = StoredDigest {
        source: job.source.clone(),
        item_id: job.item_id.clone(),
        text: Some(text.to_string()),
        state: "generated".into(),
        shape: shape.as_str().to_string(),
        depth: Depth::Standard.as_str().to_string(),
        focus: String::new(),
        producer: cloud_producer_revision(role),
        // The reviewed document is what the provider read, so it is what this
        // digest was made from. Recording the raw transcript length instead
        // would describe a text nobody sent.
        source_chars: job.document.chars().count() as i64,
        redactions: 0,
        attempts: 0,
        last_error: None,
        diagram: previous.as_ref().and_then(|row| row.diagram.clone()),
        diagram_state: previous.as_ref().and_then(|row| row.diagram_state.clone()),
        diagram_error: previous.as_ref().and_then(|row| row.diagram_error.clone()),
        chart: previous.as_ref().and_then(|row| row.chart.clone()),
        chart_state: previous.as_ref().and_then(|row| row.chart_state.clone()),
        chart_error: previous.as_ref().and_then(|row| row.chart_error.clone()),
        generated_at: String::new(),
    };
    store
        .upsert_content_digest(&stored)
        .map_err(|error| detail(error.as_ref()))
}

/// The producer string a queued cloud role would write, for a job that never
/// got far enough to have one. Falls back to the role name, which is still a
/// truthful answer to "who was this asked of".
fn cloud_producer(cfg: &Config, provider_role: &str) -> String {
    match cfg.inference.role(provider_role) {
        Some(role) => cloud_producer_revision(&role),
        None => summarize::producer(provider_role, summarize::DIGEST_PROMPT_REVISION),
    }
}

/// Record that a cloud digest did not arrive.
///
/// Best effort: failing to write the note must not turn a reported dispatch
/// failure into a store error nobody can act on. The producer is the cloud
/// role's own, so the row counts as current against `producer_revisions` and
/// the `cloud_error` backoff is what schedules the next attempt — a producer
/// nothing recognises would put this row back in the queue on every pass.
pub fn store_cloud_failure(
    store: &Store,
    source: &str,
    item_id: &str,
    producer: &str,
    detail: &str,
) {
    if sjel_runtime::is_deferred(detail) {
        return;
    }
    let previous = store.content_digest(source, item_id).ok().flatten();
    let stored = StoredDigest {
        source: source.to_string(),
        item_id: item_id.to_string(),
        text: None,
        state: CLOUD_ERROR.into(),
        shape: previous
            .as_ref()
            .map(|row| row.shape.clone())
            .unwrap_or_else(|| summarize::Shape::Sectioned.as_str().to_string()),
        depth: Depth::Standard.as_str().to_string(),
        focus: String::new(),
        producer: producer.to_string(),
        source_chars: previous.as_ref().map(|row| row.source_chars).unwrap_or(0),
        redactions: 0,
        attempts: previous
            .as_ref()
            .map(|row| row.attempts.saturating_add(1))
            .unwrap_or(1),
        last_error: Some(detail.chars().take(500).collect()),
        diagram: previous.as_ref().and_then(|row| row.diagram.clone()),
        diagram_state: previous.as_ref().and_then(|row| row.diagram_state.clone()),
        diagram_error: previous.as_ref().and_then(|row| row.diagram_error.clone()),
        chart: previous.as_ref().and_then(|row| row.chart.clone()),
        chart_state: previous.as_ref().and_then(|row| row.chart_state.clone()),
        chart_error: previous.as_ref().and_then(|row| row.chart_error.clone()),
        generated_at: String::new(),
    };
    let _ = store.upsert_content_digest(&stored);
}

/// The wire shape, built from the stored row.
pub fn to_contract(row: &StoredDigest) -> content_item::Digest {
    content_item::Digest {
        text: row.text.clone(),
        state: row.state.clone(),
        shape: row.shape.clone(),
        depth: row.depth.clone(),
        focus: Directive::parse_focus(&row.focus),
        producer: row.producer.clone(),
        source_chars: row.source_chars,
        redactions: row.redactions,
        attempts: row.attempts,
        last_error: row.last_error.clone(),
        diagram: row.diagram.clone(),
        diagram_state: row.diagram_state.clone(),
        diagram_error: row.diagram_error.clone(),
        // Re-parsed rather than passed through as a string: the reader indexes
        // into it, and a JSON blob delivered as a quoted string is a second
        // parse every consumer has to remember to do.
        chart: row
            .chart
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        chart_state: row.chart_state.clone(),
        chart_error: row.chart_error.clone(),
        generated_at: row.generated_at.clone(),
    }
}

/// Parse a reader's refine request. An unknown depth is rejected rather than
/// defaulted: a button that silently does something else than it says is worse
/// than one that reports it could not.
pub fn parse_directive(
    depth: Option<&str>,
    focus: Vec<String>,
) -> std::result::Result<Directive, String> {
    let depth = match depth {
        None => Depth::Standard,
        Some(value) => Depth::parse(value)
            .ok_or_else(|| format!("depth must be \"standard\" or \"detailed\", not {value:?}"))?,
    };
    Ok(Directive::new(depth, focus))
}

/// A short, reader-facing account of why there is no digest text.
pub fn state_explanation(state: &str, shape: &str) -> &'static str {
    match state {
        "generated" => "",
        "skipped_short" if shape == "none" => {
            "Too short to be worth a digest — the source is already the summary."
        }
        "skipped_short" => "Nothing to digest.",
        "remote_refused" => "This item is not public and the configured model is not local.",
        // Not "the model failed" and not "try again": no model was asked. A
        // Secret item enters no prompt at all, and the reader has no override
        // for it short of reclassifying the item.
        LOCAL_REFUSED => "Secret content enters no prompt, so nothing was sent to any model.",
        // Deliberately says what to do about it. This state is not a failure of
        // anything — it is the automatic pass declining to wake the big local
        // model, which is policy, and the reader is the one who can override it.
        SKIPPED_OVER_WINDOW => {
            "Too long for the on-device model. Press Regenerate to run it on the larger local \
             model."
        }
        CLOUD_ERROR => "The cloud provider could not produce a digest. It will retry.",
        "policy_deferred" => {
            "Deferred by the runtime profile. Work resumes when the profile allows it."
        }
        "unconfigured" => "No summarization model is configured on this machine.",
        "timeout" => "The local model did not answer in time.",
        // Deliberately about the machine, not the model. The server took this
        // request and ran out of memory part-way through it; the same item
        // digests fine once something stops holding the GPU. Saying "the model
        // answered with nothing" here sent a reader looking at the model, the
        // prompt and the transcript, none of which were involved.
        "capacity_aborted" => "The local server ran out of memory part-way through. It will retry.",
        "empty_response" => "The local model answered with nothing.",
        _ => "The local model could not be reached.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_depth_is_rejected_rather_than_defaulted() {
        assert!(parse_directive(Some("deeper"), Vec::new()).is_err());
        assert!(parse_directive(Some(""), Vec::new()).is_err());
        assert_eq!(
            parse_directive(None, Vec::new()).unwrap().depth,
            Depth::Standard
        );
        assert_eq!(
            parse_directive(Some("detailed"), vec!["cost".into()])
                .unwrap()
                .focus,
            vec!["cost".to_string()]
        );
    }

    fn gathered(class: &str) -> SourceText {
        SourceText {
            text: String::new(),
            data_class: class.into(),
        }
    }

    /// The one rule that keeps mail off a cloud endpoint: mail is never `c0`,
    /// and nothing but `c0` may be sent verbatim. A digest sends the source
    /// text verbatim, so this is the whole question.
    #[test]
    fn nothing_but_a_c0_item_may_reach_a_cloud_tier() {
        for tier in [None, Some("public"), Some("pseudonymized_personal")] {
            for class in ["c1", "c2", "c3", "something-new"] {
                assert_eq!(
                    gathered(class).reach(tier),
                    Reach::LoopbackOnly,
                    "{class} must stay local against tier {tier:?}"
                );
            }
        }
        assert_eq!(gathered("c0").reach(Some("public")), Reach::CloudCleared);
        assert_eq!(
            gathered("c0").reach(Some("pseudonymized_personal")),
            Reach::CloudCleared
        );
    }

    /// A role with no reviewed cloud policy has no `cloud_data_tier`, and an
    /// undeclared tier admits nothing — including c0 content. The previous rule
    /// was laxer: it read the class alone and would have sent a c0 item to any
    /// https endpoint somebody pointed the summarization role at.
    #[test]
    fn an_endpoint_with_no_declared_tier_receives_nothing() {
        for class in ["c0", "c1", "c2", "c3"] {
            assert_eq!(gathered(class).reach(None), Reach::LoopbackOnly);
        }
    }

    /// No role resolved means no request; the verdict still defaults closed.
    #[test]
    fn an_unresolved_role_is_loopback_only() {
        assert_eq!(reach_for(&gathered("c0"), None), Reach::LoopbackOnly);
    }

    #[test]
    fn only_local_only_content_is_redacted_before_the_digest_is_stored() {
        for class in ["c2", "c3"] {
            assert!(SourceText {
                text: String::new(),
                data_class: class.into(),
            }
            .redact_before_persistence());
        }
        for class in ["c0", "c1"] {
            assert!(!SourceText {
                text: String::new(),
                data_class: class.into(),
            }
            .redact_before_persistence());
        }
    }

    /// A skip is a claim about the source, so the reader has to be able to say
    /// which one it was without re-deriving anything.
    #[test]
    fn every_non_generated_state_explains_itself() {
        for state in [
            "skipped_short",
            "remote_refused",
            "unconfigured",
            "timeout",
            "empty_response",
            "http_error",
            "model_error",
        ] {
            assert!(
                !state_explanation(state, "none").is_empty(),
                "{state} has no explanation"
            );
        }
        assert!(state_explanation("generated", "brief").is_empty());
    }

    /// T3, at the class level: `c3` and every unrecognized value are refused a
    /// prompt, and `c0`, `c1`, `c2` still get one. c2 matters as much as c3
    /// here — Others is local-only, not local-forbidden, and a gate that
    /// refused it would take the operator's own mail away from them.
    #[test]
    fn only_secret_and_unknown_classes_are_refused_a_local_prompt() {
        for refused in ["c3", "vault", "personal", "private", "c4", ""] {
            assert!(
                !gathered(refused).local_prompt_allowed(),
                "{refused} was admitted to a local prompt"
            );
        }
        for allowed in ["c0", "c1", "c2"] {
            assert!(
                gathered(allowed).local_prompt_allowed(),
                "{allowed} lost its local processing"
            );
        }
    }

    /// The refusal has to be readable, or the row is a digest that failed for
    /// no stated reason.
    #[test]
    fn the_local_refusal_explains_itself_and_is_not_retried() {
        assert!(!state_explanation(LOCAL_REFUSED, "none").is_empty());
        assert!(
            !crate::store::RETRYABLE_DIGEST_STATES.contains(&LOCAL_REFUSED),
            "a class verdict must not be queued for another attempt"
        );
    }

    /// The redactor the digest path reuses is the sweep's own. This is the
    /// property that matters: a model asked to summarize a one-time-code mail
    /// will quote the code, and the digest must not be where it gets published.
    #[test]
    fn a_private_digest_cannot_republish_a_one_time_code() {
        let model_answer = "- Your verification code is 448215\n- It expires in 10 minutes";
        let mut findings: Vec<RedactionFinding> = Vec::new();
        let stored = redact_review_field(Some(model_answer), &mut findings).unwrap();
        assert!(!stored.contains("448215"), "the code survived: {stored}");
        assert!(
            findings.iter().map(|finding| finding.count).sum::<usize>() > 0,
            "a redaction that removes something must be counted"
        );
    }

    /// The tests that need a real store. Own module because the module name is
    /// what CI splits on — see `cloud_run`'s.
    #[cfg(test)]
    mod db_tests {
        use super::*;

        /// A local role pointed at a port nothing listens on.
        ///
        /// The refusal has to be told apart from a failure, and this is what
        /// tells them apart: if the gate let a prompt through, the request would
        /// reach a closed port and the row would read `http_error`. Asserting
        /// `local_refused` therefore asserts that no request was attempted, with
        /// no network mock to be wrong about.
        fn unreachable_local_role() -> sjel_inference::InferenceConfig {
            serde_json::from_value(serde_json::json!({
                "backends": {
                    "omlx": { "api": "openai", "base_url": "http://127.0.0.1:9/v1" },
                },
                "roles": {
                    "summarization": { "backend": "omlx", "model": "a-local-model" },
                },
            }))
            .expect("the probe config is well formed")
        }

        fn stored_feed(store: &Store, data_class: &str) -> String {
            let mut item = crate::store::FeedItem::new(
                &format!("https://example.com/axon-t3-{data_class}"),
                "news",
                "article",
            );
            item.title = Some("A stored item".into());
            // Over the ladder's floor, so a `skipped_short` verdict cannot be
            // mistaken for the refusal or for the request the control expects.
            item.transcript = Some("paragraph ".repeat(400));
            item.data_class = data_class.into();
            store.upsert_feed(&item).expect("the source row is stored");
            item.id
        }

        /// T3's exit test on the local side: a `c3` source produces a digest row
        /// that says it was refused, with no text, no attempt on the ledger and
        /// no request made.
        #[test]
        fn a_c3_source_produces_no_local_digest_call() {
            let store = crate::store::db_tests::open_test_store("digest_c3_refusal");
            let cfg = Config::with_inference(unreachable_local_role());
            let id = stored_feed(&store, "c3");

            let row = generate(&store, &cfg, "feed", &id, &Directive::default())
                .expect("the digest path answers")
                .expect("the row exists, so a digest row is written");
            assert_eq!(row.state, LOCAL_REFUSED);
            assert_eq!(row.text, None);
            assert_eq!(row.attempts, 0, "a verdict is not a failed attempt");
            assert_eq!(row.redactions, 0, "nothing was produced to redact");
        }

        /// The same call on a `c2` item, which is the control: it must reach the
        /// model and fail against the closed port. Without this the test above
        /// would pass on a gate that refused everything.
        #[test]
        fn a_c2_source_still_reaches_the_local_model() {
            let store = crate::store::db_tests::open_test_store("digest_c2_local");
            let cfg = Config::with_inference(unreachable_local_role());
            let id = stored_feed(&store, "c2");

            let row = generate(&store, &cfg, "feed", &id, &Directive::default())
                .expect("the digest path answers")
                .expect("the row exists, so a digest row is written");
            assert_ne!(
                row.state, LOCAL_REFUSED,
                "c2 is local-only, not local-forbidden"
            );
            assert!(
                crate::store::RETRYABLE_DIGEST_STATES.contains(&row.state.as_str()),
                "a request was made and the closed port answered: got {}",
                row.state
            );
        }

        /// A light rung too small to hold anything, so every source is
        /// `Rung::OverWindow` and the drain's over-window branch is the one
        /// under test.
        fn over_window_light_role() -> sjel_inference::InferenceConfig {
            serde_json::from_value(serde_json::json!({
                "backends": {
                    "omlx": { "api": "openai", "base_url": "http://127.0.0.1:9/v1" },
                },
                "roles": {
                    "summarization_light": {
                        "backend": "omlx",
                        "model": "a-small-local-model",
                        "max_input_tokens": 256,
                    },
                },
            }))
            .expect("the probe config is well formed")
        }

        /// One stored mail thread, with no Gmail credentials to read.
        ///
        /// `source_text`'s mail branch tries a token first and falls back to the
        /// stored snippet when it cannot get one, so pointing `google_env_path`
        /// at a file that does not exist is what keeps this test off the network
        /// and out of a mailbox. `database_path` is redirected for the same
        /// reason: it is only ever read to site the local gate's lock file, and
        /// a test must not put one beside the deployed store.
        fn stored_mail(store: &Store, id: &str, data_class: &str) -> (Config, String) {
            let mut cfg = Config::with_inference(over_window_light_role());
            cfg.google_env_path =
                std::env::temp_dir().join(format!("axon-absent-google-env-{}", std::process::id()));
            cfg.database_path = crate::store::db_tests::test_database("digest_mail_drain_gate");
            let mut item = crate::store::db_tests::mk_triage(id, "aktiv");
            item.snippet = Some("paragraph ".repeat(400));
            item.data_class = data_class.into();
            store.upsert_triage(&item).expect("the mail row is stored");
            (cfg, item.id)
        }

        /// The class verdict outranks the window verdict on the drain's own
        /// path, which is where `c3` actually lives: almost all of it is mail,
        /// and every mail thread takes `over_window`'s non-feed branch.
        ///
        /// `skipped_over_window` is not a harmless stand-in here. Its dashboard
        /// text tells the reader the skip is theirs to override by pressing for
        /// more detail, and that press refuses — so the row would invite an
        /// action that cannot work and name the wrong reason while doing it.
        #[test]
        fn an_over_window_c3_mail_thread_records_the_refusal_not_the_window() {
            let store = crate::store::db_tests::open_test_store("digest_c3_mail_drain");
            let (cfg, id) = stored_mail(&store, "thread:t3-drain-c3", "c3");

            refresh_one(&store, &cfg, "mail", &id, &Directive::default())
                .expect("the drain answers");
            let row = store
                .content_digest("mail", &id)
                .expect("the digest row reads back")
                .expect("the drain wrote a row");
            assert_eq!(
                row.state, LOCAL_REFUSED,
                "an over-window c3 thread was recorded as a window skip"
            );
            assert_eq!(row.text, None);
            assert_eq!(row.attempts, 0, "a verdict is not a failed attempt");
        }

        /// The control, and the reason the test above says something: the same
        /// thread at `c2` is genuinely over the window, and the window verdict
        /// is still what gets stored. Without this, a drain that answered
        /// `local_refused` for everything would pass.
        #[test]
        fn an_over_window_c2_mail_thread_still_records_the_window() {
            let store = crate::store::db_tests::open_test_store("digest_c2_mail_drain");
            let (cfg, id) = stored_mail(&store, "thread:t3-drain-c2", "c2");

            refresh_one(&store, &cfg, "mail", &id, &Directive::default())
                .expect("the drain answers");
            let row = store
                .content_digest("mail", &id)
                .expect("the digest row reads back")
                .expect("the drain wrote a row");
            assert_eq!(row.state, SKIPPED_OVER_WINDOW);
        }

        const A_DIAGRAM: &str = "graph TD; A-->B;";
        const A_CHART: &str = "| year | value |\n| 2026 | 1 |";

        /// The row a reader would be looking at: a digest, plus the two derived
        /// fields, each written through its own press's statement — which is the
        /// only way they reach those columns, since `upsert_content_digest` does
        /// not write them.
        fn digested_and_diagrammed(store: &Store, id: &str) {
            store
                .upsert_content_digest(&StoredDigest {
                    source: "feed".into(),
                    item_id: id.into(),
                    text: Some("- The item said something worth keeping.".into()),
                    state: "ok".into(),
                    shape: "brief".into(),
                    depth: Depth::Standard.as_str().into(),
                    focus: String::new(),
                    producer: "a-model:a-revision".into(),
                    source_chars: 4_000,
                    redactions: 0,
                    attempts: 0,
                    last_error: None,
                    diagram: None,
                    diagram_state: None,
                    diagram_error: None,
                    chart: None,
                    chart_state: None,
                    chart_error: None,
                    generated_at: String::new(),
                })
                .expect("the digest row is stored");
            store
                .update_content_diagram("feed", id, Some(A_DIAGRAM), "ok", None, "a-model:diagram")
                .expect("the diagram is attached");
            store
                .update_content_chart("feed", id, Some(A_CHART), "ok", None, "a-model:chart")
                .expect("the chart is attached");
            let row = store
                .content_digest("feed", id)
                .expect("the row reads back")
                .expect("the row exists");
            assert_eq!(row.diagram.as_deref(), Some(A_DIAGRAM));
            assert_eq!(row.chart.as_deref(), Some(A_CHART));
        }

        /// A class refusal takes the model output with it.
        ///
        /// The diagram and the chart survive a *regenerated* digest on purpose —
        /// they are separate presses, and the digest upsert does not touch their
        /// columns. They must not survive a *refused* one: both were written by
        /// a model from this item's text, so a row escalated to `c3` after it
        /// was diagrammed would otherwise keep a Mermaid diagram and an
        /// extracted table on the item page under a digest that says nothing was
        /// sent to any model.
        #[test]
        fn an_escalation_clears_the_digest_the_diagram_and_the_chart_together() {
            let store = crate::store::db_tests::open_test_store("digest_escalation_clears");
            let cfg = Config::with_inference(unreachable_local_role());
            let id = stored_feed(&store, "c1");
            digested_and_diagrammed(&store, &id);

            store
                .set_feed_data_class(&id, "c3", Some("The item carries a credential."))
                .expect("an escalation is admitted");

            let row = generate(&store, &cfg, "feed", &id, &Directive::default())
                .expect("the digest path answers")
                .expect("the row exists, so a digest row is written");
            assert_eq!(row.state, LOCAL_REFUSED);
            assert_eq!(row.text, None, "the digest survived the escalation");
            assert_eq!(row.diagram, None, "the diagram survived the escalation");
            assert_eq!(row.chart, None, "the chart survived the escalation");
            // The same verdict `generate_diagram` and `generate_chart` write on
            // their own refusal, so the fields say why they are empty rather
            // than reading as never generated.
            assert_eq!(row.diagram_state.as_deref(), Some(LOCAL_REFUSED));
            assert_eq!(row.chart_state.as_deref(), Some(LOCAL_REFUSED));
        }

        /// The other half: an ordinary regeneration still keeps both. Without
        /// this, clearing them unconditionally would pass the test above and
        /// throw away a figure the operator asked for every time the drain ran.
        #[test]
        fn an_ordinary_regeneration_still_keeps_the_diagram_and_the_chart() {
            let store = crate::store::db_tests::open_test_store("digest_regeneration_keeps");
            let mut cfg = Config::with_inference(unreachable_local_role());
            // This one reaches the model, so it sites a local-gate lock file.
            // Kept in the temp directory rather than beside the deployed store.
            cfg.database_path = crate::store::db_tests::test_database("digest_regeneration_gate");
            let id = stored_feed(&store, "c2");
            digested_and_diagrammed(&store, &id);

            let row = generate(&store, &cfg, "feed", &id, &Directive::default())
                .expect("the digest path answers")
                .expect("the row exists, so a digest row is written");
            assert_ne!(row.state, LOCAL_REFUSED, "c2 is local-only, not forbidden");
            assert_eq!(row.diagram.as_deref(), Some(A_DIAGRAM));
            assert_eq!(row.chart.as_deref(), Some(A_CHART));
        }
    }
}
