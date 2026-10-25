//! Client for the Sjel shell (`capabilities/sjel-status`), authenticated with the inbound token.
//!
//! The same contracts and token as the menu-bar app (`apps/mac/Sources/SjelRelay`): the token
//! comes from the login Keychain item that `tools/setup-inbound-auth.sh` writes, through
//! `/usr/bin/security`, which that script puts on the item's access list.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
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

/// A registry entry that exposes a standalone interface (`GET /api/sjel-status/capabilities`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CapabilityPanel {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub panel_port: String,
    #[serde(default)]
    pub panel_path: String,
    #[serde(default)]
    pub up: Option<bool>,
    #[serde(default)]
    pub links_to: Vec<String>,
}

impl CapabilityPanel {
    pub fn url(&self, endpoint: &str) -> Option<Url> {
        let mut url = Url::parse(endpoint).ok()?;
        url.set_port(Some(self.panel_port.parse::<u16>().ok()?))
            .ok()?;
        url.set_path(&self.panel_path);
        Some(url)
    }
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

/// An entry read by id, which includes its capability-owned payload for forward references.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CalendarEntryDetail {
    #[serde(flatten)]
    pub entry: CalendarEntry,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CalendarGoogleExportOptIn {
    pub entry_id: String,
    pub google_calendar_id: String,
    pub google_event_id: Option<String>,
    pub pushed_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CalendarGoogleExportReport {
    #[serde(default)]
    pub calendar_id: String,
    #[serde(default)]
    pub home_timezone: String,
    #[serde(default)]
    pub opted_in: usize,
    #[serde(default)]
    pub inserted: usize,
    #[serde(default)]
    pub patched: usize,
    #[serde(default)]
    pub pushed: Vec<CalendarGoogleExportOutcome>,
    #[serde(default)]
    pub skipped: Vec<CalendarGoogleExportSkipped>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CalendarGoogleExportOutcome {
    pub entry_id: String,
    pub title: String,
    pub operation: String,
    pub google_event_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CalendarGoogleExportSkipped {
    pub google_event_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct CalendarGoogleExportOptOut {
    opted_out: bool,
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
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub shared_cents: i64,
    #[serde(default)]
    pub cash_amount_cents: i64,
    #[serde(default)]
    pub reimbursement_for: Option<String>,
}

/// Finance CSV profile returned by `GET /finance/api/import/csv/mappings`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FinanceCsvMappingProfile {
    pub label: String,
    pub mapping: serde_json::Value,
}

/// Candidate shown in the import review queue.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinanceCandidate {
    pub id: String,
    pub fingerprint: String,
    pub booked_at: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    pub source_account: String,
    pub source_reference: Option<String>,
    pub proposed_account: String,
    pub confidence_basis_points: u16,
    pub state: String,
    #[serde(default)]
    pub transfer_match_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinanceCsvPreview {
    pub preview_id: String,
    pub candidate_count: usize,
    pub duplicate_rows: usize,
    pub preserved_repetitions: usize,
    pub ignored_non_transaction_rows: usize,
    pub outflow_count: usize,
    pub inflow_count: usize,
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
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub shared_expenses: Vec<SharedExpenseSummary>,
    #[serde(default)]
    pub balance_snapshot: Option<ManualBalanceSnapshot>,
    #[serde(default)]
    pub tracked_net_worth: Option<TrackedNetWorth>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SharedExpenseSummary {
    pub source_id: String,
    pub candidate_id: String,
    pub date: String,
    pub description: String,
    pub account: String,
    pub category: String,
    pub purpose: Option<String>,
    pub trip_id: Option<String>,
    pub gross_cents: i64,
    pub personal_cents: i64,
    pub shared_cents: i64,
    pub reimbursed_cents: i64,
    pub outstanding_cents: i64,
    pub currency: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BalanceCoverage {
    Complete,
    #[default]
    Partial,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BalanceKind {
    Asset,
    Liability,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManualBalance {
    pub id: String,
    pub label: String,
    pub kind: BalanceKind,
    pub amount_cents: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManualBalanceSnapshot {
    pub schema_version: u32,
    pub as_of: String,
    pub updated_at: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManualBalanceUpdate {
    pub as_of: String,
    pub currency: String,
    #[serde(default)]
    pub coverage: BalanceCoverage,
    pub balances: Vec<ManualBalance>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TrackedNetWorth {
    pub currency: String,
    pub value: FinanceDecimalValue,
    pub manual_balance_cents: i64,
    pub portfolio_included: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FinanceDecimalValue {
    pub mantissa: String,
    pub scale: u32,
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct EntityRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub note_ref: Option<String>,
    #[serde(default)]
    pub revision: u32,
    /// The registry serves `{ value, source, updated_at }` per key; the Mac's mirrored copy holds
    /// the bare value. `people::field_value` reads both.
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
    /// Dated place facts. The mirrored copy does not select them, so a phone's list is empty.
    #[serde(default)]
    pub facts: Vec<EntityFact>,
}

/// One dated place fact (`capabilities/entities` `Fact`): a home base, or an away period.
/// `places` resolves the coordinate from `place` when it can, and the fact is kept either way.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct EntityFact {
    pub id: String,
    #[serde(default)]
    pub predicate: String,
    #[serde(default)]
    pub place: String,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_to: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub source: String,
}

/// A field a kind carries (`GET /entities/api/fields`): built in, or declared by the operator.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct EntityField {
    #[serde(default)]
    pub kind: String,
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub field_type: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub data_class: String,
    #[serde(default)]
    pub builtin: bool,
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
    /// The project the record is filed under, which is not the path's first segment: the vault
    /// files `Archive/Soma/Tasks/Reopen.md` under `Soma` (`capabilities/projects`). Absent from a
    /// read that did not carry it, in which case the path prefix is the best answer there is.
    #[serde(default)]
    pub project: String,
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

/// One server search result (`capabilities/sjel-status/src/status/search.rs`).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ServerHit {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// A `sjel://` deep link, opened through [`crate::route::parse_deep_link`].
    pub link: String,
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
    #[serde(default)]
    pub page_count: usize,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub page_range: Option<DocumentPageRange>,
    #[serde(default)]
    pub matched_pages: Vec<u32>,
    #[serde(default)]
    pub workflow_stage: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct DocumentPageRange {
    pub first: u32,
    pub last: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentSplitChildInput {
    pub first: u32,
    pub last: u32,
    pub filename: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentSplitRequest {
    pub page_count: u32,
    pub children: Vec<DocumentSplitChildInput>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DocumentExtractedPage {
    pub page_number: u32,
    pub text: String,
    pub engine: String,
    pub failure: Option<String>,
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
    #[serde(default)]
    pub pages: Vec<DocumentExtractedPage>,
    #[serde(default)]
    pub children: Vec<DocumentSummary>,
    #[serde(default)]
    pub split_suggestions: Option<DocumentSplitSuggestion>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DocumentBoundaryEvidence {
    pub before_page: u32,
    pub marker: String,
    pub quote: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DocumentSplitSuggestion {
    pub page_count: u32,
    pub ranges: Vec<DocumentPageRange>,
    pub boundaries: Vec<DocumentBoundaryEvidence>,
    pub abstention: Option<String>,
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

/// The shared reader projection for a feed or mail item (`GET /comms/content/:source/:id`).
/// The feed endpoint remains the source of feed triage state; this projection supplies the
/// bounded source body and local digest without coupling the reader to Comms' storage.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FeedContentDetail {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_label: String,
    #[serde(default)]
    pub day: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub content_status: String,
    #[serde(default)]
    pub digest: Option<FeedDigest>,
    #[serde(default)]
    pub calendar: Option<FeedCalendarContent>,
    #[serde(default)]
    pub cloud_processing: FeedCloudProcessingState,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FeedCloudProcessingState {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub preview_hash: Option<String>,
    #[serde(default)]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub dispatch_status: String,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub provider_role: Option<String>,
    #[serde(default)]
    pub queued_at: Option<String>,
    #[serde(default)]
    pub provider_calls: u8,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct CloudDerivativePreview {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub source_revision: String,
    #[serde(default)]
    pub preview_hash: String,
    #[serde(default)]
    pub original_data_class: String,
    #[serde(default)]
    pub derivative_data_class: String,
    #[serde(default)]
    pub transformation: String,
    #[serde(default)]
    pub document: String,
    #[serde(default)]
    pub redaction_count: usize,
    #[serde(default)]
    pub redactions: Vec<CloudRedactionFinding>,
    #[serde(default)]
    pub redaction_receipt: Option<String>,
    #[serde(default)]
    pub entity_detection: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub provider_calls: u8,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct CloudRedactionFinding {
    #[serde(default)]
    pub entity_type: String,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FeedDigest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FeedCalendarContent {
    #[serde(default)]
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub commitment: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub entry_source: String,
    #[serde(default)]
    pub rhythm_id: Option<String>,
}

/// One signal of `GET /comms/feed/quality`, comms' `QualityReviewRow`
/// (`capabilities/comms/src/store.rs`). An entry with several signals comes back as several rows.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedQualityFlag {
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

/// What `POST /comms/feed/quality/refresh` checked (`server/content.rs`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FeedQualityRefresh {
    pub reviewed: usize,
    pub flagged_items: usize,
    pub flag_count: usize,
    pub provider_calls: usize,
}

/// Where a keep, dismiss or open happened. Comms' ledger CHECK constraint
/// (`capabilities/comms/src/store/migrations.rs`) holds the full list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedSurface {
    Inbox,
    Reader,
    Library,
}

impl FeedSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Reader => "reader",
            Self::Library => "library",
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

/// Metadata returned by `POST /ytalbum/api/probe`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct YtAlbumProbe {
    pub title: String,
    pub uploader: String,
    pub duration_hms: String,
    #[serde(default)]
    pub chapters: Vec<YtAlbumChapter>,
    #[serde(default)]
    pub tracklist_from_chapters: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct YtAlbumChapter {
    pub start: u64,
    pub title: String,
}

/// Request and response contract for a ytalbum build.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtAlbumJob {
    pub id: String,
    pub album: String,
    pub state: String,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub outdir: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct YtAlbumBuild {
    pub url: String,
    pub album: String,
    pub artist: String,
    pub year: String,
    pub tracklist: String,
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

/// Secret-free presence for one supported provider API key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderKeyStatus {
    pub provider: String,
    pub present: bool,
}

/// One feature package installed into the writable runtime, from its package receipt.
#[derive(Clone, Debug, Deserialize)]
pub struct InstalledFeature {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub feature: InstalledFeatureMetadata,
    pub requires: Vec<String>,
    pub package_sha256: String,
    /// `verified` when every owned payload file still matches its receipt, otherwise `modified`.
    pub integrity: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstalledFeatureMetadata {
    pub schema: u32,
    pub title: String,
    pub summary: String,
    pub category: String,
    pub recommend_topics: Vec<String>,
    pub data_access: Vec<String>,
}

/// A package advertised by the latest official Sjel release.
#[derive(Clone, Debug, Deserialize)]
pub struct AvailableFeature {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub requires: Vec<String>,
    pub feature: InstalledFeatureMetadata,
    pub asset: String,
    pub bytes: u64,
    pub sha256: String,
    pub download_url: String,
    pub installable: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct ReleaseCatalogue {
    schema: u32,
    version: String,
    source: ReleaseSource,
    friend_ready: bool,
    assets: Vec<ReleaseAsset>,
    capabilities: Vec<ReleaseCapability>,
}

#[derive(Deserialize)]
struct ReleaseSource {
    repository: String,
    commit: String,
    dirty: bool,
    rebuilt: bool,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    bytes: u64,
    sha256: String,
    platform: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseCapability {
    name: String,
    version: String,
    platform: String,
    requires: Vec<String>,
    feature: InstalledFeatureMetadata,
    asset: String,
    bytes: u64,
    sha256: String,
}

/// Reads the official latest release and checks its catalogue before exposing feature entries.
pub async fn feature_catalogue() -> ShellResult<Vec<AvailableFeature>> {
    const RELEASE_URL: &str = "https://api.github.com/repos/larsboes/Sjel/releases/latest";
    const CATALOGUE_LIMIT: usize = 4 * 1024 * 1024;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| ShellError::PackageManager("could not start catalog client".into()))?;
    let release_response = client
        .get(RELEASE_URL)
        .header(header::USER_AGENT, "Sjel-Desktop")
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| {
            ShellError::PackageManager(format!("official catalog request failed: {error}"))
        })?;
    if !release_response.status().is_success() {
        return Err(ShellError::PackageManager(format!(
            "official catalog request returned HTTP {}",
            release_response.status()
        )));
    }
    let release: GithubRelease = release_response
        .json()
        .await
        .map_err(|_| ShellError::Unreadable)?;
    if !valid_release_version(&release.tag_name) {
        return Err(ShellError::PackageManager(
            "latest official release has an invalid tag".into(),
        ));
    }
    let catalogue_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "release.json")
        .ok_or_else(|| {
            ShellError::PackageManager("latest release has no package catalog".into())
        })?;
    validate_release_asset_url(
        &catalogue_asset.browser_download_url,
        &release.tag_name,
        "release.json",
    )?;
    let catalogue_response = client
        .get(&catalogue_asset.browser_download_url)
        .header(header::USER_AGENT, "Sjel-Desktop")
        .send()
        .await
        .map_err(|error| {
            ShellError::PackageManager(format!("official catalog download failed: {error}"))
        })?;
    if !catalogue_response.status().is_success()
        || catalogue_response
            .content_length()
            .is_some_and(|size| size > CATALOGUE_LIMIT as u64)
    {
        return Err(ShellError::PackageManager(
            "official catalog download is unavailable or too large".into(),
        ));
    }
    use futures::StreamExt as _;
    let mut stream = catalogue_response.bytes_stream();
    let mut catalogue_bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ShellError::Unreadable)?;
        if catalogue_bytes.len().saturating_add(chunk.len()) > CATALOGUE_LIMIT {
            return Err(ShellError::PackageManager(
                "official catalog exceeds the size limit".into(),
            ));
        }
        catalogue_bytes.extend_from_slice(&chunk);
    }
    let catalogue: ReleaseCatalogue =
        serde_json::from_slice(&catalogue_bytes).map_err(|_| ShellError::Unreadable)?;
    if catalogue.schema != 1
        || catalogue.version != release.tag_name
        || catalogue.source.repository != "https://github.com/larsboes/Sjel"
        || catalogue.source.commit.len() != 40
        || !catalogue
            .source
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || catalogue.source.dirty
        || !catalogue.source.rebuilt
    {
        return Err(ShellError::PackageManager(
            "official catalog failed its release identity checks".into(),
        ));
    }

    let host_platform = host_platform();
    let runtime = runtime_identity();
    let mut features = Vec::new();
    let mut package_names = std::collections::BTreeSet::new();
    for package in catalogue
        .capabilities
        .into_iter()
        .filter(|package| package.platform == host_platform)
    {
        if package.version != catalogue.version
            || !valid_package_identifier(&package.name)
            || !package_names.insert(package.name.clone())
            || package
                .requires
                .iter()
                .any(|name| !valid_package_identifier(name) || name == &package.name)
            || package
                .requires
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != package.requires.len()
            || !valid_feature_metadata(&package.feature)
            || package.bytes == 0
            || package.bytes > 1_073_741_824
            || !valid_sha256(&package.sha256)
            || package.asset
                != format!(
                    "sjel-{}-{}-{}.tar.gz",
                    package.name, package.version, package.platform
                )
        {
            return Err(ShellError::PackageManager(format!(
                "invalid catalog entry for {}",
                package.name
            )));
        }
        let matching_assets: Vec<_> = catalogue
            .assets
            .iter()
            .filter(|asset| asset.name == package.asset)
            .collect();
        let Some(asset) = matching_assets.first() else {
            return Err(ShellError::PackageManager(format!(
                "catalog asset missing for {}",
                package.name
            )));
        };
        if matching_assets.len() != 1
            || asset.bytes != package.bytes
            || asset.sha256 != package.sha256
            || asset
                .platform
                .as_deref()
                .is_some_and(|platform| platform != package.platform)
        {
            return Err(ShellError::PackageManager(format!(
                "catalog asset does not match {}",
                package.name
            )));
        }
        let matching_release_assets: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| asset.name == package.asset)
            .collect();
        let release_asset = matching_release_assets.first().copied().ok_or_else(|| {
            ShellError::PackageManager(format!("release download missing for {}", package.name))
        })?;
        if matching_release_assets.len() != 1 {
            return Err(ShellError::PackageManager(format!(
                "release has ambiguous downloads for {}",
                package.name
            )));
        }
        validate_release_asset_url(
            &release_asset.browser_download_url,
            &release.tag_name,
            &package.asset,
        )?;
        let reason = if !catalogue.friend_ready {
            Some("This release is not cleared for feature installation.".to_owned())
        } else if runtime.as_ref().map(|identity| identity.0.as_str())
            != Some(catalogue.version.as_str())
        {
            Some("Place the matching Sjel runtime before installing this feature.".to_owned())
        } else if runtime.as_ref().map(|identity| identity.1.as_str())
            != Some(host_platform.as_str())
        {
            Some("The placed Sjel runtime does not match this device.".to_owned())
        } else if !installer_available() {
            Some("The placed runtime package installer is unavailable.".to_owned())
        } else {
            None
        };
        features.push(AvailableFeature {
            name: package.name,
            version: package.version,
            platform: package.platform,
            requires: package.requires,
            feature: package.feature,
            asset: package.asset,
            bytes: package.bytes,
            sha256: package.sha256,
            download_url: release_asset.browser_download_url.clone(),
            installable: reason.is_none(),
            unavailable_reason: reason,
        });
    }
    features.sort_by(|left, right| {
        left.feature
            .category
            .cmp(&right.feature.category)
            .then_with(|| left.feature.title.cmp(&right.feature.title))
    });
    Ok(features)
}

/// Downloads and installs one feature from a freshly validated official catalogue.
pub async fn install_feature(
    name: &str,
    expected_version: &str,
    expected_sha256: &str,
) -> ShellResult<()> {
    if !valid_package_identifier(name) {
        return Err(ShellError::PackageManager("invalid feature name".into()));
    }
    let package = feature_catalogue()
        .await?
        .into_iter()
        .find(|package| package.name == name)
        .ok_or_else(|| {
            ShellError::PackageManager("feature is not in the official catalog".into())
        })?;
    if !package.installable {
        return Err(ShellError::PackageManager(
            package
                .unavailable_reason
                .unwrap_or_else(|| "feature is not ready for installation".into()),
        ));
    }
    if package.version != expected_version || package.sha256 != expected_sha256 {
        return Err(ShellError::PackageManager(
            "the release catalog changed; refresh feature details before installing".into(),
        ));
    }
    let executable = std::env::var_os("SJEL_CLI_BIN")
        .or_else(|| {
            std::env::var_os("SJEL_ROOT")
                .map(std::path::PathBuf::from)
                .map(|root| root.join("target/release/sjel-cli").into_os_string())
        })
        .ok_or_else(|| {
            ShellError::PackageManager("placed runtime installer is unavailable".into())
        })?;

    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        use std::sync::atomic::{AtomicU64, Ordering};

        static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|error| ShellError::PackageManager(error.to_string()))?;
        let response = client
            .get(&package.download_url)
            .header(header::USER_AGENT, "Sjel-Desktop")
            .send()
            .map_err(|error| {
                ShellError::PackageManager(format!("feature download failed: {error}"))
            })?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length != package.bytes)
        {
            return Err(ShellError::PackageManager(
                "feature download does not match the catalog size".into(),
            ));
        }
        let archive = std::env::temp_dir().join(format!(
            "sjel-feature-{}-{}.tar.gz",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut created = false;
        let result = (|| {
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&archive)
                .map_err(|error| {
                    ShellError::PackageManager(format!("cannot stage feature download: {error}"))
                })?;
            created = true;
            let copied = std::io::copy(
                &mut response.take(package.bytes.saturating_add(1)),
                &mut output,
            )
            .map_err(|error| {
                ShellError::PackageManager(format!("feature download failed: {error}"))
            })?;
            if copied != package.bytes {
                return Err(ShellError::PackageManager(
                    "feature download size differs from the catalog".into(),
                ));
            }
            output.sync_all().map_err(|error| {
                ShellError::PackageManager(format!("cannot sync feature download: {error}"))
            })?;
            drop(output);
            let output = Command::new(executable)
                .arg("distribution")
                .arg("install-package")
                .arg(&archive)
                .arg("--sha256")
                .arg(&package.sha256)
                .output()
                .map_err(|error| {
                    ShellError::PackageManager(format!("cannot run package installer: {error}"))
                })?;
            if output.status.success() {
                Ok(())
            } else {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                Err(ShellError::PackageManager(if detail.is_empty() {
                    "runtime refused the feature package".into()
                } else {
                    detail
                }))
            }
        })();
        if created {
            let _ = std::fs::remove_file(archive);
        }
        result
    })
    .await
    .map_err(|error| ShellError::PackageManager(error.to_string()))?
}

fn valid_release_version(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_package_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_feature_metadata(feature: &InstalledFeatureMetadata) -> bool {
    feature.schema == 1
        && !feature.title.trim().is_empty()
        && feature.title.len() <= 80
        && !feature.summary.trim().is_empty()
        && feature.summary.len() <= 240
        && !feature.category.trim().is_empty()
        && feature.category.len() <= 40
        && !feature.recommend_topics.is_empty()
        && feature.recommend_topics.len() <= 24
        && feature.recommend_topics.iter().all(|topic| {
            !topic.is_empty()
                && topic.len() <= 48
                && topic
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && feature
            .recommend_topics
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == feature.recommend_topics.len()
        && feature.data_access.len() <= 32
        && feature
            .data_access
            .iter()
            .all(|item| !item.trim().is_empty() && item.len() <= 120)
        && feature
            .data_access
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == feature.data_access.len()
}

fn validate_release_asset_url(value: &str, version: &str, asset: &str) -> ShellResult<()> {
    let url = Url::parse(value).map_err(|_| ShellError::Unreadable)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.path() != format!("/larsboes/Sjel/releases/download/{version}/{asset}")
    {
        return Err(ShellError::PackageManager(
            "catalog asset URL is not from the official release".into(),
        ));
    }
    Ok(())
}

fn host_platform() -> String {
    let suffix = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{suffix}", std::env::consts::ARCH)
}

fn runtime_identity() -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct RuntimeIdentity {
        version: String,
        platform: String,
    }
    let root = std::env::var_os("SJEL_ROOT")?;
    let identity: RuntimeIdentity = serde_json::from_slice(
        &std::fs::read(std::path::PathBuf::from(root).join("distribution.json")).ok()?,
    )
    .ok()?;
    Some((identity.version, identity.platform))
}

fn installer_available() -> bool {
    std::env::var_os("SJEL_CLI_BIN").is_some_and(|path| std::path::Path::new(&path).is_file())
        || std::env::var_os("SJEL_ROOT")
            .map(std::path::PathBuf::from)
            .is_some_and(|root| root.join("target/release/sjel-cli").is_file())
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
    /// A paired-device request failed or was refused.
    Transport(String),
    /// A provider credential could not be written to the OS keychain.
    CredentialUnavailable,
    /// The local packaged runtime cannot report its installed feature packages.
    PackageManager(String),
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
            Self::Transport(why) => f.write_str(why),
            Self::CredentialUnavailable => {
                f.write_str("Could not save the provider API key to the OS keychain.")
            }
            Self::PackageManager(why) => write!(f, "Feature packages are unavailable: {why}"),
            Self::Store(why) => write!(f, "This device could not save it: {why}"),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;

pub struct Shell {
    base: Url,
    client: reqwest::Client,
    token: Arc<Mutex<Option<String>>>,
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
            token: Arc::new(Mutex::new(None)),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    pub async fn health(&self) -> ShellResult<NodeHealth> {
        self.json(Method::GET, "/api/sjel-status/health", None, 10)
            .await
    }

    pub async fn capability_panels(&self) -> ShellResult<Vec<CapabilityPanel>> {
        self.json(Method::GET, "/api/sjel-status/capabilities", None, 10)
            .await
    }

    pub async fn calendar_entry(&self, id: &str) -> ShellResult<CalendarEntryDetail> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::GET,
            &format!("/calendar/api/entries/{id}"),
            None,
            10,
        )
        .await
    }

    pub async fn capability_links(
        &self,
        capability: &str,
        id: &str,
    ) -> ShellResult<crate::record::CapabilityLinks> {
        if !is_registry_name(capability) || !valid_typed_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability or typed record id.".into()),
            ));
        }
        let path = with_query(&format!("/{capability}/api/links"), &[("to", id)]);
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn travel_plans(&self) -> ShellResult<Vec<crate::travel::Plan>> {
        self.json(Method::GET, "/trips/api/plans", None, 10).await
    }

    /// A read from transit. Only `/transit/api/` paths, which `travel::connections` builds.
    pub async fn transit<T: DeserializeOwned>(&self, path: &str) -> ShellResult<T> {
        if !path.starts_with("/transit/api/") || path.contains("..") {
            return Err(ShellError::Refused(400, Some("Not a transit path.".into())));
        }
        // A split search asks for many fares, one stop pair at a time.
        self.json(Method::GET, path, None, 90).await
    }

    pub async fn travel_details(&self, id: &str) -> ShellResult<crate::travel::Details> {
        let path = crate::travel::details_path(id)
            .map_err(|error| ShellError::Refused(400, Some(error)))?;
        self.json(Method::GET, &path, None, 10).await
    }

    /// Deletes the plan only if it is still the revision the user saw.
    pub async fn trip_plan_delete(&self, id: &str) -> ShellResult<()> {
        let path = crate::travel::details_path(id)
            .map_err(|error| ShellError::Refused(400, Some(error)))?;
        let current: serde_json::Value = self.json(Method::GET, &path, None, 10).await?;
        let updated_at = current
            .get("updated_at")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ShellError::Refused(502, Some("The trip has no revision.".into())))?;
        let mut url = reqwest::Url::parse("http://localhost/")
            .map_err(|error| ShellError::Refused(400, Some(error.to_string())))?;
        url.set_path(&path);
        url.query_pairs_mut()
            .append_pair("expected_updated_at", updated_at);
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        self.send(Method::DELETE, &path, None, 10).await?;
        Ok(())
    }

    /// `POST /trips/api/plans` with a body from [`crate::travel::edit::TripDraft::body`].
    pub async fn trip_create(&self, body: serde_json::Value) -> ShellResult<crate::travel::Plan> {
        let plan: crate::travel::Plan = self
            .json(Method::POST, "/trips/api/plans", Some(body), 10)
            .await?;
        self.trip_calendar_sync(&plan.id).await;
        Ok(plan)
    }

    /// `POST …/items` with a body from [`crate::travel::edit::BookingDraft::create_body`].
    pub async fn trip_booking_add(&self, plan: &str, body: serde_json::Value) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items"]).map_err(refused)?;
        self.send(Method::POST, &path, Some(body), 10).await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Fields, then the day: only the item route moves an item between days.
    pub async fn trip_booking_edit(
        &self,
        plan: &str,
        item: &str,
        fields: serde_json::Value,
        day: Option<String>,
    ) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items", item, "fields"]).map_err(refused)?;
        self.send(Method::PATCH, &path, Some(fields), 10).await?;
        let path = crate::travel::plan_path(plan, &["items", item]).map_err(refused)?;
        self.send(
            Method::PATCH,
            &path,
            Some(serde_json::json!({ "day": day })),
            10,
        )
        .await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    pub async fn trip_item_delete(&self, plan: &str, item: &str) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items", item]).map_err(refused)?;
        self.send(Method::DELETE, &path, None, 10).await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Read, edit, write back with the revision read, so a concurrent writer gets a 409.
    pub async fn trip_stages(
        &self,
        plan: &str,
        edit: &crate::travel::edit::StageEdit,
    ) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &[]).map_err(refused)?;
        let current: serde_json::Value = self.json(Method::GET, &path, None, 10).await?;
        let mut stages = current
            .get("stages")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        edit.apply(&mut stages).map_err(refused)?;
        let body = serde_json::json!({
            "stages": stages,
            "expected_updated_at": current.get("updated_at"),
        });
        match self.send(Method::PATCH, &path, Some(body), 10).await {
            Err(ShellError::Refused(409, _)) => Err(ShellError::Refused(
                409,
                Some(
                    "The trip changed on the Mac while this was open. Refresh and try again."
                        .into(),
                ),
            )),
            other => other.map(drop),
        }?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Calendar mirrors stages and booking deadlines only when asked. Best effort: the trip write
    /// already succeeded.
    async fn trip_calendar_sync(&self, plan: &str) {
        let Ok(mut url) = reqwest::Url::parse("http://localhost/calendar/api/trip-plans/") else {
            return;
        };
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.pop_if_empty().push(plan).push("sync");
        }
        let _ = self
            .send(Method::POST, url.path(), Some(serde_json::json!({})), 10)
            .await;
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
        self.json(Method::GET, "/api/sjel-status/self", None, 15)
            .await
    }

    /// CV status, applications, profiles and built PDFs from `GET /api/sjel-status/cv`.
    pub async fn cv(&self) -> ShellResult<crate::cv::CvView> {
        self.json(Method::GET, "/api/sjel-status/cv", None, 20)
            .await
    }

    /// Run one CV verb (build, app-new, app-status, linkedin-diff, etc.) via `POST /api/sjel-status/cv`.
    pub async fn write_cv(
        &self,
        req: &crate::cv::CvWriteRequest,
    ) -> ShellResult<crate::cv::CvWriteResult> {
        let body = serde_json::to_value(req).map_err(|_| ShellError::Unreadable)?;
        self.json(Method::POST, "/api/sjel-status/cv", Some(body), 60)
            .await
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

    /// Read what this machine still needs via `GET /api/sjel-status/setup`.
    pub async fn setup_plans(&self) -> ShellResult<crate::setup::SetupPlans> {
        self.json(Method::GET, "/api/sjel-status/setup", None, 30)
            .await
    }

    /// Check connection to primary via `GET /api/sjel-status/setup/verify`.
    pub async fn setup_verify(&self) -> ShellResult<crate::setup::SetupVerification> {
        self.json(Method::GET, "/api/sjel-status/setup/verify", None, 30)
            .await
    }

    /// Apply setup steps via `POST /api/sjel-status/setup/apply`.
    pub async fn setup_apply(
        &self,
        request: &crate::setup::SetupApplyRequest,
    ) -> ShellResult<crate::setup::SetupApplied> {
        let body = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        self.json(Method::POST, "/api/sjel-status/setup/apply", Some(body), 60)
            .await
    }

    /// List paired devices via `GET /devices/api/devices`.
    pub async fn setup_devices(&self) -> ShellResult<Vec<crate::setup::PairedDevice>> {
        #[derive(serde::Deserialize)]
        struct DevicesResp {
            #[serde(default)]
            devices: Vec<crate::setup::PairedDevice>,
        }
        match self
            .json::<DevicesResp>(Method::GET, "/devices/api/devices", None, 10)
            .await
        {
            Ok(resp) => Ok(resp.devices),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Revoke one device's key via `POST /devices/api/devices/{id}/revoke`. The answer is the
    /// device as the node now holds it, which is `revoked`.
    pub async fn setup_revoke_device(&self, id: &str) -> ShellResult<crate::setup::PairedDevice> {
        let path = format!("/devices/api/devices/{id}/revoke");
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// This node's Same-Wi-Fi listener via `GET /api/sjel-status/lan`, or `None` when it has none.
    pub async fn setup_lan(&self) -> ShellResult<crate::setup::LanStatus> {
        self.json(Method::GET, "/api/sjel-status/lan", None, 10)
            .await
    }

    /// Mint a one-time code for one new device via `POST /devices/api/pairing/challenges`.
    pub async fn setup_create_challenge(&self) -> ShellResult<crate::setup::PairingChallenge> {
        self.json(
            Method::POST,
            "/devices/api/pairing/challenges",
            Some(serde_json::json!({})),
            15,
        )
        .await
    }

    /// List pending join requests via `GET /devices/api/pairing/requests`.
    pub async fn setup_pending_joins(&self) -> ShellResult<Vec<crate::setup::SetupJoinRequest>> {
        #[derive(serde::Deserialize)]
        struct JoinsResp {
            #[serde(default)]
            requests: Vec<crate::setup::SetupJoinRequest>,
        }
        match self
            .json::<JoinsResp>(Method::GET, "/devices/api/pairing/requests", None, 10)
            .await
        {
            Ok(resp) => Ok(resp.requests),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Approve a device join request via `POST /devices/api/pairing/requests/{id}/approve`.
    pub async fn setup_approve_join(
        &self,
        id: &str,
    ) -> ShellResult<crate::setup::SetupJoinRequest> {
        let path = format!("/devices/api/pairing/requests/{}/approve", id);
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// Deny a device join request via `POST /devices/api/pairing/requests/{id}/deny`.
    pub async fn setup_deny_join(&self, id: &str) -> ShellResult<crate::setup::SetupJoinRequest> {
        let path = format!("/devices/api/pairing/requests/{}/deny", id);
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// Load the combined view model for `/setup`: what this machine still needs, which devices it
    /// trusts, which joins wait, and whether a phone can reach it at all.
    pub async fn setup_view(&self) -> ShellResult<crate::setup::SetupView> {
        let plans = match self.setup_plans().await {
            Ok(p) => p.components,
            Err(_) => Vec::new(),
        };
        let devices = self.setup_devices().await.unwrap_or_default();
        let pending_joins = self.setup_pending_joins().await.unwrap_or_default();
        // A read that fails is not the same answer as a listener that is off, so the page is told
        // which one happened (its Devices tab says so in words).
        let (lan, lan_error) = match self.setup_lan().await {
            Ok(status) => (status.listener().cloned(), None),
            Err(error) => (None, Some(error.to_string())),
        };

        Ok(crate::setup::SetupView {
            plans,
            devices,
            pending_joins,
            lan,
            lan_error,
        })
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
        self.json(
            Method::POST,
            "/api/sjel-status/upstreams/watch",
            Some(body),
            30,
        )
        .await
    }

    /// Entries overlapping `[from, to)`, both `YYYY-MM-DD`: `to` is exclusive.
    pub async fn calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        let path = format!("/calendar/api/entries?from={from}&to={to}");
        self.json(Method::GET, &path, None, 5).await
    }

    /// Un-adopted external entries, including the bounded event proposals Comms writes from mail.
    pub async fn calendar_proposals(
        &self,
        from: &str,
        to: &str,
    ) -> ShellResult<Vec<CalendarEntry>> {
        let path = with_query("/calendar/api/proposals", &[("from", from), ("to", to)]);
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn calendar_google_exports(&self) -> ShellResult<Vec<CalendarGoogleExportOptIn>> {
        self.json(Method::GET, "/calendar/api/google/exports", None, 10)
            .await
    }

    /// Dry-run previews the approved queue; a real run is requested separately by the operator.
    pub async fn calendar_google_export(
        &self,
        dry_run: bool,
    ) -> ShellResult<CalendarGoogleExportReport> {
        self.json(
            Method::POST,
            "/calendar/api/google/export",
            Some(serde_json::json!({ "dry_run": dry_run })),
            60,
        )
        .await
    }

    pub async fn calendar_google_opt_in(&self, id: &str) -> ShellResult<CalendarGoogleExportOptIn> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::PUT,
            &format!("/calendar/api/entries/{id}/google-export"),
            Some(serde_json::json!({})),
            10,
        )
        .await
    }

    pub async fn calendar_google_opt_out(&self, id: &str) -> ShellResult<()> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json::<CalendarGoogleExportOptOut>(
            Method::DELETE,
            &format!("/calendar/api/entries/{id}/google-export"),
            None,
            10,
        )
        .await
        .and_then(|response| {
            if response.opted_out {
                Ok(())
            } else {
                Err(ShellError::Refused(
                    500,
                    Some("Google export opt-out was not confirmed.".into()),
                ))
            }
        })
    }

    /// A mail event is only accepted after the operator makes it a planned Calendar entry.
    pub async fn update_calendar_entry(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::PATCH,
            &format!("/calendar/api/entries/{id}"),
            Some(body),
            10,
        )
        .await
    }

    pub async fn calendar_trip_drafts(
        &self,
        from: &str,
        to: &str,
    ) -> ShellResult<crate::calendar::TripDrafts> {
        let path = with_query(
            "/calendar/api/trip-drafts",
            &[("from", from), ("to", to), ("max_gap_days", "5")],
        );
        self.json(Method::GET, &path, None, 10).await
    }

    /// Materialization is explicit and idempotent at Calendar's public endpoint.
    pub async fn calendar_materialize_trip(
        &self,
        entry_ids: &[String],
        title: &str,
    ) -> ShellResult<crate::calendar::TripMaterialization> {
        self.json(
            Method::POST,
            "/calendar/api/trip-drafts/materialize",
            Some(serde_json::json!({
                "entry_ids": entry_ids,
                "title": (!title.trim().is_empty()).then_some(title.trim()),
            })),
            30,
        )
        .await
    }

    /// Name-free nearby-companion count from Places for one trip destination and date window.
    pub async fn people_presence(
        &self,
        latitude: f64,
        longitude: f64,
        from: &str,
        to: &str,
    ) -> ShellResult<crate::travel::places::Presence> {
        let pairs = [
            ("latitude", latitude.to_string()),
            ("longitude", longitude.to_string()),
            ("from", from.to_owned()),
            ("to", to.to_owned()),
        ];
        let path = with_query(
            "/places/api/people/presence",
            &pairs
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect::<Vec<_>>(),
        );
        self.json(Method::GET, &path, None, 10).await
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
        if !valid_calendar_entry_id(id) {
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

    pub async fn finance_csv_mappings(&self) -> ShellResult<Vec<FinanceCsvMappingProfile>> {
        self.json(Method::GET, "/finance/api/import/csv/mappings", None, 10)
            .await
    }

    pub async fn finance_candidates(&self) -> ShellResult<Vec<FinanceCandidate>> {
        self.json(Method::GET, "/finance/api/import/candidates", None, 10)
            .await
    }

    pub async fn finance_preview_csv(
        &self,
        content: String,
        mapping: serde_json::Value,
    ) -> ShellResult<FinanceCsvPreview> {
        self.json(
            Method::POST,
            "/finance/api/import/csv/preview",
            Some(serde_json::json!({ "content": content, "mapping": mapping })),
            30,
        )
        .await
    }

    pub async fn finance_import_csv(
        &self,
        content: String,
        mapping: serde_json::Value,
        expected_preview_id: String,
    ) -> ShellResult<serde_json::Value> {
        self.json(
            Method::POST,
            "/finance/api/import/csv",
            Some(serde_json::json!({ "content": content, "mapping": mapping, "expected_preview_id": expected_preview_id })),
            30,
        )
        .await
    }

    pub async fn finance_review_candidate(
        &self,
        id: &str,
        decision: &str,
        account: Option<&str>,
        source_account: Option<&str>,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !matches!(decision, "confirm" | "reject") {
            return Err(ShellError::Refused(
                400,
                Some("Invalid finance review request.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/import/candidates/{}/review", id.replace(':', "%3A")),
            Some(serde_json::json!({ "decision": decision, "account": account, "source_account": source_account })),
            30,
        )
        .await
    }

    pub async fn finance_reconcile_transfer(
        &self,
        id: &str,
        counterpart_id: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !valid_typed_id(counterpart_id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid transfer candidate id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!(
                "/finance/api/import/candidates/{}/reconcile-transfer",
                id.replace(':', "%3A")
            ),
            Some(serde_json::json!({ "counterpart_id": counterpart_id })),
            30,
        )
        .await
    }

    pub async fn finance_reclassify_candidates(
        &self,
        items: Vec<(String, String)>,
    ) -> ShellResult<serde_json::Value> {
        self.json(
            Method::POST,
            "/finance/api/import/candidates/reclassify-batch",
            Some(serde_json::json!({
                "items": items.into_iter().map(|(id, account)| serde_json::json!({ "id": id, "account": account })).collect::<Vec<_>>()
            })),
            30,
        )
        .await
    }

    pub async fn finance_allocate_expense(
        &self,
        id: &str,
        personal_cents: i64,
        purpose: &str,
        trip_id: Option<&str>,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id)
            || personal_cents < 0
            || !matches!(
                purpose,
                "day_to_day" | "trip" | "work" | "housing" | "other"
            )
            || (purpose == "trip" && trip_id.is_none_or(str::is_empty))
        {
            return Err(ShellError::Refused(
                400,
                Some("Invalid expense allocation.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/import/candidates/{}/allocation", id.replace(':', "%3A")),
            Some(serde_json::json!({ "personal_cents": personal_cents, "purpose": purpose, "trip_id": trip_id })),
            30,
        )
        .await
    }

    pub async fn finance_link_reimbursement(
        &self,
        id: &str,
        expense_candidate_id: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !valid_typed_id(expense_candidate_id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid reimbursement candidate id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!(
                "/finance/api/import/candidates/{}/reimbursement",
                id.replace(':', "%3A")
            ),
            Some(serde_json::json!({ "expense_candidate_id": expense_candidate_id })),
            30,
        )
        .await
    }

    pub async fn finance_update_balance_snapshot(
        &self,
        update: ManualBalanceUpdate,
    ) -> ShellResult<ManualBalanceSnapshot> {
        let value: serde_json::Value = self
            .json(
                Method::POST,
                "/finance/api/balance-snapshot",
                Some(serde_json::to_value(update).map_err(|_| ShellError::Unreadable)?),
                30,
            )
            .await?;
        serde_json::from_value(
            value
                .get("snapshot")
                .cloned()
                .ok_or(ShellError::Unreadable)?,
        )
        .map_err(|_| ShellError::Unreadable)
    }

    /// The finder's server results (`GET /api/sjel-status/search?q=`): rows from the shared
    /// database that the app does not hold, each with a `sjel://` link to open it by.
    pub async fn search(&self, query: &str) -> ShellResult<Vec<ServerHit>> {
        #[derive(Deserialize)]
        struct Envelope {
            hits: Vec<ServerHit>,
        }
        let path = with_query("/api/sjel-status/search", &[("q", query)]);
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.hits)
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

    /// The fields `kind` carries, built in and declared (`GET /entities/api/fields?kind=`).
    pub async fn entity_fields(&self, kind: &str) -> ShellResult<Vec<EntityField>> {
        #[derive(Deserialize)]
        struct Envelope {
            fields: Vec<EntityField>,
        }
        let path = with_query("/entities/api/fields", &[("kind", kind)]);
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.fields)
    }

    /// Declares a new field on a kind (`POST /entities/api/fields`). An existing key is refused.
    pub async fn entity_declare_field(&self, body: serde_json::Value) -> ShellResult<EntityField> {
        self.json(Method::POST, "/entities/api/fields", Some(body), 10)
            .await
    }

    /// Creates an entity (`POST /entities/api/entities`). Every key in `values` must be a field
    /// the kind declares, which is why the form offers the registry's own list.
    pub async fn entity_create(&self, body: serde_json::Value) -> ShellResult<EntityRecord> {
        self.json(Method::POST, "/entities/api/entities", Some(body), 10)
            .await
    }

    /// Changes an entity (`PATCH /entities/api/entities/:id`). An absent key is left alone and a
    /// null clears a field; `expected_revision` turns a lost update into a 409.
    pub async fn entity_update(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<EntityRecord> {
        self.json(
            Method::PATCH,
            &format!("/entities/api/entities/{id}"),
            Some(body),
            10,
        )
        .await
    }

    /// Deletes an entity and its facts (`DELETE /entities/api/entities/:id`).
    pub async fn entity_delete(&self, id: &str) -> ShellResult<()> {
        self.send(
            Method::DELETE,
            &format!("/entities/api/entities/{id}"),
            None,
            10,
        )
        .await
        .map(drop)
    }

    /// Adds a dated fact (`POST /entities/api/entities/:id/facts`) and answers with the stored one,
    /// coordinate included when `places` resolved the place text.
    pub async fn entity_fact_add(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<EntityFact> {
        #[derive(Deserialize)]
        struct Added {
            fact: EntityFact,
        }
        let added: Added = self
            .json(
                Method::POST,
                &format!("/entities/api/entities/{id}/facts"),
                Some(body),
                15,
            )
            .await?;
        Ok(added.fact)
    }

    /// Removes one dated fact (`DELETE /entities/api/entities/:id/facts/:fact`).
    pub async fn entity_fact_delete(&self, id: &str, fact: &str) -> ShellResult<()> {
        self.send(
            Method::DELETE,
            &format!("/entities/api/entities/{id}/facts/{fact}"),
            None,
            10,
        )
        .await
        .map(drop)
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

    /// The wish groups, each with its members and what they cost.
    pub async fn inventory_wish_groups(&self) -> ShellResult<Vec<crate::inventory::WishGroup>> {
        let envelope: crate::inventory::WishGroups = self
            .json(Method::GET, "/inventory/api/wish-groups", None, 10)
            .await?;
        Ok(envelope.wish_groups)
    }

    /// Creates a wish group, or retitles the one that already has this id.
    pub async fn inventory_save_wish_group(
        &self,
        id: &str,
        title: &str,
        constraint_text: &str,
    ) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        self.send(
            Method::POST,
            "/inventory/api/wish-groups",
            Some(serde_json::json!({
                "id": id,
                "title": title,
                "constraint_text": constraint_text,
            })),
            10,
        )
        .await
        .map(drop)
    }

    /// Replaces a group's members with exactly these item ids.
    pub async fn inventory_set_wish_group_items(
        &self,
        id: &str,
        item_ids: &[String],
    ) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "wish-groups", id, "items"]);
        self.send(
            Method::PUT,
            url.path(),
            Some(serde_json::json!({ "item_ids": item_ids })),
            10,
        )
        .await
        .map(drop)
    }

    /// Deletes a wish group and its memberships. The wishes themselves stay.
    pub async fn inventory_delete_wish_group(&self, id: &str) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "wish-groups", id]);
        self.send(Method::DELETE, url.path(), None, 10)
            .await
            .map(drop)
    }

    /// Creates an inventory entry through the inventory capability.
    pub async fn inventory_create_item(
        &self,
        item: &crate::inventory::NewInventoryItem,
    ) -> ShellResult<()> {
        self.send(
            Method::POST,
            "/inventory/api/items",
            Some(serde_json::to_value(item).map_err(|_| ShellError::Unreadable)?),
            10,
        )
        .await
        .map(drop)
    }

    /// Appends a state transition for an inventory item.
    pub async fn inventory_set_state(
        &self,
        id: &str,
        state: crate::inventory::InventoryState,
        note: Option<&str>,
    ) -> ShellResult<crate::inventory::InventoryStateChange> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id, "state"]);
        self.json(
            Method::POST,
            url.path(),
            Some(serde_json::json!({"state": state.as_str(), "note": note})),
            10,
        )
        .await
    }

    /// Applies only named item fields and refuses a stale revision rather than overwriting it.
    pub async fn inventory_patch_item(
        &self,
        id: &str,
        revision: i64,
        mut patch: serde_json::Value,
    ) -> ShellResult<crate::inventory::InventoryItemWrite> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let Some(fields) = patch.as_object_mut() else {
            return Err(ShellError::Refused(
                400,
                Some("An inventory patch must be an object.".into()),
            ));
        };
        fields.insert("expected_revision".to_owned(), revision.into());
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id]);
        self.json(Method::PATCH, url.path(), Some(patch), 10).await
    }

    pub async fn interior_layouts(
        &self,
    ) -> ShellResult<Vec<crate::inventory::InteriorLayoutSummary>> {
        self.json(Method::GET, "/interior/api/layouts", None, 10)
            .await
    }

    pub async fn interior_layout_preview(
        &self,
        id: &str,
    ) -> ShellResult<crate::interior::RoomPlanSavedLayoutPreview> {
        if !crate::interior::valid_roomplan_layout_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not an interior layout id.".into()),
            ));
        }
        self.json(
            Method::GET,
            &format!("/interior/api/layouts/{id}"),
            None,
            20,
        )
        .await
    }

    pub async fn interior_preview_placements(
        &self,
        items: &[serde_json::Value],
    ) -> ShellResult<serde_json::Value> {
        let body = serde_json::json!({ "items": items });
        self.json(
            Method::POST,
            "/interior/api/placements/preview",
            Some(body),
            30,
        )
        .await
    }

    pub async fn interior_search_layout(
        &self,
        layout: &str,
        move_refs: &[String],
    ) -> ShellResult<crate::interior::RoomPlanJobReceipt> {
        if !crate::interior::valid_roomplan_layout_id(layout) || move_refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose a saved layout with movable items.".into()),
            ));
        }
        let body =
            serde_json::json!({ "layout": layout, "move_refs": move_refs, "step": 20, "limit": 6 });
        self.json(Method::POST, "/interior/api/search", Some(body), 10)
            .await
    }

    pub async fn interior_compose_layout(
        &self,
        refs: &[String],
    ) -> ShellResult<crate::interior::RoomPlanJobReceipt> {
        if refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose at least one item to arrange.".into()),
            ));
        }
        let body = serde_json::json!({ "refs": refs, "step": 25, "beam": 60, "limit": 5, "rotations": [0, 90] });
        self.json(Method::POST, "/interior/api/compose", Some(body), 10)
            .await
    }

    pub async fn interior_planning_job(
        &self,
        id: u64,
    ) -> ShellResult<crate::interior::RoomPlanJob> {
        self.json(Method::GET, &format!("/interior/api/jobs/{id}"), None, 10)
            .await
    }

    pub async fn interior_save_proposal(
        &self,
        id: &str,
        name: &str,
        items: Vec<serde_json::Value>,
        note: &str,
    ) -> ShellResult<serde_json::Value> {
        if !crate::interior::valid_roomplan_layout_id(id) || items.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A valid layout name and at least one placed item are required.".into()),
            ));
        }
        let body = serde_json::json!({ "id": id, "name": name, "items": items, "note_text": note });
        self.json(Method::POST, "/interior/api/layouts", Some(body), 30)
            .await
    }

    pub async fn interior_model(&self) -> ShellResult<crate::interior::RoomPlanModel> {
        self.json(Method::GET, "/interior/api/model", None, 10)
            .await
    }

    pub async fn interior_placements(
        &self,
        flat: &str,
    ) -> ShellResult<Vec<crate::interior::RoomPlanPlacement>> {
        let path = format!("/interior/api/placements/{flat}");
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn interior_roomplan_apply_placement(
        &self,
        space_id: &str,
        item_id: &str,
        revision_id: &str,
        observation_id: &str,
        expected_revision: i64,
        x: i32,
        y: i32,
        rot: i32,
    ) -> ShellResult<serde_json::Value> {
        let path = format!("/interior/api/roomplan/spaces/{space_id}/placements/{item_id}");
        let body = serde_json::json!({
            "revision_id": revision_id,
            "observation_id": observation_id,
            "expected_revision": expected_revision,
            "x": x,
            "y": y,
            "rot": rot
        });
        self.json(Method::PUT, &path, Some(body), 30).await
    }

    pub async fn interior_roomplan_propose_placement(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanPlacementProposalResponse> {
        let body = serde_json::json!({
            "space_id": space_id,
            "revision_id": revision_id,
            "observation_id": observation_id,
            "item_id": item_id
        });
        self.json(
            Method::POST,
            "/interior/api/roomplan/propose-placement",
            Some(body),
            30,
        )
        .await
    }

    /// Reads the semantic RoomPlan scans archived by the interior capability.
    pub async fn interior_roomplan_revisions(
        &self,
    ) -> ShellResult<crate::interior::RoomPlanRevisionSet> {
        self.json(Method::GET, "/interior/api/roomplan/revisions", None, 10)
            .await
    }

    /// Reads one immutable RoomPlan revision's parametric USDZ asset.
    pub async fn interior_roomplan_revision_asset(
        &self,
        revision_id: &str,
    ) -> ShellResult<Vec<u8>> {
        let response = self
            .send(
                Method::GET,
                &format!("/interior/api/roomplan/revisions/{revision_id}/asset"),
                None,
                60,
            )
            .await?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|_| ShellError::Unreadable)
    }

    pub async fn interior_roomplan_spaces(&self) -> ShellResult<crate::interior::RoomPlanSpaceSet> {
        self.json(Method::GET, "/interior/api/roomplan/spaces", None, 10)
            .await
    }

    pub async fn interior_roomplan_create_space(
        &self,
        revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanSpaceReceipt> {
        let body = serde_json::json!({ "revision_id": revision_id });
        self.json(
            Method::POST,
            "/interior/api/roomplan/spaces",
            Some(body),
            10,
        )
        .await
    }

    pub async fn interior_roomplan_add_revision(
        &self,
        space_id: &str,
        revision_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!("/interior/api/roomplan/spaces/{space_id}/revisions");
        let body = serde_json::json!({ "revision_id": revision_id });
        self.json(Method::POST, &path, Some(body), 10).await
    }

    pub async fn interior_roomplan_link_object(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!("/interior/api/roomplan/spaces/{space_id}/links");
        let body = serde_json::json!({
            "revision_id": revision_id,
            "observation_id": observation_id,
            "item_id": item_id
        });
        self.json(Method::PUT, &path, Some(body), 10).await
    }

    /// Aligns two archived RoomPlan captures and returns review-only match suggestions.
    pub async fn interior_roomplan_reconcile(
        &self,
        before_revision_id: &str,
        after_revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanReconciliationResponse> {
        let body = serde_json::json!({
            "before_revision_id": before_revision_id,
            "after_revision_id": after_revision_id
        });
        self.json(
            Method::POST,
            "/interior/api/roomplan/reconcile",
            Some(body),
            30,
        )
        .await
    }

    /// Records an accept or reject decision for one RoomPlan revision.
    pub async fn interior_roomplan_review(
        &self,
        revision_id: &str,
        decision: crate::interior::RoomPlanDecision,
    ) -> ShellResult<crate::interior::RoomPlanReview> {
        let path = format!("/interior/api/roomplan/revisions/{revision_id}/review");
        let body = serde_json::json!({
            "decision": match decision {
                crate::interior::RoomPlanDecision::Accept => "accept",
                crate::interior::RoomPlanDecision::Reject => "reject",
            }
        });
        self.json(Method::POST, &path, Some(body), 10).await
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
        let token_cache = Arc::clone(&self.token);
        spawn(async move {
            let mut token = token;
            while !to.is_closed() {
                match follow(&client, &url, &token, &to).await {
                    Followed::Unsupported => return,
                    Followed::Lost => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Followed::Unauthorized => {
                        *token_cache.lock().expect("token lock") = None;
                        let _ = to.unbounded_send(Snapshot {
                            error: Some(
                                "The local shell rejected its saved token. Refreshing it from Keychain."
                                    .into(),
                            ),
                            ..Snapshot::default()
                        });
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        token = match cached_token(&token_cache) {
                            Ok(token) => token,
                            Err(error) => {
                                let _ = to.unbounded_send(Snapshot {
                                    error: Some(error.to_string()),
                                    ..Snapshot::default()
                                });
                                return;
                            }
                        };
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

    /// Reports whether provider API keys are present without returning key values.
    pub async fn provider_key_status(&self) -> ShellResult<Vec<ProviderKeyStatus>> {
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider, Presence,
            };
            use std::sync::Arc;

            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            ["openai", "deepseek", "opencode-go", "nvidia-nim", "gemini"]
                .into_iter()
                .map(|slug| {
                    let id = CredentialId::parse(format!("inference-{slug}"))
                        .map_err(|_| ShellError::CredentialUnavailable)?;
                    let present = match manager.status(&id) {
                        Ok(Presence::Present) => true,
                        Ok(Presence::Missing)
                        | Err(sjel_credentials::CredentialError::NotFound) => false,
                        Err(_) => return Err(ShellError::CredentialUnavailable),
                    };
                    Ok(ProviderKeyStatus {
                        provider: slug.to_owned(),
                        present,
                    })
                })
                .collect()
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Lists receipt-backed packages from the writable runtime selected by the desktop launcher.
    pub async fn installed_features(&self) -> ShellResult<Vec<InstalledFeature>> {
        let executable = std::env::var_os("SJEL_CLI_BIN").ok_or_else(|| {
            ShellError::PackageManager("this app was not started from a placed runtime".into())
        })?;
        tokio::task::spawn_blocking(move || {
            let output = Command::new(executable)
                .args(["distribution", "list", "--json"])
                .output()
                .map_err(|error| ShellError::PackageManager(error.to_string()))?;
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(ShellError::PackageManager(if detail.is_empty() {
                    "the runtime could not read its package inventory".into()
                } else {
                    detail
                }));
            }
            serde_json::from_slice(&output.stdout).map_err(|_| ShellError::Unreadable)
        })
        .await
        .map_err(|error| ShellError::PackageManager(error.to_string()))?
    }

    /// Uninstalls a receipt-backed package, retaining its capability data.
    pub async fn uninstall_feature(&self, name: &str) -> ShellResult<()> {
        if name.is_empty()
            || name.len() > 80
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || name.starts_with('-')
        {
            return Err(ShellError::PackageManager("invalid feature name".into()));
        }
        let executable = std::env::var_os("SJEL_CLI_BIN").ok_or_else(|| {
            ShellError::PackageManager("this app was not started from a placed runtime".into())
        })?;
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let output = Command::new(executable)
                .args([
                    "distribution",
                    "uninstall-package",
                    &name,
                    "--preserve-data",
                ])
                .output()
                .map_err(|error| ShellError::PackageManager(error.to_string()))?;
            if output.status.success() {
                Ok(())
            } else {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                Err(ShellError::PackageManager(if detail.is_empty() {
                    "the runtime refused to uninstall this feature".into()
                } else {
                    detail
                }))
            }
        })
        .await
        .map_err(|error| ShellError::PackageManager(error.to_string()))?
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
        note_dashboard_fallback(next);
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
        self.documents_query_stream(None, tx).await
    }

    /// Stream ranked metadata and matching page numbers through the local Documents API.
    pub async fn documents_search_stream(
        &self,
        query: &str,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        self.documents_query_stream(Some(query), tx).await
    }

    async fn documents_query_stream(
        &self,
        query: Option<&str>,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<DocumentSummary>>,
    ) -> ShellResult<usize> {
        let mut loaded = 0;
        loop {
            let offset = loaded.to_string();
            let mut pairs = vec![("limit", "500"), ("offset", offset.as_str())];
            if let Some(query) = query {
                pairs.push(("q", query));
            }
            let path = with_query("/documents/api/documents", &pairs);
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

    /// Re-run local extraction and read the updated page evidence.
    pub async fn analyze_document(&self, id: &str) -> ShellResult<DocumentDetail> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let _: serde_json::Value = self
            .json(
                Method::POST,
                &format!("/documents/api/documents/{id}/analyze"),
                Some(serde_json::json!({})),
                120,
            )
            .await?;
        self.document_detail(id).await
    }

    /// Create child records for a complete, operator-approved PDF partition.
    pub async fn split_document(
        &self,
        id: &str,
        request: &DocumentSplitRequest,
    ) -> ShellResult<Vec<DocumentSummary>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        let body = serde_json::to_value(request).map_err(|_| ShellError::Unreadable)?;
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/split"),
            Some(body),
            30,
        )
        .await
    }

    /// Read the metadata-only child records of one source document.
    pub async fn document_children(&self, id: &str) -> ShellResult<Vec<DocumentSummary>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::GET,
            &format!("/documents/api/documents/{id}/children"),
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

    pub async fn generate_document_model_proposal(
        &self,
        id: &str,
    ) -> ShellResult<Vec<DocumentProposalRecord>> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a document id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/documents/api/documents/{id}/proposals/model"),
            Some(serde_json::json!({})),
            55,
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

    pub async fn content_item_detail(
        &self,
        source: &str,
        id: &str,
    ) -> ShellResult<FeedContentDetail> {
        let valid_source = matches!(source, "feed" | "mail" | "calendar");
        let allowed_id_byte = |byte: u8| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-')
                || (source == "calendar" && byte == b':')
        };
        let valid_id = !id.is_empty() && id.len() <= 128 && id.bytes().all(allowed_id_byte);
        if !valid_source || !valid_id || (source == "feed" && !is_sha256(id)) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid content source or id.".into()),
            ));
        }
        let path = match source {
            "feed" | "mail" => format!("/comms/content/{source}/{id}"),
            "calendar" => format!("/calendar/api/content/calendar/{id}"),
            _ => unreachable!("validated content source"),
        };
        self.json(Method::GET, &path, None, 10).await
    }

    /// Build the local-only derivative the operator can inspect before approval.
    pub async fn content_cloud_preview(
        &self,
        source: &str,
        id: &str,
    ) -> ShellResult<CloudDerivativePreview> {
        if !valid_cloud_content_id(source, id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid cloud preview source or id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/{source}/{id}/cloud-preview"),
            None,
            30,
        )
        .await
    }

    /// Approval binds to the exact hash the operator reviewed.
    pub async fn content_cloud_approve(
        &self,
        source: &str,
        id: &str,
        preview_hash: &str,
    ) -> ShellResult<FeedCloudProcessingState> {
        if !valid_cloud_content_id(source, id) || !is_sha256(preview_hash) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid cloud approval source, id, or preview hash.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/{source}/{id}/cloud-approval"),
            Some(serde_json::json!({ "preview_hash": preview_hash })),
            30,
        )
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

    /// The stored review queue: one row per signal, as the last refresh computed it.
    pub async fn feed_quality(&self) -> ShellResult<Vec<FeedQualityFlag>> {
        self.json(Method::GET, "/comms/feed/quality?limit=500", None, 10)
            .await
    }

    /// Recomputes the queue from stored evidence. Comms checks at most 500 entries and calls no
    /// provider, so this changes no feed decision.
    pub async fn refresh_feed_quality(&self) -> ShellResult<FeedQualityRefresh> {
        let body = serde_json::json!({"days": 3650});
        self.json(Method::POST, "/comms/feed/quality/refresh", Some(body), 60)
            .await
    }

    /// Configured Scouting sources. This includes source paths from the machine configuration;
    /// callers should use them only for source identity and labels, never display local paths.
    pub async fn scouting_sources(&self) -> ShellResult<crate::scouting::ScoutingSources> {
        self.json(Method::GET, "/scouting/sources", None, 10).await
    }

    /// Stored opportunities for the Discover queue, including decisions the operator made.
    pub async fn scouting_opportunities(
        &self,
    ) -> ShellResult<crate::scouting::ScoutingOpportunities> {
        let path = with_query(
            "/scouting/opportunities",
            &[("limit", "500"), ("include_dismissed", "true")],
        );
        self.json(Method::GET, &path, None, 20).await
    }

    /// Runs one selected source. The server intentionally treats this GET as a write because it
    /// fetches third-party pages and persists ranked results.
    pub async fn scouting_discover(
        &self,
        adapter: &str,
        location: Option<&str>,
        query: Option<&str>,
    ) -> ShellResult<crate::scouting::DiscoverResponse> {
        if !valid_scouting_id(adapter) {
            return Err(ShellError::Refused(
                400,
                Some("Not a Scouting adapter name.".into()),
            ));
        }
        let mut pairs = vec![("adapter", adapter), ("limit", "20")];
        if let Some(location) = location.filter(|value| !value.trim().is_empty()) {
            pairs.push(("location", location));
        }
        if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            pairs.push(("query", query));
        }
        let path = with_query("/scouting/discover", &pairs);
        let value: serde_json::Value = self.json(Method::GET, &path, None, 120).await?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return Err(ShellError::Refused(502, Some(error.to_owned())));
        }
        serde_json::from_value(value).map_err(|_| ShellError::Unreadable)
    }

    /// Saves, dismisses, or restores one stored opportunity.
    pub async fn scouting_set_status(
        &self,
        id: &str,
        status: crate::scouting::OpportunityStatus,
    ) -> ShellResult<()> {
        if !valid_scouting_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a Scouting opportunity id.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["scouting", "opportunities", id, "status"]);
        let path = url.path().to_owned();
        let body = serde_json::json!({"status": status.as_str()});
        self.send(Method::POST, &path, Some(body), 10)
            .await
            .map(drop)
    }

    /// Idempotently promotes a Scouting result into the Calendar capability.
    pub async fn calendar_upsert_external(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        self.json(
            Method::PUT,
            "/calendar/api/entries/external",
            Some(body),
            10,
        )
        .await
    }

    /// Calendar-owned feasibility judgments for dated Scouting opportunities.
    pub async fn calendar_candidate_verdicts(
        &self,
        candidates: &[crate::scouting::CalendarCandidate],
    ) -> ShellResult<crate::scouting::CalendarVerdicts> {
        self.json(
            Method::POST,
            "/calendar/api/verdicts",
            Some(serde_json::json!({ "candidates": candidates })),
            10,
        )
        .await
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

    /// Reads a YouTube set's metadata and chapter-derived tracklist without downloading audio.
    pub async fn ytalbum_probe(&self, url: &str) -> ShellResult<YtAlbumProbe> {
        self.json(
            Method::POST,
            "/ytalbum/api/probe",
            Some(serde_json::json!({ "url": url })),
            90,
        )
        .await
    }

    /// Starts the explicit album build in ytalbum.
    pub async fn ytalbum_build(&self, build: &YtAlbumBuild) -> ShellResult<YtAlbumJob> {
        self.json(
            Method::POST,
            "/ytalbum/api/jobs",
            Some(serde_json::to_value(build).map_err(|_| ShellError::Unreadable)?),
            10,
        )
        .await
    }

    /// Reads one build's progress and recent log.
    pub async fn ytalbum_job(&self, id: &str) -> ShellResult<YtAlbumJob> {
        self.json(Method::GET, &format!("/ytalbum/api/jobs/{id}"), None, 10)
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

    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<T> {
        let response = self.send(method, path, body, timeout_secs).await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    pub(crate) async fn send(
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
        let url = self.base.join(path).map_err(|_| ShellError::Unreadable)?;
        // A stale Keychain token cannot have changed a GET's state. Refresh it and retry that
        // read once; never replay writes or uploads after a refusal.
        let has_payload = payload.is_some();
        let mut payload = payload;
        let mut attempt = 0;
        loop {
            let token = self.token()?;
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(token)
                .timeout(Duration::from_secs(timeout_secs));
            request = match payload.take() {
                None => request,
                Some(Payload::Json(body)) => request.json(&body),
                Some(Payload::Bytes(bytes)) => request
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(bytes),
            };
            let response = request.send().await.map_err(|_| ShellError::Unreachable)?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED {
                let body = response.bytes().await.unwrap_or_default();
                let rejected_token = authentication_rejection(&body);
                *self.token.lock().expect("token lock") = None;
                if should_retry_auth(&method, has_payload, rejected_token, attempt) {
                    attempt += 1;
                    continue;
                }
                return Err(ShellError::Refused(status.as_u16(), error_text(&body)));
            }
            if status.is_success() {
                return Ok(response);
            }
            let body = response.bytes().await.unwrap_or_default();
            return Err(ShellError::Refused(status.as_u16(), error_text(&body)));
        }
    }

    fn token(&self) -> ShellResult<String> {
        cached_token(&self.token)
    }
}

fn cached_token(cache: &Mutex<Option<String>>) -> ShellResult<String> {
    let mut cached = cache.lock().expect("token lock");
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

fn authentication_rejection(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| {
            body.get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|error| error == "invalid or missing authentication token")
}

fn should_retry_auth(
    method: &Method,
    has_payload: bool,
    auth_rejection: bool,
    attempt: usize,
) -> bool {
    attempt == 0
        && !has_payload
        && auth_rejection
        && (method == Method::GET || method == Method::HEAD)
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

fn valid_calendar_entry_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

fn valid_cloud_content_id(source: &str, id: &str) -> bool {
    matches!(source, "feed" | "mail")
        && !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && (source != "feed" || is_sha256(id))
}

fn valid_typed_id(id: &str) -> bool {
    id.len() <= 256
        && id.split(':').count() >= 2
        && id.split(':').all(|part| !part.is_empty())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

fn valid_scouting_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

/// Why one connection to the change stream ended.
enum Followed {
    /// The shell answered, but does not serve the stream: a build from before it existed.
    Unsupported,
    /// The saved token expired; the owner loop reloads it before reconnecting.
    Unauthorized,
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
    if status == StatusCode::UNAUTHORIZED {
        return Followed::Unauthorized;
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

/// A check this client made before asking, in the same shape as the shell's own refusal.
fn refused(why: String) -> ShellError {
    ShellError::Refused(400, Some(why))
}

/// Appends `<timestamp>\t<path>` to `dashboard-fallbacks.tsv`: each page still handed to the
/// browser. A failed write loses the row, never the navigation.
fn note_dashboard_fallback(next: &str) {
    use std::io::Write;
    let dir = match std::env::var("SJEL_CONFIG_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => match std::env::var("HOME") {
            Ok(home) => std::path::PathBuf::from(home).join(".config/sjel"),
            Err(_) => return,
        },
    };
    let line = fallback_line(chrono::Local::now().to_rfc3339(), next);
    let _ = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("dashboard-fallbacks.tsv"))?
            .write_all(line.as_bytes())
    });
}

/// A tab or newline in a path would split the row, so both become spaces.
fn fallback_line(at: String, next: &str) -> String {
    format!("{at}\t{}\n", next.replace(['\t', '\n'], " "))
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

/// The reason in a refused response: `{"error": …}`, or a short plain-text body. Some
/// capabilities answer a refusal as text (transit's `fail`, the proxy's 502), and dropping it
/// leaves only a status code to show.
pub(crate) fn error_text(body: &[u8]) -> Option<String> {
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
        return json.get("error")?.as_str().map(str::to_owned);
    }
    let text = std::str::from_utf8(body).ok()?.trim();
    (!text.is_empty() && text.len() <= 300 && !text.starts_with('<')).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_its_reason_in_json_or_text() {
        assert_eq!(error_text(br#"{"error":"bad"}"#).as_deref(), Some("bad"));
        assert_eq!(error_text(br#"{"detail":"x"}"#), None);
        assert_eq!(
            error_text(b"no cheaper split for this route\n").as_deref(),
            Some("no cheaper split for this route")
        );
        assert_eq!(error_text(b""), None);
        assert_eq!(error_text(b"<html>502</html>"), None);
        assert_eq!(error_text("x".repeat(301).as_bytes()), None);
    }

    #[test]
    fn only_the_inbound_auth_refusal_triggers_a_read_retry() {
        assert!(authentication_rejection(
            br#"{"error":"invalid or missing authentication token"}"#
        ));
        assert!(!authentication_rejection(br#"{"error":"not allowed"}"#));
        assert!(!authentication_rejection(
            b"invalid or missing authentication token"
        ));
    }

    #[test]
    fn auth_retry_never_replays_writes_or_a_second_refusal() {
        assert!(should_retry_auth(&Method::GET, false, true, 0));
        assert!(should_retry_auth(&Method::HEAD, false, true, 0));
        assert!(!should_retry_auth(&Method::POST, false, true, 0));
        assert!(!should_retry_auth(&Method::GET, true, true, 0));
        assert!(!should_retry_auth(&Method::GET, false, false, 0));
        assert!(!should_retry_auth(&Method::GET, false, true, 1));
    }

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
    fn a_fallback_is_one_row() {
        assert_eq!(
            fallback_line("2026-10-24T14:00:00+02:00".into(), "/feed/review?q=a\tb\nc"),
            "2026-10-24T14:00:00+02:00\t/feed/review?q=a b c\n"
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
