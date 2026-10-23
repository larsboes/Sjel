//! Client for the Sjel shell (`capabilities/sjel-status`), authenticated with the inbound token.
//!
//! The same contracts and token as the menu-bar app (`apps/mac/Sources/SjelRelay`): the token
//! comes from the login Keychain item that `tools/setup-inbound-auth.sh` writes, through
//! `/usr/bin/security`, which that script puts on the item's access list.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use reqwest::{header, Method, StatusCode, Url};

use crate::capture::{self, Capture, Route};
use crate::mail::{BulkResult, MailAction, MailProposal};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use futures::channel::mpsc::UnboundedReceiver;

/// The default shell endpoint, from `capabilities/sjel-status/service.toml`.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8082";
const KEYCHAIN_SERVICE: &str = "sjel-inbound-token";
const TICKET_PATH: &str = "/api/sjel-status/session/ticket";
const OPEN_PATH: &str = "/session/open?";
const EVENTS_PATH: &str = "/api/sjel-status/events";

/// The ceiling on one original, matching `MAX_BYTES` in `capabilities/documents/src/server.rs`.
/// A reader holds one document at a time, and this is the most it can be asked to hold; the two
/// numbers are one edit apart on purpose.
const MAX_DOCUMENT_BYTES: usize = 128 * 1024 * 1024;

/// `GET /api/sjel-status/health` (`SjelStatusHealth` in
/// `capabilities/sjel-status/src/status/registry.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct NodeHealth {
    pub version: String,
    pub uptime_seconds: u64,
    pub capabilities: BTreeMap<String, CapabilityHealth>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CapabilityHealth {
    pub up: bool,
    /// Kept running by the machine. A shell from before the field kept every capability running.
    #[serde(default = "kept_running")]
    pub autostart: bool,
}

fn kept_running() -> bool {
    true
}

/// What a capability's state means to the reader. On-demand capabilities that are stopped are
/// normal, not down (`NodeHealth.down` in `apps/mac/Sources/SjelRelay/NodeBridge.swift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    Up,
    Idle,
    Down,
}

impl CapabilityHealth {
    pub fn liveness(self) -> Liveness {
        match (self.up, self.autostart) {
            (true, _) => Liveness::Up,
            (false, false) => Liveness::Idle,
            (false, true) => Liveness::Down,
        }
    }
}

impl NodeHealth {
    /// Kept-running capabilities that did not answer, by name.
    pub fn down(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|(_, health)| health.liveness() == Liveness::Down)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// `GET /api/sjel-status/agent`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentView {
    /// False until this machine holds an agent token: every mode below is then unused.
    #[serde(default)]
    pub enrolled: bool,
    /// Every mode the policy admits: `off`, `read-only`, `ask`, `auto`.
    #[serde(default)]
    pub modes: Vec<String>,
    /// The mode a capability with no entry runs at.
    #[serde(default)]
    pub default_mode: String,
    /// One entry per capability that has started a gate on this machine.
    #[serde(default)]
    pub capabilities: Vec<AgentGate>,
    #[serde(default)]
    pub pending: Vec<Approval>,
    /// Newest first, at most 50.
    #[serde(default)]
    pub calls: Vec<AgentCall>,
}

/// One capability's agent gate: the mode it runs at, and the routes that always ask.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentGate {
    pub capability: String,
    /// `off` · `read-only` · `ask` · `auto`.
    #[serde(default)]
    pub mode: String,
    /// Routes recorded as `METHOD /path` that always wait for the owner.
    #[serde(default)]
    pub confirm: Vec<String>,
    /// Writes reachable over GET, recorded as `METHOD /path`.
    #[serde(default, rename = "get_writes")]
    pub get_writes: Vec<String>,
}

/// One agent write that waits for the owner.
#[derive(Clone, Debug, Deserialize)]
pub struct Approval {
    pub id: String,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub preview: String,
    /// Unix seconds.
    #[serde(default)]
    pub created_at: i64,
    /// `pending`, `allowed`, `denied` or `used`.
    #[serde(default)]
    pub state: String,
}

/// One call an agent made through the gate (`AgentCall` in `libs/sjel-server/src/agent_policy.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct AgentCall {
    /// Unix seconds.
    pub at: i64,
    pub capability: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// `read` for a plain read; anything else is a write or a refusal.
    pub decision: String,
}

/// `GET /calendar/api/entries` (`Entry` in `capabilities/calendar/src/store.rs`). Home's
/// direct database read fills only the first five fields; the rest default.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CalendarEntry {
    pub title: String,
    /// `YYYY-MM-DD` when `all_day`, else local `YYYY-MM-DDTHH:MM:SS`.
    pub starts_at: String,
    /// Exclusive for an all-day entry.
    pub ends_at: String,
    pub all_day: bool,
    /// `possible`, `planned` or `committed`.
    pub commitment: String,
    #[serde(default)]
    pub id: String,
    /// Open text; `core::calendar::KINDS` names the known ones.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Who wrote it: `manual`, `google`, a capability.
    #[serde(default)]
    pub source: String,
}

/// A transaction in `GET /finance/api/dashboard`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinanceTransaction {
    pub id: String,
    pub date: String,
    pub description: String,
    pub kind: String,
    pub account: String,
    pub category: String,
    pub amount_cents: i64,
    pub currency: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub trip_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinanceDashboardSummary {
    #[serde(default)]
    pub income_cents: i64,
    #[serde(default)]
    pub personal_spending_cents: i64,
    #[serde(default)]
    pub currency: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FinanceDashboardProjection {
    #[serde(default)]
    pub summary: FinanceDashboardSummary,
    #[serde(default)]
    pub transactions: Vec<FinanceTransaction>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PricePointRecord {
    pub valid_from: String,
    pub amount_cents: i64,
    pub currency: String,
    pub cycle: String,
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StateChangeRecord {
    pub effective: String,
    pub state: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubscriptionRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub value_rating: Option<i16>,
    #[serde(default)]
    pub prices: Vec<PricePointRecord>,
    #[serde(default)]
    pub states: Vec<StateChangeRecord>,
}

/// One entity record from `GET /entities/api/entities`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EntityRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub note_ref: Option<String>,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
}

/// `GET /import/api/import/plan` (`capabilities/import/src/server.rs`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ImportPlan {
    pub folder: String,
    pub files: usize,
    pub routes: Vec<ImportRoute>,
    pub skipped: Vec<ImportSkip>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImportRoute {
    /// `documents`, `media`, `finance`, `notes` or `skip`.
    pub route: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImportSkip {
    pub reason: String,
    pub files: usize,
}

/// `GET /import/api/import/runs/current`: the running or last finished run.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ImportJob {
    pub folder: String,
    pub total: usize,
    pub done: usize,
    pub unchanged: usize,
    /// Files per outcome: `imported`, `present`, `pending`, `skipped`, `failed`.
    pub outcomes: std::collections::BTreeMap<String, usize>,
    pub finished: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct ImportJobEnvelope {
    job: Option<ImportJob>,
}

/// `GET /import/api/import/status`: the ledger for one folder.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ImportStatus {
    pub folder: String,
    pub rows: Vec<ImportStatusRow>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImportStatusRow {
    pub route: String,
    pub outcome: String,
    /// The reason for a skipped, pending or failed row.
    pub detail: Option<String>,
    pub files: usize,
}

/// `GET /software/api/apps` (`App` in `capabilities/software/src/store.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct SoftwareApps {
    pub apps: Vec<SoftwareApp>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SoftwareApp {
    /// `app`, `leftover` or `tool`.
    #[serde(default)]
    pub kind: String,
    /// Who installed it: `app-store`, `cask:<token>`, `other`, or a tool's manager.
    #[serde(default)]
    pub source: String,
    pub bundle_id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub app_bytes: u64,
    pub data_bytes: u64,
    /// `YYYY-MM-DD HH:MM:SS +0000`, or none when Spotlight holds no date.
    pub last_used: Option<String>,
    pub present: bool,
    /// `use`, `remove`, `replace` or `rebuild`.
    pub decision: Option<String>,
    pub note: Option<String>,
    /// Why the row is due another look, or none.
    #[serde(default)]
    pub review: Option<String>,
}

/// `GET /api/sjel-status/backups` (`backups_handler` in
/// `capabilities/sjel-status/src/status/backup.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct Backups {
    pub backups: Vec<BackupStatus>,
    #[serde(default)]
    pub attempt_error: Option<String>,
}

/// One capability with a backup contract.
#[derive(Clone, Debug, Deserialize)]
pub struct BackupStatus {
    pub capability: String,
    /// `ok`, `due`, `overdue`, `never` or `unknown` (`backup_state` in sjel-status).
    pub state: String,
    pub last_success: Option<String>,
    pub age_seconds: Option<u64>,
    pub bytes: Option<u64>,
    #[serde(default)]
    pub holds_service: bool,
    #[serde(default)]
    pub advise_days: Option<i64>,
    #[serde(default)]
    pub stale_days: Option<i64>,
    /// The run this sjel-status process started, if any.
    pub run: Option<BackupRun>,
    #[serde(default)]
    pub attempt: Option<crate::backup::BackupRunAttemptRecord>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BackupRun {
    /// `running`, `succeeded` or `failed`.
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

impl Backups {
    pub fn running(&self) -> bool {
        self.backups
            .iter()
            .any(|b| b.run.as_ref().is_some_and(|run| run.state == "running"))
    }
}

/// `GET /projects/api/projects` (`ProjectSummary` in `capabilities/projects/src/server.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub open: usize,
    #[serde(default)]
    pub done: usize,
    #[serde(default)]
    pub notes: usize,
}

/// One record of `GET /projects/api/project` and `GET /projects/api/record`.
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectRecord {
    pub path: String,
    /// `project`, `task` or `note`.
    pub kind: String,
    pub title: String,
    /// Absent from a list read with `content=false`; the note editor reads the record first.
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub due: Option<String>,
    /// 1 to 3; the capability reads a missing or other value as 2.
    #[serde(default)]
    pub priority: u8,
}

/// One task across projects (`GET /projects/api/tasks`, and the reply to every task write).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct GlobalTask {
    /// The record's path, which every write takes as `id` or `path`.
    pub id: String,
    /// `task` or `note`.
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub done: bool,
    pub due: Option<String>,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub project: String,
}

/// One note across projects (`GET /projects/api/records`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GlobalNote {
    pub path: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
}

/// Metadata row from the operator-only Documents API. Document contents stay local to the host.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DocumentSummary {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub byte_len: i64,
    pub imported_at: String,
    pub status: String,
    pub category: Option<String>,
    pub project: Option<String>,
    pub extracted_title: Option<String>,
    pub extractor: Option<String>,
    pub extracted_chars: usize,
    pub suggested_category: Option<String>,
    pub suggestion_reason: Option<String>,
    pub data_class: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReview {
    Accepted,
    Rejected,
    Overridden,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalSource {
    Rules {
        identity: String,
        version: String,
    },
    Model {
        role: String,
        model: String,
        revision: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalEvidenceSource {
    Filename,
    ExtractedText,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProposalEvidence {
    pub source: ProposalEvidenceSource,
    pub quote: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAbstentionReason {
    NoMatch,
    MissingEvidence,
    ModelUnavailable,
    CategoryNotConfigured,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalInvalidReason {
    MalformedOutput,
    EvidenceNotExact,
    EvidenceTooLong,
    InvalidCategory,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalOutcome {
    Proposed {
        category: String,
        evidence: ProposalEvidence,
    },
    Abstained {
        reason: ProposalAbstentionReason,
    },
    Invalid {
        reason: ProposalInvalidReason,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ClassificationProposal {
    pub document_id: String,
    pub source: ProposalSource,
    pub outcome: ProposalOutcome,
    pub confidence: Option<f32>,
    pub review: Option<ProposalReview>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProposalReviewEvent {
    pub id: i64,
    pub outcome: ProposalReview,
    pub reviewed_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DocumentProposalRecord {
    pub id: String,
    pub generation_id: String,
    pub generated_at: String,
    pub proposal: ClassificationProposal,
    pub review_history: Vec<ProposalReviewEvent>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DocumentDetail {
    pub document: DocumentSummary,
    pub text: Option<String>,
    #[serde(default)]
    pub proposals: Vec<DocumentProposalRecord>,
}

/// The answer to an upload (`POST /api/documents?filename=...`). `created` is false when the same
/// bytes under the same filename were already stored; the record is then the first upload's, with
/// its filing decision untouched.
#[derive(Clone, Debug, Deserialize)]
pub struct DocumentUpload {
    pub document: DocumentSummary,
    pub created: bool,
}

#[derive(Deserialize)]
struct DocumentPage {
    documents: Vec<DocumentSummary>,
    total: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateCandidateKind {
    ExactCopy,
    Rescan,
    Part,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DuplicateCandidatePage {
    pub candidates: Vec<DuplicateCandidate>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DuplicateCandidate {
    pub candidate_key: String,
    pub kind: DuplicateCandidateKind,
    pub member_ids: Vec<String>,
    pub score: Option<f64>,
    pub sha256: Option<String>,
    pub part: Option<DuplicatePartRelation>,
    pub locations: Vec<DuplicateSourceLocation>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DuplicatePartRelation {
    pub part_id: String,
    pub parent_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DuplicateSourceLocation {
    pub document_id: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateDecision {
    pub candidate_key: String,
    pub kind: DuplicateCandidateKind,
    pub member_ids: Vec<String>,
    pub action: DuplicateDecisionAction,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DuplicateDecisionAction {
    ConfirmGroup {
        survivor_id: String,
    },
    Dismiss,
    Split {
        parent_id: String,
        page_count: u32,
        ranges: Vec<DuplicatePageRange>,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DuplicatePageRange {
    pub first: u32,
    pub last: u32,
}

#[derive(Deserialize)]
struct DuplicateDecisionResponse {
    decision: serde_json::Value,
}

/// A feed entry's best profile match (`relevance` in `GET /comms/feed`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedRelevance {
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
}

/// Why the evaluator ranked an entry where it did.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedEvaluation {
    pub overall_score: f64,
    pub explanation: String,
}

/// One row of `GET /comms/feed`. Comms' `FeedListItem` (`capabilities/comms/src/server/contracts.rs`)
/// owns the shape; only what the native list draws is read.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedEntry {
    pub id: String,
    pub stream: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    /// The digest's opening, sent only when the entry has no summary of its own.
    #[serde(default)]
    pub digest_preview: Option<String>,
    pub day: String,
    pub status: String,
    pub relevance: Option<FeedRelevance>,
    pub evaluation: Option<FeedEvaluation>,
}

impl FeedEntry {
    /// The order the dashboard ranks the inbox by (`dashboard/src/lib/home/kinds/feed.ts`): the
    /// evaluator's score, else the relevance score, else nothing.
    pub fn score(&self) -> f64 {
        self.evaluation
            .as_ref()
            .map(|e| e.overall_score)
            .or(self.relevance.as_ref().map(|r| r.score))
            .unwrap_or(0.0)
    }
}

/// `GET /comms/feed/:id`, comms' `FeedFullItem`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedDetail {
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub transcript: Option<String>,
    pub day: String,
    pub status: String,
    pub content_status: String,
    #[serde(default)]
    pub relevance: Vec<FeedRelevance>,
    pub evaluation: Option<FeedEvaluation>,
}

/// Where a keep, dismiss or open happened. Comms' ledger CHECK constraint
/// (`capabilities/comms/src/store/migrations.rs`) holds the full list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedSurface {
    Inbox,
    Reader,
}

impl FeedSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Reader => "reader",
        }
    }
}

/// Comms answers a failed list query with 200 and `{"error": …}` (`server/content.rs`), so the
/// list reads either shape.
#[derive(Deserialize)]
#[serde(untagged)]
enum FeedList {
    Rows(Vec<FeedEntry>),
    Failed { error: String },
}

/// A feed or document id: 64 hex characters, a SHA-256. Anything else never reaches a URL.
fn is_sha256(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The reader route for one document's original, or `None` for an id that is not a digest. The
/// check runs before any request, so a malformed id never reaches the capability.
fn document_original_path(id: &str) -> Option<String> {
    is_sha256(id).then(|| format!("/documents/api/documents/{id}/original"))
}

fn too_large() -> ShellError {
    ShellError::Refused(413, Some("That document is too large to open here.".into()))
}

/// The body of one shell request. An upload is raw bytes and must not carry a JSON content type:
/// the capability takes the filename from the query and reads the body as the file itself.
enum Payload {
    Json(serde_json::Value),
    Bytes(Vec<u8>),
}

/// The soundscape playback state (`GET /soundscape/api/soundscape/state`).
///
/// Every field but `preset` is optional on the wire, because a conductor from before a field
/// existed still answers: a missing parameter falls back to the same default the conductor uses
/// (`capabilities/soundscape/src/main.rs`). That equality is the contract, not a convenience --
/// a surface that invents its own default opens a preset in the wrong place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundscapeState {
    pub preset: String,
    #[serde(default)]
    pub playing: bool,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_energy")]
    pub energy: f32,
    #[serde(default)]
    pub params: SoundscapeParams,
    #[serde(default)]
    pub layers: SoundscapeLayers,
    #[serde(default)]
    pub seed: u32,
    #[serde(default)]
    pub session: Option<SoundscapeSession>,
}

fn default_volume() -> f32 {
    0.65
}

fn default_energy() -> f32 {
    0.7
}

/// The six scene parameters, same names and range as the browser engine's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundscapeParams {
    pub pace: f32,
    pub density: f32,
    pub brightness: f32,
    pub space: f32,
    pub pulse: f32,
    pub texture: f32,
}

impl SoundscapeParams {
    pub fn get(&self, name: crate::soundscape::ParamName) -> f32 {
        self.into_params().get(name)
    }

    pub fn set(&mut self, name: crate::soundscape::ParamName, value: f32) {
        let mut params = self.into_params();
        params.set(name, value);
        *self = params.into();
    }

    fn into_params(self) -> crate::soundscape::Params {
        self.into()
    }
}

impl Default for SoundscapeParams {
    fn default() -> Self {
        Self {
            pace: 0.5,
            density: 0.5,
            brightness: 0.5,
            space: 0.5,
            pulse: 0.4,
            texture: 0.4,
        }
    }
}

impl From<SoundscapeParams> for crate::soundscape::Params {
    fn from(params: SoundscapeParams) -> Self {
        Self {
            pace: params.pace,
            density: params.density,
            brightness: params.brightness,
            space: params.space,
            pulse: params.pulse,
            texture: params.texture,
        }
    }
}

impl From<crate::soundscape::Params> for SoundscapeParams {
    fn from(params: crate::soundscape::Params) -> Self {
        Self {
            pace: params.pace,
            density: params.density,
            brightness: params.brightness,
            space: params.space,
            pulse: params.pulse,
            texture: params.texture,
        }
    }
}

/// How much of each layer is in the mix, 0-1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundscapeLayers {
    pub drums: f32,
    pub bass: f32,
    pub pads: f32,
    pub melody: f32,
    pub texture: f32,
}

impl SoundscapeLayers {
    pub fn get(&self, layer: crate::soundscape::LayerName) -> f32 {
        let layers: crate::soundscape::Layers = (*self).into();
        layers.get(layer)
    }

    pub fn set(&mut self, layer: crate::soundscape::LayerName, value: f32) {
        let mut layers: crate::soundscape::Layers = (*self).into();
        layers.set(layer, value);
        *self = layers.into();
    }
}

impl Default for SoundscapeLayers {
    fn default() -> Self {
        Self {
            drums: 1.,
            bass: 1.,
            pads: 1.,
            melody: 1.,
            texture: 1.,
        }
    }
}

impl From<SoundscapeLayers> for crate::soundscape::Layers {
    fn from(layers: SoundscapeLayers) -> Self {
        Self {
            drums: layers.drums,
            bass: layers.bass,
            pads: layers.pads,
            melody: layers.melody,
            texture: layers.texture,
        }
    }
}

impl From<crate::soundscape::Layers> for SoundscapeLayers {
    fn from(layers: crate::soundscape::Layers) -> Self {
        Self {
            drums: layers.drums,
            bass: layers.bass,
            pads: layers.pads,
            melody: layers.melody,
            texture: layers.texture,
        }
    }
}

/// A timed session: a scenario, how long it runs, and how much of it has run. The conductor owns
/// this because every surface has to agree whether a 50-minute focus session is still running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundscapeSession {
    pub scenario: String,
    pub duration_ms: u64,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub running_since_ms: Option<u64>,
}

impl SoundscapeState {
    /// The preset as the engine's own type, or `Edm` for a name this build does not know. The
    /// conductor rejects unknown presets on the way in, so a name that does not parse here is a
    /// conductor newer than this app.
    pub fn preset(&self) -> crate::soundscape::Preset {
        crate::soundscape::Preset::from_key(&self.preset).unwrap_or(crate::soundscape::Preset::Edm)
    }

    /// The parameters as the engine's own type.
    pub fn params(&self) -> crate::soundscape::Params {
        self.params.into()
    }

    pub fn layers(&self) -> crate::soundscape::Layers {
        self.layers.into()
    }
}

/// One message in an Ask conversation (`POST /assistant/api/ask`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskTurn {
    pub role: String,
    pub content: String,
}

/// Configured cloud role exposed by the local assistant's privacy-safe discovery route.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AskProvider {
    pub role: String,
    pub provider: String,
    pub model: String,
    pub data_tier: String,
    pub available: bool,
    pub reason: Option<String>,
}

/// Reply from `POST /assistant/api/ask` (or deterministic capability fallback).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AskReply {
    pub text: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// One streamed event from `POST /assistant/api/ask/stream`.
#[derive(Clone, Debug, Deserialize)]
pub struct AskEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub message: String,
}

/// One frame of `GET /api/sjel-status/events`: both answers, pushed when either changes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Snapshot {
    pub health: Option<NodeHealth>,
    pub agent: Option<AgentView>,
    /// Why the shell could not read one of the two answers.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// The endpoint is not on loopback.
    NotLoopback,
    /// No token in the Keychain.
    NotSetUp,
    /// Nothing answered.
    Unreachable,
    /// The shell answered with an error status, and its `error` text when it sent one.
    Refused(u16, Option<String>),
    /// The shell answered 200 with a body this client cannot read.
    Unreadable,
    /// A provider credential could not be written to the OS keychain.
    CredentialUnavailable,
    /// The phone's own store failed, and why. No shell was asked.
    Store(String),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback => {
                f.write_str("SJEL_STATUS_URL must point to 127.0.0.1 or localhost.")
            }
            Self::NotSetUp => {
                f.write_str("No Sjel token on this Mac. Run tools/setup-inbound-auth.sh.")
            }
            Self::Unreachable => f.write_str("Sjel is not running on this Mac."),
            Self::Refused(_, Some(error)) => f.write_str(error),
            Self::Refused(status, None) => write!(f, "Sjel refused the request ({status})."),
            Self::Unreadable => f.write_str("Sjel answered with something this app cannot read."),
            Self::CredentialUnavailable => {
                f.write_str("Could not save the provider API key to the OS keychain.")
            }
            Self::Store(why) => write!(f, "This device could not save it: {why}"),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;

pub struct Shell {
    base: Url,
    client: reqwest::Client,
    token: Mutex<Option<String>>,
}

impl Shell {
    /// # Errors
    /// `NotLoopback` when `endpoint` is not an `http` URL on 127.0.0.1 or localhost.
    pub fn new(endpoint: &str) -> ShellResult<Self> {
        let base = Url::parse(endpoint).map_err(|_| ShellError::NotLoopback)?;
        let loopback =
            base.scheme() == "http" && matches!(base.host_str(), Some("127.0.0.1" | "localhost"));
        if !loopback {
            return Err(ShellError::NotLoopback);
        }
        Ok(Self {
            base,
            client: reqwest::Client::new(),
            token: Mutex::new(None),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    pub async fn health(&self) -> ShellResult<NodeHealth> {
        self.json(Method::GET, "/api/sjel-status/health", None, 10)
            .await
    }

    pub async fn travel_plans(&self) -> ShellResult<Vec<crate::travel::Plan>> {
        self.json(Method::GET, "/trips/api/plans", None, 10).await
    }

    pub async fn travel_details(&self, id: &str) -> ShellResult<crate::travel::Details> {
        let path = crate::travel::details_path(id)
            .map_err(|error| ShellError::Refused(400, Some(error)))?;
        self.json(Method::GET, &path, None, 10).await
    }

    /// `places` resolves a typed name to a coordinate; a miss answers `place: null`.
    ///
    /// The 20 s budget covers the provider request behind the capability, which its own
    /// throttle may hold back — a shorter one would report a slow geocoder as a failure.
    pub async fn places_geocode(
        &self,
        query: &str,
    ) -> ShellResult<Option<crate::travel::Coordinate>> {
        #[derive(serde::Deserialize)]
        struct Answer {
            place: Option<Point>,
        }
        #[derive(serde::Deserialize)]
        struct Point {
            latitude: Option<f64>,
            longitude: Option<f64>,
        }
        let body = serde_json::json!({ "query": query });
        let answer: Answer = self
            .json(Method::POST, "/places/api/geocode", Some(body), 20)
            .await?;
        Ok(answer
            .place
            .and_then(|point| crate::travel::Coordinate::new(point.latitude?, point.longitude?)))
    }

    pub async fn agent(&self) -> ShellResult<AgentView> {
        self.json(Method::GET, "/api/sjel-status/agent", None, 5)
            .await
    }

    /// Sets one capability's agent mode (`off` · `read-only` · `ask` · `auto`).
    ///
    /// Every gate reads the same policy on its next request, so nothing restarts. The shell
    /// refuses a mode it does not know and a capability with no gate, both as a 400 that names
    /// the reason; that text arrives as `ShellError::Refused`.
    pub async fn set_agent_mode(&self, capability: &str, mode: &str) -> ShellResult<()> {
        if !is_registry_name(capability) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let body = serde_json::json!({ "capability": capability, "mode": mode });
        self.send(Method::POST, "/api/sjel-status/agent/mode", Some(body), 5)
            .await
            .map(drop)
    }

    /// The live hardware sample. macmon is a capability, so the shell proxies `/macmon` to it;
    /// a stopped macmon is a refused request, which the page reports as "not running" rather
    /// than as a fault.
    pub async fn macmon(&self) -> ShellResult<crate::systems::MacmonSample> {
        // macmon samples the SMC every 5 s, so a shorter budget than that would report a slow
        // read as a failure on a loaded machine.
        self.json(Method::GET, "/macmon/json", None, 10).await
    }

    /// What fills the disk, from `tools/storage report --json`. A `du` walk per class, so it
    /// takes seconds and the page reads it on demand rather than on a timer.
    pub async fn storage(&self) -> ShellResult<crate::systems::StorageReport> {
        self.json(Method::GET, "/api/sjel-status/storage", None, 120)
            .await
    }

    /// The disk's own hourly readings, kept by sjel-status.
    pub async fn storage_history(&self) -> ShellResult<crate::systems::StorageHistory> {
        self.json(Method::GET, "/api/sjel-status/storage-history", None, 10)
            .await
    }

    /// Installed software outside this checkout, from `tools/updates --json`. The shell answers
    /// a cached report for up to 15 minutes, so a read is cheap.
    pub async fn updates(&self) -> ShellResult<crate::systems::UpdatesReport> {
        self.json(Method::GET, "/api/sjel-status/updates", None, 30)
            .await
    }

    /// Starts one class moving. The shell answers `202` before the work starts — a cargo step
    /// compiles for minutes — and the outcome arrives on the next report as `lastApply`.
    pub async fn apply_updates(&self, class: &str) -> ShellResult<()> {
        if !is_registry_name(class) {
            return Err(ShellError::Refused(
                400,
                Some("Not an update class.".into()),
            ));
        }
        let body = serde_json::json!({ "class": class });
        self.send(
            Method::POST,
            "/api/sjel-status/updates/apply",
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// The hourly host watch's open findings.
    pub async fn host_watch(&self) -> ShellResult<Vec<crate::systems::Finding>> {
        let findings: crate::systems::Findings = self
            .json(Method::GET, "/api/sjel-status/host-watch", None, 10)
            .await?;
        Ok(findings.findings)
    }

    /// The runtime profile in force on this machine.
    pub async fn runtime(&self) -> ShellResult<crate::systems::RuntimeStatus> {
        self.json(Method::GET, "/api/sjel-status/runtime", None, 10)
            .await
    }

    /// Saves a selection or an exception set. `expected_revision` makes a change from a stale
    /// read a `409` instead of silently overwriting whatever was saved in between.
    pub async fn set_runtime(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<crate::systems::RuntimeStatus> {
        self.json(Method::POST, "/api/sjel-status/runtime", Some(body), 10)
            .await
    }

    /// Agent skills across installed harnesses from `tools/harnesses status --json`.
    pub async fn packs(&self) -> ShellResult<crate::packs::PacksView> {
        self.json(Method::GET, "/api/sjel-status/packs", None, 30)
            .await
    }

    /// Deploy, sync or remove Packs on one harness.
    pub async fn write_pack(
        &self,
        action: &str,
        harness: &str,
        packs: &[String],
    ) -> ShellResult<crate::packs::PackWriteResult> {
        let body = serde_json::json!({
            "action": action,
            "harness": harness,
            "packs": packs,
        });
        self.json(Method::POST, "/api/sjel-status/packs", Some(body), 60)
            .await
    }

    /// Architectural self-model and live state from `GET /api/sjel-status/self`.
    pub async fn self_model(&self) -> ShellResult<crate::self_model::SelfModelResponse> {
        self.json(Method::GET, "/api/sjel-status/self", None, 15).await
    }

    /// CV status, applications, profiles and built PDFs from `GET /api/sjel-status/cv`.
    pub async fn cv(&self) -> ShellResult<crate::cv::CvView> {
        self.json(Method::GET, "/api/sjel-status/cv", None, 20).await
    }

    /// Run one CV verb (build, app-new, app-status, linkedin-diff, etc.) via `POST /api/sjel-status/cv`.
    pub async fn write_cv(&self, req: &crate::cv::CvWriteRequest) -> ShellResult<crate::cv::CvWriteResult> {
        let body = serde_json::to_value(req).map_err(|_| ShellError::Unreadable)?;
        self.json(Method::POST, "/api/sjel-status/cv", Some(body), 60).await
    }

    /// Read index status totals and indexed volumes via `GET /media/api/status`.
    pub async fn media_status(&self) -> ShellResult<crate::media::MediaStatus> {
        self.json(Method::GET, "/media/api/status", None, 15).await
    }

    /// Read the recent ingest runs via `GET /media/api/ingests`.
    pub async fn media_ingests(&self) -> ShellResult<crate::media::MediaIngestRunsResponse> {
        self.json(Method::GET, "/media/api/ingests", None, 15).await
    }

    /// Read full details of one ingest run via `GET /media/api/ingests/{id}`.
    pub async fn media_ingest(&self, id: i64) -> ShellResult<crate::media::MediaIngestDetail> {
        let path = format!("/media/api/ingests/{id}");
        self.json(Method::GET, &path, None, 20).await
    }

    /// Read combined media status and recent runs for the primary media view.
    pub async fn media_view(&self) -> ShellResult<crate::media::MediaView> {
        let (status, ingests) = futures::try_join!(self.media_status(), self.media_ingests())?;
        Ok(crate::media::MediaView {
            status,
            runs: ingests.runs,
        })
    }

    /// Read the research view containing markdown articles, systems, upstreams, and benchmarks.
    pub async fn research_view(&self) -> ShellResult<crate::research::ResearchView> {
        Ok(crate::research::default_research_view())
    }

    /// Add a watch entry to `upstreams.toml` via `POST /api/sjel-status/upstreams/watch`.
    pub async fn watch_upstream(
        &self,
        url: &str,
        summary: &str,
        name: Option<&str>,
    ) -> ShellResult<crate::research::UpstreamRow> {
        let mut body = serde_json::json!({
            "url": url,
            "summary": summary,
        });
        if let Some(n) = name {
            body["name"] = serde_json::Value::String(n.to_string());
        }
        self.json(Method::POST, "/api/sjel-status/upstreams/watch", Some(body), 30).await
    }

    /// Entries overlapping `[from, to)`, both `YYYY-MM-DD`: `to` is exclusive.
    pub async fn calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        let path = format!("/calendar/api/entries?from={from}&to={to}");
        self.json(Method::GET, &path, None, 5).await
    }

    /// Creates an entry from a `core::calendar::Draft` body.
    pub async fn create_calendar_entry(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        self.json(Method::POST, "/calendar/api/entries", Some(body), 10)
            .await
    }

    pub async fn delete_calendar_entry(&self, id: &str) -> ShellResult<()> {
        // Ids are `<prefix>:<hex>` (`generated_id` in the calendar store).
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
        {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.send(
            Method::DELETE,
            &format!("/calendar/api/entries/{id}"),
            None,
            10,
        )
        .await
        .map(drop)
    }

    /// Read the finance projection and transactions for a date window (or all if None).
    pub async fn finance_dashboard(
        &self,
        start: Option<&str>,
        end: Option<&str>,
    ) -> ShellResult<FinanceDashboardProjection> {
        let mut query = Vec::new();
        if let Some(s) = start {
            query.push(("start", s));
        }
        if let Some(e) = end {
            query.push(("end", e));
        }
        let path = with_query("/finance/api/dashboard", &query);
        self.json(Method::GET, &path, None, 10).await
    }

    /// Read all subscriptions with prices and state history (`GET /finance/api/subscriptions`).
    pub async fn subscriptions(&self) -> ShellResult<Vec<SubscriptionRecord>> {
        self.json(Method::GET, "/finance/api/subscriptions", None, 10)
            .await
    }

    /// Read people entities (`GET /entities/api/entities?kind=person`).
    pub async fn people(&self, query: Option<&str>) -> ShellResult<Vec<EntityRecord>> {
        let mut pairs = vec![("kind", "person")];
        if let Some(q) = query {
            if !q.is_empty() {
                pairs.push(("q", q));
            }
        }
        let path = with_query("/entities/api/entities", &pairs);
        #[derive(Deserialize)]
        struct Envelope {
            entities: Vec<EntityRecord>,
        }
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.entities)
    }

    pub async fn projects(&self) -> ShellResult<Vec<ProjectSummary>> {
        #[derive(Deserialize)]
        struct List {
            projects: Vec<ProjectSummary>,
        }
        let list: List = self
            .json(Method::GET, "/projects/api/projects", None, 5)
            .await?;
        Ok(list.projects)
    }

    /// A project's records without their content: what a list of notes draws.
    pub async fn project(&self, name: &str) -> ShellResult<Vec<ProjectRecord>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<ProjectRecord>,
        }
        let path = with_query(
            "/projects/api/project",
            &[("name", name), ("content", "false")],
        );
        let records: Records = self.json(Method::GET, &path, None, 10).await?;
        Ok(records.records)
    }

    /// One record with its content: what the note editor opens.
    pub async fn record(&self, path: &str) -> ShellResult<ProjectRecord> {
        #[derive(Deserialize)]
        struct One {
            record: ProjectRecord,
        }
        let path = with_query("/projects/api/record", &[("path", path)]);
        let one: One = self.json(Method::GET, &path, None, 10).await?;
        Ok(one.record)
    }

    /// Every note across projects (`GET /projects/api/records`).
    pub async fn notes(&self) -> ShellResult<Vec<GlobalNote>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<RecordItem>,
        }
        #[derive(Deserialize)]
        struct RecordItem {
            id: String,
            #[serde(default)]
            kind: String,
            title: String,
            #[serde(default)]
            project: String,
        }
        let notes_req: Result<Records, _> = self
            .json(
                Method::GET,
                "/projects/api/records?kind=note&content=false",
                None,
                10,
            )
            .await;
        let mut list = Vec::new();
        if let Ok(records) = notes_req {
            for r in records.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        if let Ok(projects) = self
            .json::<Records>(
                Method::GET,
                "/projects/api/records?kind=project&content=false",
                None,
                10,
            )
            .await
        {
            for r in projects.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        list.sort_by(|a, b| {
            a.project
                .cmp(&b.project)
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(list)
    }

    /// Replaces a record's content. `base` is the content the edit started from; the capability
    /// answers 409 and writes nothing when the record changed since.
    pub async fn save_record(&self, path: &str, content: &str, base: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path, "content": content, "base": base });
        self.send(Method::POST, "/projects/api/record", Some(body), 10)
            .await
            .map(drop)
    }

    /// Marks a task done, or open again, and answers with the record's new frontmatter.
    pub async fn set_task_done(&self, path: &str, done: bool) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "id": path, "done": done });
        let task: Task = self
            .json(Method::POST, "/projects/api/tasks/done", Some(body), 10)
            .await?;
        Ok(task.task)
    }

    /// Sets one frontmatter key and answers with the record's new frontmatter, so a caller can
    /// update one row instead of reading the project again.
    pub async fn set_property(
        &self,
        path: &str,
        key: &str,
        value: &str,
    ) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "path": path, "key": key, "value": value });
        let task: Task = self
            .json(
                Method::POST,
                "/projects/api/record/property",
                Some(body),
                10,
            )
            .await?;
        Ok(task.task)
    }

    pub async fn add_task(&self, project: &str, title: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "project": project, "title": title });
        self.send(Method::POST, "/projects/api/tasks", Some(body), 10)
            .await
            .map(drop)
    }

    /// Tasks across all live projects, optionally filtered by ?status=open|done.
    pub async fn tasks(&self, status: Option<&str>) -> ShellResult<Vec<GlobalTask>> {
        #[derive(Deserialize)]
        struct List {
            tasks: Vec<GlobalTask>,
        }
        let path = match status {
            Some(s) => format!("/projects/api/tasks?status={s}"),
            None => "/projects/api/tasks".to_string(),
        };
        let list: List = self.json(Method::GET, &path, None, 10).await?;
        Ok(list.tasks)
    }

    /// Creates a project hub.
    pub async fn add_project(&self, name: &str, summary: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "name": name, "summary": summary });
        self.send(Method::POST, "/projects/api/projects", Some(body), 10)
            .await
            .map(drop)
    }

    /// Creates a new note in a live project.
    pub async fn add_note(&self, project: &str, title: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "project": project, "title": title, "summary": "" });
        self.send(Method::POST, "/projects/api/notes", Some(body), 10)
            .await
            .map(drop)
    }

    /// Removes a record from the projects store and disk.
    pub async fn remove_record(&self, path: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path });
        self.send(Method::POST, "/projects/api/record/remove", Some(body), 10)
            .await
            .map(drop)
    }

    /// Allows or denies one waiting agent write.
    pub async fn decide(&self, id: &str, allow: bool) -> ShellResult<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not an approval id.".into())));
        }
        let path = format!("/api/sjel-status/agent/approvals/{id}");
        let body = serde_json::json!({ "allow": allow });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Starts or stops one capability. A start waits for the service to come up.
    pub async fn set_running(&self, name: &str, running: bool) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let action = if running { "start" } else { "stop" };
        let path = format!("/api/sjel-status/capabilities/{name}/{action}");
        self.send(Method::POST, &path, None, 60).await.map(drop)
    }

    pub async fn backups(&self) -> ShellResult<Backups> {
        self.json(Method::GET, "/api/sjel-status/backups", None, 10)
            .await
    }

    pub async fn backup_targets(&self) -> ShellResult<Vec<crate::backup::BackupTargetRecord>> {
        let res: crate::backup::BackupTargetsResponse = self
            .json(Method::GET, "/api/sjel-status/backup/targets", None, 10)
            .await?;
        Ok(res.targets)
    }

    pub async fn backup_runs(
        &self,
        limit: usize,
    ) -> ShellResult<(Vec<crate::backup::BackupRunAttemptRecord>, usize)> {
        let path = format!("/api/sjel-status/backup/runs?limit={limit}");
        let res: crate::backup::BackupRunsResponse =
            self.json(Method::GET, &path, None, 10).await?;
        Ok((res.runs, res.running))
    }

    pub async fn verify_backup_target(
        &self,
        target: &str,
    ) -> ShellResult<crate::backup::VerifyTargetResult> {
        let body = serde_json::json!({ "target": target });
        self.json(
            Method::POST,
            "/api/sjel-status/backup/verify",
            Some(body),
            60,
        )
        .await
    }

    pub async fn set_backup_policy(
        &self,
        target: &str,
        interval_hours: Option<i64>,
    ) -> ShellResult<()> {
        let body = serde_json::json!({
            "target": target,
            "interval_hours": interval_hours,
        });
        self.send(
            Method::POST,
            "/api/sjel-status/backup/policy",
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// Starts one capability's backup to the target its manifest declares or an optional specific target.
    pub async fn backup(&self, name: &str) -> ShellResult<()> {
        self.backup_capability(name, None).await
    }

    pub async fn backup_capability(&self, name: &str, target: Option<&str>) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let path = format!("/api/sjel-status/capabilities/{name}/backup");
        let body = target.map(|t| serde_json::json!({ "target": t }));
        self.send(Method::POST, &path, body, 10).await.map(drop)
    }

    /// Sorts every file under `folder` without importing. Walks the whole tree, so a large
    /// archive takes seconds.
    pub async fn import_plan(&self, folder: &str) -> ShellResult<ImportPlan> {
        let path = with_query("/import/api/import/plan", &[("folder", folder)]);
        self.json(Method::GET, &path, None, 120).await
    }

    pub async fn import_status(&self, folder: &str) -> ShellResult<ImportStatus> {
        let path = with_query("/import/api/import/status", &[("folder", folder)]);
        self.json(Method::GET, &path, None, 10).await
    }

    /// Starts an import in the background. 409 while another runs.
    pub async fn import_start(&self, folder: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "folder": folder });
        self.send(Method::POST, "/import/api/import/runs", Some(body), 10)
            .await
            .map(drop)
    }

    pub async fn import_current(&self) -> ShellResult<Option<ImportJob>> {
        let envelope: ImportJobEnvelope = self
            .json(Method::GET, "/import/api/import/runs/current", None, 10)
            .await?;
        Ok(envelope.job)
    }

    pub async fn software_apps(&self) -> ShellResult<SoftwareApps> {
        self.json(Method::GET, "/software/api/apps", None, 10).await
    }

    pub async fn inventory_rows(&self) -> ShellResult<Vec<crate::inventory::InventoryRow>> {
        self.json(Method::GET, "/inventory/api/inventory", None, 10)
            .await
    }

    pub async fn inventory_wishlist(&self) -> ShellResult<crate::inventory::InventoryWishlist> {
        self.json(Method::GET, "/inventory/api/wishlist", None, 10)
            .await
    }

    pub async fn interior_layouts(
        &self,
    ) -> ShellResult<Vec<crate::inventory::InteriorLayoutSummary>> {
        self.json(Method::GET, "/interior/api/layouts", None, 10)
            .await
    }

    /// Scans now. Waits for the scan, which takes tens of seconds on a full /Applications.
    pub async fn software_scan(&self) -> ShellResult<()> {
        let body = serde_json::json!({});
        self.send(Method::POST, "/software/api/apps/scan", Some(body), 120)
            .await
            .map(drop)
    }

    /// Appends a decision about one app. `decision` is `use`, `remove`, `replace` or `rebuild`.
    /// Uninstalls a row whose decision is `remove`. Waits for it: a cask or a large app takes a
    /// while, and the service rescans afterwards.
    pub async fn software_uninstall(&self, bundle_id: &str) -> ShellResult<()> {
        let path = format!("/software/api/apps/{}/uninstall", software_key(bundle_id)?);
        let body = serde_json::json!({});
        self.send(Method::POST, &path, Some(body), 300)
            .await
            .map(drop)
    }

    pub async fn software_decide(
        &self,
        bundle_id: &str,
        decision: &str,
        note: &str,
    ) -> ShellResult<()> {
        let path = format!("/software/api/apps/{}/decision", software_key(bundle_id)?);
        let body = serde_json::json!({ "decision": decision, "note": note });
        self.send(Method::POST, &path, Some(body), 10)
            .await
            .map(drop)
    }

    /// Follows the shell's change stream. The receiver ends when the shell has no stream to
    /// send, so a caller that gets no frame keeps reading on its own timer; after a restart the
    /// connection is reopened silently.
    ///
    /// # Errors
    /// `NotSetUp`, `NotLoopback` or `Unreadable`, before any connection is opened.
    pub fn subscribe(&self) -> ShellResult<UnboundedReceiver<Snapshot>> {
        let token = self.token()?;
        let url = self
            .base
            .join(EVENTS_PATH)
            .map_err(|_| ShellError::Unreadable)?;
        let (to, from) = futures::channel::mpsc::unbounded();
        let client = self.client.clone();
        spawn(async move {
            while !to.is_closed() {
                match follow(&client, &url, &token, &to).await {
                    Followed::Unsupported => return,
                    Followed::Lost => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
        Ok(from)
    }

    /// Runs one turn in a native Ask session through `POST /assistant/api/ask`, starting the
    /// on-demand `assistant` capability if it is stopped, and falling back to a direct capability
    /// summary when the local model role is unavailable.
    pub async fn ask(&self, prompt: &str, history: &[AskTurn]) -> ShellResult<AskReply> {
        let body = serde_json::json!({ "prompt": prompt, "history": history });
        match self
            .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body.clone()), 60)
            .await
        {
            Ok(reply) => return Ok(reply),
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                if let Ok(reply) = self
                    .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body), 60)
                    .await
                {
                    return Ok(reply);
                }
            }
            Err(ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback) => {
                return self
                    .json::<AskReply>(Method::POST, "/assistant/api/ask", Some(body), 5)
                    .await;
            }
            Err(_) => {}
        }
        self.capability_fallback(prompt).await
    }

    /// Lists configured provider roles without retrieving credential values or endpoint URLs.
    pub async fn ask_providers(&self) -> ShellResult<Vec<AskProvider>> {
        let result = self
            .json::<Vec<AskProvider>>(Method::GET, "/assistant/api/ask/providers", None, 10)
            .await;
        match result {
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                self.json::<Vec<AskProvider>>(Method::GET, "/assistant/api/ask/providers", None, 10)
                    .await
            }
            other => other,
        }
    }

    /// `GET /api/sjel-status/credentials`: every declared credential with its presence. No value.
    pub async fn credentials(&self) -> ShellResult<Vec<crate::accounts::CredentialRow>> {
        self.json(Method::GET, "/api/sjel-status/credentials", None, 10)
            .await
    }

    /// Store a provider key without sending it to the Sjel HTTP service. The input is held in a
    /// zeroizing buffer, and the private catalog receives only the stable credential ID.
    pub async fn store_provider_key(
        &self,
        provider: &str,
        key: zeroize::Zeroizing<String>,
    ) -> ShellResult<()> {
        let (slug, label) = match provider {
            "openai" => ("openai", "OpenAI API"),
            "deepseek" => ("deepseek", "DeepSeek API"),
            "opencode-go" => ("opencode-go", "OpenCode Go API"),
            "nvidia-nim" => ("nvidia-nim", "NVIDIA NIM API"),
            "gemini" => ("gemini", "Google Gemini API (AI Studio)"),
            _ => return Err(ShellError::CredentialUnavailable),
        };
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider,
                ProviderKind, SecretValue,
            };
            use std::sync::Arc;

            let id = CredentialId::parse(format!("inference-{slug}"))
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let secret = SecretValue::new(key.as_bytes().to_vec())
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let exists = manager
                .list()
                .map_err(|_| ShellError::CredentialUnavailable)?
                .iter()
                .any(|entry| entry.id == id);
            if exists {
                manager.update(&id, &secret)
            } else {
                manager
                    .create(
                        id,
                        label.to_owned(),
                        ProviderKind::Keychain,
                        vec!["sjel-inference".to_owned()],
                        &secret,
                    )
                    .map(|_| ())
            }
            .map_err(|_| ShellError::CredentialUnavailable)
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Remove a saved provider key by its public provider slug.
    pub async fn remove_provider_key(&self, provider: &str) -> ShellResult<()> {
        let slug = match provider {
            "openai" | "deepseek" | "opencode-go" | "nvidia-nim" | "gemini" => provider,
            _ => return Err(ShellError::CredentialUnavailable),
        }
        .to_owned();
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider,
            };
            use std::sync::Arc;

            let id = CredentialId::parse(format!("inference-{slug}"))
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            manager
                .delete(&id)
                .map_err(|_| ShellError::CredentialUnavailable)
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Starts an event-streamed Ask run and returns its steering ID with a receiver for events.
    pub async fn ask_stream(
        &self,
        prompt: &str,
        history: &[AskTurn],
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        self.ask_stream_with_provider(prompt, history, None, false)
            .await
    }

    /// Stream Ask with an explicit cloud role and per-prompt public-data acknowledgement.
    pub async fn ask_stream_with_provider(
        &self,
        prompt: &str,
        history: &[AskTurn],
        provider_role: Option<&str>,
        confirm_public_send: bool,
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        let mut body = serde_json::json!({
            "prompt": prompt,
            "history": history,
            "confirm_public_send": confirm_public_send,
        });
        if let Some(role) = provider_role {
            body["provider_role"] = serde_json::Value::String(role.to_owned());
        }
        if provider_role.is_some() && !history.is_empty() {
            return Err(ShellError::Refused(
                409,
                Some("cloud Ask does not send local conversation history".into()),
            ));
        }
        let response = match self
            .send(
                Method::POST,
                "/assistant/api/ask/stream",
                Some(body.clone()),
                600,
            )
            .await
        {
            Ok(response) => response,
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                match self
                    .send(Method::POST, "/assistant/api/ask/stream", Some(body), 600)
                    .await
                {
                    Ok(response) => response,
                    Err(error) if provider_role.is_some() => return Err(error),
                    Err(_) => return self.fallback_ask_stream(prompt).await,
                }
            }
            Err(ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback) => {
                if provider_role.is_some() {
                    return Err(ShellError::Unreachable);
                }
                return self.fallback_ask_stream(prompt).await;
            }
            Err(ShellError::Refused(404 | 405, _)) => {
                if provider_role.is_some() {
                    return Err(ShellError::Refused(
                        503,
                        Some("the assistant API does not support cloud Ask".into()),
                    ));
                }
                let reply = self.ask(prompt, history).await?;
                return Ok(Self::completed_ask_stream(reply));
            }
            Err(ShellError::Refused(503, _)) if provider_role.is_none() => {
                return self.fallback_ask_stream(prompt).await;
            }
            Err(error) => return Err(error),
        };
        let run_id = response
            .headers()
            .get("x-sjel-ask-run")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(ShellError::Unreadable)?;
        let (to, from) = futures::channel::mpsc::unbounded();
        spawn(async move {
            let mut response = response;
            let mut buffer = Vec::new();
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) | Err(_) => break,
                };
                buffer.extend_from_slice(&chunk);
                while let Some(data) = next_frame(&mut buffer) {
                    if let Ok(event) = serde_json::from_str::<AskEvent>(&data) {
                        let terminal = matches!(event.kind.as_str(), "done" | "error" | "stopped");
                        if to.unbounded_send(event).is_err() || terminal {
                            return;
                        }
                    }
                }
            }
        });
        Ok((run_id, from))
    }

    async fn fallback_ask_stream(
        &self,
        prompt: &str,
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        let reply = self.capability_fallback(prompt).await?;
        Ok(Self::completed_ask_stream(reply))
    }

    fn completed_ask_stream(reply: AskReply) -> (u64, UnboundedReceiver<AskEvent>) {
        let (to, from) = futures::channel::mpsc::unbounded();
        let _ = to.unbounded_send(AskEvent {
            kind: "done".into(),
            content: String::new(),
            name: String::new(),
            arguments: String::new(),
            text: reply.text,
            model: reply.model,
            source: reply.source,
            tools: reply.tools,
            message: String::new(),
        });
        (0, from)
    }

    /// Queues an instruction for the next model request in an active Ask run.
    pub async fn steer_ask(&self, run_id: u64, message: &str) -> ShellResult<()> {
        let path = format!("/assistant/api/ask/{run_id}/steer");
        let body = serde_json::json!({ "message": message });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Requests cooperative cancellation of an active Ask run.
    pub async fn stop_ask(&self, run_id: u64) -> ShellResult<()> {
        let path = format!("/assistant/api/ask/{run_id}/stop");
        self.send(Method::POST, &path, Some(serde_json::json!({})), 5)
            .await
            .map(drop)
    }

    async fn capability_fallback(&self, prompt: &str) -> ShellResult<AskReply> {
        if !is_capability_status_question(prompt) {
            return Ok(AskReply {
                text: "The local assistant is unavailable, so I can’t answer that prompt yet. I can still show a live capability summary when you ask which capabilities are up.".into(),
                model: "assistant unavailable".into(),
                source: "fallback".into(),
                tools: Vec::new(),
            });
        }
        let health = self.health().await?;
        let up = health.capabilities.values().filter(|c| c.up).count();
        let down = health.down();
        let projects = self.projects().await.unwrap_or_default();
        let mut lines = vec![format!(
            "Sjel {} (up {}m): {} of {} capabilities running{}.",
            health.version,
            health.uptime_seconds / 60,
            up,
            health.capabilities.len(),
            if down.is_empty() {
                String::new()
            } else {
                format!("; down: {}", down.join(", "))
            }
        )];
        if !projects.is_empty() {
            let names: Vec<String> = projects
                .iter()
                .filter(|p| !p.archived)
                .map(|p| format!("{} ({})", p.name, p.notes))
                .collect();
            lines.push(format!("Active projects: {}.", names.join(", ")));
        }
        Ok(AskReply {
            text: lines.join("\n"),
            model: "capabilities".into(),
            source: "deterministic".into(),
            tools: vec!["sjel_health".into(), "projects_list".into()],
        })
    }

    /// A single-use URL that logs the browser in and lands on `next`
    /// (`landing` in `capabilities/sjel-status/src/session.rs`).
    pub async fn dashboard_url(&self, next: &str) -> ShellResult<Url> {
        #[derive(Deserialize)]
        struct Ticket {
            open: String,
        }
        let ticket: Ticket = self.json(Method::POST, TICKET_PATH, None, 5).await?;
        landing_url(&self.base, &ticket.open, next).ok_or(ShellError::Unreadable)
    }

    /// Read soundscape state (`GET /soundscape/api/soundscape/state`).
    /// Stream document metadata in batches through `tx` (typically 500 records at a time).
    pub async fn documents_stream(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        let mut loaded = 0;
        loop {
            let path = format!("/documents/api/documents?limit=500&offset={loaded}");
            let page: DocumentPage = self.json(Method::GET, &path, None, 10).await?;
            let page_len = page.documents.len();
            let total = page.total;
            loaded += page_len;
            let _ = tx.send(page.documents);
            if loaded >= total || page_len == 0 {
                return Ok(loaded);
            }
        }
    }

    /// Documents metadata through the authenticated local capability proxy. Text is only read
    /// for a single document through `document_detail`.
    pub async fn documents(&self) -> ShellResult<Vec<DocumentSummary>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let stream = self.documents_stream(tx);
        let mut rows = Vec::new();
        let (res, _) = futures::join!(stream, async {
            while let Some(batch) = rx.recv().await {
                rows.extend(batch);
            }
        });
        res?;
        Ok(rows)
    }

    /// Read one page of unresolved duplicate candidates from the local operator queue.
    pub async fn duplicate_candidates(
        &self,
        kind: Option<DuplicateCandidateKind>,
        offset: usize,
        limit: usize,
    ) -> ShellResult<DuplicateCandidatePage> {
        if !(1..=100).contains(&limit) {
            return Err(ShellError::Refused(
                400,
                Some("Candidate page limit must be 1..100.".into()),
            ));
        }
        let kind = kind.map(|kind| match kind {
            DuplicateCandidateKind::ExactCopy => "exact_copy",
            DuplicateCandidateKind::Rescan => "rescan",
            DuplicateCandidateKind::Part => "part",
        });
        let kind = kind.map_or_else(String::new, |kind| format!("&kind={kind}"));
        let path =
            format!("/documents/api/documents/duplicates?limit={limit}&offset={offset}{kind}");
        self.json(Method::GET, &path, None, 30).await
    }

    /// Save an explicit human decision for a candidate without changing source files.
    pub async fn review_duplicate(&self, decision: DuplicateDecision) -> ShellResult<()> {
        let body = serde_json::to_value(decision).map_err(|_| ShellError::Unreadable)?;
        let response: DuplicateDecisionResponse = self
            .json(
                Method::POST,
                "/documents/api/documents/duplicates/review",
                Some(body),
                30,
            )
            .await?;
        let _ = response.decision;
        Ok(())
    }

    pub async fn document_detail(&self, id: &str) -> ShellResult<DocumentDetail> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::GET,
            &format!("/documents/api/documents/{id}"),
            None,
            10,
        )
        .await
    }

    /// Append a classification review outcome. This never files the document.
    pub async fn review_document_proposal(
        &self,
        id: &str,
        proposal_id: &str,
        outcome: ProposalReview,
    ) -> ShellResult<DocumentProposalRecord> {
        if !is_sha256(id) || !is_sha256(proposal_id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a document proposal id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/{proposal_id}/review"),
            Some(serde_json::json!({"outcome": outcome})),
            10,
        )
        .await
    }

    /// Regenerate local deterministic rule proposals and append a new run.
    pub async fn regenerate_document_proposals(
        &self,
        id: &str,
    ) -> ShellResult<Vec<DocumentProposalRecord>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/regenerate"),
            Some(serde_json::json!({})),
            10,
        )
        .await
    }

    pub async fn review_document(
        &self,
        id: &str,
        category: &str,
        project: Option<&str>,
    ) -> ShellResult<DocumentSummary> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let body = serde_json::json!({"category": category, "project": project});
        self.json(
            Method::PATCH,
            &format!("/documents/api/documents/{id}"),
            Some(body),
            10,
        )
        .await
    }

    pub async fn archive_document(&self, id: &str) -> ShellResult<DocumentSummary> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::DELETE,
            &format!("/documents/api/documents/{id}"),
            None,
            10,
        )
        .await
    }

    /// The hash-verified original bytes (`GET /api/documents/{id}/original`).
    ///
    /// A successful read is the original and not a substitute: the capability checks the digest
    /// before it answers. The read is bounded by [`MAX_DOCUMENT_BYTES`] whether or not the response
    /// declares a length, because a reader that draws pages has to know the most it can be handed.
    pub async fn document_original(&self, id: &str) -> ShellResult<Vec<u8>> {
        let path = document_original_path(id)
            .ok_or_else(|| ShellError::Refused(400, Some("Not a document id.".into())))?;
        let mut response = self.send(Method::GET, &path, None, 30).await?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut original = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ShellError::Unreadable)? {
            if original.len() + chunk.len() > MAX_DOCUMENT_BYTES {
                return Err(too_large());
            }
            original.extend_from_slice(&chunk);
        }
        Ok(original)
    }

    /// Upload one file as raw bytes (`POST /api/documents?filename=...`), idempotent by bytes and
    /// filename. The capability refuses names that suggest confidential material before it reads
    /// the body, and that refusal arrives here as `ShellError::Refused(403, ...)`. Repeating an
    /// upload keeps the record and its filing decision, so `created` is false the second time.
    pub async fn upload_document(
        &self,
        filename: &str,
        bytes: Vec<u8>,
    ) -> ShellResult<DocumentUpload> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(too_large());
        }
        let path = with_query("/documents/api/documents", &[("filename", filename)]);
        let response = self
            .send_payload(Method::POST, &path, Some(Payload::Bytes(bytes)), 60)
            .await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    /// The feed for the last `days` days, dismissed entries included so Kept and Dismissed can
    /// be shown without a second read.
    pub async fn feed(&self, days: u32) -> ShellResult<Vec<FeedEntry>> {
        let days = days.to_string();
        let path = with_query(
            "/comms/feed",
            &[("days", &days), ("include_dismissed", "true")],
        );
        match self.json(Method::GET, &path, None, 15).await? {
            FeedList::Rows(rows) => Ok(rows),
            FeedList::Failed { error } => Err(ShellError::Refused(500, Some(error))),
        }
    }

    pub async fn feed_detail(&self, id: &str) -> ShellResult<FeedDetail> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        self.json(Method::GET, &format!("/comms/feed/{id}"), None, 10)
            .await
    }

    /// Keep (`keeper`), dismiss (`dismissed`) or undo (`new`). The only route that writes those
    /// decisions; it records the ledger row in the same transaction.
    pub async fn set_feed_status(
        &self,
        id: &str,
        status: &str,
        surface: FeedSurface,
    ) -> ShellResult<()> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        let body = serde_json::json!({"status": status, "surface": surface.as_str()});
        self.send(
            Method::POST,
            &format!("/comms/feed/{id}/status"),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// Record that the reader opened an entry: `opened` the first time, `reopened` after a
    /// decision. The learned ranking factor counts these.
    pub async fn record_feed_open(&self, id: &str, first: bool) -> ShellResult<()> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        let event = if first { "opened" } else { "reopened" };
        let body = serde_json::json!({"event": event, "surface": FeedSurface::Reader.as_str()});
        self.send(
            Method::POST,
            &format!("/comms/feed/{id}/interactions"),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// The capture inbox, newest first.
    pub async fn captures(&self) -> ShellResult<Vec<Capture>> {
        self.json(Method::GET, "/comms/captures", None, 10).await
    }

    /// Stores `text` as typed; comms takes a leading `#buy`, `#look`, `#go` or `#learn` as the tag.
    pub async fn add_capture(&self, text: Option<&str>, url: Option<&str>) -> ShellResult<Capture> {
        let body = serde_json::json!({"text": text, "url": url, "client": "desktop"});
        self.json(Method::POST, "/comms/captures", Some(body), 10)
            .await
    }

    /// Changes the fields `patch` names (`text`, `tag`, `status`, `routed_to`).
    pub async fn patch_capture(&self, id: &str, patch: serde_json::Value) -> ShellResult<Capture> {
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a capture id.".into())));
        }
        self.json(
            Method::PATCH,
            &format!("/comms/captures/{id}"),
            Some(patch),
            10,
        )
        .await
    }

    /// Sends a capture on and marks it done. The capture stays as it is when the target refuses.
    /// Follows `route` in `dashboard/src/lib/capture.ts`.
    pub async fn route_capture(&self, capture: &Capture, to: Route) -> ShellResult<Capture> {
        let target = match to {
            Route::Feed => {
                #[derive(Deserialize)]
                struct Ingested {
                    id: String,
                }
                let body = serde_json::json!({"url": capture.url, "client": "inbox"});
                let item: Ingested = self
                    .json(Method::POST, "/comms/ingest", Some(body), 60)
                    .await?;
                format!("feed:{}", item.id)
            }
            Route::Places => {
                #[derive(Deserialize)]
                struct Created {
                    id: String,
                }
                let text = capture
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty());
                let url = capture
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|u| !u.is_empty());
                let name = text
                    .map(str::to_owned)
                    .or_else(|| capture::host(url))
                    .or_else(|| url.map(str::to_owned))
                    .unwrap_or_else(|| "Unbenannter Ort".into());
                let body = serde_json::json!({
                    "name": name, "link": url, "note": text, "query": text.or(url),
                });
                let place: Created = self
                    .json(Method::POST, "/places/api/places", Some(body), 20)
                    .await?;
                format!("places:{}", place.id)
            }
            Route::Wishlist => {
                let (label, price_cents) = capture::extract_price(capture.text.as_deref());
                let label = Some(label)
                    .filter(|l| !l.is_empty())
                    .or_else(|| capture::host(capture.url.as_deref()))
                    .or_else(|| capture.url.clone())
                    .unwrap_or_else(|| "wunsch".into());
                let base = capture::slug(&label);
                let mut n = 1;
                loop {
                    let id = if n == 1 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    };
                    let mut body = serde_json::json!({
                        "id": id, "kind": "piece", "label": label, "link": capture.url,
                        "state": "wanted", "note": "aus der Inbox",
                    });
                    if let Some(cents) = price_cents {
                        body["price_cents"] = cents.into();
                    }
                    // A taken id answers 409; the next suffix is tried, twenty at most.
                    match self
                        .send(Method::POST, "/inventory/api/items", Some(body), 10)
                        .await
                    {
                        Ok(_) => break format!("inventory:{id}"),
                        Err(ShellError::Refused(409, _)) if n <= 20 => n += 1,
                        Err(error) => return Err(error),
                    }
                }
            }
        };
        self.patch_capture(
            &capture.id,
            serde_json::json!({"status": "done", "routed_to": target}),
        )
        .await
    }

    /// Every mail proposal; the client filters by status and category.
    pub async fn mail_proposals(&self) -> ShellResult<Vec<MailProposal>> {
        self.json(Method::GET, "/comms/triage", None, 15).await
    }

    /// One action across `ids`. Comms answers per id, so a partial failure is a result, not an
    /// error. Archive and trash reach Gmail.
    pub async fn mail_action(
        &self,
        ids: &[String],
        action: &MailAction,
    ) -> ShellResult<BulkResult> {
        let stream = match action {
            MailAction::Categorize(key) => Some(key.as_str()),
            _ => None,
        };
        let body = serde_json::json!({"ids": ids, "action": action.key(), "stream": stream});
        self.json(Method::POST, "/comms/triage/bulk", Some(body), 60)
            .await
    }

    pub async fn soundscape_state(&self) -> ShellResult<SoundscapeState> {
        self.json(Method::GET, "/soundscape/api/soundscape/state", None, 3)
            .await
    }

    /// Update playback status (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_playing(&self, playing: bool) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "playing": playing,
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Update soundscape preset (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_preset(&self, preset: &str) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "preset": preset,
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Update the scene parameters (`POST /soundscape/api/soundscape/state`).
    ///
    /// The conductor's update is a patch, so this asserts exactly the six values the operator
    /// moved and leaves preset, layers and volume to whoever else is driving them.
    pub async fn set_soundscape_params(
        &self,
        params: SoundscapeParams,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "params": params,
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Update the layer mix (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_layers(
        &self,
        layers: SoundscapeLayers,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "layers": layers,
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Update the master volume (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_volume(&self, volume: f32) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "volume": volume.clamp(0., 1.),
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Update how full the arrangement is (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_energy(&self, energy: f32) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "energy": energy.clamp(0., 1.),
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    /// Start a timed session (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_session(
        &self,
        session: Option<SoundscapeSession>,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "session": session,
            "origin": "gpui",
        });
        self.json(
            Method::POST,
            "/soundscape/api/soundscape/state",
            Some(body),
            3,
        )
        .await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<T> {
        let response = self.send(method, path, body, timeout_secs).await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        self.send_payload(method, path, body.map(Payload::Json), timeout_secs)
            .await
    }

    /// One request path for every body shape: JSON, raw bytes for an upload, or none. Keeping the
    /// credential read, the 401 recovery and the error text in one place is what stops the upload
    /// route from growing its own idea of what a refusal looks like.
    async fn send_payload(
        &self,
        method: Method,
        path: &str,
        payload: Option<Payload>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        let token = self.token()?;
        let url = self.base.join(path).map_err(|_| ShellError::Unreadable)?;
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(timeout_secs));
        request = match payload {
            None => request,
            Some(Payload::Json(body)) => request.json(&body),
            // The capability takes the filename from the query and the file from the body; a JSON
            // content type would make the body a parsed document instead of bytes.
            Some(Payload::Bytes(bytes)) => request
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(bytes),
        };
        let response = request.send().await.map_err(|_| ShellError::Unreachable)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            // The token was rotated: read the Keychain again on the next request.
            *self.token.lock().expect("token lock") = None;
        }
        if status.is_success() {
            return Ok(response);
        }
        let error = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
        Err(ShellError::Refused(status.as_u16(), error))
    }

    fn token(&self) -> ShellResult<String> {
        let mut cached = self.token.lock().expect("token lock");
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let user = std::env::var("USER").map_err(|_| ShellError::NotSetUp)?;
        let output = Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                &user,
                "-s",
                KEYCHAIN_SERVICE,
                "-w",
            ])
            .output()
            .map_err(|_| ShellError::NotSetUp)?;
        let token = String::from_utf8(output.stdout).map_err(|_| ShellError::NotSetUp)?;
        let token = token.trim();
        if !output.status.success() || token.is_empty() {
            return Err(ShellError::NotSetUp);
        }
        *cached = Some(token.to_owned());
        Ok(token.to_owned())
    }
}

fn is_capability_status_question(prompt: &str) -> bool {
    let words: Vec<String> = prompt
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let asks_about_capabilities = words
        .iter()
        .any(|word| word.starts_with("capabilit") || word == "services");
    let asks_about_status = ["up", "running", "status", "health", "available"]
        .iter()
        .any(|term| words.iter().any(|word| word == term));
    asks_about_capabilities && asks_about_status
}

/// A path with its query, each pair encoded once. The capability owns the names; this only joins
/// them, so a project name with a `/` or a space reaches it intact.
fn with_query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
    url.set_path(path);
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// Why one connection to the change stream ended.
enum Followed {
    /// The shell answered, but does not serve the stream: a build from before it existed.
    Unsupported,
    /// The shell or the network dropped it. Reconnecting is worth a try.
    Lost,
}

async fn follow(
    client: &reqwest::Client,
    url: &Url,
    token: &str,
    to: &UnboundedSender<Snapshot>,
) -> Followed {
    let response = client
        .get(url.clone())
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await;
    let Ok(response) = response else {
        return Followed::Lost;
    };
    let status = response.status();
    if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
        return Followed::Unsupported;
    }
    if !status.is_success() {
        return Followed::Lost;
    }
    let mut response = response;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Followed::Lost,
            Err(_) => return Followed::Lost,
        };
        buffer.extend_from_slice(&chunk);
        while let Some(data) = next_frame(&mut buffer) {
            if let Ok(snapshot) = serde_json::from_str::<Snapshot>(&data) {
                if to.unbounded_send(snapshot).is_err() {
                    return Followed::Unsupported;
                }
            }
        }
    }
}

/// One event's data, if a whole event has arrived. A comment line carries no data and yields an
/// empty string, which no snapshot parses as, so it passes through unremarked.
fn next_frame(buffer: &mut Vec<u8>) -> Option<String> {
    let end = buffer.windows(2).position(|pair| pair == b"\n\n")?;
    let block = String::from_utf8_lossy(&buffer[..end]).into_owned();
    buffer.drain(..end + 2);
    let mut data = String::new();
    for line in block.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    }
    Some(data)
}

/// Letters, digits and dashes: nothing a path can be built from.
fn is_registry_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Accepts only `/session/open?…` on the shell's own origin, so a wrong answer cannot send the
/// browser anywhere else.
fn landing_url(base: &Url, open: &str, next: &str) -> Option<Url> {
    if !open.starts_with(OPEN_PATH) {
        return None;
    }
    let mut url = base.join(open).ok()?;
    if url.origin() != base.origin() {
        return None;
    }
    url.query_pairs_mut().append_pair("next", next);
    Some(url)
}

/// Two worker threads for every request. GPUI's executor does not provide Tokio's reactor,
/// which reqwest needs. A token read through `/usr/bin/security` blocks its worker, so one
/// read must not be able to stall an unrelated request.
fn runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: LazyLock<tokio::runtime::Handle> = LazyLock::new(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build the HTTP runtime");
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("sjel-http".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("start the HTTP thread");
        handle
    });
    &RUNTIME
}

/// Runs `work` on the HTTP runtime; the returned future can be awaited from GPUI.
/// A software row key as one URL path segment. Keys are a bundle id (`com.example.app`) or
/// `<manager>:<name>`, where an npm name may be scoped (`npm:@playwright/cli`); `/` and `@` are
/// percent-encoded so the key stays one segment. Anything else is refused before it reaches a URL.
pub fn software_key(key: &str) -> ShellResult<String> {
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '@' | '/');
    // A key of only dots (`..`) would be a path segment the URL join collapses.
    if !key.chars().any(|c| c.is_ascii_alphanumeric()) || !key.chars().all(allowed) {
        return Err(ShellError::Refused(400, Some("Not a software key.".into())));
    }
    Ok(key.replace('@', "%40").replace('/', "%2F"))
}

pub fn run<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(work.await);
    });
    async move { rx.await.ok() }
}

/// Runs `work` on the HTTP runtime with nothing waiting for its answer, for a connection that
/// outlives one call.
pub fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(work);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_keys_stay_one_path_segment() {
        assert_eq!(software_key("com.example.app").unwrap(), "com.example.app");
        assert_eq!(
            software_key("npm:@playwright/cli").unwrap(),
            "npm:%40playwright%2Fcli"
        );
        assert!(software_key("../etc").is_ok_and(|k| !k.contains('/')));
        assert!(software_key("a b").is_err());
        assert!(software_key("x?y=1").is_err());
        assert!(software_key("").is_err());
        assert!(software_key("..").is_err());
    }

    #[test]
    fn deterministic_fallback_only_answers_capability_status_questions() {
        assert!(is_capability_status_question(
            "Which capabilities are up right now?"
        ));
        assert!(is_capability_status_question("Show capability health"));
        assert!(!is_capability_status_question("hi"));
        assert!(!is_capability_status_question("List my active projects"));
    }

    #[test]
    fn a_document_original_is_only_asked_for_by_a_digest() {
        let id = "a".repeat(64);
        assert_eq!(
            document_original_path(&id).as_deref(),
            Some(format!("/documents/api/documents/{id}/original").as_str())
        );
        assert!(document_original_path("../etc/passwd").is_none());
        assert!(document_original_path(&"a".repeat(63)).is_none());
        assert!(document_original_path(&"a".repeat(65)).is_none());
        assert!(document_original_path(&"g".repeat(64)).is_none());
        assert!(document_original_path("").is_none());
    }

    #[test]
    fn document_detail_decodes_separate_proposal_evidence_and_review_history() {
        let document_id = "a".repeat(64);
        let proposal_id = "b".repeat(64);
        let detail: DocumentDetail = serde_json::from_value(serde_json::json!({
            "document": {
                "id": document_id,
                "filename": "invoice.txt",
                "media_type": "text/plain",
                "byte_len": 10,
                "imported_at": "2026-10-22T10:00:00Z",
                "status": "inbox",
                "category": null,
                "project": null,
                "extracted_title": null,
                "extractor": "plain-text",
                "extracted_chars": 7,
                "suggested_category": "Finance",
                "suggestion_reason": "local rule",
                "data_class": "c2"
            },
            "text": "invoice",
            "proposals": [{
                "id": proposal_id,
                "generation_id": "generation-one",
                "generated_at": "2026-10-22T10:01:00Z",
                "proposal": {
                    "document_id": document_id,
                    "source": {"kind": "rules", "identity": "finance", "version": "v1"},
                    "outcome": {
                        "status": "proposed",
                        "category": "Finance",
                        "evidence": {"source": "extracted_text", "quote": "invoice"}
                    },
                    "confidence": null,
                    "review": "accepted"
                },
                "review_history": [{
                    "id": 1,
                    "outcome": "accepted",
                    "reviewed_at": "2026-10-22T10:02:00Z"
                }]
            }]
        }))
        .unwrap();
        assert_eq!(detail.proposals.len(), 1);
        assert_eq!(detail.proposals[0].id, proposal_id);
        assert_eq!(
            detail.proposals[0].proposal.review,
            Some(ProposalReview::Accepted)
        );
        assert_eq!(detail.proposals[0].review_history.len(), 1);
        assert_eq!(
            detail.proposals[0].proposal.outcome,
            ProposalOutcome::Proposed {
                category: "Finance".into(),
                evidence: ProposalEvidence {
                    source: ProposalEvidenceSource::ExtractedText,
                    quote: "invoice".into(),
                },
            }
        );
    }

    #[test]
    fn endpoint_must_be_loopback_http() {
        assert!(Shell::new("http://127.0.0.1:8082").is_ok());
        assert!(Shell::new("http://localhost:9000/").is_ok());
        assert_eq!(
            Shell::new("https://127.0.0.1:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://example.com:8082").err(),
            Some(ShellError::NotLoopback)
        );
        assert_eq!(
            Shell::new("http://127.0.0.1.evil:8082").err(),
            Some(ShellError::NotLoopback)
        );
    }

    #[test]
    fn landing_stays_on_the_shell() {
        let base = Url::parse(DEFAULT_ENDPOINT).unwrap();
        let url = landing_url(&base, "/session/open?ticket=ab", "/calendar").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8082/session/open?ticket=ab&next=%2Fcalendar"
        );
        assert!(landing_url(&base, "//evil.example/session/open?x", "/").is_none());
        assert!(landing_url(&base, "/elsewhere?ticket=ab", "/").is_none());
    }

    #[test]
    fn stopped_on_demand_capability_is_idle_not_down() {
        let health: NodeHealth = serde_json::from_str(
            r#"{"ok":true,"version":"v1","uptime_seconds":5,"capabilities":{
                "a":{"up":true,"autostart":true},
                "b":{"up":false,"autostart":false},
                "c":{"up":false}
            }}"#,
        )
        .unwrap();
        assert_eq!(health.capabilities["b"].liveness(), Liveness::Idle);
        assert_eq!(health.down(), vec!["c".to_owned()]);
    }

    #[test]
    fn registry_names_reject_paths() {
        assert!(is_registry_name("sjel-status"));
        assert!(!is_registry_name("../x"));
        assert!(!is_registry_name(""));
    }

    #[test]
    fn a_query_encodes_what_a_project_name_can_hold() {
        assert_eq!(
            with_query(
                "/projects/api/project",
                &[("name", "Home Lab"), ("content", "false")]
            ),
            "/projects/api/project?name=Home+Lab&content=false"
        );
        assert_eq!(
            with_query("/projects/api/record", &[("path", "A/B & C.md")]),
            "/projects/api/record?path=A%2FB+%26+C.md"
        );
    }

    #[test]
    fn a_list_read_without_content_still_parses() {
        let record: ProjectRecord = serde_json::from_str(
            r#"{"path":"P/a.md","kind":"note","title":"A","done":false,
                "due":null,"priority":2}"#,
        )
        .unwrap();
        assert_eq!(record.content, "");
        assert_eq!(record.title, "A");
    }

    #[test]
    fn a_task_reply_carries_the_path_as_its_id() {
        let task: GlobalTask = serde_json::from_str(
            r#"{"id":"P/t.md","kind":"task","title":"T","done":true,
                "due":"2026-10-24","priority":1,"project":"P"}"#,
        )
        .unwrap();
        assert_eq!(task.id, "P/t.md");
        assert!(task.done);
    }

    #[test]
    fn cloud_ask_rejects_history_before_any_request() {
        let shell = Shell::new("http://127.0.0.1:1").unwrap();
        let history = [AskTurn {
            role: "user".into(),
            content: "local conversation".into(),
        }];

        let error = futures::executor::block_on(shell.ask_stream_with_provider(
            "public prompt",
            &history,
            Some("cloud_assistant_demo"),
            true,
        ))
        .unwrap_err();

        assert!(
            matches!(error, ShellError::Refused(409, Some(reason)) if reason.contains("does not send local conversation history"))
        );
    }

    #[test]
    fn frames_come_out_whole_and_leave_the_rest() {
        let mut buffer =
            b"event: snapshot\ndata: {\"agent\":1}\n\n: keep-alive\n\nevent: snapshot\ndata: {\"hea"
                .to_vec();
        assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"agent\":1}"));
        // A comment carries no data and is not a snapshot.
        assert_eq!(next_frame(&mut buffer).as_deref(), Some(""));
        // The third event is still split across chunks.
        assert_eq!(next_frame(&mut buffer), None);
        buffer.extend_from_slice(b"lth\":2}\n\n");
        assert_eq!(next_frame(&mut buffer).as_deref(), Some("{\"health\":2}"));
    }

    #[test]
    fn a_frame_carries_both_answers_or_why_not() {
        let both: Snapshot = serde_json::from_str(
            r#"{"health":{"version":"v","uptime_seconds":1,"capabilities":{}},
                "agent":{"pending":[],"calls":[]}}"#,
        )
        .unwrap();
        assert!(both.health.is_some() && both.agent.is_some());
        assert!(both.error.is_none());

        let failed: Snapshot = serde_json::from_str(r#"{"error":"agent policy: x"}"#).unwrap();
        assert_eq!(failed.error.as_deref(), Some("agent policy: x"));
        assert!(failed.health.is_none());
    }

    #[test]
    fn an_ask_event_preserves_live_tool_activity() {
        let event: AskEvent = serde_json::from_str(
            r#"{"type":"tool_call","name":"calendar_entries","arguments":"{\"from\":\"2026-10-21\"}"}"#,
        )
        .unwrap();
        assert_eq!(event.kind, "tool_call");
        assert_eq!(event.name, "calendar_entries");
        assert!(event.arguments.contains("2026-10-21"));
    }
}
