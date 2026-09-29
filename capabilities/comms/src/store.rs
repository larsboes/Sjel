//! Persistence under the table prefix `comms` in the one shared SQLite file
//! (PRD Q45), same discipline as scouting's store.rs. rusqlite is a blocking API
//! over that file, which matches reqwest's "blocking" feature -- this crate carries
//! no async runtime outside comms-server.
//!
//! The status-preserving upsert is the load-bearing correctness property on
//! both tables: a human's triage/keeper decision must survive the same item
//! being re-swept or re-ingested. `status` is set only on first INSERT and is
//! deliberately absent from the ON CONFLICT DO UPDATE SET list. See
//! `upsert_preserves_status_across_refetch_*` for the proof.
//!
//! [`Store`] remains the only connection-owning facade. Its inherent methods
//! are grouped under `store/` by the workflow that owns their SQL: migrations,
//! triage/Gmail, cloud/digests, feed, evaluation, capture origins, source run
//! state, and row mapping. Callers therefore keep one stable type without one
//! file becoming the owner of every persistence concern.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::types::ToSql;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sjel_store::QueryAll;

use crate::evaluation::{EvaluationFactor, EvaluationFactorContext, FeedEvaluation};
use crate::provenance::{self, StageProvenance};
use crate::quality::QualityFlag;
use crate::relevance::RelevanceMatch;

pub struct Store {
    /// Shared with every other `Store` in this process on the same database. A
    /// `Store` is now a cheap handle rather than a connection, which is what makes
    /// 43 handlers each opening one acceptable.
    pool: sjel_store::Pool,
    /// Prefixes this capability's tables in the one shared file (PRD Q45):
    /// `comms` here means `comms_feed_items` and its eighteen siblings.
    prefix: String,
}
/// One feed row, reduced to what a re-derivation decides on: which row, which source would
/// produce it, and what it is classified as now. Not a `FeedItem`: reading whole items to
/// compare one column would pull every transcript in the table into memory.
#[derive(Debug, Clone)]
pub struct FeedClassRow {
    pub id: String,
    pub kind: String,
    pub data_class: String,
}

/// A media/news feed item. On write, `day`/`created_at`/`status` are owned by
/// the DB (CURRENT_DATE / now() / default 'new') and ignored; on read they are
/// populated. `transcript` is None in list views and Some in single-item reads.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: String,
    pub stream: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub transcript: Option<String>,
    pub day: String,
    pub created_at: String,
    pub status: String,
    /// Extraction quality: `full` (≥1k chars), `thin` (abstract/card), `none`
    /// (no transcript extracted), `unknown` (legacy, not yet classified).
    pub content_status: String,
    /// What the stored text IS: `full-text` (the document), `abstract` (a
    /// stand-in the source offered instead), `unknown` (legacy). A separate
    /// axis from `content_status`, which answers how much text there is — a
    /// long abstract is `full`/`abstract`, a one-line article `thin`/`full-text`.
    ///
    /// Queryable on purpose (#78): an `Abstract:` prefix inside the text would
    /// have to be parsed back out by every reader, and would be indexed by the
    /// embedder as if the paper had said it.
    pub transcript_source: String,
    /// How many times summarization was attempted and failed.
    pub summary_attempts: i32,
    /// Error class of the last failed summarization attempt, if any.
    pub summary_last_error: Option<String>,
    /// Earliest time the next summarization retry is allowed (exponential
    /// backoff). `None` means immediately eligible.
    pub summary_next_attempt: Option<String>,
    /// Which client handed this content over, or `None` when the server fetched
    /// it itself (#81). A login-gated page and a page the server could have
    /// fetched are otherwise indistinguishable once stored, which matters for
    /// judging why an item's content looks the way it does.
    pub captured_via: Option<String>,
    /// What the extractor emitted, before normalization (#86). Carried on the
    /// item only between `media::fetch` and `upsert_feed`; it lives in its own
    /// table, so reading an item back leaves this `None` unless the caller asks
    /// for it via `get_raw_content`. Retaining it is what lets a normalization
    /// rule change re-run over stored content instead of re-fetching the web.
    pub raw_content: Option<String>,
    pub summary_provenance: Option<StageProvenance>,
    /// What this item is worth protecting: `c0`, `c1`, `c2` or `c3`. Never
    /// absent — an item nobody classified reads back c1, decided by `legacy`,
    /// which is the value the cloud gate is meant to see.
    pub data_class: String,
    pub data_class_rationale: String,
    pub data_classification_method: String,
    pub data_classification_version: String,
}

impl FeedItem {
    /// Build a fresh item for ingest. DB-owned fields are left blank/defaulted,
    /// and the class starts undeclared — c1, method `legacy`.
    ///
    /// Every ingest path goes through here, which is what makes the default
    /// actually default: a page pasted by hand, a link imported from the vault
    /// and a URL captured from a logged-in session all arrive c1 unless
    /// something positively declares otherwise. Only a collector that declares
    /// a class calls [`FeedItem::declare_class`] on top.
    pub fn new(url: &str, stream: &str, kind: &str) -> Self {
        let undeclared = crate::content_item::DataClass::undeclared();
        Self {
            id: feed_id(url),
            stream: stream.to_string(),
            kind: kind.to_string(),
            title: None,
            url: url.to_string(),
            author: None,
            summary: None,
            transcript: None,
            day: String::new(),
            created_at: String::new(),
            status: "new".into(),
            content_status: "unknown".into(),
            transcript_source: "unknown".into(),
            summary_attempts: 0,
            summary_last_error: None,
            summary_next_attempt: None,
            captured_via: None,
            raw_content: None,
            summary_provenance: None,
            data_class: undeclared.value,
            data_class_rationale: undeclared.rationale,
            data_classification_method: undeclared.method,
            data_classification_version: undeclared.version,
        }
    }

    /// Stamp a collector's declaration onto an item it discovered.
    pub fn declare_class(&mut self, classification: &crate::content_item::DataClass) {
        self.data_class = classification.value.clone();
        self.data_class_rationale = classification.rationale.clone();
        self.data_classification_method = classification.method.clone();
        self.data_classification_version = classification.version.clone();
    }
}

/// Enrichment backlog counts for the evaluation status endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct EnrichmentCounts {
    pub pending_summaries: i64,
    pub failed_summaries: i64,
}

/// Content status distribution across all feed items.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentStatusCounts {
    pub full: i64,
    pub thin: i64,
    pub none: i64,
    pub unknown: i64,
}

/// One persisted signal in the Feed review queue, joined with the item facts a
/// reviewer needs. Reasons and evidence are stored rather than reconstructed by
/// the dashboard, so the UI cannot drift from the computation that fired.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QualityReviewRow {
    pub feed_id: String,
    pub title: Option<String>,
    pub url: String,
    pub status: String,
    pub content_status: String,
    pub signal: String,
    pub reason: String,
    pub evidence: String,
    pub derived_at: String,
}

/// What one class write did to a row.
///
/// Two fields because a class write is two decisions, not one: the class the
/// row now carries, and whether the two review fields that class governs had to
/// be narrowed to match it. They are reported together because they are written
/// together — a caller that had to run a second query to learn the second half
/// is a caller that can forget to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassWrite {
    /// The class columns were rewritten. False for a row that is not there, and
    /// for a refresh the escalation guard refused.
    pub changed: bool,
    /// The same transaction removed material from `subject`/`snippet` because
    /// the class the row now holds does not admit it.
    pub narrowed: bool,
}

/// A triage proposal for one inbox thread. On write `status`/`first_seen`/
/// `last_seen` are DB-owned and ignored; on read they are populated.
#[derive(Debug, Clone)]
pub struct TriageItem {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// Gmail internalDate in epoch milliseconds (latest message). Write-side
    /// input; on read this stays None and `internal_date_text` carries the
    /// stored TIMESTAMPTZ instead.
    pub internal_date_ms: Option<i64>,
    /// Read-side text form of the stored `internal_date` TIMESTAMPTZ. None on
    /// write. Surfaced by the server as `internal_date`.
    pub internal_date_text: Option<String>,
    pub stream: String,
    pub rationale: String,
    /// `rules` for the deterministic sweep, `human` after an explicit
    /// dashboard correction. Human corrections survive later sweeps.
    pub classification_method: String,
    pub classification_version: String,
    /// Shared trust class, `c0`-`c3`. `libs/content-item` owns the label each
    /// one is presented under.
    pub data_class: String,
    pub data_class_rationale: String,
    pub data_classification_method: String,
    pub data_classification_version: String,
    pub status: String,
    /// Last Gmail lifecycle action recorded after the Gmail request succeeded.
    pub gmail_action: Option<String>,
    pub gmail_action_at: Option<String>,
    /// Only trashed items have a purge deadline. Archived items remain in Axon.
    pub purge_after: Option<String>,
    /// Last observed Gmail label location, kept separate from an Axon-requested
    /// action so direct Gmail changes are not misattributed.
    pub gmail_location: Option<String>,
    pub gmail_observed_at: Option<String>,
    pub gmail_sync_status: Option<String>,
    /// Action owned by the current queued or attention job, if one exists.
    pub gmail_sync_action: Option<String>,
    pub gmail_sync_error: Option<String>,
    /// The doctrine's one state label, mirrored from Gmail so the board can rank
    /// and render it without asking Gmail per row. Gmail stays authoritative: this
    /// is written only after its modify call succeeds.
    pub waiting: bool,
    /// Only meaningful while `waiting` is true, and cleared with it. "Blocked since"
    /// is the question a Waiting list is actually asked.
    pub waiting_since: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
}

/// One row of `{prefix}_triage_rules`: which deterministic rung decided a
/// thread, and what it decided.
///
/// `decided_by` is the stored string rather than `rules::DecidedBy` because
/// this is a read of whatever the file holds, and a value outside the CHECK
/// vocabulary must be reportable rather than a parse failure at the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RulesVerdictRow {
    pub triage_id: String,
    pub decided_by: String,
    pub stream: String,
    pub rationale: String,
    pub rules_version: String,
}

/// One thread the model rung may look at, with whatever verdict it already
/// carries.
///
/// The verdict travels with the candidate so the staleness check happens once,
/// in Rust, against an `item_revision` SQL cannot compute.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCandidate {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub data_class: String,
    pub rule_stream: String,
    pub rule_rationale: String,
    pub rule_decided_by: String,
    pub stored: Option<StoredVerdictState>,
}

/// The part of a stored verdict that decides whether the thread is asked again.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVerdictState {
    pub producer: String,
    pub prompt_revision: String,
    pub item_revision: String,
    pub state: String,
    pub mode: String,
    pub attempts: i64,
    /// Whether the retry deadline has passed, answered by SQLite against the
    /// same clock that wrote it. A raw stamp here would need the caller to
    /// parse a timestamp to learn the one thing it wants.
    pub backoff_expired: bool,
}

/// What one pass decided about one thread. Written whole; there is no partial
/// update, because a verdict with a new state and an old rationale would be a
/// row describing two different runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelVerdict {
    pub triage_id: String,
    /// `shadow`, `applied`, or `held` — apply refused this proposal because it
    /// would raise the data class, and a human owns that decision.
    pub mode: String,
    pub state: String,
    pub rule_decided_by: String,
    pub rule_stream: String,
    pub model_stream: Option<String>,
    pub confidence_bp: Option<i64>,
    pub urgency_bp: Option<i64>,
    pub rationale: Option<String>,
    pub urgency_rationale: Option<String>,
    pub redactions: i64,
    /// The class at prompt time. This is what a receipt proves "no Secret mail
    /// was prompted" from.
    pub data_class: String,
    /// The class the free text was redacted against: the higher of `data_class`
    /// and the class `model_stream` implies.
    pub redaction_class: String,
    pub producer: String,
    pub item_revision: String,
    pub prompt_revision: String,
    pub classification_version: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub next_attempt: Option<String>,
    pub held_reason: Option<String>,
    pub applied_at: Option<String>,
}

/// One verdict as the report counts it. Deliberately carries no `rationale`
/// and no `urgency_rationale`: the report body is meant to be safe to log and
/// to paste, and the cheapest way to keep it so is for the query that feeds it
/// never to select the text at all.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelVerdictSummary {
    pub triage_id: String,
    pub mode: String,
    pub state: String,
    pub rule_stream: String,
    pub model_stream: Option<String>,
    pub confidence_bp: Option<i64>,
    pub urgency_bp: Option<i64>,
    pub data_class: String,
    pub held_reason: Option<String>,
    pub producer: String,
    pub prompt_revision: String,
}

/// What applying one verdict did to the row.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModelWrite {
    pub stream_changed: bool,
}

/// What one remediation write did: the item's review fields, and the model
/// verdict's own two sentences, which the same class governs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RedactWrite {
    pub changed: bool,
    pub verdict_narrowed: bool,
}

/// What a human category change did to the row and to the two columns the
/// class governs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StreamWrite {
    pub changed: bool,
    pub class_changed: bool,
    pub narrowed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmailActionJob {
    pub job_id: i64,
    pub triage_id: String,
    pub action: String,
    pub source_status: String,
    pub attempts: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmailReconcileCandidate {
    pub triage_id: String,
    pub status: String,
}

/// A reviewed, bounded derivative staged locally before queueing.
/// The original content is never stored here and staging never dispatches it.
#[derive(Debug, Clone)]
pub struct CloudDerivativeApproval {
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: String,
    pub document: String,
    pub redaction_count: i32,
}

#[derive(Debug, Clone)]
pub struct CloudQueueRequest {
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub provider_role: String,
    /// Which question this job asks of the provider. Carried rather than
    /// defaulted in SQL: two tasks now exist over the same reviewed derivative
    /// — `cloud_dispatch::TASK_VERSION` (structured analysis) and
    /// `cloud_dispatch::DIGEST_TASK_VERSION` (a digest for a long public item)
    /// — and a column default is not a thing a caller can be wrong about in
    /// review.
    pub task: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudDispatchJob {
    pub job_id: String,
    pub source: String,
    pub item_id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub provider_role: String,
    pub task: String,
    /// The class the source row carried when the derivative was staged and a
    /// human approved it. Frozen: it describes the document beside it.
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: String,
    pub document: String,
    pub provider_calls: i32,
    /// The class the source row carries **now**, or `None` when the row is
    /// gone. Read at dispatch rather than at staging, because nothing
    /// invalidates a staged derivative when its source is reclassified: a
    /// derivative approved while a mail was `c1` stayed dispatchable after the
    /// mail became `c2`, and the c1 → c2 escalation happens without a human.
    /// `None` refuses, like every other unknown class here.
    pub current_source_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressEntry {
    pub id: i64,
    pub timestamp: String,
    pub job_id: Option<String>,
    pub task: String,
    pub provider: String,
    pub provider_role: String,
    pub model: String,
    pub data_class: String,
    pub preview_hash: String,
    pub document_payload: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cost_cents: f64,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewEgressEntry<'a> {
    pub job_id: Option<&'a str>,
    pub task: &'a str,
    pub provider: &'a str,
    pub provider_role: &'a str,
    pub model: &'a str,
    pub data_class: &'a str,
    pub preview_hash: &'a str,
    pub document_payload: &'a str,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cost_cents: f64,
    pub status: &'a str,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressAuditReport {
    pub total_calls: usize,
    pub succeeded_calls: usize,
    pub failed_calls: usize,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_cents: f64,
    pub raw_c2_violations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudAttemptClaim {
    Started(i64),
    DailyLimitReached,
    JobUnavailable,
}

/// The digest states a later run could plausibly do better on — the storage
/// mirror of `summarize::Outcome::retryable`.
///
/// One list because two statements key on it: the query that finds work and the
/// upsert that decides whether to arm a backoff. Two hand-maintained copies is
/// how a state ends up retryable in one and terminal in the other, which strands
/// rows in a way nothing reports.
pub const RETRYABLE_DIGEST_STATES: [&str; 6] = [
    "http_error",
    "model_error",
    "capacity_aborted",
    "empty_response",
    "timeout",
    // A cloud provider that was unreachable, rate-limited or in a bad mood.
    // Retryable for the same reason as the local ones, and bounded by the same
    // three attempts — the daily request budget is a separate ceiling on top.
    crate::digest::CLOUD_ERROR,
];

/// The verdict states the model rung will ask again about.
///
/// `Outcome::retryable()`'s five, plus `unparseable`: an answer this parser
/// could not find JSON in is a fact about the prompt, and a bounded retry is
/// cheaper than a prompt change while the rate is unknown. Deliberately NOT
/// here: `local_refused` and `skipped_over_window`, which are verdicts about
/// the item, and `invalid_stream`, where a second identical prompt reaches the
/// same answer.
pub const RETRYABLE_MODEL_VERDICT_STATES: [&str; 6] = [
    "http_error",
    "model_error",
    "capacity_aborted",
    "empty_response",
    "timeout",
    "unparseable",
];

/// Three attempts, matching the digest ladder's cap. A fourth would be the
/// pass spending local model time to reach the same answer.
pub const MAX_MODEL_VERDICT_ATTEMPTS: i64 = 3;

/// [`RETRYABLE_MODEL_VERDICT_STATES`] as a SQL literal list, built from the
/// const so the two cannot drift. Every element is a compile-time literal.
fn retryable_model_verdict_states_sql() -> String {
    RETRYABLE_MODEL_VERDICT_STATES
        .iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The states above, as a SQL literal list. Built from the const rather
/// than typed out, so the two cannot drift. Every element is a compile-time
/// literal, so this never carries caller input.
fn retryable_digest_states_sql() -> String {
    RETRYABLE_DIGEST_STATES
        .iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One row of `content_digests`, as stored.
///
/// The wire shape is `content_item::Digest`; this is the database's view of the
/// same thing, kept separate so a column type change does not reach the reader
/// contract by accident. `focus` is comma-joined here and split for the wire —
/// it is display state read back into a text field, not something anything
/// queries by.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDigest {
    pub source: String,
    pub item_id: String,
    pub text: Option<String>,
    pub state: String,
    pub shape: String,
    pub depth: String,
    pub focus: String,
    pub producer: String,
    pub source_chars: i64,
    pub redactions: i32,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub diagram: Option<String>,
    pub diagram_state: Option<String>,
    pub diagram_error: Option<String>,
    pub chart: Option<String>,
    pub chart_state: Option<String>,
    pub chart_error: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudDerivativeState {
    pub status: String,
    pub preview_hash: Option<String>,
    pub approved_at: Option<String>,
    pub dispatch_status: String,
    pub job_id: Option<String>,
    pub provider_role: Option<String>,
    pub queued_at: Option<String>,
    pub provider_calls: u8,
    pub task: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<serde_json::Value>,
}

impl CloudDerivativeState {
    pub fn not_prepared() -> Self {
        Self {
            status: "not_prepared".into(),
            preview_hash: None,
            approved_at: None,
            dispatch_status: "not_queued".into(),
            job_id: None,
            provider_role: None,
            queued_at: None,
            provider_calls: 0,
            task: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            result: None,
        }
    }
}

/// Per-source run bookkeeping (round-trips via record_run/get_source_state).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceState {
    pub source_name: String,
    pub last_run_at: String,
    pub cursor: Option<String>,
    /// Distinct from `last_run_at`: a run that failed still ran. Kept apart so
    /// "we last actually collected something at T" survives a failing streak,
    /// which is the number a human wants when deciding whether to intervene.
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    /// Error *class*, never a message body or a mail field.
    pub last_error: Option<String>,
    pub considered_count: i64,
    pub new_count: i64,
    /// Drives the backoff. Reset to 0 by any success.
    pub consecutive_failures: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationSummary {
    pub evaluated: i64,
    pub reranked: i64,
    pub semantic: i64,
    pub lexical: i64,
    pub unscored: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TravelContextSnapshot {
    pub revision: String,
    pub payload: String,
    pub refreshed_at: String,
}

/// A revisioned blob feeding `context_revision`, addressed by its kind.
/// `travel` and `feedback-model` are the two kinds today; the shape is the
/// same, which is why one accessor pair serves both.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextSnapshot {
    pub revision: String,
    pub payload: String,
    pub refreshed_at: String,
}

/// How often the operator has decided anything, for the cold-start gate to
/// report against its own thresholds. Verbs only, never content.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InteractionCounts {
    pub opened: i64,
    pub kept: i64,
    pub dismissed: i64,
    pub reopened: i64,
    pub unkept: i64,
    pub shared: i64,
    pub total: i64,
}

/// One training row: the item that was decided, what the decision was, and
/// when — with the ledger's own date where it has one, and the item's capture
/// date where the decision predates the ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackLabel {
    pub feed_id: String,
    /// `true` for kept, `false` for dismissed. A retracted decision is not a
    /// label at all and never reaches this list.
    pub kept: bool,
    /// Epoch seconds of the decisive event, for the time decay and the
    /// time-ordered holdout split.
    pub decided_at: i64,
    /// `true` when the decision has no ledger row and was read from the item's
    /// `status`, so the trainer can weight it at the decay floor: the decision
    /// is real, its date is not.
    pub seeded_from_status: bool,
}

/// Every label the trainer may see, with the two counts the gate has to report:
/// how many decisions the class ladder refused, and how many carry no date.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrainingLabels {
    pub labels: Vec<FeedbackLabel>,
    pub skipped_class: usize,
    pub seeded_from_status: usize,
}

/// Canonicalize a URL for stable identity: trim, drop the `#fragment`, drop a
/// single trailing slash, lowercase the scheme+host. Deliberately conservative
/// -- it does not try to normalize query params or youtu.be↔youtube.com.
pub fn canonical_url(url: &str) -> String {
    let mut s = url.trim().to_string();
    if let Some(hash) = s.find('#') {
        s.truncate(hash);
    }
    if let Some(scheme_end) = s.find("://") {
        let host_start = scheme_end + 3;
        let host_end = s[host_start..]
            .find('/')
            .map(|i| host_start + i)
            .unwrap_or(s.len());
        let lowered = s[..host_end].to_lowercase();
        s = format!("{}{}", lowered, &s[host_end..]);
    }
    if s.ends_with('/') {
        s.pop();
    }
    s
}

/// feed_items PK: sha256 hex of the canonical URL.
pub fn feed_id(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_url(url).as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

/// The classification a human's reclassification request should write, or the
/// reason it is refused.
///
/// Shared by feed and mail because it is one rule, and the two tables having
/// their own copy of it is how they would come to disagree. Escalating needs no
/// reason and none is invented: the canned sentence is honest about being a
/// manual change. Lowering needs the operator's own words, and gets them —
/// stored, so the decision is answerable afterwards.
pub(crate) fn human_reclassification(
    current: &str,
    current_method: &str,
    proposed: &str,
    rationale: Option<&str>,
) -> Result<crate::content_item::DataClass, Box<dyn std::error::Error>> {
    let written = rationale.map(str::trim).unwrap_or_default();
    crate::content_item::admit_reclassification(
        current,
        current_method,
        proposed,
        crate::content_item::METHOD_HUMAN,
        written,
    )?;
    let rationale = if written.is_empty() {
        "Data class set manually in Axon.".to_string()
    } else {
        written.to_string()
    };
    Ok(crate::content_item::DataClass::set_by_human(
        proposed, &rationale,
    ))
}

fn cloud_job_id(request: &CloudQueueRequest) -> String {
    let mut hasher = Sha256::new();
    for part in [
        request.source.as_str(),
        request.item_id.as_str(),
        request.source_revision.as_str(),
        request.preview_hash.as_str(),
        request.provider_role.as_str(),
        request.task.as_str(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!(
        "cloud-job-{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

mod cloud;
mod evaluation;
mod feed;
mod feedback;
mod migrations;
mod origins;
mod rows;
mod source_state;
mod triage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedOrigin {
    pub source_id: String,
    pub source_ref: String,
    pub label: Option<String>,
}

/// Gap between two arrivals of the same source that reads as "a different run".
const RUN_GAP_MINUTES: i64 = 30;

/// One item's place in a collector run, derived at read time. The reader groups
/// on `run_key`; an item with no origin row appears in none of these and stays
/// an ordinary ungrouped row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedRun {
    pub feed_id: String,
    pub source_id: String,
    pub label: Option<String>,
    pub run_key: String,
    pub run_started: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginSummary {
    pub source_id: String,
    pub item_count: i64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

use rows::{epoch_now, row_to_feed_full, row_to_feed_list, row_to_triage};

#[cfg(test)]
mod unit_tests {
    use super::{canonical_url, feed_id};

    #[test]
    fn canonical_url_and_feed_id_stable() {
        assert_eq!(
            canonical_url("HTTPS://YouTube.com/watch?v=abc/#t=10"),
            "https://youtube.com/watch?v=abc"
        );
        // Identity is stable across trailing slash / fragment / host case.
        assert_eq!(
            feed_id("https://example.com/x/"),
            feed_id("https://EXAMPLE.com/x#frag")
        );
        assert_eq!(feed_id("https://a.example").len(), 64);
    }
}

/// Database-backed; named for the selector CI splits on — see
/// `capabilities/scouting/src/store.rs` for why the name is the contract.
#[cfg(test)]
pub(crate) mod db_tests {
    use super::*;

    /// A file per test, in a directory this process owns.
    ///
    /// It replaces the per-pid schema and the guard that dropped it. That guard
    /// existed because four abandoned test schemas were sitting in the shared
    /// database on 2026-07-28, from two long-finished processes, and every one
    /// would have gone into the next `pg_dumpall`. A temp file is neither shared
    /// nor backed up, so the failure it prevented cannot occur.
    ///
    /// `pub(crate)` because `cloud_run`'s tests need a real store too: the
    /// enqueue refusal they pin is only worth pinning against a store that
    /// would happily have written the row.
    pub(crate) fn open_test_store(name: &str) -> Store {
        let path = test_database(name);
        Store::open(&path)
            .unwrap_or_else(|e| panic!("could not open test store at {}: {e}", path.display()))
    }

    /// The path `open_test_store` opens, for a test that needs raw SQL beside
    /// the store's own statements.
    pub(crate) fn test_database(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("comms-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a writable temp directory");
        let path = directory.join(format!("{name}.db"));
        // The pid is recycled eventually, so the file starts empty -- which is
        // what the old DROP SCHEMA was buying.
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The readiness probe has to reach the database, not merely hold a pool handle —
    /// a check that passes without touching the file is the bug #126 is about.
    #[test]
    fn ping_reaches_the_database() {
        open_test_store("ping")
            .ping()
            .expect("a live store answers its own ping");
    }

    /// The readiness handler turns exactly this into a 503. It replaces "port 1
    /// is unreachable": there is no port any more, so an unusable path is the
    /// failure a deployment can actually have.
    #[test]
    fn a_store_cannot_be_opened_against_an_unusable_path() {
        let blocker = std::env::temp_dir().join(format!("comms-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"not a directory").unwrap();
        assert!(
            Store::open(&blocker.join("axon.db")).is_err(),
            "an unusable path opened anyway"
        );
    }

    /// The rebuild opens a raw connection before the pool does, and
    /// `Connection::open` creates a file but never a directory. Every existing
    /// test mkdirs its own path first, so the one run that would have failed —
    /// a genuine first deploy, where `~/.local/state/axon/` does not exist yet —
    /// is exactly the one no test covered.
    #[test]
    fn a_first_run_creates_the_directory_its_database_lives_in() {
        let directory = std::env::temp_dir()
            .join(format!("comms-first-run-{}", std::process::id()))
            .join("state")
            .join("axon");
        let _ = std::fs::remove_dir_all(&directory);
        assert!(!directory.exists(), "the test starts with no directory");

        let store = Store::open(&directory.join("axon.db"))
            .expect("a first run creates the directory it was pointed at");
        store.ping().expect("and the database in it is usable");
    }

    /// Gmail's `internalDate` is epoch-ms and reached the column through
    /// `to_timestamp($5)`. Its replacement is `strftime(..., ?5, 'unixepoch')`,
    /// which has to land in the same canonical format every other stamp is in --
    /// otherwise `ORDER BY internal_date` in `list_triage` stops being time order.
    #[test]
    fn the_gmail_epoch_lands_in_the_canonical_format() {
        let store = open_test_store("internal_date");
        store
            .upsert_triage(&mk_triage("thread-epoch", "werbung"))
            .unwrap();
        let stored = store
            .get_triage("thread-epoch")
            .unwrap()
            .expect("just written")
            .internal_date_text
            .expect("internalDate was supplied");
        assert_eq!(stored.len(), 29, "got {stored}");
        assert!(stored.ends_with("+00:00"), "got {stored}");
        // 1_700_000_000_000 ms is 2023-11-14T22:13:20Z.
        assert!(stored.starts_with("2023-11-14 22:13:20"), "got {stored}");
    }

    /// `pub(crate)` for the same reason [`open_test_store`] is: `cloud_run`'s
    /// dispatch tests need a real mail row, because the stale-derivative seam
    /// they pin is the mail one.
    pub(crate) fn mk_triage(id: &str, stream: &str) -> TriageItem {
        TriageItem {
            id: id.into(),
            from_addr: Some("news@shop.example".into()),
            subject: Some("SALE".into()),
            snippet: Some("snippet".into()),
            internal_date_ms: Some(1_700_000_000_000),
            internal_date_text: None,
            stream: stream.into(),
            rationale: "test".into(),
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: crate::rules::MAIL_RULES_VERSION.into(),
            data_class: "c1".into(),
            data_class_rationale: "Mail metadata is Mine by default.".into(),
            data_classification_method: content_item::METHOD_DETERMINISTIC.into(),
            data_classification_version: content_item::MAIL_CLASSIFIER_VERSION.into(),
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

    /// `set_triage_stream` with the class the new category implies, re-derived
    /// the way the handler does it. A helper because the class is now a
    /// parameter and every caller has to answer the same question.
    pub(crate) fn set_stream(store: &Store, id: &str, stream: &str) -> StreamWrite {
        let item = store.get_triage(id).unwrap().expect("the row exists");
        let classification = crate::intake::classify_mail(
            stream,
            item.from_addr.as_deref().unwrap_or_default(),
            item.subject.as_deref().unwrap_or_default(),
            item.snippet.as_deref().unwrap_or_default(),
        );
        store
            .set_triage_stream(id, stream, &classification)
            .unwrap()
    }

    fn mk_feed(url: &str, kind: &str, stream: &str) -> FeedItem {
        let mut f = FeedItem::new(url, stream, kind);
        f.title = Some("A Title".into());
        f.transcript = Some("some transcript".into());
        f
    }

    #[test]
    fn triage_upsert_is_idempotent_and_updates_fields() {
        let store = open_test_store("triage_idem");
        let mut item = mk_triage("thread:1", "werbung");
        assert!(store.upsert_triage(&item).unwrap(), "first insert is new");
        item.rationale = "changed".into();
        item.stream = "feed".into();
        assert!(!store.upsert_triage(&item).unwrap(), "second is not new");
        let rows = store.list_triage(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stream, "feed");
        assert_eq!(rows[0].rationale, "changed");
    }

    #[test]
    fn triage_set_status_validates() {
        let store = open_test_store("triage_status");
        store
            .upsert_triage(&mk_triage("thread:s", "aktiv"))
            .unwrap();
        assert!(store.set_triage_status("thread:s", "approved").unwrap());
        assert_eq!(
            store.get_triage_status("thread:s").unwrap().as_deref(),
            Some("approved")
        );
        assert!(
            store.set_triage_status("thread:s", "bogus").is_err(),
            "invalid status must error"
        );
        assert!(
            !store
                .set_triage_status("thread:missing", "dismissed")
                .unwrap(),
            "unknown id -> false"
        );
    }

    /// `waiting` is its own axis, and clearing it takes the timestamp with it.
    ///
    /// The stale-timestamp case is the one worth pinning: a row left with
    /// `waiting = false` but a `waiting_at` still set reads as a currently
    /// blocked thread to any query that ranks on the timestamp and forgets the
    /// boolean, which is exactly the query someone writes first.
    #[test]
    fn waiting_is_recorded_and_fully_cleared() {
        let store = open_test_store("triage_waiting");
        store
            .upsert_triage(&mk_triage("thread:w", "aktiv"))
            .unwrap();

        let waiting_at = || -> Option<String> {
            let conn = store.conn().unwrap();
            conn.query_row(
                &format!(
                    "SELECT waiting, waiting_at FROM {}_triage_items WHERE id = ?1",
                    store.prefix
                ),
                params!["thread:w"],
                |row| {
                    let flag: bool = row.get(0)?;
                    let at: Option<String> = row.get(1)?;
                    assert_eq!(flag, at.is_some(), "the flag and the timestamp must agree");
                    Ok(at)
                },
            )
            .unwrap()
        };

        assert!(waiting_at().is_none(), "a fresh proposal is not waiting");

        assert!(store.set_triage_waiting("thread:w", true).unwrap());
        assert!(waiting_at().is_some(), "marking must stamp when");

        assert!(store.set_triage_waiting("thread:w", false).unwrap());
        assert!(
            waiting_at().is_none(),
            "clearing must drop the timestamp, not only the flag"
        );

        assert!(
            !store.set_triage_waiting("thread:missing", true).unwrap(),
            "unknown id -> false"
        );
    }

    #[test]
    fn gmail_lifecycle_distinguishes_archive_trash_and_restore() {
        let store = open_test_store("gmail_lifecycle");
        store
            .upsert_triage(&mk_triage("thread:archive", "aktiv"))
            .unwrap();
        store
            .upsert_triage(&mk_triage("thread:trash", "aktiv"))
            .unwrap();

        assert!(store
            .record_gmail_action("thread:archive", "archive")
            .unwrap());
        let archived = store.get_triage("thread:archive").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_action.as_deref(), Some("archive"));
        assert!(archived.gmail_action_at.is_some());
        assert!(archived.purge_after.is_none());

        assert!(store.record_gmail_action("thread:trash", "trash").unwrap());
        let trashed = store.get_triage("thread:trash").unwrap().unwrap();
        assert_eq!(trashed.status, "trashed");
        assert_eq!(trashed.gmail_action.as_deref(), Some("trash"));
        assert!(trashed.purge_after.is_some());

        assert!(store
            .record_gmail_action("thread:trash", "restore")
            .unwrap());
        let restored = store.get_triage("thread:trash").unwrap().unwrap();
        assert_eq!(restored.status, "proposed");
        assert_eq!(restored.gmail_action.as_deref(), Some("restore"));
        assert!(restored.purge_after.is_none());
        assert!(store.record_gmail_action("thread:trash", "delete").is_err());
    }

    #[test]
    fn gmail_action_intent_is_durable_idempotent_and_bounded() {
        let store = open_test_store("gmail_action_jobs");
        store
            .upsert_triage(&mk_triage("thread:job", "aktiv"))
            .unwrap();

        let job = store.queue_gmail_action("thread:job", "archive").unwrap();
        assert_eq!(job.action, "archive");
        assert_eq!(job.source_status, "proposed");
        assert!(store.queue_gmail_action("thread:job", "trash").is_err());
        assert_eq!(store.pending_gmail_actions(10).unwrap(), vec![job.clone()]);
        let queued = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(queued.gmail_sync_status.as_deref(), Some("queued"));

        assert!(store.complete_gmail_action(job.job_id).unwrap());
        assert!(store.complete_gmail_action(job.job_id).unwrap());
        assert!(store.pending_gmail_actions(10).unwrap().is_empty());
        let archived = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_location.as_deref(), Some("archive"));
        assert_eq!(archived.gmail_sync_status.as_deref(), Some("synced"));

        let restore = store.queue_gmail_action("thread:job", "restore").unwrap();
        for attempt in 1..=5 {
            let state = store
                .fail_gmail_action(restore.job_id, "bounded test error")
                .unwrap();
            assert_eq!(state, if attempt == 5 { "abandoned" } else { "queued" });
            if attempt < 5 {
                let conn = store.conn().unwrap();
                conn.execute(
                    &format!(
                        "UPDATE {}_gmail_action_jobs SET next_attempt = {now} WHERE job_id = ?1",
                        store.prefix,
                        now = sjel_store::NOW
                    ),
                    params![restore.job_id],
                )
                .unwrap();
            }
        }
        assert!(store.pending_gmail_actions(10).unwrap().is_empty());
        let attention = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(attention.gmail_sync_status.as_deref(), Some("attention"));
        assert_eq!(attention.gmail_sync_action.as_deref(), Some("restore"));
        assert_eq!(
            attention.gmail_sync_error.as_deref(),
            Some("bounded test error")
        );
        assert!(store
            .gmail_reconcile_candidates(10)
            .unwrap()
            .iter()
            .all(|candidate| candidate.triage_id != "thread:job"));

        let retried = store.retry_abandoned_gmail_action("thread:job").unwrap();
        assert_eq!(retried.action, "restore");
        assert_eq!(retried.attempts, 0);
        assert_eq!(
            store
                .get_triage("thread:job")
                .unwrap()
                .unwrap()
                .gmail_sync_status
                .as_deref(),
            Some("queued")
        );
        for _ in 0..5 {
            store
                .fail_gmail_action(retried.job_id, "still unavailable")
                .unwrap();
        }
        assert!(store.cancel_abandoned_gmail_action("thread:job").unwrap());
        assert!(!store.cancel_abandoned_gmail_action("thread:job").unwrap());
        let canceled = store.get_triage("thread:job").unwrap().unwrap();
        assert_eq!(canceled.gmail_sync_status.as_deref(), Some("synced"));
        assert!(canceled.gmail_sync_action.is_none());
        assert!(canceled.gmail_sync_error.is_none());
    }

    #[test]
    fn observed_gmail_location_reconciles_direct_changes() {
        let store = open_test_store("gmail_observed_location");
        store
            .upsert_triage(&mk_triage("thread:observed", "aktiv"))
            .unwrap();
        assert!(store
            .observe_gmail_location("thread:observed", "archive")
            .unwrap());
        let archived = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.gmail_location.as_deref(), Some("archive"));
        assert!(archived.gmail_action.is_none());

        store
            .observe_gmail_location("thread:observed", "trash")
            .unwrap();
        let trashed = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(trashed.status, "trashed");
        assert!(trashed.purge_after.is_some());

        store
            .observe_gmail_location("thread:observed", "inbox")
            .unwrap();
        let restored = store.get_triage("thread:observed").unwrap().unwrap();
        assert_eq!(restored.status, "proposed");
        assert_eq!(restored.gmail_location.as_deref(), Some("inbox"));
        assert!(restored.purge_after.is_none());
        assert!(store
            .observe_gmail_location("thread:observed", "spam")
            .is_err());
    }

    #[test]
    fn missing_gmail_thread_is_retained_and_closes_pending_work() {
        let store = open_test_store("gmail_missing");
        store
            .upsert_triage(&mk_triage("thread:missing", "aktiv"))
            .unwrap();
        store
            .observe_gmail_location("thread:missing", "trash")
            .unwrap();
        let deadline = store
            .get_triage("thread:missing")
            .unwrap()
            .unwrap()
            .purge_after;

        assert!(store.observe_gmail_missing("thread:missing").unwrap());
        let missing = store.get_triage("thread:missing").unwrap().unwrap();
        assert_eq!(missing.status, "missing");
        assert_eq!(missing.gmail_location.as_deref(), Some("missing"));
        assert_eq!(missing.gmail_sync_status.as_deref(), Some("synced"));
        assert_eq!(missing.purge_after, deadline);

        store
            .upsert_triage(&mk_triage("thread:missing", "aktiv"))
            .unwrap();
        let returned = store.get_triage("thread:missing").unwrap().unwrap();
        assert_eq!(returned.status, "proposed");
        assert_eq!(returned.gmail_location.as_deref(), Some("inbox"));
        assert!(returned.purge_after.is_none());
    }

    #[test]
    fn expired_trash_is_purged_but_archive_is_retained() {
        let store = open_test_store("gmail_trash_purge");
        store
            .upsert_triage(&mk_triage("thread:expired", "aktiv"))
            .unwrap();
        store
            .upsert_triage(&mk_triage("thread:archive", "aktiv"))
            .unwrap();
        store
            .record_gmail_action("thread:expired", "trash")
            .unwrap();
        store
            .record_gmail_action("thread:archive", "archive")
            .unwrap();
        {
            let conn = store.conn().unwrap();
            conn.execute(
                &format!(
                    "UPDATE {}_triage_items SET purge_after = {past} WHERE id = ?1",
                    store.prefix,
                    past = sjel_store::now_offset("'-1 second'")
                ),
                params!["thread:expired"],
            )
            .unwrap();
        }

        assert_eq!(store.purge_expired_trashed().unwrap(), 1);
        assert!(store.get_triage("thread:expired").unwrap().is_none());
        assert_eq!(
            store
                .get_triage_status("thread:archive")
                .unwrap()
                .as_deref(),
            Some("archived")
        );
    }

    #[test]
    fn human_triage_category_survives_a_resweep() {
        let store = open_test_store("triage_human_category");
        store
            .upsert_triage(&mk_triage("thread:manual", "aktiv"))
            .unwrap();
        assert!(set_stream(&store, "thread:manual", "belege").changed);

        let mut refetched = mk_triage("thread:manual", "werbung");
        refetched.rationale = "new rule result".into();
        store.upsert_triage(&refetched).unwrap();

        let row = store.list_triage(None).unwrap().remove(0);
        assert_eq!(row.stream, "belege");
        assert_eq!(row.rationale, "Category set manually in Axon.");
        assert_eq!(row.classification_method, "human");
        assert_eq!(row.classification_version, "manual-v1");
        assert_eq!(
            row.status, "proposed",
            "categorizing does not resolve the proposal"
        );
        let classification =
            crate::content_item::DataClass::classify_mail("aktiv", "a@b.example", "x");
        assert!(store
            .set_triage_stream("thread:manual", "bogus", &classification)
            .is_err());
    }

    #[test]
    fn human_data_class_survives_rule_refresh_and_resweep() {
        let store = open_test_store("triage_human_data_class");
        store
            .upsert_triage(&mk_triage("thread:private", "aktiv"))
            .unwrap();
        assert!(
            store
                .set_triage_data_class("thread:private", "c3", None)
                .unwrap()
                .changed
        );

        let rules =
            crate::content_item::DataClass::classify_mail("aktiv", "friend@example.com", "Hello");
        assert!(!store
            .refresh_triage_data_class("thread:private", &rules)
            .unwrap());
        store
            .upsert_triage(&mk_triage("thread:private", "feed"))
            .unwrap();

        let row = store.get_triage("thread:private").unwrap().unwrap();
        assert_eq!(row.data_class, "c3");
        assert_eq!(row.data_classification_method, "human");
        assert_eq!(row.data_classification_version, "manual-v1");
        assert_eq!(row.data_class_rationale, "Data class set manually in Axon.");
        assert!(store
            .set_triage_data_class("thread:private", "secret", Some("why"))
            .is_err());
    }

    /// The redaction a class demanded outlives a re-sweep that no longer sees
    /// the reason for it. Ruling 3 made `classify_mail` depend on the people
    /// registry, so a sweep with the overlay unmounted answers c1 for a thread
    /// the refresh raised to c2 — and the upsert used to write that sweep's
    /// verbatim subject over the redacted one while keeping the c2 label.
    ///
    /// What survives is the *rule*, not the old text: the row keeps following
    /// its thread and the incoming message is redacted on the way in. Freezing
    /// the stored pair instead would leave the row showing one message's date
    /// and sender beside an older message's subject.
    ///
    /// The fixtures below are redactable without the people registry — a
    /// salutation cue and a URL, both rung-1 shapes — because the machine's
    /// registry is a process-wide `OnceLock` and a gate asserted against the
    /// operator's contact list is a gate whose result changes when they meet
    /// someone.
    #[test]
    fn a_weaker_resweep_cannot_unredact_an_escalated_row() {
        let store = open_test_store("triage_resweep_unredact");
        store
            .upsert_triage(&mk_triage("thread:escalated", "aktiv"))
            .unwrap();

        // What `POST /triage/data-class/refresh` does with the registry loaded.
        let escalated = content_item::DataClass::new(
            "c2",
            crate::intake::KNOWN_PERSON_RATIONALE,
            content_item::METHOD_DETERMINISTIC,
            content_item::MAIL_CLASSIFIER_VERSION,
        );
        assert!(store
            .refresh_triage_data_class("thread:escalated", &escalated)
            .unwrap());
        assert!(
            store
                .redact_triage_review_fields(
                    "thread:escalated",
                    "c2",
                    Some("Re: [person], next week"),
                    Some("[person] asked about the schedule"),
                )
                .unwrap()
                .changed
        );

        // The next sweep, registry absent: c1, and a *newer* message's raw
        // Gmail metadata.
        let mut blind = mk_triage("thread:escalated", "aktiv");
        blind.subject = Some("Hallo Mustermann, der neue Termin".into());
        blind.snippet = Some("Antwort bitte an https://example.com/t/AbCd1234567890Ef".into());
        store.upsert_triage(&blind).unwrap();

        let row = store.get_triage("thread:escalated").unwrap().unwrap();
        assert_eq!(row.data_class, "c2", "the strict class survives");
        let subject = row.subject.clone().unwrap();
        let snippet = row.snippet.clone().unwrap();
        assert!(
            !subject.contains("Mustermann"),
            "no verbatim name is stored on a c2 row: {subject}"
        );
        assert!(
            !snippet.contains("AbCd1234567890Ef"),
            "no verbatim link is stored on a c2 row: {snippet}"
        );
        assert!(subject.contains("[person]"), "redacted, not discarded");

        // Freshness: the stored text is derived from the new message, not
        // frozen at the old one. The `[link]` marker can only come from the new
        // snippet — the redacted text it replaced held no link.
        assert!(
            snippet.contains("[link]"),
            "the stored snippet follows the thread: {snippet}"
        );
        assert!(
            !snippet.contains("asked about the schedule"),
            "the previous message's text is not frozen in place: {snippet}"
        );
        assert!(
            subject.contains("der neue Termin"),
            "the new subject's readable half survives: {subject}"
        );
    }

    /// The operator's own escalation narrows the row it lands on, in the same
    /// transaction — both review fields, not whichever one the eye fell on.
    /// Selecting Others or Secret on a mail the rules never matched must not
    /// leave the row labelled Redacted in the dashboard while the material it
    /// says it removed is still in `subject` and `snippet`.
    #[test]
    fn setting_a_strict_class_by_hand_redacts_the_row_it_lands_on() {
        let store = open_test_store("triage_human_class_redacts");
        let mut item = mk_triage("thread:invoice", "aktiv");
        item.subject = Some("Rechnung 4482159900 faellig".into());
        item.snippet = Some("Rueckfragen an buchhaltung@example.com".into());
        store.upsert_triage(&item).unwrap();
        // The state the operator is correcting: c1, stored verbatim.
        let raw = store.get_triage("thread:invoice").unwrap().unwrap();
        assert_eq!(raw.subject.as_deref(), Some("Rechnung 4482159900 faellig"));

        let write = store
            .set_triage_data_class("thread:invoice", "c2", None)
            .unwrap();
        assert!(write.changed);
        assert!(write.narrowed, "the same call removed the material");

        let row = store.get_triage("thread:invoice").unwrap().unwrap();
        assert_eq!(row.data_class, "c2");
        let subject = row.subject.clone().unwrap();
        let snippet = row.snippet.clone().unwrap();
        assert!(
            !subject.contains("4482159900"),
            "the number survived: {subject}"
        );
        assert!(subject.contains("[number]"));
        assert!(
            !snippet.contains("buchhaltung"),
            "the address survived: {snippet}"
        );
        assert!(snippet.contains("[email]"), "both fields, not just one");

        // Idempotent, and honest about it: a second pass finds nothing left.
        let again = store
            .set_triage_data_class("thread:invoice", "c2", None)
            .unwrap();
        assert!(again.changed);
        assert!(!again.narrowed, "a clean row reports no narrowing");
    }

    /// Scoped by class, on this path too. Setting c1 rewrites the class columns
    /// and leaves the review fields exactly as they were, or the review list
    /// stops being reviewable.
    #[test]
    fn setting_an_ordinary_class_by_hand_leaves_the_text_alone() {
        let store = open_test_store("triage_human_class_verbatim");
        let mut item = mk_triage("thread:lunch", "aktiv");
        item.subject = Some("Lunch on Tuesday?".into());
        item.snippet = Some("Half twelve at the usual place".into());
        store.upsert_triage(&item).unwrap();

        let write = store
            .set_triage_data_class("thread:lunch", "c1", None)
            .unwrap();
        assert!(write.changed);
        assert!(!write.narrowed);

        let row = store.get_triage("thread:lunch").unwrap().unwrap();
        assert_eq!(row.subject.as_deref(), Some("Lunch on Tuesday?"));
        assert_eq!(
            row.snippet.as_deref(),
            Some("Half twelve at the usual place")
        );
    }

    /// The guard above is scoped to rows that were narrowed. A c1 row still
    /// tracks its thread, or the review list stops matching the mailbox.
    #[test]
    fn an_ordinary_row_still_follows_its_thread() {
        let store = open_test_store("triage_resweep_verbatim");
        store
            .upsert_triage(&mk_triage("thread:ordinary", "aktiv"))
            .unwrap();
        let mut resweep = mk_triage("thread:ordinary", "aktiv");
        resweep.snippet = Some("a newer message in the thread".into());
        store.upsert_triage(&resweep).unwrap();

        let row = store.get_triage("thread:ordinary").unwrap().unwrap();
        assert_eq!(row.data_class, "c1");
        assert_eq!(
            row.snippet.as_deref(),
            Some("a newer message in the thread")
        );
    }

    /// The rung a deterministic verdict fired on, as a re-upsert carries it.
    fn verdict(stream: &str, decided_by: crate::rules::DecidedBy) -> crate::rules::Verdict {
        crate::rules::Verdict {
            stream: stream.into(),
            rationale: "a fresh rules result".into(),
            decided_by,
        }
    }

    /// A row the model rung wrote, built the way `apply_model_stream` will:
    /// one upsert at `method = 'model'` over an existing deterministic row.
    fn make_model_row(store: &Store, id: &str, stream: &str) {
        let mut written = mk_triage(id, stream);
        written.classification_method = content_item::METHOD_MODEL.into();
        written.classification_version = "mail-model-v1".into();
        written.rationale = "the model's sentence".into();
        store.upsert_triage(&written).unwrap();
        let row = store.get_triage(id).unwrap().unwrap();
        assert_eq!(row.classification_method, "model", "setup did not take");
    }

    /// THE defect this stream had to fix before the model rung could store
    /// anything. The category axis preserved only against the literal `'human'`
    /// (`store/triage.rs`), so the next deterministic sweep reverted a
    /// `method = 'model'` row — and the unattended sweep is enabled in this
    /// overlay, so the window was one sweep interval, not a hypothetical.
    ///
    /// Fails on the code that shipped before this test.
    #[test]
    fn a_model_row_survives_a_deterministic_resweep() {
        let store = open_test_store("triage_model_survives_resweep");
        store
            .upsert_triage(&mk_triage("thread:model", "aktiv"))
            .unwrap();
        make_model_row(&store, "thread:model", "issue");

        let mut resweep = mk_triage("thread:model", "aktiv");
        resweep.rationale = "No rule matched; kept active as the conservative default.".into();
        store
            .upsert_triage_with_rules(
                &resweep,
                &verdict("aktiv", crate::rules::DecidedBy::Fallback),
            )
            .unwrap();

        let row = store.get_triage("thread:model").unwrap().unwrap();
        assert_eq!(row.stream, "issue");
        assert_eq!(row.rationale, "the model's sentence");
        assert_eq!(row.classification_method, "model");
        assert_eq!(row.classification_version, "mail-model-v1");
    }

    /// The clause a bare method rank would have got wrong. The model rung ran
    /// only because the rules fell through, so a rule that now FIRES takes the
    /// row back — which is what keeps a new overlay rule able to correct the
    /// model rather than being frozen out by it.
    #[test]
    fn a_firing_rule_takes_a_model_row_back() {
        for decided_by in [
            crate::rules::DecidedBy::ConfigRule,
            crate::rules::DecidedBy::Heuristic,
        ] {
            let store = open_test_store(&format!("triage_rule_reclaims_{}", decided_by.as_str()));
            store
                .upsert_triage(&mk_triage("thread:reclaim", "aktiv"))
                .unwrap();
            make_model_row(&store, "thread:reclaim", "issue");

            let mut resweep = mk_triage("thread:reclaim", "werbung");
            resweep.rationale = "a fresh rules result".into();
            store
                .upsert_triage_with_rules(&resweep, &verdict("werbung", decided_by))
                .unwrap();

            let row = store.get_triage("thread:reclaim").unwrap().unwrap();
            assert_eq!(row.stream, "werbung", "{decided_by:?} did not take the row");
            assert_eq!(row.classification_method, "deterministic");
        }
    }

    /// And the other half of the same clause: a deterministic FALLBACK is not a
    /// rule firing, and a caller that passes no verdict at all reads as one.
    /// `NULL IN (…)` is NULL in SQLite, so this is also the regression test for
    /// the `COALESCE` in the predicate.
    #[test]
    fn a_deterministic_fallback_does_not() {
        let store = open_test_store("triage_fallback_keeps_off");
        store
            .upsert_triage(&mk_triage("thread:fallback", "aktiv"))
            .unwrap();
        make_model_row(&store, "thread:fallback", "issue");

        store
            .upsert_triage_with_rules(
                &mk_triage("thread:fallback", "werbung"),
                &verdict("werbung", crate::rules::DecidedBy::Fallback),
            )
            .unwrap();
        assert_eq!(
            store.get_triage("thread:fallback").unwrap().unwrap().stream,
            "issue"
        );

        // No verdict at all: the 30-odd callers that never learned about rungs.
        store
            .upsert_triage(&mk_triage("thread:fallback", "feed"))
            .unwrap();
        assert_eq!(
            store.get_triage("thread:fallback").unwrap().unwrap().stream,
            "issue",
            "a caller with no verdict must not outrank the model"
        );
    }

    /// The rank is a ladder, not a swap: nothing a machine writes displaces a
    /// human, whichever rung the machine was.
    #[test]
    fn a_human_row_still_survives_a_model_write() {
        let store = open_test_store("triage_human_beats_model");
        store
            .upsert_triage(&mk_triage("thread:human", "aktiv"))
            .unwrap();
        assert!(set_stream(&store, "thread:human", "steuern").changed);

        let mut model = mk_triage("thread:human", "werbung");
        model.classification_method = content_item::METHOD_MODEL.into();
        model.classification_version = "mail-model-v1".into();
        model.rationale = "the model's sentence".into();
        store.upsert_triage(&model).unwrap();

        let row = store.get_triage("thread:human").unwrap().unwrap();
        assert_eq!(row.stream, "steuern");
        assert_eq!(row.rationale, "Category set manually in Axon.");
        assert_eq!(row.classification_method, "human");
        assert_eq!(row.classification_version, "manual-v1");
    }

    /// The SQL `CASE` ranks and `content_item::method_rank` are two spellings of
    /// one ladder, and the only thing stopping them drifting is this. All
    /// sixteen (stored, incoming) method pairs crossed with the three
    /// `decided_by` values, through a real upsert against a real file.
    #[test]
    fn the_stream_guard_agrees_with_method_rank() {
        let store = open_test_store("triage_stream_guard_matrix");
        for stored_method in content_item::CLASSIFICATION_METHODS {
            for incoming_method in content_item::CLASSIFICATION_METHODS {
                for decided_by in [
                    crate::rules::DecidedBy::ConfigRule,
                    crate::rules::DecidedBy::Heuristic,
                    crate::rules::DecidedBy::Fallback,
                ] {
                    let id = format!("thread:{stored_method}:{incoming_method}:{decided_by:?}");
                    let mut first = mk_triage(&id, "aktiv");
                    first.classification_method = stored_method.into();
                    first.classification_version = "stored-version".into();
                    store.upsert_triage(&first).unwrap();

                    let mut second = mk_triage(&id, "werbung");
                    second.classification_method = incoming_method.into();
                    second.classification_version = "incoming-version".into();
                    store
                        .upsert_triage_with_rules(&second, &verdict("werbung", decided_by))
                        .unwrap();

                    // The Rust twin of the predicate. A firing rule outranks a
                    // stored model row; a fallback does not.
                    let rule_fired = matches!(
                        decided_by,
                        crate::rules::DecidedBy::ConfigRule | crate::rules::DecidedBy::Heuristic
                    );
                    let preserved = stored_method == content_item::METHOD_HUMAN
                        || (content_item::method_rank(incoming_method)
                            < content_item::method_rank(stored_method)
                            && !(stored_method == content_item::METHOD_MODEL && rule_fired));

                    let row = store.get_triage(&id).unwrap().unwrap();
                    let expected_stream = if preserved { "aktiv" } else { "werbung" };
                    assert_eq!(
                        row.stream, expected_stream,
                        "stored={stored_method} incoming={incoming_method} \
                         decided_by={decided_by:?}"
                    );
                    assert_eq!(
                        row.classification_version,
                        if preserved {
                            "stored-version"
                        } else {
                            "incoming-version"
                        },
                        "stored={stored_method} incoming={incoming_method} \
                         decided_by={decided_by:?}"
                    );
                }
            }
        }
    }

    /// The migration backfills `{prefix}_triage_rules` for rows written before
    /// that table existed, and it partitions them on five rationale literals.
    /// Four of them are `rules::classify`'s own heuristic sentences, copied
    /// into a SQL string where no compiler checks them. A heuristic whose
    /// wording changed would be backfilled as `config_rule` and would then be
    /// invisible to the model rung — or worse, visible to it.
    #[test]
    fn the_backfill_literals_match_the_classifier() {
        use crate::rules::{classify, DecidedBy, MailFacts};
        let cases = [
            ("news@shop.example", "Winter SALE -50% Rabatt", true),
            ("hello@bytes.dev", "This week in AI: new release", true),
            ("noreply@vendor.example", "Ihre Rechnung 2026-07", false),
            ("info@social.example", "Weekly community update", true),
        ];
        let sql = include_str!("store/migrations.rs");
        for (from, subject, list_unsubscribe) in cases {
            let verdict = classify(
                &MailFacts {
                    from,
                    subject,
                    has_list_unsubscribe: list_unsubscribe,
                },
                &[],
            );
            assert_eq!(verdict.decided_by, DecidedBy::Heuristic);
            assert!(
                sql.contains(&format!("'{}'", verdict.rationale)),
                "the migration's heuristic list does not carry {:?}",
                verdict.rationale
            );
        }
        let fallback = classify(
            &MailFacts {
                from: "a.person@example.com",
                subject: "Re: lunch?",
                has_list_unsubscribe: false,
            },
            &[],
        );
        assert_eq!(fallback.decided_by, DecidedBy::Fallback);
        assert!(sql.contains(&format!("rationale = '{}'", fallback.rationale)));
    }

    /// The rung is written in the same transaction as the row, and a resweep
    /// after a rule edit rewrites it rather than keeping the first answer.
    #[test]
    fn the_rules_verdict_is_stored_beside_the_row_and_follows_a_rule_edit() {
        let store = open_test_store("triage_rules_verdict");
        store
            .upsert_triage_with_rules(
                &mk_triage("thread:rules", "aktiv"),
                &verdict("aktiv", crate::rules::DecidedBy::Fallback),
            )
            .unwrap();
        let stored = store
            .triage_rules_verdict("thread:rules")
            .unwrap()
            .expect("the sweep wrote a rung");
        assert_eq!(stored.decided_by, "fallback");
        assert_eq!(stored.stream, "aktiv");
        assert_eq!(stored.rules_version, crate::rules::MAIL_RULES_VERSION);

        store
            .upsert_triage_with_rules(
                &mk_triage("thread:rules", "werbung"),
                &verdict("werbung", crate::rules::DecidedBy::ConfigRule),
            )
            .unwrap();
        let stored = store.triage_rules_verdict("thread:rules").unwrap().unwrap();
        assert_eq!(stored.decided_by, "config_rule");
        assert_eq!(stored.stream, "werbung");

        // A caller with no verdict writes no rung, which is what keeps the
        // existing call sites untouched rather than quietly wrong.
        store
            .upsert_triage(&mk_triage("thread:no-rung", "aktiv"))
            .unwrap();
        assert!(store
            .triage_rules_verdict("thread:no-rung")
            .unwrap()
            .is_none());
    }

    /// A verdict as the pass writes one, with only the fields a test cares
    /// about set.
    fn mk_verdict(
        id: &str,
        model_stream: &str,
        data_class: &str,
        redaction_class: &str,
    ) -> ModelVerdict {
        ModelVerdict {
            triage_id: id.into(),
            mode: "shadow".into(),
            state: "generated".into(),
            rule_decided_by: "fallback".into(),
            rule_stream: "aktiv".into(),
            model_stream: Some(model_stream.into()),
            confidence_bp: Some(8_000),
            urgency_bp: Some(4_000),
            rationale: Some("the model's sentence".into()),
            urgency_rationale: Some("nothing is asked".into()),
            redactions: 0,
            data_class: data_class.into(),
            redaction_class: redaction_class.into(),
            producer: "foundation-models/apple:mail-stream-v1-english".into(),
            item_revision: "revision-1".into(),
            prompt_revision: "mail-stream-v1-english".into(),
            classification_version: "mail-model-v1".into(),
            attempts: 0,
            last_error: None,
            next_attempt: None,
            held_reason: None,
            applied_at: None,
        }
    }

    /// A fallback row with its rung recorded, which is the only shape the model
    /// rung ever sees.
    fn seed_fallback(store: &Store, id: &str) {
        store
            .upsert_triage_with_rules(
                &mk_triage(id, "aktiv"),
                &crate::rules::Verdict {
                    stream: "aktiv".into(),
                    rationale: "No rule matched; kept active as the conservative default.".into(),
                    decided_by: crate::rules::DecidedBy::Fallback,
                },
            )
            .unwrap();
    }

    /// A human correction is also a redaction decision, because `steuern` and
    /// `belege` are Others by rule. Before this, the class settled on a second
    /// endpoint nobody remembers to call, and the dashboard printed "Redacted"
    /// from the class alone — so the gap was invisible in the one place a human
    /// would look.
    #[test]
    fn a_human_stream_change_settles_the_class_and_narrows_in_one_call() {
        let store = open_test_store("triage_human_stream_settles_class");
        let mut item = mk_triage("thread:settle", "aktiv");
        // One token, because the redactor is token-based: a spaced IBAN is
        // six short words to it, and none of them looks like an account.
        item.subject = Some("Rechnung DE89370400440532013000".into());
        item.snippet = Some("Bitte zahlen Sie an DE89370400440532013000".into());
        store.upsert_triage(&item).unwrap();
        assert_eq!(
            store
                .get_triage("thread:settle")
                .unwrap()
                .unwrap()
                .data_class,
            "c1",
            "the setup row starts Mine"
        );

        let write = set_stream(&store, "thread:settle", "belege");
        assert!(write.changed);
        assert!(
            write.class_changed,
            "moving to belege must settle the class"
        );
        assert!(write.narrowed, "and narrow what that class does not admit");

        let row = store.get_triage("thread:settle").unwrap().unwrap();
        assert_eq!(row.stream, "belege");
        assert_eq!(row.classification_method, "human");
        assert_eq!(row.data_class, "c2");
        assert_eq!(row.data_classification_method, "deterministic");
        assert!(
            !row.subject.unwrap().contains("DE89370400440532013000"),
            "the subject still holds what c2 exists to hide"
        );
    }

    /// The one thing a machine may not do here. The class UPDATE is
    /// escalation-only and the narrowing that follows it cannot be undone, so a
    /// wrong `belege` on a thread nobody read would leave a permanently Others
    /// row with a permanently redacted subject.
    #[test]
    fn apply_refuses_a_class_escalating_proposal() {
        let store = open_test_store("triage_apply_refuses_escalation");
        seed_fallback(&store, "thread:escalate");
        let before = store.get_triage("thread:escalate").unwrap().unwrap();

        let verdict = mk_verdict("thread:escalate", "belege", "c1", "c2");
        let error = store
            .apply_model_stream(&verdict)
            .expect_err("a class-raising proposal must be refused")
            .to_string();
        assert!(error.contains("held for a human"), "got {error}");

        let after = store.get_triage("thread:escalate").unwrap().unwrap();
        assert_eq!(after.stream, before.stream);
        assert_eq!(after.classification_method, "deterministic");
        assert_eq!(after.data_class, before.data_class);
        assert_eq!(after.subject, before.subject);
        assert_eq!(after.snippet, before.snippet);
    }

    /// Applying moves the category and nothing else, and a human still outranks
    /// it. `ModelWrite.stream_changed` is false when the guard refused.
    #[test]
    fn apply_writes_the_category_and_loses_to_a_human() {
        let store = open_test_store("triage_apply_writes_category");
        seed_fallback(&store, "thread:apply");
        let write = store
            .apply_model_stream(&mk_verdict("thread:apply", "werbung", "c1", "c1"))
            .unwrap();
        assert!(write.stream_changed);
        let row = store.get_triage("thread:apply").unwrap().unwrap();
        assert_eq!(row.stream, "werbung");
        assert_eq!(row.classification_method, "model");
        assert_eq!(row.classification_version, "mail-model-v1");
        assert_eq!(row.data_class, "c1", "apply never touches the class axis");

        seed_fallback(&store, "thread:human-apply");
        set_stream(&store, "thread:human-apply", "feed");
        let write = store
            .apply_model_stream(&mk_verdict("thread:human-apply", "werbung", "c1", "c1"))
            .unwrap();
        assert!(!write.stream_changed);
        assert_eq!(
            store
                .get_triage("thread:human-apply")
                .unwrap()
                .unwrap()
                .stream,
            "feed"
        );
    }

    /// The loop a second pass would otherwise run: apply writes
    /// `classification_method = 'model'`, and the candidate query filters on
    /// `= 'deterministic'` rather than `<> 'human'`, so the row leaves the
    /// candidate set for good.
    #[test]
    fn a_second_apply_pass_prompts_nothing() {
        let store = open_test_store("triage_second_pass_empty");
        seed_fallback(&store, "thread:once");
        assert_eq!(store.model_rung_candidates().unwrap().len(), 1);

        let verdict = mk_verdict("thread:once", "werbung", "c1", "c1");
        store.apply_model_stream(&verdict).unwrap();
        store.upsert_model_verdict(&verdict).unwrap();

        assert!(
            store.model_rung_candidates().unwrap().is_empty(),
            "an applied row must not be prompted again"
        );
    }

    /// Only the rows the rules did not decide, and only the rows that carry a
    /// stored rung at all.
    #[test]
    fn only_fallback_rows_are_candidates() {
        let store = open_test_store("triage_candidates_are_fallback");
        for (id, stream, decided_by) in [
            ("thread:cfg", "feed", crate::rules::DecidedBy::ConfigRule),
            ("thread:heur", "werbung", crate::rules::DecidedBy::Heuristic),
            ("thread:fall", "aktiv", crate::rules::DecidedBy::Fallback),
        ] {
            store
                .upsert_triage_with_rules(
                    &mk_triage(id, stream),
                    &crate::rules::Verdict {
                        stream: stream.into(),
                        rationale: "a rules result".into(),
                        decided_by,
                    },
                )
                .unwrap();
        }
        // Written by a caller that knows nothing about rungs: no rules row, so
        // no eligibility, which is the conservative reading of "unknown".
        store
            .upsert_triage(&mk_triage("thread:norung", "aktiv"))
            .unwrap();

        let ids: Vec<String> = store
            .model_rung_candidates()
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.id)
            .collect();
        assert_eq!(ids, vec!["thread:fall".to_string()]);

        // And an archived thread has no category decision left to make.
        store.set_triage_status("thread:fall", "archived").unwrap();
        assert!(store.model_rung_candidates().unwrap().is_empty());
    }

    /// Rollback is the config boolean plus this. It puts the deterministic
    /// verdict back exactly, and it does NOT put the class back — which the
    /// route says in words, because it cannot.
    #[test]
    fn revert_restores_the_rules_verdict() {
        let store = open_test_store("triage_revert_restores");
        seed_fallback(&store, "thread:revert");
        let mut verdict = mk_verdict("thread:revert", "werbung", "c1", "c1");
        store.apply_model_stream(&verdict).unwrap();
        verdict.mode = "applied".into();
        verdict.applied_at = Some("2026-09-05 20:00:00+00:00".into());
        store.upsert_model_verdict(&verdict).unwrap();
        // Raise the class the way a later refresh would, so the test can prove
        // the revert does not lower it back.
        store
            .set_triage_data_class("thread:revert", "c2", None)
            .unwrap();

        let (reverted, not_model, no_rules) = store.revert_model_streams(None).unwrap();
        assert_eq!((reverted, not_model, no_rules), (1, 0, 0));

        let rules = store
            .triage_rules_verdict("thread:revert")
            .unwrap()
            .unwrap();
        let row = store.get_triage("thread:revert").unwrap().unwrap();
        assert_eq!(row.stream, rules.stream);
        assert_eq!(row.rationale, rules.rationale);
        assert_eq!(row.classification_method, "deterministic");
        assert_eq!(row.classification_version, rules.rules_version);
        assert_eq!(
            row.data_class, "c2",
            "revert restores the stream axis only; the class is escalation-only"
        );
        let summaries = store.model_verdict_summaries(None).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].mode, "shadow");
    }

    /// The report is meant to be safe to log and to paste. The query that feeds
    /// it cannot return a rationale, which is a stronger guarantee than a
    /// handler that remembers not to serialize one.
    #[test]
    fn the_summary_query_cannot_return_mail_content() {
        let store = open_test_store("triage_summary_no_content");
        seed_fallback(&store, "thread:summary");
        let mut verdict = mk_verdict("thread:summary", "werbung", "c1", "c1");
        verdict.rationale = Some("ZZTOPSECRETTOKEN".into());
        verdict.urgency_rationale = Some("ZZTOPSECRETTOKEN".into());
        store.upsert_model_verdict(&verdict).unwrap();

        let rendered = format!("{:?}", store.model_verdict_summaries(None).unwrap());
        assert!(!rendered.contains("ZZTOPSECRETTOKEN"), "got {rendered}");
        // The reader contract does carry it, because the dashboard renders it
        // beside the rule's rationale.
        let full = store.model_verdicts().unwrap();
        assert_eq!(
            full["thread:summary"].rationale.as_deref(),
            Some("ZZTOPSECRETTOKEN")
        );
    }

    /// A machine write onto the category axis has to leave a trace. The stamp
    /// is derived in SQL from the mode, so it is in the same format and off the
    /// same clock as every other stamp in the file.
    #[test]
    fn an_applied_verdict_carries_the_stamp_of_the_write() {
        let store = open_test_store("triage_verdict_applied_at");
        seed_fallback(&store, "thread:stamped");
        let mut verdict = mk_verdict("thread:stamped", "werbung", "c1", "c1");
        store.upsert_model_verdict(&verdict).unwrap();
        assert!(
            store.model_verdicts().unwrap()["thread:stamped"]
                .applied_at
                .is_none(),
            "a shadow verdict moved nothing, so it is stamped with nothing"
        );

        assert!(store.apply_model_stream(&verdict).unwrap().stream_changed);
        verdict.mode = "applied".into();
        store.upsert_model_verdict(&verdict).unwrap();
        let stamped = store.model_verdicts().unwrap()["thread:stamped"]
            .applied_at
            .clone()
            .expect("an applied verdict carries a stamp");
        assert!(stamped.ends_with("+00:00"), "got {stamped}");

        // Re-storing it does not restamp: the write happened once.
        let mut again = store.model_verdicts().unwrap()["thread:stamped"].clone();
        again.state = "generated".into();
        store.upsert_model_verdict(&again).unwrap();
        assert_eq!(
            store.model_verdicts().unwrap()["thread:stamped"]
                .applied_at
                .as_deref(),
            Some(stamped.as_str())
        );

        // And a revert clears it, because the write it recorded was undone.
        store.revert_model_streams(None).unwrap();
        assert!(store.model_verdicts().unwrap()["thread:stamped"]
            .applied_at
            .is_none());
    }

    /// The gap a later escalation used to leave. A verdict's two sentences are
    /// model text about the mail, redacted once against the class the row held
    /// at prompt time; a row that rises to `c2` afterwards left them behind at
    /// `c1`, and neither the human path nor `POST /triage/redact` reached them.
    #[test]
    fn an_escalation_narrows_the_stored_verdict_too() {
        let store = open_test_store("triage_verdict_escalation");
        seed_fallback(&store, "thread:escalate");
        let mut verdict = mk_verdict("thread:escalate", "aktiv", "c1", "c1");
        verdict.rationale = Some("It quotes DE89370400440532013000 as the account.".into());
        verdict.urgency_rationale = Some("It asks for DE89370400440532013000 today.".into());
        store.upsert_model_verdict(&verdict).unwrap();

        let write = store
            .set_triage_data_class("thread:escalate", "c2", None)
            .unwrap();
        assert!(write.changed);

        let stored = store.model_verdicts().unwrap();
        let after = &stored["thread:escalate"];
        assert!(
            !after.rationale.as_deref().unwrap().contains("DE8937040044"),
            "the verdict still holds what c2 exists to hide: {:?}",
            after.rationale
        );
        assert!(!after
            .urgency_rationale
            .as_deref()
            .unwrap()
            .contains("DE8937040044"));
        assert_eq!(
            after.redaction_class, "c2",
            "the class the text was narrowed against moves with the row"
        );
        assert_eq!(
            after.data_class, "c1",
            "the class at PROMPT time is a receipt and does not move"
        );
    }

    /// A class that refuses prompts outright loses the sentences rather than
    /// narrowing them: the rung would never have produced them for a `c3` row.
    #[test]
    fn an_escalation_to_secret_drops_the_verdict_text() {
        let store = open_test_store("triage_verdict_secret");
        seed_fallback(&store, "thread:secret");
        let verdict = mk_verdict("thread:secret", "aktiv", "c1", "c1");
        store.upsert_model_verdict(&verdict).unwrap();

        store
            .set_triage_data_class("thread:secret", "c3", None)
            .unwrap();

        let stored = store.model_verdicts().unwrap();
        assert_eq!(stored["thread:secret"].rationale, None);
        assert_eq!(stored["thread:secret"].urgency_rationale, None);
        assert_eq!(stored["thread:secret"].redaction_class, "c3");
    }

    /// The remediation route reaches the verdict as well as the item, on a row
    /// whose subject a sweep already cleaned. That row is exactly the one a
    /// `changed`-only loop skipped.
    #[test]
    fn the_remediation_write_narrows_a_verdict_on_an_already_clean_row() {
        let store = open_test_store("triage_redact_reaches_verdict");
        seed_fallback(&store, "thread:remediate");
        let mut verdict = mk_verdict("thread:remediate", "aktiv", "c1", "c1");
        verdict.rationale = Some("Sent from DE89370400440532013000.".into());
        store.upsert_model_verdict(&verdict).unwrap();
        // The class rises by a route that does not narrow, the way a row
        // predating the gate got here.
        store
            .refresh_triage_data_class(
                "thread:remediate",
                &content_item::DataClass::new(
                    "c2",
                    crate::intake::KNOWN_PERSON_RATIONALE,
                    content_item::METHOD_DETERMINISTIC,
                    content_item::MAIL_CLASSIFIER_VERSION,
                ),
            )
            .unwrap();

        let write = store
            .redact_triage_review_fields("thread:remediate", "c2", Some("SALE"), Some("snippet"))
            .unwrap();
        assert!(write.verdict_narrowed, "the verdict was in scope");
        assert!(!store.model_verdicts().unwrap()["thread:remediate"]
            .rationale
            .as_deref()
            .unwrap()
            .contains("DE8937040044"));

        // Run it twice and the second run reports nothing left to do.
        let again = store
            .redact_triage_review_fields("thread:remediate", "c2", Some("SALE"), Some("snippet"))
            .unwrap();
        assert!(!again.verdict_narrowed);
    }

    #[test]
    fn reviewed_cloud_derivative_is_staged_and_becomes_stale_with_its_source() {
        let store = open_test_store("cloud_derivative");
        let approval = CloudDerivativeApproval {
            source: "mail".into(),
            item_id: "thread:cloud".into(),
            source_revision: "source-v1".into(),
            preview_hash: "preview-v1".into(),
            // Was the strictest class until the column stopped accepting it.
            // Nothing else about this test is about the class — it is about
            // staleness — and that value had only ever reached this row through
            // a `prepare()` that used to hand out a preview for local-only
            // content.
            original_data_class: "c1".into(),
            derivative_data_class: "c1".into(),
            transformation: "deterministic-entity-redaction-v2".into(),
            document: "Title\n[identity removed]".into(),
            redaction_count: 1,
        };

        let staged = store.stage_cloud_derivative(&approval).unwrap();
        assert_eq!(staged.status, "staged");
        assert_eq!(staged.preview_hash.as_deref(), Some("preview-v1"));
        assert_eq!(staged.dispatch_status, "not_queued");
        assert_eq!(staged.provider_calls, 0);

        let current = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(current.status, "staged");

        let stale = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v2", "preview-v2")
            .unwrap();
        assert_eq!(stale.status, "stale");
        assert_eq!(stale.provider_calls, 0);

        let changed_policy = store
            .cloud_derivative_state("mail", "thread:cloud", "source-v1", "preview-v2")
            .unwrap();
        assert_eq!(changed_policy.status, "stale");
    }

    /// The column carries the same refusal `cloud_derivative::prepare` does.
    /// Belt and braces on purpose: `prepare` is the door every current caller
    /// goes through, and the CHECK is what still holds if a future one does not
    /// — or if an older binary is pointed at this database.
    #[test]
    fn a_local_only_derivative_cannot_be_staged_at_all() {
        let store = open_test_store("cloud_local_only_refused");
        let error = store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:secret".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "c3".into(),
                derivative_data_class: "c1".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Title\n[identity removed]".into(),
                redaction_count: 1,
            })
            .expect_err("c3 must not be stageable");
        // Matched on the column and its allowed set rather than on a constraint
        // *name*: Postgres named the constraint and quoted the name back; SQLite
        // quotes the CHECK expression itself. The claim is the same one -- the
        // refusal comes from the class constraint, not from Rust -- and this is
        // how the database states it now.
        assert!(
            format!("{error:?}").contains("original_data_class IN ('c0','c1')"),
            "the refusal must come from the class CHECK, got: {error:?}"
        );
    }

    #[test]
    fn approved_derivative_queues_once_for_an_explicit_cloud_role() {
        let store = open_test_store("cloud_queue");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:queue".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "c1".into(),
                derivative_data_class: "c1".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Title\n[person]".into(),
                redaction_count: 1,
            })
            .unwrap();
        let request = CloudQueueRequest {
            source: "mail".into(),
            item_id: "thread:queue".into(),
            source_revision: "source-v1".into(),
            preview_hash: "preview-v1".into(),
            provider_role: "cloud_summarization".into(),
            task: crate::cloud_dispatch::TASK_VERSION.into(),
        };

        let first = store.queue_cloud_derivative(&request).unwrap();
        let again = store.queue_cloud_derivative(&request).unwrap();
        assert_eq!(first.dispatch_status, "queued");
        assert_eq!(first.job_id, again.job_id);
        assert_eq!(first.provider_calls, 0);

        let state = store
            .cloud_derivative_state("mail", "thread:queue", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(state.dispatch_status, "queued");
        assert_eq!(state.provider_role.as_deref(), Some("cloud_summarization"));
        let stale = CloudQueueRequest {
            preview_hash: "stale".into(),
            ..request
        };
        assert!(store.queue_cloud_derivative(&stale).is_err());
    }

    #[test]
    fn queued_cloud_job_claims_once_and_persists_a_bounded_result() {
        let store = open_test_store("cloud_dispatch");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:dispatch".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "c1".into(),
                derivative_data_class: "c1".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Title\n[person] visits on 2026-08-10".into(),
                redaction_count: 1,
            })
            .unwrap();
        let queued = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: "mail".into(),
                item_id: "thread:dispatch".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                provider_role: "cloud_summarization".into(),
                task: crate::cloud_dispatch::TASK_VERSION.into(),
            })
            .unwrap();
        let job_id = queued.job_id.unwrap();
        let job = store.cloud_job_for_dispatch(&job_id).unwrap().unwrap();
        assert_eq!(job.task, "content-analysis-v1");
        assert_eq!(job.original_data_class, "c1");
        assert_eq!(job.derivative_data_class, "c1");
        assert_eq!(job.transformation, "deterministic-entity-redaction-v2");
        assert_eq!(job.document, "Title\n[person] visits on 2026-08-10");
        assert_eq!(job.provider_calls, 0);
        let attempt_id = match store
            .claim_cloud_job_attempt(&job_id, "cloud_summarization", "model-a", 10)
            .unwrap()
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        assert_eq!(
            store
                .claim_cloud_job_attempt(&job_id, "cloud_summarization", "model-a", 10)
                .unwrap(),
            CloudAttemptClaim::JobUnavailable
        );

        let result = serde_json::json!({
            "schema_version": "cloud-content-analysis-v1",
            "summary": "A visit is planned.",
            "importance": "high",
            "importance_rationale": "A fixed date is present.",
            "important_dates": [{ "label": "Visit", "date": "2026-08-10", "source_text": "2026-08-10" }],
            "action_items": [],
            "topics": ["travel"]
        });
        assert!(store
            .complete_cloud_job_attempt(&job_id, attempt_id, &result)
            .unwrap());

        let state = store
            .cloud_derivative_state("mail", "thread:dispatch", "source-v1", "preview-v1")
            .unwrap();
        assert_eq!(state.dispatch_status, "succeeded");
        assert_eq!(state.provider_calls, 1);
        assert_eq!(
            state.result.unwrap()["important_dates"][0]["date"],
            "2026-08-10"
        );
        assert!(store.cloud_job_for_dispatch(&job_id).unwrap().is_none());

        let conn = store.conn().unwrap();
        let attempt = conn
            .query_row(
                &format!(
                    "SELECT provider_role, model, preview_hash, status, result_json
                     FROM {}_content_cloud_attempts WHERE attempt_id = ?1",
                    store.prefix
                ),
                params![attempt_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(attempt.0, "cloud_summarization");
        assert_eq!(attempt.1, "model-a");
        assert_eq!(attempt.2, "preview-v1");
        assert_eq!(attempt.3, "succeeded");
        assert!(attempt.4.is_some());
    }

    /// The evidence a roster needs to stop leading with a provider that is refusing.
    ///
    /// `cloud_provider_calls_today` answers "may I spend another", which said 81 remaining while
    /// Cloudflare answered 429 to 119 consecutive requests on 2026-08-30. This answers the other
    /// question -- would spending it work -- and a single success has to clear it, or a provider
    /// coming back stays shut out.
    #[test]
    fn recent_provider_outcomes_separate_a_bad_hour_from_a_spent_budget() {
        let store = open_test_store("cloud_provider_health");
        let queue = |item: &str| {
            store
                .stage_cloud_derivative(&CloudDerivativeApproval {
                    source: "mail".into(),
                    item_id: item.into(),
                    source_revision: "source-v1".into(),
                    preview_hash: "preview-v1".into(),
                    original_data_class: "c1".into(),
                    derivative_data_class: "c1".into(),
                    transformation: "deterministic-entity-redaction-v3".into(),
                    document: "Reviewed pseudonymized text".into(),
                    redaction_count: 1,
                })
                .unwrap();
            store
                .queue_cloud_derivative(&CloudQueueRequest {
                    source: "mail".into(),
                    item_id: item.into(),
                    source_revision: "source-v1".into(),
                    preview_hash: "preview-v1".into(),
                    provider_role: "cloud_flaky".into(),
                    task: crate::cloud_dispatch::TASK_VERSION.into(),
                })
                .unwrap()
                .job_id
                .unwrap()
        };

        assert_eq!(
            store
                .cloud_provider_recent_outcomes("cloud_flaky", 60)
                .unwrap(),
            (0, 0),
            "a role nobody has called yet is not failing"
        );

        // One job each, which is how the digest drain actually produces them.
        for item in ["thread:a", "thread:b", "thread:c"] {
            let job_id = queue(item);
            let attempt_id = match store
                .claim_cloud_job_attempt(&job_id, "cloud_flaky", "model-a", 100)
                .unwrap()
            {
                CloudAttemptClaim::Started(attempt_id) => attempt_id,
                other => panic!("unexpected claim: {other:?}"),
            };
            store
                .fail_cloud_job_attempt(&job_id, attempt_id, "cloud provider returned HTTP 429")
                .unwrap();
        }
        assert_eq!(
            store
                .cloud_provider_recent_outcomes("cloud_flaky", 60)
                .unwrap(),
            (3, 0)
        );

        // And it clears on the first thing that works.
        let job_id = queue("thread:d");
        let attempt_id = match store
            .claim_cloud_job_attempt(&job_id, "cloud_flaky", "model-a", 100)
            .unwrap()
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        store
            .complete_cloud_job_attempt(&job_id, attempt_id, &serde_json::json!({"ok": true}))
            .unwrap();
        let (failed, succeeded) = store
            .cloud_provider_recent_outcomes("cloud_flaky", 60)
            .unwrap();
        assert_eq!((failed, succeeded), (3, 1));
        assert!(
            succeeded > 0,
            "one success is what lets the role lead the roster again"
        );

        // Scoped per role, which is the whole point: one provider having a bad hour must not
        // push the roster past the one that is answering.
        assert_eq!(
            store
                .cloud_provider_recent_outcomes("cloud_healthy", 60)
                .unwrap(),
            (0, 0),
            "another role's failures are not this role's"
        );
    }

    #[test]
    fn cloud_daily_ceiling_blocks_before_a_second_provider_attempt() {
        let store = open_test_store("cloud_daily_budget");
        store
            .stage_cloud_derivative(&CloudDerivativeApproval {
                source: "mail".into(),
                item_id: "thread:budget".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                original_data_class: "c1".into(),
                derivative_data_class: "c1".into(),
                transformation: "deterministic-entity-redaction-v2".into(),
                document: "Reviewed pseudonymized text".into(),
                redaction_count: 1,
            })
            .unwrap();
        let job_id = store
            .queue_cloud_derivative(&CloudQueueRequest {
                source: "mail".into(),
                item_id: "thread:budget".into(),
                source_revision: "source-v1".into(),
                preview_hash: "preview-v1".into(),
                provider_role: "cloud_primary".into(),
                task: crate::cloud_dispatch::TASK_VERSION.into(),
            })
            .unwrap()
            .job_id
            .unwrap();
        let attempt_id = match store
            .claim_cloud_job_attempt(&job_id, "cloud_primary", "model-a", 1)
            .unwrap()
        {
            CloudAttemptClaim::Started(attempt_id) => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        assert!(store
            .fail_cloud_job_attempt(&job_id, attempt_id, "synthetic provider failure")
            .unwrap());
        assert_eq!(
            store.cloud_provider_calls_today("cloud_primary").unwrap(),
            1
        );
        assert_eq!(
            store
                .claim_cloud_job_attempt(&job_id, "cloud_primary", "model-a", 1)
                .unwrap(),
            CloudAttemptClaim::DailyLimitReached
        );
        assert_eq!(
            store
                .cloud_job_for_dispatch(&job_id)
                .unwrap()
                .unwrap()
                .provider_calls,
            1,
            "policy rejection must not consume another provider call"
        );
    }

    #[test]
    fn triage_relevance_replaces_stale_profiles_without_changing_the_proposal() {
        let store = open_test_store("triage_relevance");
        store
            .upsert_triage(&mk_triage("thread:relevant", "feed"))
            .unwrap();
        let first = vec![
            RelevanceMatch {
                profile_key: "systems".into(),
                profile_label: "Systems".into(),
                score: 0.9,
                rationale: "Semantic similarity for Systems".into(),
                mode: "semantic".into(),
                profile_revision: "systems-v1".into(),
            },
            RelevanceMatch {
                profile_key: "travel".into(),
                profile_label: "Travel".into(),
                score: 0.4,
                rationale: "Semantic similarity for Travel".into(),
                mode: "semantic".into(),
                profile_revision: "travel-v1".into(),
            },
        ];
        store
            .replace_triage_relevance("thread:relevant", &first)
            .unwrap();
        store
            .replace_triage_relevance("thread:relevant", &first[..1])
            .unwrap();

        let stored = store.triage_relevance("thread:relevant").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].profile_key, "systems");
        let proposal = store.list_triage(None).unwrap().remove(0);
        assert_eq!(proposal.stream, "feed");
        assert_eq!(proposal.status, "proposed");
    }

    /// Critical: a human's triage decision must survive a re-sweep of the same
    /// thread. upsert_triage's ON CONFLICT must not touch `status`.
    #[test]
    fn upsert_preserves_status_across_refetch_triage() {
        let store = open_test_store("triage_preserve");
        let item = mk_triage("thread:keep", "werbung");
        store.upsert_triage(&item).unwrap();
        store.set_triage_status("thread:keep", "dismissed").unwrap();

        // Re-sweep: same id, fresh rationale/stream.
        let mut refetched = mk_triage("thread:keep", "feed");
        refetched.rationale = "re-swept".into();
        assert!(!store.upsert_triage(&refetched).unwrap(), "still same id");

        assert_eq!(
            store.get_triage_status("thread:keep").unwrap().as_deref(),
            Some("dismissed"),
            "dismiss decision must survive re-sweep"
        );
        // Other fields legitimately update.
        let rows = store.list_triage(None).unwrap();
        assert_eq!(rows[0].rationale, "re-swept");
    }

    /// The fail-closed default, proved against the database rather than against
    /// the constructor. A pasted URL is what an operator ingests from a page
    /// they were logged into, and it must not come back cloud-eligible.
    #[test]
    fn an_ingested_item_nobody_declared_is_stored_c1_and_legacy() {
        let store = open_test_store("feed_class_default");
        let item = mk_feed("https://example.com/undeclared", "article", "news");
        store.upsert_feed(&item).unwrap();

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c1");
        assert_eq!(stored.data_classification_method, "legacy");
        assert_eq!(
            content_item::processing_policy(&stored.data_class).cloud_handling,
            "pseudonymization_required"
        );
    }

    /// A collector declares at discovery. That is the whole window: the class
    /// lands with the INSERT, and every later pass may only raise it.
    #[test]
    fn a_collector_declares_its_class_when_it_first_stores_the_item() {
        let store = open_test_store("feed_class_declared");
        let mut item = mk_feed("https://example.com/declared", "arxiv", "news");
        item.declare_class(&content_item::DataClass::declared_by_source(
            "c0",
            "Declared by feed source 'arxiv-ai-recent'.",
        ));
        store.upsert_feed(&item).unwrap();

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c0");
        assert_eq!(stored.data_classification_method, "deterministic");
        assert_eq!(
            content_item::processing_policy(&stored.data_class).cloud_handling,
            "eligible",
            "a positively declared c0 item is the only kind that is"
        );
    }

    /// Ingest is a machine path: it may raise a class and never lower one.
    ///
    /// The first assertion is the one the anti-claim rests on. A row stored
    /// undeclared is c1, and no collector can relabel it afterwards -- so a
    /// `legacy` item has no machine route to c0 at all, and the 187 backfilled
    /// rows can only be lifted by a human who says why.
    #[test]
    fn a_re_ingest_can_raise_a_feed_class_but_never_lower_one() {
        let store = open_test_store("feed_class_escalation");
        let mut item = mk_feed("https://example.com/escalate", "article", "news");
        store.upsert_feed(&item).unwrap();

        item.declare_class(&content_item::DataClass::declared_by_source(
            "c0",
            "Declared by feed source 'test'.",
        ));
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(
            stored.data_class, "c1",
            "a collector cannot lift a row that was already stored undeclared"
        );
        assert_eq!(stored.data_classification_method, "legacy");

        // Escalation, on the other hand, needs nobody's permission.
        item.declare_class(&content_item::DataClass::declared_by_source(
            "c3",
            "Declared Secret by its collector.",
        ));
        store.upsert_feed(&item).unwrap();
        assert_eq!(store.get_feed(&item.id).unwrap().unwrap().data_class, "c3");

        // A human lowers it, with a reason. The collector then re-scans and
        // re-declares Secret, which is an escalation, so that one lands.
        store
            .set_feed_data_class(&item.id, "c0", Some("Published preprint."))
            .unwrap();
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c3");
        assert_eq!(stored.data_classification_method, "deterministic");
    }

    /// The erasure a verifier found live on 2026-08-13, pinned in the shape it
    /// happened: an authenticated `POST /ingest` of a URL already in the feed
    /// reverted a hand-classified arXiv row's method to `legacy` and replaced
    /// the operator's rationale with the undeclared-default sentence. The class
    /// never moved, which is why the escalation rule waved it through — at
    /// equal class the only thing an ingest can write is the record of who
    /// decided, and that record was the one thing worth keeping.
    #[test]
    fn a_re_ingest_at_equal_class_keeps_a_humans_feed_record() {
        let store = open_test_store("feed_class_human_record");
        let mut item = mk_feed("https://arxiv.org/abs/1706.03762", "article", "news");
        store.upsert_feed(&item).unwrap();
        assert!(store
            .set_feed_data_class(
                &item.id,
                "c1",
                Some("Hand-ingested from a page I was reading; the capture is mine.")
            )
            .unwrap());

        // The probe: the same URL ingested again, declaring nothing, so the
        // item arrives c1/legacy -- equal class, machine method.
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c1");
        assert_eq!(
            stored.data_classification_method, "human",
            "the re-ingest reverted a human decision to a machine one"
        );
        assert_eq!(
            stored.data_class_rationale,
            "Hand-ingested from a page I was reading; the capture is mine.",
            "the operator's own words were overwritten by the default sentence"
        );
        assert_eq!(stored.data_classification_version, "manual-v1");

        // Escalation is untouched by any of this: the collector may still raise
        // the class of a row a human classified, record and all.
        item.declare_class(&content_item::DataClass::declared_by_source(
            "c3",
            "Declared Secret by its collector.",
        ));
        store.upsert_feed(&item).unwrap();
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c3");
        assert_eq!(stored.data_classification_method, "deterministic");
    }

    /// The de-escalation door: it opens for a human with a reason, and for
    /// nobody else. `Ok(false)` is reserved for a missing item, so a refusal is
    /// distinguishable from a typo'd id — that distinction is what lets the
    /// server answer 400 rather than 404.
    #[test]
    fn lowering_a_feed_class_needs_a_written_reason_and_says_so() {
        let store = open_test_store("feed_class_deescalation");
        let mut item = mk_feed("https://example.com/lower", "article", "news");
        item.declare_class(&content_item::DataClass::declared_by_source(
            "c3",
            "Declared Secret by its collector.",
        ));
        store.upsert_feed(&item).unwrap();

        for empty in [None, Some(""), Some("   ")] {
            let error = store
                .set_feed_data_class(&item.id, "c0", empty)
                .expect_err("a silent de-escalation must be refused");
            assert!(
                error.to_string().contains("rationale"),
                "the refusal must name what is missing, got: {error}"
            );
        }
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().data_class,
            "c3",
            "a refused request writes nothing"
        );

        assert!(store
            .set_feed_data_class(&item.id, "c0", Some("Published preprint, no session."))
            .unwrap());
        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(stored.data_class, "c0");
        assert_eq!(
            stored.data_class_rationale, "Published preprint, no session.",
            "the operator's own words are what gets stored"
        );

        assert!(
            !store.set_feed_data_class("no-such-id", "c3", None).unwrap(),
            "a missing item is false, not an error"
        );
        assert!(
            store
                .set_feed_data_class(&item.id, "confidential", Some("why"))
                .is_err(),
            "a class outside the vocabulary is refused"
        );
    }

    /// The mail sweep re-runs the rules on every pass. A rule edit that made
    /// the classifier less suspicious must not walk the inbox downgrading rows
    /// it had already called Secret.
    #[test]
    fn a_resweep_cannot_downgrade_a_mail_the_rules_once_called_secret() {
        let store = open_test_store("triage_class_escalation");
        let mut item = mk_triage("thread:class", "aktiv");
        item.data_class = "c3".into();
        item.data_class_rationale = "Authentication metadata is Secret.".into();
        item.data_classification_method = content_item::METHOD_DETERMINISTIC.into();
        store.upsert_triage(&item).unwrap();

        item.data_class = "c1".into();
        item.data_class_rationale = "Mail metadata is Mine by default.".into();
        store.upsert_triage(&item).unwrap();

        let stored = store.list_triage(None).unwrap();
        assert_eq!(stored[0].data_class, "c3");
        assert_eq!(
            stored[0].data_class_rationale,
            "Authentication metadata is Secret."
        );
    }

    #[test]
    fn feed_upsert_is_idempotent_and_coalesces_summary() {
        let store = open_test_store("feed_idem");
        let url = "https://youtu.be/xyz";
        let mut item = mk_feed(url, "youtube", "media");
        assert!(store.upsert_feed(&item).unwrap(), "first insert is new");

        // Give it a summary out of band (as summarize would).
        store
            .update_feed_summary(&item.id, "distilled", "test-summarizer-v1")
            .unwrap();

        // Re-ingest with summary=None must NOT wipe the stored summary.
        item.summary = None;
        item.title = Some("Better Title".into());
        assert!(!store.upsert_feed(&item).unwrap(), "second is not new");

        let stored = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(
            stored.summary.as_deref(),
            Some("distilled"),
            "summary preserved via COALESCE"
        );
        assert_eq!(
            stored.title.as_deref(),
            Some("Better Title"),
            "title updated"
        );

        item.summary = Some("older imported summary".into());
        item.summary_provenance = Some(StageProvenance::legacy("old-import"));
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store
                .get_feed(&item.id)
                .unwrap()
                .unwrap()
                .summary
                .as_deref(),
            Some("distilled"),
            "a legacy summary cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        assert_eq!(
            stages
                .iter()
                .find(|stage| stage.stage == "summary")
                .unwrap()
                .tier,
            "model"
        );
    }

    #[test]
    fn relevance_replace_removes_stale_profiles_without_touching_status() {
        let store = open_test_store("feed_relevance_replace");
        let item = mk_feed("https://example.com/relevant", "article", "news");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper", "api").unwrap();
        let first = vec![
            RelevanceMatch {
                profile_key: "a".into(),
                profile_label: "Polymath".into(),
                score: 0.8,
                rationale: "match".into(),
                mode: "reranked".into(),
                profile_revision: "one".into(),
            },
            RelevanceMatch {
                profile_key: "b".into(),
                profile_label: "Career".into(),
                score: 0.4,
                rationale: "match".into(),
                mode: "reranked".into(),
                profile_revision: "one".into(),
            },
        ];
        store.replace_feed_relevance(&item.id, &first).unwrap();
        store.replace_feed_relevance(&item.id, &first[..1]).unwrap();
        let stored = store.feed_relevance(&item.id).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].profile_label, "Polymath");
        assert_eq!(stored[0].mode, "reranked");
        assert_eq!(
            store.get_feed_status(&item.id).unwrap().as_deref(),
            Some("keeper")
        );
    }

    #[test]
    fn quality_flags_round_trip_replaces_stale_reasons_and_keeps_item_status() {
        let store = open_test_store("feed_quality_flags");
        let item = mk_feed("https://example.com/quality", "article", "news");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper", "api").unwrap();

        store
            .replace_feed_quality_flags(
                &item.id,
                &[
                    QualityFlag {
                        signal: "retention".into(),
                        reason: "retention fired: old reason".into(),
                        evidence: "retained=95.0%".into(),
                    },
                    QualityFlag {
                        signal: "summary_attempts".into(),
                        reason: "summary_attempts fired: retrying".into(),
                        evidence: "attempts=2; last_error=timeout".into(),
                    },
                ],
            )
            .unwrap();
        store
            .replace_feed_quality_flags(
                &item.id,
                &[QualityFlag {
                    signal: "retention".into(),
                    reason: "retention fired: current reason".into(),
                    evidence: "retained=92.0%".into(),
                }],
            )
            .unwrap();

        let rows = store.feed_quality_review_queue(20).unwrap();
        assert_eq!(rows.len(), 1, "signals absent from the new set are removed");
        assert_eq!(rows[0].signal, "retention");
        assert_eq!(rows[0].reason, "retention fired: current reason");
        assert_eq!(rows[0].evidence, "retained=92.0%");
        assert_eq!(rows[0].status, "keeper", "flagging never owns status");

        store.replace_feed_quality_flags(&item.id, &[]).unwrap();
        assert!(store.feed_quality_review_queue(20).unwrap().is_empty());
    }

    #[test]
    fn evaluation_replace_round_trips_factors_and_revision() {
        let store = open_test_store("feed_evaluation_replace");
        let item = mk_feed("https://example.com/evaluated", "article", "news");
        store.upsert_feed(&item).unwrap();
        let mut evaluation = FeedEvaluation {
            feed_id: item.id.clone(),
            overall_score: 0.72,
            explanation: "transparent".into(),
            mode: "reranked".into(),
            item_revision: "item-one".into(),
            context_revision: "context-one".into(),
            evaluator_revision: "feed-evaluator-v4-english".into(),
            evaluated_at: String::new(),
            factors: vec![EvaluationFactor {
                key: "interest".into(),
                label: "Interest fit".into(),
                score: 0.8,
                weight: 0.6,
                rationale: "matched".into(),
                context: Some(EvaluationFactorContext {
                    kind: "trip".into(),
                    id: "trip:one".into(),
                    label: "Berlin".into(),
                    date_start: Some("2026-09-10".into()),
                    date_end: Some("2026-09-12".into()),
                    matched_terms: vec!["Berlin".into()],
                }),
            }],
        };
        store.replace_feed_evaluation(&evaluation).unwrap();
        evaluation.overall_score = 0.4;
        evaluation.item_revision = "item-two".into();
        evaluation.factors[0].score = 0.3;
        store.replace_feed_evaluation(&evaluation).unwrap();

        let stored = store.feed_evaluation(&item.id).unwrap().unwrap();
        assert_eq!(stored.item_revision, "item-two");
        assert_eq!(stored.mode, "reranked");
        assert_eq!(stored.factors.len(), 1);
        assert_eq!(stored.factors[0].score, 0.3);
        assert_eq!(
            stored.factors[0]
                .context
                .as_ref()
                .map(|context| context.id.as_str()),
            Some("trip:one")
        );
        assert_eq!(store.evaluation_summary().unwrap().reranked, 1);

        let mut lower = evaluation.clone();
        lower.mode = "lexical".into();
        lower.overall_score = 0.1;
        lower.evaluator_revision = "fallback-v2".into();
        assert!(!store.replace_feed_evaluation(&lower).unwrap());
        assert_eq!(
            store
                .feed_evaluation(&item.id)
                .unwrap()
                .unwrap()
                .overall_score,
            0.4,
            "a deterministic ranking cannot replace a model-tier result"
        );
        let stages = store.feed_stage_results(&item.id).unwrap();
        let ranking = stages
            .iter()
            .find(|stage| stage.stage == "ranking")
            .unwrap();
        assert_eq!(ranking.tier, "model");
        assert_eq!(ranking.revision, "feed-evaluator-v4-english");
    }

    #[test]
    fn travel_context_snapshot_round_trips() {
        let store = open_test_store("travel_context_snapshot");
        store
            .replace_travel_context_snapshot("revision-one", "[{\"id\":\"trip:one\"}]")
            .unwrap();
        let snapshot = store.travel_context_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.revision, "revision-one");
        assert_eq!(snapshot.payload, "[{\"id\":\"trip:one\"}]");
        assert!(!snapshot.refreshed_at.is_empty());
    }

    #[test]
    fn enrichment_ledger_and_attempts_cap() {
        let store = open_test_store("enrichment_ledger");
        let mut item = mk_feed("https://example.com/failed-item", "article", "news");
        item.transcript = Some("Short transcript for test".into());
        item.content_status = "thin".into();
        store.upsert_feed(&item).unwrap();

        // Initially 1 pending summary, 0 failed.
        let counts = store.feed_enrichment_counts(None).unwrap();
        assert_eq!(counts.pending_summaries, 1);
        assert_eq!(counts.failed_summaries, 0);

        let status_counts = store.feed_content_status_counts().unwrap();
        assert_eq!(status_counts.thin, 1);

        // Record 3 failed attempts.
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();
        store
            .record_summary_attempt(&item.id, "http_error", "summary-v1")
            .unwrap();

        // Item should now be marked failed (summary_attempts >= 3) and no longer returned by feed_pending_summaries.
        let pending = store.feed_pending_summaries(Some("summary-v1")).unwrap();
        assert!(pending.iter().all(|i| i.id != item.id));

        let counts_after = store.feed_enrichment_counts(Some("summary-v1")).unwrap();
        assert_eq!(counts_after.pending_summaries, 0);
        assert_eq!(counts_after.failed_summaries, 1);

        // A new producer revision gets its own bounded retry ledger.
        let new_revision = store.feed_enrichment_counts(Some("summary-v2")).unwrap();
        assert_eq!(new_revision.pending_summaries, 1);
        assert_eq!(new_revision.failed_summaries, 0);
        store
            .record_summary_attempt(&item.id, "timeout", "summary-v2")
            .unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().summary_attempts,
            1
        );

        // Updating summary resets attempt counters.
        store
            .update_feed_summary(&item.id, "Summary fixed", "test-summarizer-v1")
            .unwrap();
        let fetched = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(fetched.summary.as_deref(), Some("Summary fixed"));
        assert_eq!(fetched.summary_attempts, 0);
        assert!(fetched.summary_last_error.is_none());
        assert!(fetched.summary_next_attempt.is_none());
    }

    #[test]
    fn feed_list_filters_stream_and_dismissed() {
        let store = open_test_store("feed_list");
        store
            .upsert_feed(&mk_feed("https://youtu.be/a", "youtube", "media"))
            .unwrap();
        store
            .upsert_feed(&mk_feed("https://example.com/post", "article", "news"))
            .unwrap();
        let dismissed = mk_feed("https://youtu.be/b", "youtube", "media");
        store.upsert_feed(&dismissed).unwrap();
        store
            .set_feed_status(&dismissed.id, "dismissed", "api")
            .unwrap();

        let media = store.list_feed(Some("media"), None, 7, false).unwrap();
        assert_eq!(
            media.len(),
            1,
            "one visible media item (other is dismissed)"
        );
        assert!(media[0].transcript.is_none(), "list view omits transcript");

        let media_all = store.list_feed(Some("media"), None, 7, true).unwrap();
        assert_eq!(media_all.len(), 2, "include_dismissed shows both");

        let news = store.list_feed(Some("news"), None, 7, false).unwrap();
        assert_eq!(news.len(), 1);
    }

    #[test]
    fn feed_origins_queries_and_grouping() {
        let store = open_test_store("feed_origins");
        let item1 = mk_feed("https://example.com/item1", "article", "news");
        let item2 = mk_feed("https://example.com/item2", "article", "news");
        store.upsert_feed(&item1).unwrap();
        store.upsert_feed(&item2).unwrap();

        store
            .record_feed_origin(
                &item1.id,
                "github-trending",
                "https://github.com/trending",
                Some("Trending Repo 1"),
            )
            .unwrap();
        store
            .record_feed_origin(
                &item2.id,
                "github-trending",
                "https://github.com/trending",
                Some("Trending Repo 2"),
            )
            .unwrap();
        store
            .record_feed_origin(
                &item1.id,
                "vault-scan",
                "/notes/ai.md",
                Some("Obsidian Link"),
            )
            .unwrap();

        let origins1 = store.feed_origins(&item1.id).unwrap();
        assert_eq!(origins1.len(), 2);

        let filtered = store
            .list_feed(None, Some("github-trending"), 7, false)
            .unwrap();
        assert_eq!(filtered.len(), 2);

        let filtered_vault = store.list_feed(None, Some("vault-scan"), 7, false).unwrap();
        assert_eq!(filtered_vault.len(), 1);

        let summaries = store.list_origin_summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        let gh_summary = summaries
            .iter()
            .find(|s| s.source_id == "github-trending")
            .unwrap();
        assert_eq!(gh_summary.item_count, 2);
    }

    /// Critical: a keeper/dismiss decision must survive a re-ingest of the same
    /// URL. upsert_feed's ON CONFLICT must not touch `status`.
    #[test]
    fn upsert_preserves_status_across_refetch_feed() {
        let store = open_test_store("feed_preserve");
        let item = mk_feed("https://youtu.be/keepme", "youtube", "media");
        store.upsert_feed(&item).unwrap();
        store.set_feed_status(&item.id, "keeper", "api").unwrap();

        let mut refetched = mk_feed("https://youtu.be/keepme", "youtube", "media");
        refetched.title = Some("Re-ingested".into());
        assert!(!store.upsert_feed(&refetched).unwrap(), "still same id");

        assert_eq!(
            store.get_feed_status(&item.id).unwrap().as_deref(),
            Some("keeper"),
            "keeper decision must survive re-ingest"
        );
    }

    #[test]
    fn runs_are_derived_from_arrival_gaps_and_leave_ungrouped_items_alone() {
        let store = open_test_store("feed_runs");

        let together_a = mk_feed("https://github.com/o/a", "github", "news");
        let together_b = mk_feed("https://github.com/o/b", "github", "news");
        let later = mk_feed("https://github.com/o/c", "github", "news");
        let manual = mk_feed("https://example.com/pasted", "article", "news");
        for item in [&together_a, &together_b, &later, &manual] {
            store.upsert_feed(item).unwrap();
        }

        for item in [&together_a, &together_b, &later] {
            store
                .record_feed_origin(
                    &item.id,
                    "gh-trending",
                    "https://github.com/trending",
                    Some("GitHub Trending (daily)"),
                )
                .unwrap();
        }

        // Push one item's arrival two hours back: same source, different run.
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {prefix}_feed_origins SET first_seen = {past} WHERE feed_id = ?1",
                    prefix = store.prefix,
                    past = sjel_store::now_offset("'-2 hours'")
                ),
                params![&later.id],
            )
            .unwrap();

        let runs = store.list_feed_runs(7).unwrap();
        let key_of = |id: &str| {
            runs.iter()
                .find(|r| r.feed_id == id)
                .map(|r| r.run_key.clone())
        };

        assert_eq!(
            key_of(&together_a.id),
            key_of(&together_b.id),
            "items that arrived together share a run"
        );
        assert_ne!(
            key_of(&together_a.id),
            key_of(&later.id),
            "an arrival two hours later is a different run"
        );
        assert_eq!(
            key_of(&manual.id),
            None,
            "an item with no origin is ungrouped"
        );
        assert!(runs
            .iter()
            .all(|r| r.label.as_deref() == Some("GitHub Trending (daily)")));
    }

    #[test]
    fn capture_provenance_follows_the_body_it_describes() {
        let store = open_test_store("captured_via");

        let mut captured = mk_feed("https://example.com/members", "article", "news");
        captured.captured_via = Some("axon-clip".into());
        store.upsert_feed(&captured).unwrap();
        assert_eq!(
            store
                .get_feed(&captured.id)
                .unwrap()
                .unwrap()
                .captured_via
                .as_deref(),
            Some("axon-clip")
        );

        // A later server-side fetch that yields nothing must not relabel a
        // captured body as fetched — the column describes the stored content.
        let mut empty_refetch = FeedItem::new("https://example.com/members", "news", "article");
        empty_refetch.transcript = None;
        store.upsert_feed(&empty_refetch).unwrap();
        assert_eq!(
            store
                .get_feed(&captured.id)
                .unwrap()
                .unwrap()
                .captured_via
                .as_deref(),
            Some("axon-clip"),
            "an empty re-fetch left the old body but took its provenance"
        );

        // A fetch that DOES replace the body owns the provenance again.
        let mut real_refetch = FeedItem::new("https://example.com/members", "news", "article");
        real_refetch.transcript = Some("server-fetched body".into());
        store.upsert_feed(&real_refetch).unwrap();
        assert_eq!(
            store.get_feed(&captured.id).unwrap().unwrap().captured_via,
            None,
            "content the server fetched must not still claim to be a capture"
        );
    }

    #[test]
    fn transcript_source_round_trips_and_is_not_relabelled_by_an_empty_refetch() {
        let store = open_test_store("transcript_source");

        // A paper stored as its abstract, which is what every arXiv item is
        // until a PDF extractor is registered (#78).
        let mut item = mk_feed("https://arxiv.org/abs/2501.00001", "arxiv", "news");
        item.transcript = Some("We show that ...".into());
        item.transcript_source = "abstract".into();
        store.upsert_feed(&item).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "abstract"
        );

        // A re-fetch that brings back nothing must not relabel it: the field
        // describes the text actually stored, and the stored text did not
        // change. Same guard content_status is under.
        let mut empty = item.clone();
        empty.transcript = None;
        empty.transcript_source = "full-text".into();
        store.upsert_feed(&empty).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "abstract",
            "an empty re-fetch relabelled an abstract as full text"
        );

        // A re-fetch that DOES bring the paper back relabels it, because now
        // the stored text really is the document.
        let mut full = item.clone();
        full.transcript = Some("1 Introduction ...".into());
        full.transcript_source = "full-text".into();
        store.upsert_feed(&full).unwrap();
        assert_eq!(
            store.get_feed(&item.id).unwrap().unwrap().transcript_source,
            "full-text"
        );

        // Legacy rows predate the distinction and stay unknown rather than
        // being backfilled into a claim nobody checked.
        let legacy = mk_feed("https://example.com/legacy", "article", "news");
        store.upsert_feed(&legacy).unwrap();
        assert_eq!(
            store
                .get_feed(&legacy.id)
                .unwrap()
                .unwrap()
                .transcript_source,
            "unknown"
        );
    }

    #[test]
    fn raw_content_is_retained_beside_the_normalized_transcript() {
        let store = open_test_store("raw_content");
        let mut item = mk_feed("https://example.com/raw", "article", "news");
        item.raw_content = Some("Menu\n\nThe body.".into());
        item.transcript = Some("The body.".into());
        store.upsert_feed(&item).unwrap();

        assert_eq!(
            store.get_raw_content(&item.id).unwrap().as_deref(),
            Some("Menu\n\nThe body.")
        );
        assert_eq!(
            store
                .get_feed(&item.id)
                .unwrap()
                .unwrap()
                .transcript
                .as_deref(),
            Some("The body.")
        );
        assert_eq!(
            store.feed_ids_with_raw_content().unwrap(),
            vec![item.id.clone()]
        );

        // The point of retention: a rule change rewrites the body from stored
        // raw, and the raw itself is untouched so it can be done again.
        store
            .set_normalized(&item.id, Some("Rewritten."), "thin")
            .unwrap();
        let after = store.get_feed(&item.id).unwrap().unwrap();
        assert_eq!(after.transcript.as_deref(), Some("Rewritten."));
        assert_eq!(after.content_status, "thin");
        assert_eq!(
            store.get_raw_content(&item.id).unwrap().as_deref(),
            Some("Menu\n\nThe body."),
            "re-normalizing must never disturb the extractor's output"
        );
    }

    fn mk_digest(source: &str, item_id: &str, producer: &str) -> StoredDigest {
        StoredDigest {
            source: source.into(),
            item_id: item_id.into(),
            text: Some("- A point\n- Another".into()),
            state: "generated".into(),
            shape: "brief".into(),
            depth: "standard".into(),
            focus: String::new(),
            producer: producer.into(),
            source_chars: 1_200,
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
        }
    }

    #[test]
    fn content_digest_round_trips_and_replaces_in_place() {
        let store = open_test_store("digest_round_trip");
        let item = mk_feed("https://example.com/digested", "article", "news");
        store.upsert_feed(&item).unwrap();
        assert!(store.content_digest("feed", &item.id).unwrap().is_none());

        store
            .upsert_content_digest(&mk_digest("feed", &item.id, "p1"))
            .unwrap();
        let stored = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(stored.state, "generated");
        assert_eq!(stored.shape, "brief");
        assert!(
            !stored.generated_at.is_empty(),
            "the row stamps its own time"
        );

        // Replace in place: a refine overwrites rather than appending, and the
        // directive that produced it comes back with it.
        let mut refined = mk_digest("feed", &item.id, "p1");
        refined.text = Some("## Method\n- Deeper".into());
        refined.shape = "sectioned".into();
        refined.depth = "detailed".into();
        refined.focus = "cost, latency".into();
        store.upsert_content_digest(&refined).unwrap();
        let after = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(after.depth, "detailed");
        assert_eq!(after.focus, "cost, latency");
        assert_eq!(after.text.as_deref(), Some("## Method\n- Deeper"));

        // The diagram is a separate press and updates without touching the text.
        assert_eq!(
            store
                .update_content_diagram(
                    "feed",
                    &item.id,
                    Some("flowchart TD\n  A --> B"),
                    "generated",
                    None,
                    "d1"
                )
                .unwrap(),
            1
        );
        let with_diagram = store.content_digest("feed", &item.id).unwrap().unwrap();
        assert_eq!(
            with_diagram.diagram.as_deref(),
            Some("flowchart TD\n  A --> B")
        );
        assert_eq!(with_diagram.text.as_deref(), Some("## Method\n- Deeper"));
    }

    /// The automatic pass must never overwrite a digest an operator asked for.
    /// A model upgrade changes the producer on every row, and without the
    /// `depth = 'standard'` guard that upgrade would silently throw away every
    /// refinement in the store.
    #[test]
    fn the_automatic_pass_leaves_an_operators_refinement_alone() {
        let store = open_test_store("digest_queue");
        let missing = mk_feed("https://example.com/no-digest", "article", "news");
        let stale = mk_feed("https://example.com/stale-digest", "article", "news");
        let refined = mk_feed("https://example.com/refined-digest", "article", "news");
        let current = mk_feed("https://example.com/current-digest", "article", "news");
        let parked = mk_feed("https://example.com/parked-digest", "article", "news");
        for item in [&missing, &stale, &refined, &current, &parked] {
            store.upsert_feed(item).unwrap();
        }

        // A list, because the role is chosen per item: one machine can hold a
        // light model for short sources and a strong one for long sources, and
        // a digest from either is current.
        let producers = vec!["current-producer".to_string()];
        // The subset an unattended pass can produce. Here it is the whole set:
        // the split only matters on a machine that also has a strong local
        // role, which the attempt-cap case below pins separately.
        let unattended = producers.clone();

        store
            .upsert_content_digest(&mk_digest("feed", &stale.id, "old-producer"))
            .unwrap();
        let mut refined_row = mk_digest("feed", &refined.id, "old-producer");
        refined_row.depth = "detailed".into();
        store.upsert_content_digest(&refined_row).unwrap();
        store
            .upsert_content_digest(&mk_digest("feed", &current.id, "current-producer"))
            .unwrap();
        let mut parked_row = mk_digest("feed", &parked.id, "current-producer");
        parked_row.state = "timeout".into();
        parked_row.text = None;
        parked_row.attempts = 3;
        store.upsert_content_digest(&parked_row).unwrap();

        let queued = store
            .items_needing_digest("feed", &producers, &unattended, 3, 50)
            .unwrap();
        assert!(queued.contains(&missing.id), "no digest at all");
        assert!(queued.contains(&stale.id), "produced by an older model");
        assert!(
            !queued.contains(&refined.id),
            "an operator's detailed digest must survive a model change"
        );
        assert!(!queued.contains(&current.id), "already current");
        assert!(
            !queued.contains(&parked.id),
            "a row at the attempt cap is parked, not retried forever"
        );

        // One retry left is no longer enough on its own: writing the failure
        // arms a backoff, and the row stays parked until that window elapses.
        parked_row.attempts = 2;
        store.upsert_content_digest(&parked_row).unwrap();
        assert!(
            !store
                .items_needing_digest("feed", &producers, &unattended, 3, 50)
                .unwrap()
                .contains(&parked.id),
            "a failure just written is inside its own backoff window"
        );

        // Once it has, the retry is due.
        expire_digest_backoff(&store, "feed", &parked.id);
        assert!(store
            .items_needing_digest("feed", &producers, &unattended, 3, 50)
            .unwrap()
            .contains(&parked.id));

        // The attempt cap is scoped to the producers the pass can actually
        // write. A row that spent every attempt on a model the unattended pass
        // no longer uses is offered back to it — which is the whole of why six
        // long public items were invisible to the cloud rung while sitting at
        // `http_error`, attempt 4, against a stopped local server.
        parked_row.attempts = 9;
        store.upsert_content_digest(&parked_row).unwrap();
        expire_digest_backoff(&store, "feed", &parked.id);
        assert!(
            !store
                .items_needing_digest("feed", &producers, &unattended, 3, 50)
                .unwrap()
                .contains(&parked.id),
            "a row parked by the rung this pass uses stays parked"
        );
        assert!(
            store
                .items_needing_digest(
                    "feed",
                    &producers,
                    &["a-rung-this-pass-cannot-use".to_string()],
                    3,
                    50
                )
                .unwrap()
                .contains(&parked.id),
            "attempts spent by another model are not this pass's attempts"
        );

        assert!(store
            .items_needing_digest("scouting", &producers, &unattended, 3, 50)
            .is_err());
    }

    /// Move a digest's retry deadline into the past.
    ///
    /// Raw SQL rather than a field on `StoredDigest`: the deadline is derived
    /// from the database's own clock at write time precisely so no caller can
    /// set it, and adding a settable field to production code to serve tests
    /// would give that guarantee away. Tests live inside this module, so they
    /// can reach the connection without one.
    fn expire_digest_backoff(store: &Store, source: &str, item_id: &str) {
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {prefix}_content_digests
                        SET next_attempt = {past}
                      WHERE source = ?1 AND item_id = ?2",
                    prefix = store.prefix,
                    past = sjel_store::now_offset("'-1 hour'")
                ),
                params![&source, &item_id],
            )
            .unwrap();
    }

    /// Concurrent openers must not deadlock.
    ///
    /// `Store::open` used to migrate every time it was called, and the comms
    /// server calls it from several timers plus every HTTP handler. Three of
    /// those timers share a fifteen-minute period, so "two sessions migrate at
    /// the same instant" was the normal case rather than the rare one, and it
    /// produced `deadlock detected` in the drain log roughly every other pass.
    ///
    /// Migration now runs once per process per (file, prefix), so seven of
    /// these eight threads do no DDL at all. Kept at eight anyway: this is the
    /// shape that failed reliably before, and it is the only place the
    /// in-process guard is exercised under real contention rather than in
    /// isolation (libs/sjel-store has the isolated cases).
    #[test]
    fn concurrent_openers_do_not_deadlock() {
        let path = test_database("open_race");
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                // Flattened to a String inside the thread: `Box<dyn Error>` is
                // not Send, and the detail lives on the source chain anyway, so
                // joining the causes is the only way this failure names itself.
                std::thread::spawn(move || {
                    Store::open(&path).map(|_| ()).map_err(|error| {
                        let mut text = error.to_string();
                        let mut cause = error.source();
                        while let Some(next) = cause {
                            text.push_str(&format!(": {next}"));
                            cause = next.source();
                        }
                        text
                    })
                })
            })
            .collect();

        let failures: Vec<String> = threads
            .into_iter()
            .filter_map(|t| t.join().expect("opener panicked").err())
            .collect();
        assert!(
            failures.is_empty(),
            "every concurrent open must succeed, got: {failures:?}"
        );
    }

    /// Twenty opens are not twenty connections.
    ///
    /// The whole point of the pool, asserted against r2d2's own count rather than
    /// against a stopwatch: a latency assertion on a shared database is a flake
    /// generator, while "how many sessions did this open" is the thing that
    /// actually changed.
    ///
    /// The bound is the pool ceiling rather than an exact number, because the test
    /// binary runs cases in parallel against one process-wide pool per file and
    /// the others contribute connections too. It is still the claim that matters:
    /// before this, twenty opens meant twenty connect-and-authenticate round trips,
    /// and there was no ceiling at all.
    #[test]
    fn many_opens_share_one_pool() {
        let path = test_database("pool_shared");
        let store = Store::open(&path).expect("open");

        let opened: Vec<Store> = (0..20).map(|_| Store::open(&path).expect("open")).collect();
        assert_eq!(opened.len(), 20);

        let connections = store.pool.state().connections;
        assert!(
            connections <= 10,
            "20 opens produced {connections} connections; the pool ceiling is 10"
        );
    }

    /// The second open of a (file, prefix) this process already migrated does no DDL.
    ///
    /// Asserted by dropping the tables behind the process's back: an `open` that
    /// still migrated would put them straight back, and the read below would
    /// succeed. It failing is the proof that the DDL ran exactly once.
    ///
    /// That is also the honest cost of the design, which is why it is pinned
    /// rather than left implicit. Rebuilding tables dropped underneath a live
    /// process would mean DDL on every open again, and DDL on every open takes
    /// the write lock 43 handlers are behind.
    #[test]
    fn a_second_open_does_no_ddl() {
        let path = test_database("migrate_once");

        let first = Store::open(&path).expect("first open migrates");
        first
            .list_feed(None, None, 1, false)
            .expect("the first open must leave usable tables behind");
        drop(first);

        let drop_feed_items = || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute_batch("DROP TABLE IF EXISTS comms_feed_items")
                .unwrap();
        };
        drop_feed_items();

        let second = Store::open(&path).expect("the second open still connects");
        let read = second.list_feed(None, None, 1, false);

        assert!(
            read.is_err(),
            "the second open re-ran the migration; it must trust the first"
        );
    }

    /// The backoff is what makes a *timed* drain safe. With the attempt cap
    /// alone, a drain every 15 minutes spends all three attempts inside
    /// three-quarters of an hour, so an outage lasting an hour would leave the
    /// row permanently dead — the exact failure the drain exists to end.
    #[test]
    fn a_retryable_failure_waits_out_a_growing_backoff() {
        let store = open_test_store("digest_backoff");
        let item = mk_feed("https://example.com/backoff-digest", "article", "news");
        store.upsert_feed(&item).unwrap();

        let mut row = mk_digest("feed", &item.id, "current-producer");
        row.state = "empty_response".into();
        row.text = None;

        // Each successive failure parks the row for longer: 5, 10, then 20
        // minutes. Read back as a delta so this asserts the ladder, not a clock.
        let mut previous = 0_f64;
        for attempt in 1..=3 {
            row.attempts = attempt;
            store.upsert_content_digest(&row).unwrap();
            let seconds = backoff_seconds(&store, "feed", &item.id)
                .expect("a retryable state arms a deadline");
            assert!(
                seconds > previous,
                "attempt {attempt} must wait longer than the one before it \
                 ({seconds}s vs {previous}s)"
            );
            previous = seconds;
        }

        // A success clears the deadline outright — there is no next attempt to
        // schedule, and a stale one left behind would park a healthy row.
        row.state = "generated".into();
        row.text = Some("- A point".into());
        row.attempts = 0;
        store.upsert_content_digest(&row).unwrap();
        assert!(
            backoff_seconds(&store, "feed", &item.id).is_none(),
            "a generated digest carries no retry deadline"
        );
    }

    /// Seconds from now until the row's retry deadline, or None when it has none.
    fn backoff_seconds(store: &Store, source: &str, item_id: &str) -> Option<f64> {
        // `EXTRACT(EPOCH FROM (a - b))` becomes a difference of julian days,
        // scaled to seconds. It reads the deadline the way the drain does --
        // through SQLite's own date functions on the stored text -- which is the
        // property `NOW`'s `+00:00` offset exists to keep.
        store
            .conn()
            .unwrap()
            .query_row(
                &format!(
                    "SELECT (julianday(next_attempt) - julianday('now')) * 86400.0
                       FROM {prefix}_content_digests
                      WHERE source = ?1 AND item_id = ?2",
                    prefix = store.prefix
                ),
                params![&source, &item_id],
                |row| row.get::<_, Option<f64>>(0),
            )
            .unwrap()
    }

    #[test]
    fn feed_pending_summaries_includes_stale_model_output() {
        let store = open_test_store("feed_pending");
        let with_t = mk_feed("https://youtu.be/hastranscript", "youtube", "media");
        store.upsert_feed(&with_t).unwrap();
        let mut no_t = FeedItem::new("https://example.com/no-transcript", "news", "article");
        no_t.transcript = None;
        store.upsert_feed(&no_t).unwrap();
        let stale = mk_feed("https://example.com/stale-summary", "article", "news");
        store.upsert_feed(&stale).unwrap();
        store
            .update_feed_summary(&stale.id, "Old model output", "summary-v1")
            .unwrap();
        let current = mk_feed("https://example.com/current-summary", "article", "news");
        store.upsert_feed(&current).unwrap();
        store
            .update_feed_summary(&current.id, "Current model output", "summary-v2")
            .unwrap();
        let mut legacy = mk_feed("https://example.com/legacy-summary", "article", "news");
        legacy.summary = Some("Historical generated output".into());
        store.upsert_feed(&legacy).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "UPDATE {}_feed_items SET summary_revision = 'legacy-unknown' WHERE id = ?1",
                    store.prefix
                ),
                params![&legacy.id],
            )
            .unwrap();

        let pending = store.feed_pending_summaries(Some("summary-v2")).unwrap();
        let ids = pending
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids.len(),
            3,
            "missing, stale model, and pre-provenance summaries need work"
        );
        assert!(ids.contains(&with_t.id.as_str()));
        assert!(ids.contains(&stale.id.as_str()));
        assert!(ids.contains(&legacy.id.as_str()));
        assert!(!ids.contains(&current.id.as_str()));
        assert!(store
            .feed_summary_needs_revision(&stale.id, "summary-v2")
            .unwrap());
        assert!(!store
            .feed_summary_needs_revision(&current.id, "summary-v2")
            .unwrap());
        assert!(store
            .feed_summary_needs_revision(&legacy.id, "summary-v2")
            .unwrap());
    }

    #[test]
    fn source_state_round_trip() {
        let store = open_test_store("source_state");
        assert!(store.get_source_state("gmail").unwrap().is_none());
        store.record_run("gmail", Some("cur-1")).unwrap();
        let st = store.get_source_state("gmail").unwrap().unwrap();
        assert_eq!(st.cursor.as_deref(), Some("cur-1"));
        store.record_run("gmail", None).unwrap();
        let st2 = store.get_source_state("gmail").unwrap().unwrap();
        assert_eq!(
            st2.cursor.as_deref(),
            Some("cur-1"),
            "cursor preserved when not given"
        );
    }

    /// A failing streak must not erase the last time collection actually
    /// worked: "last success" is the number that tells a human whether a red
    /// schedule is an outage or a five-minute blip.
    #[test]
    fn a_failure_streak_preserves_the_last_success_and_recovery_clears_it() {
        let store = open_test_store("sweep_outcome");

        store.record_sweep_success("gmail-inbox", 25, 3).unwrap();
        let ok = store.get_source_state("gmail-inbox").unwrap().unwrap();
        let first_success = ok.last_success_at.clone().expect("success is recorded");
        assert_eq!((ok.considered_count, ok.new_count), (25, 3));
        assert_eq!(ok.consecutive_failures, 0);
        assert!(ok.last_error.is_none());

        assert_eq!(
            store.record_sweep_failure("gmail-inbox", "auth").unwrap(),
            1
        );
        assert_eq!(
            store.record_sweep_failure("gmail-inbox", "quota").unwrap(),
            2
        );
        let failing = store.get_source_state("gmail-inbox").unwrap().unwrap();
        assert_eq!(failing.consecutive_failures, 2);
        assert_eq!(failing.last_error.as_deref(), Some("quota"));
        assert_eq!(
            failing.last_success_at.as_deref(),
            Some(first_success.as_str()),
            "a failing run must not overwrite when collection last worked"
        );
        assert!(failing.last_failure_at.is_some());

        store.record_sweep_success("gmail-inbox", 25, 0).unwrap();
        let recovered = store.get_source_state("gmail-inbox").unwrap().unwrap();
        assert_eq!(
            recovered.consecutive_failures, 0,
            "success clears the streak"
        );
        assert!(recovered.last_error.is_none(), "and clears the error class");
        assert!(
            recovered.last_failure_at.is_some(),
            "but keeps that a failure happened"
        );
    }

    /// C17, against the real ledger rather than the arithmetic alone. Three
    /// consecutive capacity aborts alert; the two before them do not, and one
    /// answered request puts the machine back to silent — which is the half
    /// that makes "consecutive" mean anything. Runs on the same `source_state`
    /// table the inbox sweep's own streak uses, because a second counter for
    /// the same idea is the duplicate home this whole design refuses.
    #[test]
    fn three_consecutive_capacity_aborts_alert_and_one_success_silences_it() {
        let store = open_test_store("capacity_streak");
        let threshold = 3;

        assert_eq!(crate::capacity::record_failure(&store, threshold), None);
        assert_eq!(crate::capacity::record_failure(&store, threshold), None);
        assert_eq!(
            crate::capacity::record_failure(&store, threshold),
            Some(3),
            "the third consecutive abort is the alert"
        );
        assert_eq!(
            crate::capacity::record_failure(&store, threshold),
            Some(4),
            "still broken on the next pass is still an alert"
        );

        let alerting = store
            .get_source_state(crate::capacity::LOCAL_INFERENCE_SOURCE)
            .unwrap()
            .unwrap();
        assert_eq!(alerting.consecutive_failures, 4);
        assert_eq!(
            alerting.last_error.as_deref(),
            Some("capacity"),
            "a stable class, never a provider message"
        );

        crate::capacity::record_success(&store);
        assert_eq!(
            store
                .get_source_state(crate::capacity::LOCAL_INFERENCE_SOURCE)
                .unwrap()
                .unwrap()
                .consecutive_failures,
            0,
            "one answered request ends the streak"
        );
        assert_eq!(
            crate::capacity::record_failure(&store, threshold),
            None,
            "and the count starts over rather than resuming at four"
        );
    }

    /// The window that wraps midnight is the one people actually configure, so
    /// it is the one worth a test. Uses the store's own clock, so the assertion
    /// is on the two windows that must always disagree.
    #[test]
    fn quiet_hours_wrap_midnight() {
        let store = open_test_store("quiet_hours");
        let all_day = store.within_quiet_hours(0, 23).unwrap();
        let inverse = store.within_quiet_hours(23, 0).unwrap();
        assert_ne!(
            all_day, inverse,
            "a window and its complement cannot both hold at one instant"
        );
        assert!(
            !store.within_quiet_hours(9, 9).unwrap(),
            "an empty window is never quiet"
        );
    }
}
