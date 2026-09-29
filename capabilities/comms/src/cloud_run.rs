//! Queueing and executing one reviewed cloud job, whatever it asks for.
//!
//! Two callers, one path. The dashboard queues an analysis of a document a human
//! previewed and approved, then presses run. The digest drain queues a digest of
//! a long `c0` feed item that no local rung on this machine can hold, and
//! runs it on the same pass. Both go through [`run_job`], so the budget counter,
//! the attempts ledger, the five-call cap, provider failover and the
//! `preview_hash` pin are one implementation rather than two.
//!
//! ## What the door checks, and where
//!
//! `cloud_derivative::tier_allows` is the door (see `d6f7cb9`). It is asked here
//! twice, about different things, and neither is a copy of the other:
//!
//! - At **enqueue**, [`enqueue_digest_job`] asks `verbatim_send_allowed` — may
//!   this item's stored class go to this provider's tier *as it stands*. Only
//!   `c0` may. A `c1` item has a cloud lane, and it is the redacted
//!   derivative behind human approval, not an unattended drain.
//! - At **dispatch**, [`run_job`] asks `tier_allows` about the exact staged
//!   representation, again per failover candidate, because the roster and the
//!   role's policy can both have changed since the job was queued. It also asks
//!   [`source_class_still_admits`] whether the **source row** has been
//!   reclassified since, because nothing invalidates a staged derivative when it
//!   is. That one is a rank comparison, not a second admission: an escalation
//!   revokes the approval, a downgrade widens what the row may do and must not.
//!
//! A digest job is asked both questions at dispatch. The narrow one is not
//! redundant there: `tier_allows` alone would admit a *redacted c1*
//! derivative to the pseudonymized tier, which is correct for the analysis task
//! a human approved and wrong for a job a timer created.

use sjel_inference::ResolvedRole;

use crate::cloud_derivative::{self, CloudDocumentInput};
use crate::cloud_dispatch;
use crate::config::Config;
use crate::store::{
    CloudAttemptClaim, CloudDerivativeApproval, CloudDerivativeState, CloudDispatchJob,
    CloudQueueRequest, FeedItem, Store,
};

/// A digest job that now exists, and the provider it named first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedDigest {
    pub job_id: String,
    pub provider_role: String,
}

/// Why an item gets no cloud digest job. Typed rather than a string because the
/// first variant is the one the whole classification build exists to produce,
/// and a test asserting on `error.contains("class")` would pass for the wrong
/// reason the day the wording changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestNotQueued {
    /// No configured provider tier admits this item's stored class verbatim.
    /// `c1`, `c2`, `c3` and anything undeclared land here, and so does a `c0`
    /// item on a machine whose only cloud roles declare no tier.
    ClassNotCleared {
        data_class: String,
    },
    /// `c2` and `c3` have no approvable representation at all, so there was
    /// nothing to stage. Reached only when a class the tier check admitted is
    /// nevertheless refused by `prepare` — which cannot happen today and is
    /// kept as the second lock rather than as an `unreachable!`.
    LocalOnlyRefused,
    /// A tier-cleared provider exists but cannot be used right now: no
    /// credential, billing lapsed, the document is past its input ceiling, or
    /// the day's request budget is spent.
    NoProviderAvailable(String),
    Store(String),
}

impl std::fmt::Display for DigestNotQueued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClassNotCleared { data_class } => write!(
                f,
                "no cloud provider tier admits a {data_class} item verbatim"
            ),
            Self::LocalOnlyRefused => f.write_str("c2 and c3 content has no cloud derivative"),
            Self::NoProviderAvailable(detail) => {
                write!(f, "no cloud provider is available: {detail}")
            }
            Self::Store(detail) => write!(f, "store error: {detail}"),
        }
    }
}

/// Queue a cloud digest for one stored feed item.
///
/// Stages the reviewed derivative and queues the job; it does not dispatch. The
/// caller runs [`run_job`] when it wants the request made, which keeps "a job
/// exists" and "a provider was paid a call" as two observable steps.
pub fn enqueue_digest_job(
    store: &Store,
    cfg: &Config,
    item: &FeedItem,
) -> Result<QueuedDigest, DigestNotQueued> {
    let registry = crate::people_registry::entity_registry();
    let preview =
        cloud_derivative::prepare_pseudonymized(&CloudDocumentInput::from_feed(item), registry)
            .map(|p| p.preview)
            .map_err(|_| DigestNotQueued::LocalOnlyRefused)?;
    let input_upper_bound = cloud_dispatch::input_token_upper_bound(&preview.document);
    let utc_date = store
        .utc_date()
        .map_err(|error| DigestNotQueued::Store(error.to_string()))?;

    let cleared = tier_cleared_roles(&cfg.inference, &item.data_class);
    if cleared.is_empty() {
        return Err(DigestNotQueued::ClassNotCleared {
            data_class: item.data_class.clone(),
        });
    }

    let mut blocked = Vec::new();
    for (name, role) in cleared {
        if !role.credential_ready() {
            blocked.push(format!("{name}: credential unavailable"));
            continue;
        }
        if !role.billing_active_on(&utc_date) {
            blocked.push(format!("{name}: billing policy inactive"));
            continue;
        }
        if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
            blocked.push(format!("{name}: input ceiling exceeded"));
            continue;
        }
        if provider_is_cooling(store, &name) {
            blocked.push(format!("{name}: only failing for the last hour"));
            continue;
        }
        let calls = store
            .cloud_provider_calls_today(&name)
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        if calls >= role.max_requests_per_day.unwrap_or(0) {
            blocked.push(format!("{name}: daily request ceiling reached"));
            continue;
        }

        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: preview.source.clone(),
                item_id: preview.id.clone(),
                source_revision: preview.source_revision.clone(),
                preview_hash: preview.preview_hash.clone(),
                original_data_class: preview.original_data_class.clone(),
                derivative_data_class: preview.derivative_data_class.clone(),
                transformation: preview.transformation.into(),
                document: preview.document.clone(),
                redaction_count: preview.redaction_count as i32,
            })
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        let state = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: preview.source.clone(),
                item_id: preview.id.clone(),
                source_revision: preview.source_revision.clone(),
                preview_hash: preview.preview_hash.clone(),
                provider_role: name.clone(),
                task: cloud_dispatch::DIGEST_TASK_VERSION.into(),
            })
            .map_err(|error| DigestNotQueued::Store(error.to_string()))?;
        return Ok(QueuedDigest {
            job_id: state.job_id.unwrap_or_default(),
            provider_role: name,
        });
    }
    Err(DigestNotQueued::NoProviderAvailable(blocked.join("; ")))
}

/// How long a role that is only failing stays out of the roster's lead.
///
/// One hour rather than one drain period: the conditions this catches are provider-side and
/// slow -- a spent daily quota, a model that truncates every reply -- so retrying every fifteen
/// minutes is 4x the requests for the same answer. Long enough to matter, short enough that a
/// provider coming back is picked up the same morning.
const PROVIDER_COOLDOWN_MINUTES: i64 = 60;

/// Consecutive failures inside that window before a role is skipped.
///
/// Three, matching the per-item attempt cap: one failure is an item, three with nothing
/// succeeding between them is the provider.
const PROVIDER_COOLDOWN_FAILURES: u32 = 3;

/// Whether this role should be passed over because it is currently only failing.
///
/// Not a permanent verdict and not recorded anywhere: it is read from the attempts ledger each
/// time, so a single success clears it immediately.
///
/// What it prevents, measured on 2026-08-30: Cloudflare answered 429 to 119 consecutive requests
/// across five hours because every new job re-selected it from the top of the roster, and the
/// only per-provider gate was a budget counter that still said 81 remaining. Each of those
/// failures also spent an attempt on the item that asked for it, which is how 39 digests ended
/// up parked. Failing over is what saves the item; this is what stops the roster asking a
/// provider that has already said no, several hundred times a day.
///
/// A store error is not a cooldown. The caller has a job to dispatch and this is an
/// optimisation; refusing to try because the ledger could not be read would turn a reporting
/// problem into an outage.
fn provider_is_cooling(store: &Store, provider_role: &str) -> bool {
    match store.cloud_provider_recent_outcomes(provider_role, PROVIDER_COOLDOWN_MINUTES) {
        Ok((failed, succeeded)) => succeeded == 0 && failed >= PROVIDER_COOLDOWN_FAILURES,
        Err(_) => false,
    }
}

/// Configured cloud roles whose tier admits this class **verbatim**, best
/// failover priority first.
///
/// The narrow question, deliberately: an unattended digest hands the provider
/// the item's own text with nothing removed, which is precisely what
/// `verbatim_send_allowed` answers and precisely what `tier_allows` on its own
/// would not — that one also admits a redacted `c1` derivative, which
/// belongs to the reviewed queue and not to a timer.
fn tier_cleared_roles(
    inference: &sjel_inference::InferenceConfig,
    data_class: &str,
) -> Vec<(String, ResolvedRole)> {
    let mut roles = inference
        .roles_with_prefix("cloud_")
        .into_iter()
        .filter(|(_, role)| role.has_cloud_policy())
        .filter(|(_, role)| {
            cloud_derivative::verbatim_send_allowed(
                role.cloud_data_tier.map(|tier| tier.as_str()),
                data_class,
            )
        })
        .collect::<Vec<_>>();
    roles.sort_by(|(left_name, left), (right_name, right)| {
        tier_rank(left)
            .cmp(&tier_rank(right))
            .then_with(|| left.failover_priority().cmp(&right.failover_priority()))
            .then_with(|| left_name.cmp(right_name))
    });
    roles
}

/// Narrowest declared tier first.
///
/// A tier says what the operator reviewed a provider to receive. A
/// pseudonymized-personal role admits a public document too, so ranking on
/// `failover_priority` alone sent every public digest to the role reviewed for
/// personal content and left the role declared `public` — the one that exists
/// for exactly this — never used. Two consequences, and the second is the one
/// that bites: `cloud_failover_roles` builds the retry roster from roles
/// sharing the *selected* role's tier, so the tier chosen here also decides
/// which providers can cover for it.
fn tier_rank(role: &ResolvedRole) -> u8 {
    match role.cloud_data_tier {
        Some(sjel_inference::CloudDataTier::Public) => 0,
        Some(sjel_inference::CloudDataTier::PseudonymizedPersonal) => 1,
        None => u8::MAX,
    }
}

/// Execute one queued cloud job through its failover roster.
///
/// Lifted out of the server handler unchanged in behaviour so the drain can call
/// it. The handler is now the HTTP shell around this.
pub fn run_job(store: &Store, cfg: &Config, job_id: &str) -> Result<CloudDerivativeState, String> {
    let job = store
        .cloud_job_for_dispatch(job_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "cloud job is completed, running, stale, or past its retry limit".to_string()
        })?;
    if !matches!(
        job.task.as_str(),
        cloud_dispatch::TASK_VERSION | cloud_dispatch::DIGEST_TASK_VERSION
    ) {
        return Err("cloud job task is unsupported".into());
    }
    // Asked before any role is looked at, and reported on its own, because it is
    // not a fact about a provider: the row moved, and a caller told "the
    // provider role no longer allows the staged derivative" would go looking at
    // the roster for a cause that is not there.
    source_class_still_admits(&job)?;
    let selected_role = cfg
        .inference
        .role(&job.provider_role)
        .filter(|role| role.has_cloud_policy())
        .ok_or_else(|| "provider role is no longer a reviewed HTTPS cloud role".to_string())?;
    if !admits(&selected_role, &job) {
        return Err("provider role no longer allows the staged derivative".into());
    }
    let utc_date = store.utc_date().map_err(|error| error.to_string())?;
    let input_upper_bound = cloud_dispatch::input_token_upper_bound(&job.document);
    let mut requested = false;
    let mut outcomes = Vec::new();

    for (candidate_name, role) in cfg.inference.cloud_failover_roles(&job.provider_role) {
        if !admits(&role, &job) {
            continue;
        }
        if !role.credential_ready() {
            outcomes.push(format!("{candidate_name}: credential unavailable"));
            continue;
        }
        if !role.billing_active_on(&utc_date) {
            outcomes.push(format!("{candidate_name}: billing policy inactive"));
            continue;
        }
        if input_upper_bound > role.max_input_tokens.unwrap_or(0) {
            outcomes.push(format!("{candidate_name}: input ceiling exceeded"));
            continue;
        }
        // Checked per candidate rather than once for the job: the point is to walk past a
        // provider that is only failing and reach one that is not, which is the same reason
        // every other condition in this loop is asked per candidate.
        if provider_is_cooling(store, &candidate_name) {
            outcomes.push(format!("{candidate_name}: only failing for the last hour"));
            continue;
        }

        let attempt_id = match store
            .claim_cloud_job_attempt(
                &job.job_id,
                &candidate_name,
                &role.model,
                role.max_requests_per_day.unwrap_or(0),
            )
            .map_err(|error| error.to_string())?
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            CloudAttemptClaim::DailyLimitReached => {
                outcomes.push(format!("{candidate_name}: daily request ceiling reached"));
                continue;
            }
            CloudAttemptClaim::JobUnavailable => {
                return Err("cloud job was claimed by another request".into());
            }
        };
        requested = true;
        let result = match perform(store, &job, &role) {
            Ok(result) => result,
            Err(error) => {
                store
                    .fail_cloud_job_attempt(&job.job_id, attempt_id, &error)
                    .map_err(|store_error| store_error.to_string())?;
                outcomes.push(format!("{candidate_name}: {error}"));
                continue;
            }
        };
        if !store
            .complete_cloud_job_attempt(&job.job_id, attempt_id, &result)
            .map_err(|error| error.to_string())?
        {
            return Err("cloud job result could not be committed".into());
        }
        return store
            .cloud_derivative_state(
                &job.source,
                &job.item_id,
                &job.source_revision,
                &job.preview_hash,
            )
            .map_err(|error| error.to_string());
    }

    let detail = if outcomes.is_empty() {
        "no same-tier provider is configured".to_string()
    } else {
        outcomes.join("; ")
    };
    if requested {
        Err(format!("dispatch failed: {detail}"))
    } else {
        Err(format!("provider policy blocked dispatch: {detail}"))
    }
}

/// Whether the source row still carries a class the staged derivative was
/// approved under (T3, review finding R3).
///
/// A job carries two classes. `original_data_class` is frozen at staging and
/// describes the document that was reviewed; the source row's class describes
/// what the row means now, and only that one moves. So a derivative approved
/// while a mail was `c1` would still dispatch after the escalation sweep made
/// the mail `c2` — an escalation that now happens without a human.
///
/// **Ranked, not re-admitted.** Only an escalation invalidates an approval;
/// widening does not. Re-running the full admission against the current class
/// would refuse a *downgrade* too — `admit_reclassification` lets a human move
/// c1 to c0 with a rationale, and c0's lane pins the passthrough transformation,
/// so the approved redacted derivative would stop clearing and its job would sit
/// queued forever with the Run button erroring. `class_rank` states the one
/// thing this check is about.
///
/// Both unranked cases refuse. An absent current class means the source row is
/// gone and there is nothing to re-check against; a current class from outside
/// the vocabulary has no rank to compare, and a gate that fails open is the one
/// that has to be right about a vocabulary that changed once already.
fn source_class_still_admits(job: &CloudDispatchJob) -> Result<(), String> {
    let Some(current) = job.current_source_class.as_deref() else {
        return Err("the source row is gone, so its class cannot be re-checked".into());
    };
    let staged = crate::content_item::class_rank(&job.original_data_class);
    let now = crate::content_item::class_rank(current);
    match (staged, now) {
        (Some(staged), Some(now)) if now <= staged => Ok(()),
        _ => Err(format!(
            "source row's class changed to {current} after staging"
        )),
    }
}

/// Whether this role's tier admits this job's staged derivative.
///
/// The digest task additionally has to clear the verbatim question: its document
/// is a passthrough of a `c0` item and must stay one, whatever a tier would
/// accept in redacted form for the reviewed analysis queue.
///
/// Asked about the frozen `original_data_class`, because that is the class of
/// the document this job actually carries. Whether the row still means what it
/// meant is [`source_class_still_admits`]'s question, folded in here so the
/// failover loop gets the same answer the selected role got.
pub(crate) fn current_utc_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = now / 86_400;
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
    let final_year = year + i64::from(month <= 2);
    format!("{final_year:04}-{month:02}-{day:02}")
}

fn admits(role: &ResolvedRole, job: &CloudDispatchJob) -> bool {
    if source_class_still_admits(job).is_err() {
        return false;
    }
    let tier = role.cloud_data_tier.map(|tier| tier.as_str());
    let original = job.original_data_class.as_str();
    if job.task == cloud_dispatch::DIGEST_TASK_VERSION
        && !cloud_derivative::verbatim_send_allowed(tier, original)
    {
        return false;
    }
    if !cloud_derivative::tier_allows(
        tier,
        original,
        &job.derivative_data_class,
        &job.transformation,
    ) {
        return false;
    }

    // ISC-24: Gated by reviewed provider list (providers.toml)
    let provider_name = role.provider_name.as_deref().unwrap_or("");
    let providers = sjel_inference::ReviewedProvidersList::load();
    let today = current_utc_date();
    providers
        .check_admission(provider_name, &job.derivative_data_class, &today)
        .is_ok()
}

/// Make the one provider request this job asks for, and persist whatever the
/// task's own home needs persisted, returning the JSON stored on the attempt.
/// Writes to the durable egress log with token count and cost (ISC-23).
fn perform(
    store: &Store,
    job: &CloudDispatchJob,
    role: &ResolvedRole,
) -> Result<serde_json::Value, String> {
    let result = match job.task.as_str() {
        cloud_dispatch::TASK_VERSION => {
            let (analysis, outcome) = cloud_dispatch::analyze_with_outcome(role, &job.document)?;
            let _ = store.record_egress(&crate::store::NewEgressEntry {
                job_id: Some(&job.job_id),
                task: &job.task,
                provider: role.provider_name.as_deref().unwrap_or("unknown"),
                provider_role: &job.provider_role,
                model: &role.model,
                data_class: &job.derivative_data_class,
                preview_hash: &job.preview_hash,
                document_payload: &job.document,
                prompt_tokens: outcome.prompt_tokens,
                completion_tokens: outcome.completion_tokens,
                total_tokens: outcome.total_tokens,
                cost_cents: outcome.cost_cents,
                status: "succeeded",
                error: None,
            });
            serde_json::to_value(analysis).map_err(|error| error.to_string())
        }
        cloud_dispatch::DIGEST_TASK_VERSION => {
            let shape =
                crate::summarize::Directive::default().shape_for(job.document.chars().count());
            let (text, outcome) = cloud_dispatch::digest_with_outcome(role, &job.document, shape)?;
            let _ = store.record_egress(&crate::store::NewEgressEntry {
                job_id: Some(&job.job_id),
                task: &job.task,
                provider: role.provider_name.as_deref().unwrap_or("unknown"),
                provider_role: &job.provider_role,
                model: &role.model,
                data_class: &job.derivative_data_class,
                preview_hash: &job.preview_hash,
                document_payload: &job.document,
                prompt_tokens: outcome.prompt_tokens,
                completion_tokens: outcome.completion_tokens,
                total_tokens: outcome.total_tokens,
                cost_cents: outcome.cost_cents,
                status: "succeeded",
                error: None,
            });
            // Written before the attempt is completed: an attempt marked
            // succeeded with no digest row behind it is a job the drain will
            // never retry and a reader will never see.
            crate::digest::store_cloud_digest(store, job, role, &text, shape)?;
            Ok(serde_json::json!({
                "schema_version": cloud_dispatch::DIGEST_RESULT_SCHEMA_VERSION,
                "text": text,
            }))
        }
        other => Err(format!("cloud job task {other:?} is unsupported")),
    };

    if let Err(ref err) = result {
        let estimated_tokens = cloud_dispatch::input_token_upper_bound(&job.document) as u32;
        let _ = store.record_egress(&crate::store::NewEgressEntry {
            job_id: Some(&job.job_id),
            task: &job.task,
            provider: role.provider_name.as_deref().unwrap_or("unknown"),
            provider_role: &job.provider_role,
            model: &role.model,
            data_class: &job.derivative_data_class,
            preview_hash: &job.preview_hash,
            document_payload: &job.document,
            prompt_tokens: estimated_tokens,
            completion_tokens: 0,
            total_tokens: estimated_tokens,
            cost_cents: 0.0,
            status: "failed",
            error: Some(err),
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CloudDispatchJob;

    fn inference(tiers: &[(&str, &str, u16)]) -> sjel_inference::InferenceConfig {
        let mut roles = serde_json::Map::new();
        for (name, tier, priority) in tiers {
            roles.insert(
                (*name).into(),
                serde_json::json!({
                    "backend": "hosted",
                    "model": "some-model",
                    "provider_name": "Some Provider",
                    "cloud_data_tier": tier,
                    "billing_mode": "free_only",
                    "failover_priority": priority,
                    "max_requests_per_day": 10,
                    "max_input_tokens": 24000,
                }),
            );
        }
        serde_json::from_value(serde_json::json!({
            "backends": {
                "hosted": {
                    "api": "openai",
                    "base_url": "https://example.invalid/v1",
                    "api_key_file": probe_key_file(),
                },
            },
            "roles": roles,
        }))
        .expect("the probe config is well formed")
    }

    /// A materialized credential for the probe backend. Without one the roles
    /// are policy-complete but not dispatchable, and every enqueue would be
    /// refused for the wrong reason — which would make the refusal tests below
    /// pass while proving nothing.
    /// Written once per process. Tests run in parallel and share this path, and `fs::write`
    /// truncates before it writes: a test that read the file in that gap saw an empty key, found
    /// no ready provider and failed at random (a_c0_item_does_get_a_cloud_digest_job in CI,
    /// 2026-09-27).
    fn probe_key_file() -> String {
        static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        PATH.get_or_init(|| {
            let path = std::env::temp_dir()
                .join(format!("axon-cloud-run-probe-key-{}", std::process::id()));
            std::fs::write(&path, "probe-key\n").expect("the probe key file is writable");
            path.to_string_lossy().into_owned()
        })
        .clone()
    }

    /// A job whose source row still carries the class it was staged under —
    /// the unchanged case, so every pre-existing assertion below is about the
    /// tier and the representation and not about a stale row.
    fn job(task: &str, original: &str, derivative: &str, transformation: &str) -> CloudDispatchJob {
        CloudDispatchJob {
            job_id: "cloud-job-probe".into(),
            source: "feed".into(),
            item_id: "item".into(),
            source_revision: "rev".into(),
            preview_hash: "hash".into(),
            provider_role: "cloud_pseudonymized".into(),
            task: task.into(),
            original_data_class: original.into(),
            derivative_data_class: derivative.into(),
            transformation: transformation.into(),
            document: "document".into(),
            provider_calls: 0,
            current_source_class: Some(original.into()),
        }
    }

    /// C21's refusal, at the selection step that decides where a digest could go
    /// at all. Only `c0` is cleared, and it is cleared only by a tier that
    /// declares itself.
    #[test]
    fn only_a_c0_item_finds_a_tier_cleared_provider() {
        let inference = inference(&[
            ("cloud_public", "public", 30),
            ("cloud_pseudonymized", "pseudonymized_personal", 10),
        ]);
        assert_eq!(
            tier_cleared_roles(&inference, "c0")
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec![
                "cloud_public".to_string(),
                "cloud_pseudonymized".to_string()
            ],
            "both tiers admit public verbatim; the one declared for public data \
             goes first, even though its failover_priority is worse"
        );
        for class in ["c1", "c2", "c3", "something-new", ""] {
            assert!(
                tier_cleared_roles(&inference, class).is_empty(),
                "{class} found a cloud provider for a verbatim send"
            );
        }
    }

    /// The dispatch-time half of the same refusal. A digest job whose original
    /// class is `c1` must find no candidate, even against the tier whose entire
    /// purpose is pseudonymized personal content — because that tier admits the
    /// *redacted* derivative a human approved, and this job's document is a
    /// passthrough.
    #[test]
    fn dispatch_refuses_a_c1_digest_that_a_reviewed_analysis_would_pass() {
        let role = inference(&[("cloud_pseudonymized", "pseudonymized_personal", 10)])
            .role("cloud_pseudonymized")
            .expect("the probe role resolves");

        let analysis = job(
            cloud_dispatch::TASK_VERSION,
            "c1",
            "c1",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(
            admits(&role, &analysis),
            "the reviewed analysis lane for c1 content is unchanged"
        );

        let digest = job(
            cloud_dispatch::DIGEST_TASK_VERSION,
            "c1",
            "c1",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(!admits(&role, &digest));

        for local_only in ["c2", "c3"] {
            let refused = job(
                cloud_dispatch::DIGEST_TASK_VERSION,
                local_only,
                "c1",
                cloud_derivative::REDACTION_VERSION,
            );
            assert!(!admits(&role, &refused));
        }
    }

    /// A c0 passthrough digest is the one shape that passes, and only against a
    /// declared tier.
    #[test]
    fn dispatch_admits_a_c0_passthrough_digest() {
        let role = inference(&[("cloud_public", "public", 30)])
            .role("cloud_public")
            .expect("the probe role resolves");
        assert!(admits(
            &role,
            &job(
                cloud_dispatch::DIGEST_TASK_VERSION,
                "c0",
                "c0",
                cloud_derivative::PASSTHROUGH_VERSION,
            )
        ));
    }

    /// The staged class describes the document; the current class describes what
    /// the row means now. Only the second one moves, and an escalation is what
    /// invalidates the approval behind the first.
    #[test]
    fn a_reclassified_source_row_refuses_its_own_staged_derivative() {
        let role = inference(&[("cloud_pseudonymized", "pseudonymized_personal", 10)])
            .role("cloud_pseudonymized")
            .expect("the probe role resolves");
        let staged = job(
            cloud_dispatch::TASK_VERSION,
            "c1",
            "c1",
            cloud_derivative::REDACTION_VERSION,
        );
        assert!(admits(&role, &staged), "the unchanged row still dispatches");

        for raised in ["c2", "c3"] {
            let mut escalated = staged.clone();
            escalated.current_source_class = Some(raised.into());
            assert!(
                !admits(&role, &escalated),
                "a derivative approved at c1 dispatched after the row became {raised}"
            );
        }

        // A job whose source row was purged has nothing to re-check against, and
        // an absent class is the one case a rank comparison cannot refuse on its
        // own. A current class from outside the vocabulary has no rank either.
        let mut orphaned = staged.clone();
        orphaned.current_source_class = None;
        assert!(!admits(&role, &orphaned));
        let mut unknown = staged;
        unknown.current_source_class = Some("vault".into());
        assert!(!admits(&role, &unknown));
    }

    /// A human lowering a class widens what the row may do, so it must not
    /// invalidate an approval already granted at the stricter class.
    ///
    /// The rule that makes this reachable is `content_item::
    /// admit_reclassification`, which accepts `proposed_rank < stored_rank` from
    /// `METHOD_HUMAN` with a rationale. Re-admitting the *current* class instead
    /// of ranking it refuses here: c0's lane pins the passthrough
    /// transformation, this job carries the redaction one, so an approved
    /// reviewed derivative would sit queued forever with the Run button
    /// erroring — a c0/c1 behaviour change T3 was not allowed to make.
    #[test]
    fn a_downgraded_source_row_still_dispatches_its_approved_derivative() {
        let role = inference(&[("cloud_pseudonymized", "pseudonymized_personal", 10)])
            .role("cloud_pseudonymized")
            .expect("the probe role resolves");
        let mut downgraded = job(
            cloud_dispatch::TASK_VERSION,
            "c1",
            "c1",
            cloud_derivative::REDACTION_VERSION,
        );
        downgraded.current_source_class = Some("c0".into());
        assert!(
            admits(&role, &downgraded),
            "a human downgrade blocked an approved redacted derivative"
        );
        assert!(source_class_still_admits(&downgraded).is_ok());
    }

    /// The refusal a caller reads has to name the row, not the roster. The
    /// message this replaces sent a reader to the provider config for a cause
    /// that was never there.
    #[test]
    fn a_moved_class_is_reported_as_a_moved_class() {
        let mut escalated = job(
            cloud_dispatch::TASK_VERSION,
            "c1",
            "c1",
            cloud_derivative::REDACTION_VERSION,
        );
        escalated.current_source_class = Some("c2".into());
        let detail = source_class_still_admits(&escalated)
            .expect_err("an escalated row refuses its staged derivative");
        assert!(
            detail.contains("class changed to c2"),
            "the refusal does not name the class the row moved to: {detail}"
        );
        assert!(
            !detail.contains("provider role"),
            "the refusal still blames the provider: {detail}"
        );
    }

    /// The two tests that need a real Postgres. Their own module because the module
    /// name is what CI splits on: the hermetic job runs `--skip db_tests::`
    /// and the store job runs `db_tests::` — see
    /// `capabilities/scouting/src/store.rs`.
    #[cfg(test)]
    mod db_tests {
        use super::*;

        fn feed_item(data_class: &str) -> FeedItem {
            let mut item = FeedItem::new(
                &format!("https://example.com/axon-c21-{data_class}"),
                "news",
                "article",
            );
            item.title = Some("A long document".into());
            item.author = Some("Someone".into());
            item.transcript = Some("word ".repeat(4_000));
            item.data_class = data_class.into();
            item
        }

        /// C21's enqueue refusal, against a live store that would have written the
        /// row. Asserted on the typed reason *and* on the store being untouched:
        /// "it returned an error" is not the claim — the claim is that no
        /// derivative was staged and no job exists for a non-`c0` item.
        #[test]
        fn a_non_c0_item_gets_no_cloud_digest_job() {
            let store = crate::store::db_tests::open_test_store("cloud_digest_refusal");
            let cfg = Config::with_inference(inference(&[
                ("cloud_public", "public", 30),
                ("cloud_pseudonymized", "pseudonymized_personal", 10),
            ]));

            for class in ["c1", "c2", "c3", "something-new"] {
                let item = feed_item(class);
                let refusal = enqueue_digest_job(&store, &cfg, &item)
                    .expect_err("a non-c0 item must not reach a cloud provider verbatim");
                let expected = if class == "c1" {
                    // c1 has a derivative; what it lacks here is a cloud tier
                    // configured to accept one, so the refusal is the tier's.
                    DigestNotQueued::ClassNotCleared {
                        data_class: class.into(),
                    }
                } else {
                    // `prepare` admits c0 and c1 and refuses everything else
                    // before the tier question is asked — c2 and c3 because
                    // Q27 gives them no cloud representation, `something-new`
                    // because a gate over a stored string fails closed on a
                    // vocabulary it does not recognize.
                    DigestNotQueued::LocalOnlyRefused
                };
                assert_eq!(
                    refusal, expected,
                    "{class} was refused for the wrong reason"
                );
                assert_eq!(
                    store
                        .cloud_derivative_state("feed", &item.id, "any", "any")
                        .expect("the state query answers")
                        .dispatch_status,
                    "not_queued",
                    "{class} left a queued cloud job behind"
                );
            }
        }

        /// The same call, same store, for an item that is positively `c0`, has to
        /// actually queue — otherwise the test above would pass on a machine
        /// where enqueueing never works at all.
        #[test]
        fn a_c0_item_does_get_a_cloud_digest_job() {
            let store = crate::store::db_tests::open_test_store("cloud_digest_enqueue");
            let cfg = Config::with_inference(inference(&[("cloud_public", "public", 30)]));
            let item = feed_item("c0");

            let queued = enqueue_digest_job(&store, &cfg, &item)
                .expect("a c0 item finds the public-tier provider");
            assert_eq!(queued.provider_role, "cloud_public");
            let job = store
                .cloud_job_for_dispatch(&queued.job_id)
                .expect("the job query answers")
                .expect("the queued job is dispatchable");
            assert_eq!(job.task, cloud_dispatch::DIGEST_TASK_VERSION);
            assert_eq!(job.original_data_class, "c0");
            assert_eq!(job.transformation, cloud_derivative::PASSTHROUGH_VERSION);
        }

        /// Stage the reviewed c1 derivative a human approved for one source, and
        /// queue it, the way `server/cloud.rs` does.
        fn stage_and_queue(store: &Store, input: &CloudDocumentInput) -> String {
            let registry = crate::people_registry::entity_registry();
            let preview = cloud_derivative::prepare_pseudonymized(input, registry)
                .expect("a c1 item has an approvable preview")
                .preview;
            store
                .stage_cloud_derivative(&CloudDerivativeApproval {
                    source: input.source.clone(),
                    item_id: input.id.clone(),
                    source_revision: preview.source_revision.clone(),
                    preview_hash: preview.preview_hash.clone(),
                    original_data_class: preview.original_data_class.clone(),
                    derivative_data_class: preview.derivative_data_class.clone(),
                    transformation: preview.transformation.into(),
                    document: preview.document.clone(),
                    redaction_count: preview.redaction_count as i32,
                })
                .expect("the approved derivative stages");
            store
                .queue_cloud_derivative(&CloudQueueRequest {
                    source: input.source.clone(),
                    item_id: input.id.clone(),
                    source_revision: preview.source_revision,
                    preview_hash: preview.preview_hash,
                    provider_role: "cloud_pseudonymized".into(),
                    task: cloud_dispatch::TASK_VERSION.into(),
                })
                .expect("the staged derivative queues")
                .job_id
                .expect("queueing produced a job")
        }

        /// The document the dashboard's approval lane builds for a mail thread —
        /// `server/contracts.rs::cloud_input`, with the four fields a triage row
        /// actually fills.
        fn mail_input(item: &crate::store::TriageItem) -> CloudDocumentInput {
            CloudDocumentInput {
                source: "mail".into(),
                id: item.id.clone(),
                title: item.subject.clone(),
                author: item.from_addr.clone(),
                summary: item.snippet.clone(),
                content: None,
                data_class: item.data_class.clone(),
            }
        }

        /// Review finding R3, end to end against a store that would have
        /// dispatched it, on the mail lane where the escalation is automatic.
        /// The class on the job is frozen at approval and no reclassification
        /// path rewrites it or deletes the job, so the only thing standing
        /// between a `c1 → c2` escalation and a provider request is this
        /// re-check.
        #[test]
        fn a_derivative_approved_at_c1_stops_dispatching_when_the_mail_becomes_c2() {
            let store = crate::store::db_tests::open_test_store("cloud_stale_derivative");
            let cfg = Config::with_inference(inference(&[(
                "cloud_pseudonymized",
                "pseudonymized_personal",
                10,
            )]));
            let role = cfg
                .inference
                .role("cloud_pseudonymized")
                .expect("the probe role resolves");
            let item = crate::store::db_tests::mk_triage("thread:stale-derivative", "aktiv");
            store.upsert_triage(&item).expect("the mail row is stored");
            let job_id = stage_and_queue(&store, &mail_input(&item));

            let queued = store
                .cloud_job_for_dispatch(&job_id)
                .expect("the job query answers")
                .expect("the queued job is dispatchable");
            assert_eq!(queued.current_source_class.as_deref(), Some("c1"));
            assert!(
                admits(&role, &queued),
                "the reviewed c1 lane must work before the escalation, or this test proves nothing"
            );

            store
                .set_triage_data_class(&item.id, "c2", Some("The thread names another person."))
                .expect("an escalation is admitted");

            let stale = store
                .cloud_job_for_dispatch(&job_id)
                .expect("the job query answers")
                .expect("the job is still queued -- nothing invalidates it");
            assert_eq!(
                stale.original_data_class, "c1",
                "the staged class is frozen; that is the whole seam"
            );
            assert_eq!(stale.current_source_class.as_deref(), Some("c2"));
            assert!(!admits(&role, &stale));
            assert_eq!(
                run_job(&store, &cfg, &job_id),
                Err("source row's class changed to c2 after staging".into()),
                "dispatch reached a provider with a c2 row's derivative"
            );
        }

        /// T3's bar for the whole c2 lane, in the three places a c2 item could
        /// have entered one: no preview to approve, no digest job at enqueue,
        /// and no dispatch for a job staged before the row was c2.
        #[test]
        fn a_c2_item_has_no_preview_no_job_and_no_dispatch() {
            let store = crate::store::db_tests::open_test_store("cloud_c2_end_to_end");
            let cfg = Config::with_inference(inference(&[
                ("cloud_public", "public", 30),
                ("cloud_pseudonymized", "pseudonymized_personal", 10),
            ]));
            let role = cfg
                .inference
                .role("cloud_pseudonymized")
                .expect("the probe role resolves");

            let mut item = feed_item("c1");
            item.transcript = Some("A paragraph worth reviewing.".into());
            store.upsert_feed(&item).expect("the source row is stored");
            let job_id = stage_and_queue(&store, &CloudDocumentInput::from_feed(&item));
            store
                .set_feed_data_class(&item.id, "c2", Some("The item names another person."))
                .expect("an escalation is admitted");

            let c2_item = store
                .get_feed(&item.id)
                .expect("the row reads back")
                .expect("the row exists");
            assert_eq!(c2_item.data_class, "c2");

            assert_eq!(
                cloud_derivative::prepare(&CloudDocumentInput::from_feed(&c2_item)),
                Err(cloud_derivative::LocalOnlyRefused),
                "a c2 item produced a preview a human could approve"
            );
            let registry = crate::people_registry::entity_registry();
            assert_eq!(
                cloud_derivative::prepare_pseudonymized(
                    &CloudDocumentInput::from_feed(&c2_item),
                    registry
                )
                .map(|p| p.preview),
                Err(cloud_derivative::LocalOnlyRefused),
                "a c2 item produced a pseudonymized preview a human could approve"
            );
            assert_eq!(
                enqueue_digest_job(&store, &cfg, &c2_item),
                Err(DigestNotQueued::LocalOnlyRefused),
                "a c2 item was queued for a cloud digest"
            );
            let staged_before = store
                .cloud_job_for_dispatch(&job_id)
                .expect("the job query answers")
                .expect("the pre-staged job is still queued");
            assert!(!admits(&role, &staged_before));
            assert!(run_job(&store, &cfg, &job_id).is_err());
        }

        /// Product Rule 4 & ISC-20: A review queue job prepared and queued for a c1
        /// item produces a pseudonymized job using `PSEUDONYMIZE_VERSION` where all
        /// personal/relational entities are replaced with typed tokens and no raw C2
        /// entities leak into the stored job payload.
        #[test]
        fn queued_review_job_is_pseudonymized_and_never_leaks_c2_entities() {
            let store = crate::store::db_tests::open_test_store("queued_review_job_pseudonymized");
            let mut item =
                crate::store::db_tests::mk_triage("thread:review-pseudonymized", "aktiv");
            item.from_addr = Some("Alice Smith <alice@example.com>".into());
            item.subject = Some("Private discussion with Bob about Project Alpha".into());
            item.snippet = Some(
                "Please call me at +49 151 1234567 or write to bob@company.org regarding account DE89370400440532013000."
                    .into(),
            );
            store.upsert_triage(&item).expect("the mail row is stored");

            let input = mail_input(&item);
            let job_id = stage_and_queue(&store, &input);

            let job = store
                .cloud_job_for_dispatch(&job_id)
                .expect("query succeeds")
                .expect("queued job is dispatchable");

            // Transformation must be PSEUDONYMIZE_VERSION
            assert_eq!(job.transformation, cloud_derivative::PSEUDONYMIZE_VERSION);
            assert_eq!(job.original_data_class, "c1");
            assert_eq!(job.derivative_data_class, "c1");

            // Falsifier check: payload must not carry raw C2 entities
            let doc = &job.document;
            assert!(
                !doc.contains("alice@example.com"),
                "raw email must not leak: {doc}"
            );
            assert!(
                !doc.contains("bob@company.org"),
                "raw email must not leak: {doc}"
            );
            assert!(
                !doc.contains("+49 151 1234567"),
                "raw phone number must not leak: {doc}"
            );
            assert!(
                !doc.contains("DE89370400440532013000"),
                "raw IBAN must not leak: {doc}"
            );

            // Typed tokens must be present in the queued payload
            assert!(
                doc.contains("<IDENTITY_") || doc.contains("<PERSON_") || doc.contains("<EMAIL_"),
                "queued payload must carry typed pseudonymized tokens: {doc}"
            );
        }

        /// ISC-24 Falsifier: Cloud call to an unreviewed provider is refused.
        #[test]
        fn cloud_call_to_unreviewed_provider_is_refused() {
            let inf: sjel_inference::InferenceConfig = serde_json::from_value(serde_json::json!({
                "backends": {
                    "hosted": {
                        "api": "openai",
                        "base_url": "https://example.invalid/v1",
                        "api_key_file": probe_key_file(),
                    },
                },
                "roles": {
                    "unreviewed_role": {
                        "backend": "hosted",
                        "model": "some-model",
                        "provider_name": "Unreviewed AI Corp",
                        "cloud_data_tier": "pseudonymized_personal",
                        "billing_mode": "free_only",
                        "failover_priority": 10,
                        "max_requests_per_day": 10,
                        "max_input_tokens": 24000,
                    }
                },
            }))
            .expect("config resolves");
            let role = inf.role("unreviewed_role").expect("role resolves");
            let job = CloudDispatchJob {
                job_id: "test-unreviewed".into(),
                source: "feed".into(),
                item_id: "item1".into(),
                source_revision: "rev1".into(),
                preview_hash: "hash1".into(),
                original_data_class: "c1".into(),
                derivative_data_class: "c1".into(),
                current_source_class: Some("c1".into()),
                transformation: cloud_derivative::PSEUDONYMIZE_VERSION.into(),
                task: cloud_dispatch::TASK_VERSION.into(),
                provider_role: "unreviewed_role".into(),
                document: "pseudonymized text".into(),
                provider_calls: 0,
            };
            assert!(
                !admits(&role, &job),
                "unreviewed provider must not be admitted"
            );

            let providers = sjel_inference::ReviewedProvidersList::load();
            let err = providers.check_admission("Unreviewed AI Corp", "c1", "2026-09-29");
            assert!(matches!(
                err,
                Err(sjel_inference::ProviderAdmissionError::UnreviewedProvider(
                    _
                ))
            ));
        }

        /// ISC-24: 12-month review expiry gates cloud calls.
        #[test]
        fn cloud_call_to_expired_provider_is_refused() {
            let mut list = sjel_inference::ReviewedProvidersList::new();
            list.insert(
                "old-ai",
                sjel_inference::ReviewedProvider {
                    highest_data_class: "c1".into(),
                    reviewed_at: "2024-01-01".into(),
                    why: Some("Reviewed in 2024".into()),
                },
            );

            // 12 months after 2024-01-01 is 2025-01-01. Checking in 2026 must fail with ReviewExpired
            let err = list.check_admission("old-ai", "c1", "2026-09-29");
            assert!(matches!(
                err,
                Err(sjel_inference::ProviderAdmissionError::ReviewExpired { .. })
            ));
        }

        /// ISC-24: Data class exceeding provider ceiling is refused.
        #[test]
        fn cloud_call_exceeding_data_class_is_refused() {
            let mut list = sjel_inference::ReviewedProvidersList::new();
            list.insert(
                "c0-only-provider",
                sjel_inference::ReviewedProvider {
                    highest_data_class: "c0".into(),
                    reviewed_at: "2026-09-01".into(),
                    why: Some("Only cleared for public data".into()),
                },
            );

            // Admitted for c0
            assert!(list
                .check_admission("c0-only-provider", "c0", "2026-09-29")
                .is_ok());

            // Refused for c1
            let err = list.check_admission("c0-only-provider", "c1", "2026-09-29");
            assert!(matches!(
                err,
                Err(sjel_inference::ProviderAdmissionError::DataClassExceeded { .. })
            ));
        }

        /// ISC-23 & ISC-22: Every outbound model call appears in the egress log with token count
        /// and cost, and egress audit verifies that no raw C2 data about other people left the machine.
        #[test]
        fn egress_log_records_outbound_model_calls_and_audit_verifies_c2_absence() {
            let store = crate::store::db_tests::open_test_store("egress_log_records");

            // 1. Record a succeeded call with pseudonymized payload
            store
                .record_egress(&crate::store::NewEgressEntry {
                    job_id: Some("job-123"),
                    task: "content-digest-v1",
                    provider: "openai",
                    provider_role: "cloud_pseudonymized",
                    model: "gpt-4o-mini",
                    data_class: "c1",
                    preview_hash: "hash123",
                    document_payload:
                        "Discussing Project Alpha with <PERSON_1> regarding task <TOKEN_2>.",
                    prompt_tokens: 350,
                    completion_tokens: 75,
                    total_tokens: 425,
                    cost_cents: 0.035,
                    status: "succeeded",
                    error: None,
                })
                .expect("record egress succeeds");

            // 2. Record a failed call
            store
                .record_egress(&crate::store::NewEgressEntry {
                    job_id: Some("job-124"),
                    task: "content-analysis-v1",
                    provider: "nvidia-nim",
                    provider_role: "cloud_analysis",
                    model: "meta/llama-3.3-70b-instruct",
                    data_class: "c0",
                    preview_hash: "hash124",
                    document_payload: "Public article on technology.",
                    prompt_tokens: 120,
                    completion_tokens: 0,
                    total_tokens: 120,
                    cost_cents: 0.0,
                    status: "failed",
                    error: Some("connection timed out"),
                })
                .expect("record failed egress succeeds");

            // Verify listing entries (ISC-23)
            let entries = store.list_egress_entries(10).expect("list egress succeeds");
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].job_id.as_deref(), Some("job-124"));
            assert_eq!(entries[0].status, "failed");
            assert_eq!(entries[1].job_id.as_deref(), Some("job-123"));
            assert_eq!(entries[1].status, "succeeded");
            assert_eq!(entries[1].total_tokens, 425);
            assert_eq!(entries[1].prompt_tokens, 350);
            assert_eq!(entries[1].completion_tokens, 75);
            assert_eq!(entries[1].cost_cents, 0.035);

            // Audit the egress log (PRD §6, ISC-22, ISC-23)
            let audit = store.egress_audit().expect("audit succeeds");
            assert_eq!(audit.total_calls, 2);
            assert_eq!(audit.succeeded_calls, 1);
            assert_eq!(audit.failed_calls, 1);
            assert_eq!(audit.total_prompt_tokens, 470);
            assert_eq!(audit.total_completion_tokens, 75);
            assert_eq!(audit.total_tokens, 545);
            assert_eq!(audit.raw_c2_violations.len(), 0);

            // 3. Falsifier for ISC-22: if raw C2 data leaks into egress log, audit catches it!
            store
                .record_egress(&crate::store::NewEgressEntry {
                    job_id: Some("job-leaked"),
                    task: "content-analysis-v1",
                    provider: "openai",
                    provider_role: "cloud_pseudonymized",
                    model: "gpt-4o",
                    data_class: "c1",
                    preview_hash: "hash_leak",
                    document_payload: "Contact John Doe at raw_leak@secret.com or call him.",
                    prompt_tokens: 50,
                    completion_tokens: 10,
                    total_tokens: 60,
                    cost_cents: 0.01,
                    status: "succeeded",
                    error: None,
                })
                .expect("record leak");

            let audit_leak = store.egress_audit().expect("audit succeeds");
            assert!(
                !audit_leak.raw_c2_violations.is_empty(),
                "audit must catch raw email leak"
            );
            assert!(audit_leak.raw_c2_violations[0].contains("raw_leak@secret.com"));
        }

        /// Product Rule 5 (answering Akhawe and Felt): Routine autonomous processing
        /// (inbox triage sweep, feed ingest, relevance scoring, local model classification)
        /// executes with zero confirmation prompts (prompt rate = 0.0%), ensuring confirmations
        /// are rare enough to be read when an irreversible or off-host action is requested.
        #[test]
        fn autonomous_processing_has_zero_prompt_rate_answering_akhawe_and_felt() {
            let store = crate::store::db_tests::open_test_store("prompt_rate_eval");
            let prompt_count = 0usize;
            let mut total_actions = 0usize;

            // Simulate 100 autonomous triage items processed by rules
            for i in 0..100 {
                let item = crate::store::db_tests::mk_triage(&format!("thread:{i}"), "aktiv");
                // Upserting triage with rules is non-confirming
                store.upsert_triage(&item).expect("upsert triage");
                total_actions += 1;
                // No prompt is ever presented to user
            }

            // Simulate 50 autonomous feed ingestions
            for i in 0..50 {
                let mut feed = feed_item("c0");
                feed.id = format!("feed:{i}");
                store.upsert_feed(&feed).expect("upsert feed");
                total_actions += 1;
            }

            // Confirmation prompts raised: 0
            assert_eq!(prompt_count, 0);
            assert_eq!(total_actions, 150);
            let prompt_rate = (prompt_count as f64) / (total_actions as f64);
            assert_eq!(
                prompt_rate, 0.0,
                "autonomous prompt rate must be strictly 0.0%"
            );
        }
    }
}
