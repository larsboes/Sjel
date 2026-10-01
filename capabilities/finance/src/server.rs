use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use finance::accounting::AccountingEngine;
use finance::allocation::{self, ExpenseAllocation, ReimbursementLink, SHARED_RECEIVABLE_ACCOUNT};
use finance::analytics::{self, AnalyticsFilter};
use finance::balance::{self, ManualBalanceSnapshot, ManualBalanceUpdate, TrackedNetWorth};
use finance::clock::{iso_day, now_timestamp, today, valid_iso_date};
use finance::config::{
    Config, CsvMappingProfile, InstrumentProfile, InvestmentCsvMappingProfile, ObsidianConfig,
    RecurringCommitment, TargetPolicy,
};
use finance::import::{self, CandidateState, CsvMapping, TransactionCandidate};
use finance::investment::{self, HoldingsCoverage, InvestmentCsvMapping};
use finance::obsidian::{self, WriteBack};
use finance::planning::{self, PlanningConfig, PlanningReport, SourceExpectation};
use finance::store::FinanceStore;
use finance::subscription::{burn_by_currency, PricePoint, StateChange};
use finance::JournalEngine;

/// What this capability answers, served as data beside `/health`. Query parameters
/// are named in the summary: a path alone cannot tell a caller what to send, and
/// learning it from a 400 is what this endpoint exists to prevent.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/ready",
        "Readiness: liveness plus a reachable database.",
    ),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/subscriptions",
        "Every subscription with its full price and state history.",
    ),
    r(
        "GET",
        "/api/subscriptions/burn",
        "Monthly and annual burn, computed from the price series. Optional ?at=YYYY-MM-DD, default today.",
    ),
    r(
        "POST",
        "/api/subscriptions/{id}/price",
        "Idempotently append a price point. Body: valid_from, amount_cents, currency, cycle, optional plan, required reason. Response: created.",
    ),
    r(
        "POST",
        "/api/subscriptions/{id}/state",
        "Idempotently append a state change. Body: effective, state, note. Response: created.",
    ),
    r(
        "GET",
        "/api/import/obsidian/scan",
        "Vault subscription notes that could be imported. Read-only.",
    ),
    r(
        "POST",
        "/api/import/obsidian",
        "Import every scanned vault subscription note. Idempotent by vault path.",
    ),
    r(
        "POST",
        "/api/writeback",
        "Regenerate the derived block in each note. Conflicts are reported, never resolved.",
    ),
    r(
        "POST",
        "/api/import/csv/preview",
        "Validate and summarize a CSV export without staging candidates or returning transaction details.",
    ),
    r(
        "POST",
        "/api/import/csv",
        "Recompute and stage an unchanged CSV preview as review candidates. Raw rows are not retained.",
    ),
    r(
        "GET",
        "/api/import/csv/mappings",
        "Named CSV mapping profiles loaded from the private overlay.",
    ),
    r(
        "GET",
        "/api/import/investments/mappings",
        "Named investment CSV mapping profiles loaded from the private overlay.",
    ),
    r(
        "POST",
        "/api/import/investments/preview",
        "Reconstruct holdings from signed investment activity without persisting source rows or writing the journal.",
    ),
    r(
        "POST",
        "/api/import/investments/confirm",
        "Recompute and confirm an unchanged holdings preview for one source in the configured private collection.",
    ),
    r(
        "GET",
        "/api/import/candidates",
        "List normalized transaction candidates and their review state.",
    ),
    r(
        "POST",
        "/api/import/candidates/{id}/review",
        "Confirm or reject one candidate. Reconfirming with another valid account atomically reclassifies its existing journal posting.",
    ),
    r(
        "POST",
        "/api/import/candidates/reclassify-batch",
        "Reclassify an explicit bounded set of confirmed expenses to reviewed non-uncategorized expense accounts, replacing the journal once.",
    ),
    r(
        "POST",
        "/api/import/candidates/{id}/reconcile-transfer",
        "Confirm one side of a reciprocal transfer and mark the counterpart as duplicate source evidence.",
    ),
    r(
        "POST",
        "/api/import/candidates/{id}/allocation",
        "Apply reviewed purpose, optional Trips plan, and personal/shared split to a confirmed expense.",
    ),
    r(
        "POST",
        "/api/import/candidates/{id}/reimbursement",
        "Link a confirmed inflow to an outstanding shared expense as receivable settlement, never income.",
    ),
    r(
        "POST",
        "/api/import/candidates/confirm-batch",
        "Confirm an explicit bounded list of candidate IDs with their reviewed accounts, then rebuild the projection once.",
    ),
    r(
        "POST",
        "/api/balance-snapshot",
        "Replace the configured private manual balance snapshot and stamp its update time.",
    ),
    r(
        "GET",
        "/api/ledger/check",
        "Check the configured journal through the accounting-engine boundary.",
    ),
    r(
        "POST",
        "/api/ledger/rebuild",
        "Atomically rebuild the disposable transaction projection from the canonical journal.",
    ),
    r(
        "GET",
        "/api/dashboard",
        "One reconciled projection for KPIs, trend, transactions, filters and Sankey links.",
    ),
    r(
        "GET",
        "/api/portfolio",
        "Positions with price freshness, share in basis points and drift against the configured targets. Optional ?currency=EUR, default EUR.",
    ),
    r(
        "GET",
        "/api/prices/status",
        "Per-instrument price freshness, the newest 50 fetch attempts and the last status per provider. No parameters.",
    ),
    r(
        "GET",
        "/__axon/freshness",
        "When data last arrived, for the freshness contract. No parameters.",
    ),
    r(
        "GET",
        "/api/decisions",
        "The decision ledger. Optional ?status=open|accepted|rejected|superseded|all, default open.",
    ),
    r(
        "POST",
        "/api/decisions/run",
        "Recompute proposals and reconcile them against the ledger. Body: optional currency, optional dry_run. Response: proposed, unchanged, superseded, caveats.",
    ),
    r(
        "POST",
        "/api/decisions/{id}/verdict",
        "Record a human verdict. Body: expected_proposal_id, verdict (accepted|rejected), note (required on rejected). 409 when a recompute no longer mints this id. Records a decision and moves no money.",
    ),
    r(
        "GET",
        "/api/trips/{id}/spending",
        "One trip's actuals: personal spending, gross cash outflow, reimbursed, outstanding and the posting count. No parameters.",
    ),
];

const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("finance", ROUTES))
}

/// The reporting currency when nothing narrows it, matching
/// `config::default_currency` (config.rs:57). Named here because three handlers
/// now fall back to it and a literal in each is three places to disagree.
const DEFAULT_CURRENCY: &str = "EUR";

#[derive(Clone)]
struct AppState {
    database_path: Arc<PathBuf>,
    obsidian: Option<ObsidianConfig>,
    journal: Option<std::path::PathBuf>,
    commitments: Arc<Vec<RecurringCommitment>>,
    planning: Arc<PlanningConfig>,
    csv_mappings: Arc<Vec<CsvMappingProfile>>,
    investment_csv_mappings: Arc<Vec<InvestmentCsvMappingProfile>>,
    investment_snapshot: Option<std::path::PathBuf>,
    balance_snapshot: Option<std::path::PathBuf>,
    journal_write: Arc<std::sync::Mutex<()>>,
    projection_write: Arc<std::sync::Mutex<()>>,
    balance_write: Arc<std::sync::Mutex<()>>,
    instruments: Arc<Vec<InstrumentProfile>>,
    targets: Option<Arc<TargetPolicy>>,
    comms_base_url: Arc<String>,
    /// Where the human-readable copy of the decision ledger is written.
    ///
    /// `SJEL_FINANCE_DECISIONS_ROOT` if set, else the overlay root. The override
    /// exists so a live check can redirect the WRITE without redirecting the
    /// config READ. `None` means no overlay is configured at all.
    overlay_root: Option<Arc<PathBuf>>,
}

// No decision recompute mutex on `AppState`, deliberately: `finance-cli
// decisions run` is a second process, and an in-process mutex cannot see it. The
// guard is `BEGIN IMMEDIATE` inside `FinanceStore::reconcile_decisions`, which
// two processes both respect.

type ApiResponse = (StatusCode, Json<Value>);

fn response<T: serde::Serialize>(status: StatusCode, value: T) -> ApiResponse {
    (
        status,
        Json(
            serde_json::to_value(value)
                .unwrap_or_else(|_| json!({ "error": "serialization failed" })),
        ),
    )
}

fn failed(error: String) -> ApiResponse {
    response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "ok": false, "capability": "finance", "error": error }),
    )
}

/// No vault configured is a 409 rather than a 500: nothing is broken, the operator
/// has not pointed this capability at anything yet, and a stack trace would suggest
/// otherwise.
fn no_vault() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no vault configured; set the overlay's config/finance.json or SJEL_FINANCE_OBSIDIAN_ROOT"
        }),
    )
}

fn no_journal() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no journal configured; set the overlay's config/finance.json or SJEL_FINANCE_JOURNAL"
        }),
    )
}

fn no_balance_snapshot() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no private balance snapshot is configured"
        }),
    )
}

#[derive(Debug, Deserialize)]
struct CsvImportRequest {
    content: String,
    mapping: CsvMapping,
    expected_preview_id: String,
}

#[derive(Debug, Deserialize)]
struct CsvPreviewRequest {
    content: String,
    mapping: CsvMapping,
}

async fn preview_csv(Json(request): Json<CsvPreviewRequest>) -> ApiResponse {
    match import::preview_csv(request.content.as_bytes(), &request.mapping) {
        Ok(preview) => response(StatusCode::OK, preview),
        Err(error) => response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn import_csv(
    State(state): State<AppState>,
    Json(request): Json<CsvImportRequest>,
) -> ApiResponse {
    let prepared = match import::prepare_csv(request.content.as_bytes(), &request.mapping) {
        Ok(prepared) => prepared,
        Err(error) => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            )
        }
    };
    if prepared.preview.preview_id != request.expected_preview_id {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": "CSV preview changed before staging" }),
        );
    }
    let preview = prepared.preview;
    let candidates = prepared.candidates;
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| store.stage_candidates(&candidates, &now))
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok((created, already_present))) => response(
            StatusCode::OK,
            json!({
                "ok": true,
                "created": created,
                "already_present": already_present,
                "duplicate_rows": preview.duplicate_rows,
                "preserved_repetitions": preview.preserved_repetitions,
                "ignored_non_transaction_rows": preview.ignored_non_transaction_rows,
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Serialize)]
struct CandidateReviewView {
    #[serde(flatten)]
    candidate: TransactionCandidate,
    transfer_match_ids: Vec<String>,
}

async fn list_candidates(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| store.list_candidates())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(candidates)) => {
            let views = candidates
                .iter()
                .cloned()
                .map(|candidate| CandidateReviewView {
                    transfer_match_ids: import::transfer_match_ids(&candidates, &candidate),
                    candidate,
                })
                .collect::<Vec<_>>();
            response(StatusCode::OK, views)
        }
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_csv_mappings(State(state): State<AppState>) -> Json<Vec<CsvMappingProfile>> {
    Json(state.csv_mappings.as_ref().clone())
}

#[derive(Debug, Deserialize)]
struct InvestmentPreviewRequest {
    content: String,
    mapping: InvestmentCsvMapping,
}

async fn preview_investments(Json(request): Json<InvestmentPreviewRequest>) -> ApiResponse {
    match investment::preview_csv(request.content.as_bytes(), &request.mapping) {
        Ok(preview) => response(StatusCode::OK, preview),
        Err(error) => response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct InvestmentConfirmRequest {
    content: String,
    mapping: InvestmentCsvMapping,
    source_key: String,
    expected_snapshot_id: String,
    #[serde(default)]
    coverage: HoldingsCoverage,
}

async fn confirm_investments(
    State(state): State<AppState>,
    Json(request): Json<InvestmentConfirmRequest>,
) -> ApiResponse {
    let Some(path) = state.investment_snapshot.clone() else {
        return response(
            StatusCode::CONFLICT,
            json!({ "error": "no private holdings snapshot is configured" }),
        );
    };
    let database_path = state.database_path.clone();
    let projection_write = state.projection_write.clone();
    let reviewed_at = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let preview = investment::preview_csv(request.content.as_bytes(), &request.mapping)
            .map_err(|error| error.to_string())?;
        if preview.snapshot_id != request.expected_snapshot_id {
            return Err("investment preview changed before confirmation".into());
        }
        let snapshot = investment::reviewed_snapshot(
            &preview,
            &request.mapping,
            &reviewed_at,
            request.coverage,
        )
        .map_err(|error| error.to_string())?;
        let _write_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let created = investment::write_reviewed_snapshot(&path, &request.source_key, &snapshot)
            .map_err(|error| error.to_string())?;
        let canonical = investment::read_reviewed_snapshot(&path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "holdings snapshot disappeared after confirmation".to_string())?;
        FinanceStore::open(&database_path)
            .and_then(|store| store.replace_holding_projection(&canonical))
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "ok": true,
            "created": created,
            "snapshot": canonical,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn list_investment_mappings(
    State(state): State<AppState>,
) -> Json<Vec<InvestmentCsvMappingProfile>> {
    Json(state.investment_csv_mappings.as_ref().clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewDecision {
    Confirm,
    Reject,
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    decision: ReviewDecision,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReconcileTransferRequest {
    counterpart_id: String,
}

#[derive(Debug, Deserialize)]
struct BatchConfirmation {
    id: String,
    account: String,
}

#[derive(Debug, Deserialize)]
struct BatchConfirmationRequest {
    items: Vec<BatchConfirmation>,
}

#[derive(Debug, Deserialize)]
struct BatchReclassificationRequest {
    items: Vec<BatchConfirmation>,
}

async fn reconcile_transfer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReconcileTransferRequest>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let first = store
            .candidate(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "candidate not found".to_string())?;
        let second = store
            .candidate(&request.counterpart_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "counterpart candidate not found".to_string())?;
        if !import::is_transfer_pair(&first, &second) {
            return Err("candidates are not a reciprocal transfer pair".into());
        }
        if first.state == CandidateState::Confirmed && second.state == CandidateState::Confirmed {
            return Err("both transfer candidates are already confirmed".into());
        }
        let (canonical, duplicate) = if first.state == CandidateState::Confirmed {
            (&first, &second)
        } else if second.state == CandidateState::Confirmed {
            (&second, &first)
        } else if first.source_account.starts_with("assets:")
            && !second.source_account.starts_with("assets:")
        {
            (&first, &second)
        } else if second.source_account.starts_with("assets:")
            && !first.source_account.starts_with("assets:")
        {
            (&second, &first)
        } else if first.id <= second.id {
            (&first, &second)
        } else {
            (&second, &first)
        };
        if !matches!(
            canonical.state,
            CandidateState::Pending | CandidateState::Confirmed
        ) || !matches!(
            duplicate.state,
            CandidateState::Pending | CandidateState::Duplicate
        ) {
            return Err("transfer pair is not in a reconcilable review state".into());
        }
        let _write_guard = journal_write
            .lock()
            .map_err(|_| "journal writer lock is unavailable".to_string())?;
        let journal_written = if canonical.state == CandidateState::Confirmed {
            false
        } else {
            let entry = import::render_journal_entry(canonical, &canonical.proposed_account)
                .map_err(|error| error.to_string())?;
            validate_journal_append(&journal, &entry)?;
            import::append_confirmed(&journal, canonical, &canonical.proposed_account)
                .map_err(|error| error.to_string())?
        };
        if !store
            .review_transfer_pair(
                &canonical.id,
                &duplicate.id,
                &canonical.proposed_account,
                &now,
            )
            .map_err(|error| error.to_string())?
        {
            return Err("transfer pair changed before reconciliation".into());
        }
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
        Ok(json!({
            "ok": true,
            "canonical_id": canonical.id,
            "duplicate_id": duplicate.id,
            "journal_written": journal_written,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error.ends_with("candidate not found") => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn allocate_expense(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(allocation_request): Json<ExpenseAllocation>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let candidate = store
            .candidate(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "candidate not found".to_string())?;
        let _journal_guard = journal_write
            .lock()
            .map_err(|_| "journal writer lock is unavailable".to_string())?;
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let current = std::fs::read_to_string(&journal)
            .map_err(|error| format!("journal could not be read: {error}"))?;
        let (updated, changed) =
            allocation::rewrite_expense(&current, &candidate, &allocation_request)
                .map_err(|error| error.to_string())?;
        if changed {
            replace_journal_atomically(&journal, &updated)?;
        }
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
        Ok(json!({
            "ok": true,
            "id": id,
            "journal_written": changed,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error == "candidate not found" => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn link_reimbursement(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReimbursementLink>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let candidate = store
            .candidate(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "candidate not found".to_string())?;
        let expense = store
            .candidate(&request.expense_candidate_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "expense candidate not found".to_string())?;
        if expense.state != CandidateState::Confirmed || expense.amount_cents >= 0 {
            return Err("reimbursement target must be a confirmed expense".into());
        }
        if candidate.currency != expense.currency {
            return Err("reimbursement and expense currencies must match".into());
        }
        let _journal_guard = journal_write
            .lock()
            .map_err(|_| "journal writer lock is unavailable".to_string())?;
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let rows = store
            .transaction_projection()
            .map_err(|error| error.to_string())?;
        let outstanding = analytics::outstanding_shared_cents(
            &rows,
            &expense.fingerprint,
            &candidate.currency,
            Some(&candidate.fingerprint),
        )
        .ok_or_else(|| "expense has no reviewed shared-cost allocation".to_string())?;
        if candidate.amount_cents <= 0 || candidate.amount_cents > outstanding {
            return Err("reimbursement exceeds the outstanding shared receivable".into());
        }
        let current = std::fs::read_to_string(&journal)
            .map_err(|error| format!("journal could not be read: {error}"))?;
        let (updated, changed) =
            allocation::rewrite_reimbursement(&current, &candidate, &expense.fingerprint)
                .map_err(|error| error.to_string())?;
        if changed {
            replace_journal_atomically(&journal, &updated)?;
        }
        if !store
            .review_candidate(
                &id,
                CandidateState::Confirmed,
                SHARED_RECEIVABLE_ACCOUNT,
                &now,
            )
            .map_err(|error| error.to_string())?
        {
            return Err("candidate changed before reimbursement linking".into());
        }
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
        Ok(json!({
            "ok": true,
            "id": id,
            "expense_candidate_id": request.expense_candidate_id,
            "journal_written": changed,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error.ends_with("candidate not found") => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn confirm_candidates_batch(
    State(state): State<AppState>,
    Json(request): Json<BatchConfirmationRequest>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    if request.items.is_empty() || request.items.len() > 1_000 {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "batch must contain between 1 and 1000 candidates" }),
        );
    }
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let mut seen = std::collections::HashSet::new();
        let mut prepared = Vec::with_capacity(request.items.len());
        for item in request.items {
            if !seen.insert(item.id.clone()) {
                return Err("batch contains a duplicate candidate id".into());
            }
            let candidate = store
                .candidate(&item.id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "candidate not found".to_string())?;
            if !matches!(
                candidate.state,
                CandidateState::Pending | CandidateState::Confirmed
            ) {
                return Err("batch candidate is not confirmable".into());
            }
            if item.account != candidate.proposed_account {
                return Err("batch account no longer matches the staged suggestion".into());
            }
            import::validate_account(&item.account).map_err(|error| error.to_string())?;
            let entry = (candidate.state == CandidateState::Pending)
                .then(|| import::render_journal_entry(&candidate, &item.account))
                .transpose()
                .map_err(|error| error.to_string())?;
            prepared.push((candidate, item.account, entry));
        }
        let _write_guard = journal_write
            .lock()
            .map_err(|_| "journal writer lock is unavailable".to_string())?;
        let combined: String = prepared
            .iter()
            .filter_map(|(_, _, entry)| entry.as_deref())
            .collect();
        if !combined.is_empty() {
            validate_journal_append(&journal, &combined)?;
        }
        let mut journal_writes = 0;
        for (candidate, account, _) in &prepared {
            journal_writes += usize::from(
                import::append_confirmed(&journal, candidate, account)
                    .map_err(|error| error.to_string())?,
            );
            if !store
                .review_candidate(&candidate.id, CandidateState::Confirmed, account, &now)
                .map_err(|error| error.to_string())?
            {
                return Err("candidate changed before batch confirmation".into());
            }
        }
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
        Ok(json!({
            "ok": true,
            "confirmed": prepared.len(),
            "journal_writes": journal_writes,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error == "candidate not found" => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn reclassify_candidates_batch(
    State(state): State<AppState>,
    Json(request): Json<BatchReclassificationRequest>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    if request.items.is_empty() || request.items.len() > 500 {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "reclassification batch must contain between 1 and 500 candidates" }),
        );
    }
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let mut seen = std::collections::HashSet::new();
        let mut prepared = Vec::with_capacity(request.items.len());
        for item in request.items {
            if !seen.insert(item.id.clone()) {
                return Err("reclassification batch contains a duplicate candidate id".into());
            }
            let candidate = store
                .candidate(&item.id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "candidate not found".to_string())?;
            if candidate.state != CandidateState::Confirmed
                || candidate.amount_cents >= 0
                || !candidate.proposed_account.starts_with("expenses:")
            {
                return Err("reclassification requires confirmed expense candidates".into());
            }
            import::validate_account(&item.account).map_err(|error| error.to_string())?;
            if !is_reviewed_expense_account(&item.account) {
                return Err("reclassification target must be a categorized expense account".into());
            }
            prepared.push((candidate, item.account));
        }

        let _write_guard = journal_write
            .lock()
            .map_err(|_| "journal writer lock is unavailable".to_string())?;
        let mut updated = std::fs::read_to_string(&journal)
            .map_err(|error| format!("journal could not be read: {error}"))?;
        let mut reclassified = 0;
        for (candidate, account) in &prepared {
            if candidate.proposed_account == *account {
                continue;
            }
            let (next, changed) = import::rewrite_confirmed_account(&updated, candidate, account)
                .map_err(|error| error.to_string())?;
            if !changed {
                return Err("candidate journal posting did not change".into());
            }
            updated = next;
            reclassified += 1;
        }
        if reclassified > 0 {
            replace_journal_atomically(&journal, &updated)?;
        }
        for (candidate, account) in &prepared {
            if !store
                .review_candidate(&candidate.id, CandidateState::Confirmed, account, &now)
                .map_err(|error| error.to_string())?
            {
                return Err("candidate changed before batch reclassification".into());
            }
        }
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
        Ok(json!({
            "ok": true,
            "reviewed": prepared.len(),
            "reclassified": reclassified,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error == "candidate not found" => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

fn is_reviewed_expense_account(account: &str) -> bool {
    account.starts_with("expenses:")
        && !account.split(':').any(|segment| segment == "uncategorized")
}

async fn review_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let journal_write = state.journal_write.clone();
    let projection_write = state.projection_write.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let candidate = store
            .candidate(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "candidate not found".to_string())?;
        match request.decision {
            ReviewDecision::Reject => {
                if candidate.state == CandidateState::Confirmed {
                    return Err(
                        "a confirmed candidate cannot be rejected because its posting is canonical"
                            .into(),
                    );
                }
                store
                    .review_candidate(
                        &id,
                        CandidateState::Rejected,
                        &candidate.proposed_account,
                        &now,
                    )
                    .map_err(|error| error.to_string())?;
                Ok(json!({ "ok": true, "id": id, "state": "rejected", "journal_written": false }))
            }
            ReviewDecision::Confirm => {
                if candidate.state == CandidateState::Rejected {
                    return Err("a rejected candidate must be restaged before confirmation".into());
                }
                let account = if candidate.state == CandidateState::Confirmed {
                    request
                        .account
                        .as_deref()
                        .unwrap_or(&candidate.proposed_account)
                } else {
                    request
                        .account
                        .as_deref()
                        .ok_or_else(|| "account is required for confirmation".to_string())?
                };
                import::validate_account(account).map_err(|error| error.to_string())?;
                let _write_guard = journal_write
                    .lock()
                    .map_err(|_| "journal writer lock is unavailable".to_string())?;
                let reclassified = candidate.state == CandidateState::Confirmed
                    && account != candidate.proposed_account;
                let journal_written = if reclassified {
                    let current = std::fs::read_to_string(&journal)
                        .map_err(|error| format!("journal could not be read: {error}"))?;
                    let (updated, changed) =
                        import::rewrite_confirmed_account(&current, &candidate, account)
                            .map_err(|error| error.to_string())?;
                    if changed {
                        replace_journal_atomically(&journal, &updated)?;
                    }
                    changed
                } else {
                    let entry = import::render_journal_entry(&candidate, account)
                        .map_err(|error| error.to_string())?;
                    validate_journal_append(&journal, &entry)?;
                    import::append_confirmed(&journal, &candidate, account)
                        .map_err(|error| error.to_string())?
                };
                store
                    .review_candidate(&id, CandidateState::Confirmed, account, &now)
                    .map_err(|error| error.to_string())?;
                let _projection_guard = projection_write
                    .lock()
                    .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
                rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())?;
                Ok(json!({
                    "ok": true,
                    "id": id,
                    "state": "confirmed",
                    "journal_written": journal_written,
                    "reclassified": reclassified,
                }))
            }
        }
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(error)) if error == "candidate not found" => {
            response(StatusCode::NOT_FOUND, json!({ "error": error }))
        }
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

fn validate_journal_append(journal: &std::path::Path, entry: &str) -> Result<(), String> {
    let existing = std::fs::read_to_string(journal)
        .map_err(|error| format!("journal could not be read: {error}"))?;
    let parent = journal
        .parent()
        .ok_or_else(|| "journal has no parent directory".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let name = format!(".axon-finance-check-{}-{nonce}", std::process::id());
    let temporary = parent.join(name);
    let result = (|| {
        std::fs::write(&temporary, format!("{existing}{entry}"))
            .map_err(|error| format!("journal validation file could not be written: {error}"))?;
        JournalEngine::new(&temporary)
            .check()
            .map_err(|error| error.to_string())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

fn replace_journal_atomically(journal: &std::path::Path, updated: &str) -> Result<(), String> {
    use std::io::Write;

    let parent = journal
        .parent()
        .ok_or_else(|| "journal has no parent directory".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = parent.join(format!(
        ".axon-finance-reclassify-{}-{nonce}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("journal replacement could not be created: {error}"))?;
        if let Ok(metadata) = std::fs::metadata(journal) {
            file.set_permissions(metadata.permissions())
                .map_err(|error| format!("journal permissions could not be preserved: {error}"))?;
        }
        file.write_all(updated.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("journal replacement could not be written: {error}"))?;
        JournalEngine::new(&temporary)
            .check()
            .map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, journal)
            .map_err(|error| format!("journal replacement could not be installed: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

async fn check_ledger(State(state): State<AppState>) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    match tokio::task::spawn_blocking(move || JournalEngine::new(journal).check()).await {
        Ok(Ok(())) => response(StatusCode::OK, json!({ "ok": true })),
        Ok(Err(error)) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": error.to_string() }),
        ),
        Err(_) => failed("task panicked".into()),
    }
}

async fn rebuild_ledger(State(state): State<AppState>) -> ApiResponse {
    let Some(journal) = state.journal.clone() else {
        return no_journal();
    };
    let database_path = state.database_path.clone();
    let investment_snapshot = state.investment_snapshot.clone();
    let projection_write = state.projection_write.clone();
    match tokio::task::spawn_blocking(move || {
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        rebuild_projection(&database_path, &journal, investment_snapshot.as_deref())
    })
    .await
    {
        Ok(Ok(count)) => response(StatusCode::OK, json!({ "ok": true, "rows": count })),
        Ok(Err(error)) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": error }),
        ),
        Err(_) => failed("task panicked".into()),
    }
}

fn rebuild_projection(
    database_path: &std::path::Path,
    journal: &std::path::Path,
    investment_snapshot: Option<&std::path::Path>,
) -> Result<usize, String> {
    let engine = JournalEngine::new(journal);
    engine.check().map_err(|error| error.to_string())?;
    let transactions = engine.transactions().map_err(|error| error.to_string())?;
    // EUR, and only EUR. Budget targets were the one input that could widen this
    // set, and they were never configured anywhere (PRD Q50, 2026-08-28); the set
    // they widened was already `{"EUR"}` on every deployment. Stating the single
    // currency outright is the same projection with the pretence removed -- a
    // second currency needs a real source, not a config key nothing writes.
    let rows: Vec<_> = analytics::project(&transactions, "EUR");
    let store = FinanceStore::open(database_path).map_err(|error| error.to_string())?;
    store
        .replace_transaction_projection(&rows)
        .map_err(|error| error.to_string())?;
    match investment_snapshot {
        Some(path) => {
            match investment::read_reviewed_snapshot(path).map_err(|error| error.to_string())? {
                Some(snapshot) => store
                    .replace_holding_projection(&snapshot)
                    .map_err(|error| error.to_string())?,
                None => store
                    .clear_holding_projection()
                    .map_err(|error| error.to_string())?,
            }
        }
        None => store
            .clear_holding_projection()
            .map_err(|error| error.to_string())?,
    }
    Ok(rows.len())
}

async fn dashboard_projection(
    State(state): State<AppState>,
    Query(filter): Query<AnalyticsFilter>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let balance_snapshot_path = state.balance_snapshot.clone();
    let commitments = state.commitments.clone();
    let planning_config = state.planning.clone();
    let projection_write = state.projection_write.clone();
    match tokio::task::spawn_blocking(move || {
        let _projection_guard = projection_write
            .lock()
            .map_err(|_| "finance projection writer lock is unavailable".to_string())?;
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let rows = store
            .transaction_projection()
            .map_err(|error| error.to_string())?;
        let subscriptions = store.list().map_err(|error| error.to_string())?;
        let mut view = analytics::dashboard(&rows, &filter);
        let investment = store
            .holding_projection()
            .map_err(|error| error.to_string())?;
        view.portfolio_values = investment
            .as_ref()
            .map(investment::portfolio_valuations)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        view.investment = investment;
        let balance_snapshot = balance_snapshot_path
            .as_deref()
            .map(balance::read_snapshot)
            .transpose()?
            .flatten();
        let portfolio = view
            .portfolio_values
            .iter()
            .find(|value| value.currency == view.summary.currency);
        let portfolio_complete = view.investment.as_ref().is_some_and(|snapshot| {
            snapshot.coverage == HoldingsCoverage::Complete
                && portfolio.is_some_and(|value| value.unpriced_holdings == 0)
        });
        let tracked_net_worth = balance_snapshot
            .as_ref()
            .map(|snapshot| balance::tracked_net_worth(snapshot, portfolio, portfolio_complete))
            .transpose()?;
        let commitment_as_of = today();
        let current_commitment_monthly_cents = commitments
            .iter()
            .filter(|commitment| {
                commitment.currency == view.summary.currency
                    && commitment.active_on(&commitment_as_of)
            })
            .map(|commitment| commitment.monthly_cents)
            .sum();
        let planning = planning::report(planning::PlanningInputs {
            rows: &rows,
            commitments: &commitments,
            subscriptions: &subscriptions,
            balance_snapshot: balance_snapshot.as_ref(),
            investment_snapshot: view.investment.as_ref(),
            portfolio_values: &view.portfolio_values,
            config: &planning_config,
            as_of: &commitment_as_of,
            currency: &view.summary.currency,
        });
        let journal_as_of = view.quality.latest_transaction_date.clone();
        let balance_as_of = balance_snapshot
            .as_ref()
            .map(|snapshot| snapshot.as_of.clone());
        let holdings_as_of = view
            .investment
            .as_ref()
            .map(|snapshot| snapshot.reviewed_at.clone());
        let mut source_freshness = vec![
            SourceFreshness {
                source: "journal".into(),
                label: "Journal projection".into(),
                age_days: source_age_days(journal_as_of.as_deref(), &commitment_as_of),
                freshness: source_freshness_status(
                    journal_as_of.as_deref(),
                    &commitment_as_of,
                    planning_config.journal_freshness_days,
                ),
                as_of: journal_as_of,
                coverage: match view.quality.latest_transaction_date.as_ref() {
                    None => "missing",
                    Some(_) if view.quality.observed_months == view.quality.expected_months => {
                        "complete"
                    }
                    Some(_) => "partial",
                }
                .into(),
            },
            SourceFreshness {
                source: "balances".into(),
                label: "Manual balances".into(),
                age_days: source_age_days(balance_as_of.as_deref(), &commitment_as_of),
                freshness: source_freshness_status(
                    balance_as_of.as_deref(),
                    &commitment_as_of,
                    planning_config.snapshot_freshness_days,
                ),
                as_of: balance_as_of,
                coverage: match balance_snapshot.as_ref().map(|snapshot| snapshot.coverage) {
                    None => "missing",
                    Some(balance::BalanceCoverage::Complete) => "complete",
                    Some(balance::BalanceCoverage::Partial) => "partial",
                }
                .into(),
            },
            SourceFreshness {
                source: "holdings".into(),
                label: "Reviewed holdings".into(),
                age_days: source_age_days(holdings_as_of.as_deref(), &commitment_as_of),
                freshness: source_freshness_status(
                    holdings_as_of.as_deref(),
                    &commitment_as_of,
                    planning_config.snapshot_freshness_days,
                ),
                as_of: holdings_as_of,
                coverage: match view.investment.as_ref() {
                    None => "missing",
                    Some(_) if portfolio_complete => "complete",
                    Some(_) => "partial",
                }
                .into(),
            },
        ];
        source_freshness.extend(expected_source_freshness(
            &rows,
            view.investment.as_ref(),
            &planning_config,
            &commitment_as_of,
        ));
        Ok::<_, String>(DashboardResponse {
            projection: view,
            balance_snapshot,
            tracked_net_worth,
            source_freshness,
            commitment_as_of,
            current_commitment_monthly_cents,
            commitments: commitments.as_ref().clone(),
            planning,
        })
    })
    .await
    {
        Ok(Ok(view)) => response(StatusCode::OK, view),
        Ok(Err(error)) => failed(error),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Serialize)]
struct DashboardResponse {
    #[serde(flatten)]
    projection: analytics::DashboardProjection,
    balance_snapshot: Option<ManualBalanceSnapshot>,
    tracked_net_worth: Option<TrackedNetWorth>,
    source_freshness: Vec<SourceFreshness>,
    commitment_as_of: String,
    current_commitment_monthly_cents: i64,
    commitments: Vec<RecurringCommitment>,
    planning: PlanningReport,
}

#[derive(Debug, Serialize)]
struct SourceFreshness {
    source: String,
    label: String,
    as_of: Option<String>,
    age_days: Option<i64>,
    freshness: String,
    coverage: String,
}

fn expected_source_freshness(
    rows: &[analytics::TransactionRow],
    investment: Option<&investment::ReviewedHoldingsSnapshot>,
    config: &PlanningConfig,
    today: &str,
) -> Vec<SourceFreshness> {
    config
        .source_expectations
        .iter()
        .map(|expectation| match expectation {
            SourceExpectation::Transactions {
                id,
                label,
                account_prefixes,
                freshness_days,
                coverage,
            } => {
                let as_of = rows
                    .iter()
                    .filter(|row| {
                        account_prefixes
                            .iter()
                            .any(|prefix| row.account.starts_with(prefix))
                    })
                    .map(|row| row.date.as_str())
                    .max()
                    .map(str::to_string);
                SourceFreshness {
                    source: format!("transactions:{id}"),
                    label: label.clone(),
                    age_days: source_age_days(as_of.as_deref(), today),
                    freshness: source_freshness_status(
                        as_of.as_deref(),
                        today,
                        freshness_days.unwrap_or(config.journal_freshness_days),
                    ),
                    coverage: as_of
                        .as_ref()
                        .map_or("missing", |_| coverage.as_str())
                        .into(),
                    as_of,
                }
            }
            SourceExpectation::Holdings {
                id,
                label,
                source_key,
                freshness_days,
                coverage,
            } => {
                let source = investment.and_then(|snapshot| {
                    snapshot
                        .sources
                        .iter()
                        .find(|source| source.source_key == *source_key)
                });
                let as_of = source.map(|source| source.reviewed_at.clone());
                let reported_coverage = source.map_or("missing", |source| {
                    if coverage.as_str() == "complete"
                        && source.coverage == HoldingsCoverage::Complete
                    {
                        "complete"
                    } else {
                        "partial"
                    }
                });
                SourceFreshness {
                    source: format!("holdings:{id}"),
                    label: label.clone(),
                    age_days: source_age_days(as_of.as_deref(), today),
                    freshness: source_freshness_status(
                        as_of.as_deref(),
                        today,
                        freshness_days.unwrap_or(config.snapshot_freshness_days),
                    ),
                    coverage: reported_coverage.into(),
                    as_of,
                }
            }
        })
        .collect()
}

fn source_age_days(as_of: Option<&str>, today: &str) -> Option<i64> {
    let age = iso_day(today)?.checked_sub(iso_day(as_of?)?)?;
    Some(age.max(0))
}

fn source_freshness_status(as_of: Option<&str>, today: &str, threshold_days: u32) -> String {
    match source_age_days(as_of, today) {
        None => "missing",
        Some(age) if age <= i64::from(threshold_days) => "current",
        Some(_) => "stale",
    }
    .into()
}

async fn update_balance_snapshot(
    State(state): State<AppState>,
    Json(update): Json<ManualBalanceUpdate>,
) -> ApiResponse {
    let Some(path) = state.balance_snapshot.clone() else {
        return no_balance_snapshot();
    };
    let balance_write = state.balance_write.clone();
    match tokio::task::spawn_blocking(move || -> Result<ManualBalanceSnapshot, String> {
        let snapshot = balance::snapshot_from_update(update, now_timestamp())?;
        let _write_guard = balance_write
            .lock()
            .map_err(|_| "balance snapshot writer lock is unavailable".to_string())?;
        balance::write_snapshot(&path, &snapshot)?;
        Ok(snapshot)
    })
    .await
    {
        Ok(Ok(snapshot)) => response(StatusCode::OK, json!({ "ok": true, "snapshot": snapshot })),
        Ok(Err(error)) => response(StatusCode::BAD_REQUEST, json!({ "error": error })),
        Err(_) => failed("task panicked".into()),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "capability": "finance" }))
}

/// Readiness: whether this capability can actually serve, which liveness cannot
/// answer. `health` is a literal, so during a Postgres outage it would report up
/// while every query behind it failed (#126).
async fn ready(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| store.ping())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => response(
            StatusCode::OK,
            json!({ "ok": true, "capability": "finance" }),
        ),
        // 503, not 500: the request was fine, the dependency is not, and a caller
        // that retries should be told to come back rather than to fix its input.
        Ok(Err(error)) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "finance", "error": error }),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "ok": false, "capability": "finance", "error": "readiness check failed" }),
        ),
    }
}

async fn list_subscriptions(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| store.list())
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(subs)) => response(StatusCode::OK, subs),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

#[derive(Debug, Deserialize)]
struct AtQuery {
    at: Option<String>,
}

fn validate_price(price: &PricePoint) -> Result<(), String> {
    if !valid_iso_date(&price.valid_from) {
        return Err("valid_from must be a real date in YYYY-MM-DD form".into());
    }
    if price.amount_cents < 0 {
        return Err("amount_cents must not be negative".into());
    }
    if price.currency.len() != 3 || !price.currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("currency must be a three-letter uppercase code".into());
    }
    if price.reason.trim().is_empty() {
        return Err("reason is required for an append-only price point".into());
    }
    if price
        .plan
        .as_ref()
        .is_some_and(|plan| plan.trim().is_empty())
    {
        return Err("plan must be omitted instead of blank".into());
    }
    Ok(())
}

/// Burn on a date, computed from each subscription's series.
///
/// There is no stored total to return, by design: a cached figure is a second
/// source of truth that goes stale the moment a price point lands.
async fn burn(State(state): State<AppState>, Query(query): Query<AtQuery>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let at = query.at.unwrap_or_else(today);
    if !valid_iso_date(&at) {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "at must be a real date in YYYY-MM-DD form" }),
        );
    }
    let at_for_body = at.clone();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| Ok((store.list()?, store.latest_fx_rates()?)))
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((subs, rates))) => {
            let burn = burn_by_currency(&subs, &at);
            // PRD Q103 declares EUR, so the one figure to act on is `eur`. It contains
            // only amounts that are EUR or were converted with a published rate;
            // `eur.not_convertible` names what is missing from it, so a short total is
            // never a silent one. `currencies` stays as the unconverted breakdown.
            let eur = finance::money::burn_in_eur(&subs, &at, &rates);
            response(
                StatusCode::OK,
                json!({
                    "at": at_for_body,
                    "eur": eur,
                    "currencies": burn.currencies,
                    "billing_count": burn.billing_count,
                    "covered_count": burn.covered_count,
                    "unknown_price_count": burn.unknown_price_count,
                    "total_count": subs.len(),
                }),
            )
        }
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn append_price(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(price): Json<PricePoint>,
) -> ApiResponse {
    if let Err(error) = validate_price(&price) {
        return response(StatusCode::BAD_REQUEST, json!({ "error": error }));
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| {
                store
                    .append_price(&id, &price, &now)
                    .map(|created| (id.clone(), created))
            })
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((id, created))) => response(
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({ "ok": true, "id": id, "created": created }),
        ),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn append_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(change): Json<StateChange>,
) -> ApiResponse {
    if !valid_iso_date(&change.effective) {
        return response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "effective must be a real date in YYYY-MM-DD form" }),
        );
    }
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || {
        FinanceStore::open(&database_path)
            .and_then(|store| {
                store
                    .append_state(&id, &change, &now)
                    .map(|created| (id.clone(), created))
            })
            .map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((id, created))) => response(
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({ "ok": true, "id": id, "created": created }),
        ),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn scan_vault(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    match tokio::task::spawn_blocking(move || scan_notes(&vault)).await {
        Ok(Ok(notes)) => response(
            StatusCode::OK,
            json!({
                "count": notes.len(),
                "notes": notes.iter().map(|n| json!({
                    "name": n.name,
                    "source_path": n.source_path,
                })).collect::<Vec<_>>(),
            }),
        ),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

async fn import_vault(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let notes = scan_notes(&vault)?;
        let store = FinanceStore::open(&database_path).map_err(|e| e.to_string())?;
        let (mut created, mut existing) = (0usize, 0usize);
        for note in &notes {
            let (_, is_new) = store.import_note(note, &now).map_err(|e| e.to_string())?;
            if is_new {
                created += 1;
            } else {
                existing += 1;
            }
        }
        // PRD Q103: an already-imported note is never re-seeded, so a currency
        // corrected in the frontmatter cannot reach the series by itself. Report the
        // divergence rather than resolve it — a price point is appended by a person
        // who knows which side is right.
        let subs = store.list().map_err(|e| e.to_string())?;
        let mismatches = obsidian::currency_mismatches(&notes, &subs, &now);
        Ok(json!({
            "ok": mismatches.is_empty(),
            "created": created,
            "already_present": existing,
            "currency_mismatches": mismatches,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

/// Regenerate every note's derived block, and project the subscriptions that have no
/// note left to hold one.
///
/// A conflict is reported and counted, never resolved. The response names each
/// conflicting note so the operator can look at it, which is the entire difference
/// between this and a machine that overwrites what somebody wrote.
///
/// The second half exists because the first half had nothing to write to. Measured
/// 2026-08-28: all seven live subscriptions point at notes that left the vault on
/// 2026-08-23 (vault commit `ba60231`, PRD §5.5), so the price and state series — rows
/// PRD Q47 counts as irreplaceable — were in no file anywhere. Q31 rules that case
/// directly: a region when a human note exists, a projected file when none does. The
/// region path is unchanged and wins whenever a note is there.
async fn writeback(State(state): State<AppState>) -> ApiResponse {
    let Some(vault) = state.obsidian.clone() else {
        return no_vault();
    };
    let database_path = state.database_path.clone();
    let now = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let notes = scan_notes_or_empty(&vault)?;
        let store = FinanceStore::open(&database_path).map_err(|e| e.to_string())?;
        let subs = store.list().map_err(|e| e.to_string())?;
        // PRD Q103: the block states a foreign price in EUR when a published rate
        // covers the pair, and says it cannot when none does. An empty table is the
        // second case, not a reason to skip the line.
        let rates = store.latest_fx_rates().map_err(|e| e.to_string())?;

        let (mut written, mut unchanged) = (0usize, 0usize);
        let mut conflicts: Vec<String> = Vec::new();
        let mut unimported: Vec<String> = Vec::new();

        for note in &notes {
            let Some(sub) = subs.iter().find(|s| s.source_path == note.source_path) else {
                unimported.push(note.source_path.clone());
                continue;
            };
            match obsidian::write_block(&note.absolute, sub, &now, &rates)
                .map_err(|e| e.to_string())?
            {
                WriteBack::Created | WriteBack::Updated => written += 1,
                WriteBack::Unchanged => unchanged += 1,
                WriteBack::Conflict { .. } => conflicts.push(note.source_path.clone()),
            }
        }

        let root =
            markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;
        let owned: Vec<String> = notes.iter().map(|n| n.source_path.clone()).collect();
        let projected = obsidian::export_projections(&root, &subs, &owned, &now, &rates)
            .map_err(|e| e.to_string())?;
        let mismatches = obsidian::currency_mismatches(&notes, &subs, &now);

        Ok(json!({
            "ok": conflicts.is_empty() && projected.refused.is_empty() && mismatches.is_empty(),
            "written": written,
            "unchanged": unchanged,
            "conflicts": conflicts,
            "not_imported": unimported,
            "projected": projected,
            "currency_mismatches": mismatches,
        }))
    })
    .await
    {
        Ok(Ok(body)) => response(StatusCode::OK, body),
        Ok(Err(e)) => failed(e),
        Err(_) => failed("task panicked".into()),
    }
}

fn scan_notes(vault: &ObsidianConfig) -> Result<Vec<finance::ScannedNote>, String> {
    let root =
        markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;
    obsidian::scan(&root, &vault.subscriptions_dir).map_err(|e| e.to_string())
}

/// The scan, with a missing subscriptions directory reported as zero notes.
///
/// Only for the writeback path, and only for that one error: since 2026-08-23 the
/// configured directory is not in the vault at all, and failing there would hide the
/// projection pass, which is the half that does have somewhere to write.
///
/// Matched on the error's *type*, never on its text. `ScanError::Read` renders the same
/// "cannot read" sentence for a note that exists and cannot be opened — swallowing that
/// one would project a file for a subscription whose note is right there, which is the
/// duplicate the whole boundary exists to prevent.
fn scan_notes_or_empty(vault: &ObsidianConfig) -> Result<Vec<finance::ScannedNote>, String> {
    let root =
        markdown_root::MarkdownRoot::declare(vault.root.clone()).map_err(|e| e.to_string())?;
    match obsidian::scan(&root, &vault.subscriptions_dir) {
        Ok(notes) => Ok(notes),
        Err(finance::ScanError::Root(markdown_root::RootError::Unreadable { .. })) => {
            Ok(Vec::new())
        }
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Investments: the portfolio, the price series, and the decision ledger
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CurrencyQuery {
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerdictRequest {
    /// The id the client believed it was answering. It must agree with the path
    /// segment, and that is all this field can prove: both halves come from the
    /// same caller. The gate that catches numbers which have moved is the
    /// recompute inside `record_verdict`, which re-runs the rules the way
    /// `confirm_investments` re-runs `preview_csv` rather than trusting the token
    /// it was handed.
    expected_proposal_id: String,
    verdict: String,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Deserialize, Default)]
struct DecisionRunRequest {
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

fn no_holdings() -> ApiResponse {
    response(
        StatusCode::CONFLICT,
        json!({
            "ok": false,
            "capability": "finance",
            "error": "no reviewed holdings snapshot is configured; set investment_snapshot in the overlay's config/finance.json"
        }),
    )
}

impl AppState {
    /// The parts of `Config` the investment surfaces read, rebuilt from what the
    /// server already holds.
    ///
    /// `decision::recompute` and `price::run_named` both take a `&Config`, which
    /// is what keeps the CLI and the handler reading one assembly rather than two
    /// that drift. Rebuilding it here costs a few clones per request and removes
    /// a second definition of what the rules see.
    fn investment_config(&self) -> Config {
        Config {
            database_path: self.database_path.as_ref().clone(),
            port: 0,
            obsidian: self.obsidian.clone(),
            journal: self.journal.clone(),
            investment_snapshot: self.investment_snapshot.clone(),
            balance_snapshot: self.balance_snapshot.clone(),
            commitments: self.commitments.as_ref().clone(),
            csv_mappings: self.csv_mappings.as_ref().clone(),
            investment_csv_mappings: self.investment_csv_mappings.as_ref().clone(),
            planning: self.planning.as_ref().clone(),
            instruments: self.instruments.as_ref().clone(),
            targets: self.targets.as_deref().cloned(),
            decisions_root: self.overlay_root.as_deref().cloned(),
            comms_base_url: self.comms_base_url.as_ref().clone(),
        }
    }
}

/// What `GET /api/portfolio` reads. Two fields, because that is what the report
/// takes: the reviewed snapshot and the newest observation per instrument.
struct InvestmentReads {
    snapshot: finance::investment::ReviewedHoldingsSnapshot,
    latest_prices: Vec<finance::price::PriceObservation>,
}

fn read_investments(database_path: &std::path::Path) -> Result<Option<InvestmentReads>, String> {
    let store = FinanceStore::open(database_path).map_err(|error| error.to_string())?;
    let Some(snapshot) = store.holding_projection().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(Some(InvestmentReads {
        snapshot,
        latest_prices: store.latest_prices().map_err(|e| e.to_string())?,
    }))
}

fn portfolio_report(
    reads: &InvestmentReads,
    instruments: &[InstrumentProfile],
    targets: Option<&TargetPolicy>,
    as_of: &str,
    currency: &str,
) -> Result<finance::portfolio::PortfolioReport, String> {
    finance::portfolio::report(finance::portfolio::PortfolioInputs {
        snapshot: &reads.snapshot,
        latest_prices: &reads.latest_prices,
        instruments,
        targets,
        as_of,
        currency,
    })
}

async fn portfolio(
    State(state): State<AppState>,
    Query(query): Query<CurrencyQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let instruments = state.instruments.clone();
    let targets = state.targets.clone();
    let currency = query.currency.unwrap_or_else(|| "EUR".into());
    let as_of = today();
    match tokio::task::spawn_blocking(move || -> Result<Option<Value>, String> {
        let Some(reads) = read_investments(&database_path)? else {
            return Ok(None);
        };
        let report = portfolio_report(&reads, &instruments, targets.as_deref(), &as_of, &currency)?;
        serde_json::to_value(report)
            .map(Some)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(Some(value))) => response(StatusCode::OK, value),
        Ok(Ok(None)) => no_holdings(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

async fn prices_status(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    let targets = state.targets.clone();
    let as_of = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let freshness_days = targets
            .as_deref()
            .map(|targets| targets.price_freshness_days)
            .unwrap_or(4);
        let latest = store.latest_prices().map_err(|e| e.to_string())?;
        let counts: std::collections::BTreeMap<String, i64> = store
            .price_counts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        let instruments: Vec<Value> = latest
            .iter()
            .map(|observation| {
                let age = finance::clock::days_between(&observation.observed_on, &as_of);
                json!({
                    "instrument": observation.instrument,
                    "latest_observed_on": observation.observed_on,
                    "source": observation.source,
                    "age_days": age,
                    "freshness": match age {
                        Some(age) if age <= freshness_days => "fresh",
                        Some(_) => "stale",
                        None => "unknown",
                    },
                    "observations": counts.get(&observation.instrument).copied().unwrap_or(0),
                })
            })
            .collect();
        let fetches = store.recent_fetches(50).map_err(|e| e.to_string())?;
        // The newest attempt per provider, so "is this source working" is one
        // read rather than a scan of the list below it.
        let mut providers: Vec<Value> = Vec::new();
        for name in finance::price::PROVIDERS {
            let last = fetches.iter().find(|attempt| attempt.provider == *name);
            providers.push(json!({
                "name": name,
                "last_status": last.map(|attempt| attempt.status.as_str()),
                "last_detail": last.map(|attempt| attempt.detail.clone()),
                "last_fetched_at": last.map(|attempt| attempt.fetched_at.clone()),
            }));
        }
        let rates = store.latest_fx_rates().map_err(|e| e.to_string())?;
        Ok(json!({
            "as_of": as_of,
            "freshness_days": freshness_days,
            "instruments": instruments,
            "recent_fetches": fetches,
            "providers": providers,
            "fx": rates,
        }))
    })
    .await
    {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

/// `GET /__axon/freshness` — when this capability last took delivery of data.
///
/// The shared shape behind `freshness_advise_hours` / `freshness_stale_hours` in
/// `capabilities/finance/service.toml`, and the same one
/// `capabilities/comms/src/server/source_handlers.rs` serves: one integer,
/// `last_arrival_at`, epoch seconds, or `null` if nothing has ever arrived.
/// `tools/doctor` compares it against the two declared hours; anything else this
/// endpoint might usefully say belongs to `GET /api/prices/status`, which is the
/// surface a person reads.
///
/// ARRIVAL, not the newest observation date. A market that was shut on Friday is
/// quiet, not broken, and a contract keyed on `observed_on` would call a long
/// weekend a fault and teach the reader to skim past it. What this exists to
/// catch is a producer that stopped — `capabilities/finance-prices`, whose 24h
/// schedule is the only unattended caller of `finance-cli prices fetch`.
///
/// A parse failure is a 500 and never a `null`. `null` is the specific claim
/// "nothing has ever arrived", which doctor reports as a fault, and a stamp this
/// process could not read is a different thing entirely — one is the machine's
/// state, the other is this code's bug.
async fn freshness(State(state): State<AppState>) -> ApiResponse {
    let database_path = state.database_path.clone();
    match tokio::task::spawn_blocking(move || -> Result<Option<i64>, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let Some(stamp) = store
            .newest_price_arrival()
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        finance::clock::epoch_seconds(&stamp)
            .map(Some)
            .ok_or_else(|| format!("stored fetch stamp is not a timestamp: {stamp}"))
    })
    .await
    {
        Ok(Ok(last)) => response(StatusCode::OK, json!({ "last_arrival_at": last })),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

async fn list_decisions(
    State(state): State<AppState>,
    Query(query): Query<StatusQuery>,
) -> ApiResponse {
    let database_path = state.database_path.clone();
    let status = query.status.unwrap_or_else(|| "open".into());
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let stored = store
            .decisions(Some(&status))
            .map_err(|error| error.to_string())?;
        let views: Vec<_> = stored.iter().map(finance::decision::view).collect();
        serde_json::to_value(views).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

/// The one thing here nothing can re-derive.
///
/// The gate copies `confirm_investments` (:377-395), which re-runs `preview_csv`
/// rather than trusting the token it was handed: the rules are run again here and
/// the verdict is refused with a 409 unless the run STILL mints the id being
/// answered. Comparing the request body's `expected_proposal_id` to the path
/// segment is not that gate -- both come from the same client, so it can only
/// catch a malformed request, and it is answered as a 400 for what it is.
///
/// The recompute is handed an empty feed list on purpose: `decision::proposal_id`
/// hashes the kind, the subject and the bucketed numbers and never the evidence
/// (decision.rs:589), so the gate needs no loopback call to comms and cannot
/// refuse a verdict because an unrelated capability is down.
///
/// A rejection needs a note, mirroring the required reason on an append-only
/// price point -- a rejection with no reason is a decision nobody can read a year
/// later.
///
/// Accepting appends one row and writes the month file. It never writes the
/// journal, never writes the holdings snapshot and never places an order.
async fn record_verdict(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<VerdictRequest>,
) -> ApiResponse {
    if request.expected_proposal_id != id {
        return response(
            StatusCode::BAD_REQUEST,
            json!({
                "ok": false,
                "error": "expected_proposal_id must be the id in the path",
            }),
        );
    }
    if !matches!(request.verdict.as_str(), "accepted" | "rejected") {
        return response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": "verdict must be accepted or rejected" }),
        );
    }
    if request.verdict == "rejected" && request.note.trim().is_empty() {
        return response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": "a note is required for a rejected proposal" }),
        );
    }
    let config = state.investment_config();
    let overlay_root = state.overlay_root.clone();
    let recorded_at = now_timestamp();
    let as_of = today();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&config.database_path).map_err(|error| error.to_string())?;
        let Some(stored) = store.decision(&id).map_err(|error| error.to_string())? else {
            return Err("__not_found__".into());
        };
        if stored.status() != "open" {
            return Err(format!("__closed__{}", stored.status()));
        }
        // Re-derive before recording. The numbers behind a proposal move on every
        // new price observation, and a verdict recorded against numbers that have
        // moved is the one failure this ledger cannot repair afterwards.
        let currency =
            serde_json::from_str::<finance::decision::Proposal>(&stored.proposal.proposal_json)
                .map(|proposal| proposal.currency)
                .unwrap_or_else(|_| DEFAULT_CURRENCY.to_string());
        let (minted, _) =
            finance::decision::recompute(&store, &config, &[], &as_of, &recorded_at, &currency)?;
        if !minted.iter().any(|candidate| candidate.id == id) {
            let current = minted
                .iter()
                .find(|candidate| {
                    candidate.proposal.kind == stored.proposal.kind
                        && candidate.proposal.subject == stored.proposal.subject
                })
                .map(|candidate| candidate.id.clone());
            return Err(format!("__moved__{}", current.unwrap_or_default()));
        }
        let event = finance::store::StoredDecisionEvent {
            event: "verdict".into(),
            verdict: Some(request.verdict.clone()),
            note: request.note.clone(),
            outcome_json: None,
            recorded_at: recorded_at.clone(),
        };
        if !store
            .append_decision_event(&id, &event)
            .map_err(|error| error.to_string())?
        {
            return Err("__collision__".into());
        }
        // The event row is committed by here, so an export failure can no longer
        // be answered as a failed request: a 500 would tell the reader their
        // verdict did not land while the ledger already holds it, and their retry
        // would be answered "this proposal is already rejected". The durable fact
        // is the row; the month file is the copy, and a failure to write the copy
        // is named in the response rather than swallowed or promoted to an error.
        let mut warning: Option<String> = None;
        let exported = match overlay_root.as_deref() {
            Some(root) => {
                let month = finance::clock::month_of(&stored.proposal.proposed_at[..10])
                    .unwrap_or_else(|| recorded_at[..7].to_string());
                match finance::decision::export_month(&store, root, &month) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        warning = Some(format!(
                            "the verdict is recorded; the month file could not be written: {error}"
                        ));
                        None
                    }
                }
            }
            None => None,
        };
        Ok(json!({
            "ok": true,
            "id": id,
            "status": request.verdict,
            "recorded_at": recorded_at,
            "exported_to": exported,
            "warning": warning,
        }))
    })
    .await
    {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) if error == "__not_found__" => response(
            StatusCode::NOT_FOUND,
            json!({ "ok": false, "error": "no proposal with that id" }),
        ),
        // The gate cannot run without the snapshot the rules read, and a verdict
        // recorded with the gate skipped is the failure the gate exists for.
        Ok(Err(error)) if error.starts_with("no reviewed holdings snapshot") => no_holdings(),
        Ok(Err(error)) if error == "__collision__" => response(
            StatusCode::CONFLICT,
            json!({ "ok": false, "error": "a verdict for this proposal was just recorded" }),
        ),
        Ok(Err(error)) if error.starts_with("__moved__") => {
            let current = &error["__moved__".len()..];
            response(
                StatusCode::CONFLICT,
                json!({
                    "ok": false,
                    "error": "the numbers moved since this was proposed",
                    // Null when the rules no longer produce this subject at all,
                    // which is a different fact from "here is the new id" and is
                    // not worth an invented id to hide.
                    "current_proposal_id": (!current.is_empty()).then(|| current.to_string()),
                }),
            )
        }
        Ok(Err(error)) if error.starts_with("__closed__") => response(
            StatusCode::CONFLICT,
            json!({
                "ok": false,
                "error": format!("this proposal is already {}", &error["__closed__".len()..]),
            }),
        ),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

async fn run_decisions(
    State(state): State<AppState>,
    body: Option<Json<DecisionRunRequest>>,
) -> ApiResponse {
    let request = body.map(|Json(request)| request).unwrap_or_default();
    let config = state.investment_config();
    let comms_base_url = state.comms_base_url.clone();
    let overlay_root = state.overlay_root.clone();
    let currency = request.currency.unwrap_or_else(|| "EUR".into());
    let dry_run = request.dry_run;
    let as_of = today();
    let proposed_at = now_timestamp();
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&config.database_path).map_err(|error| error.to_string())?;
        // Built inside this closure, never on the async runtime: a blocking
        // reqwest client driven from a Tokio worker panics at run time.
        let feed_items = read_feed_items(&comms_base_url);
        let (minted, caveats) = finance::decision::recompute(
            &store,
            &config,
            &feed_items,
            &as_of,
            &proposed_at,
            &currency,
        )?;
        if dry_run {
            // Classified against the ledger, read-only, exactly as
            // `reconcile_decisions` classifies it while writing. Reporting
            // `proposed: minted.len()` made a dry run and the wet run that
            // follows it answer different numbers for one unchanged state, which
            // is the one thing a preview must never do.
            //
            // `status()` is what keeps the two in step, and it now resolves the
            // supersede/reinstate pair by recency rather than by presence -- so a
            // proposal that has already been reinstated reads as open here and is
            // counted `unchanged`, which is what the wet run will do with it.
            let ledger = store.decisions(None).map_err(|error| error.to_string())?;
            let mut counts = finance::store::DecisionRunOutcome::default();
            for proposal in &minted {
                match ledger
                    .iter()
                    .find(|stored| stored.proposal.id == proposal.id)
                {
                    None => counts.proposed += 1,
                    Some(stored) if stored.status() == "superseded" => counts.reopened += 1,
                    Some(_) => counts.unchanged += 1,
                }
            }
            counts.superseded = ledger
                .iter()
                .filter(|stored| stored.status() == "open")
                .filter(|stored| {
                    !minted
                        .iter()
                        .any(|proposal| proposal.id == stored.proposal.id)
                })
                .count();
            return Ok(json!({
                "ok": true,
                "dry_run": true,
                "proposed": counts.proposed,
                "unchanged": counts.unchanged,
                "reopened": counts.reopened,
                "superseded": counts.superseded,
                "proposals": minted.iter().map(|minted| &minted.proposal).collect::<Vec<_>>(),
                "caveats": caveats,
            }));
        }
        let outcome = finance::decision::run(
            &store,
            &minted,
            &proposed_at,
            overlay_root.as_deref().map(|root| root.as_path()),
        )?;
        Ok(json!({
            "ok": true,
            "dry_run": false,
            "proposed": outcome.proposed,
            "unchanged": outcome.unchanged,
            "reopened": outcome.reopened,
            "superseded": outcome.superseded,
            "caveats": caveats,
        }))
    })
    .await
    {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) if error.starts_with("no reviewed holdings snapshot") => no_holdings(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

/// Read the feed over loopback HTTP, and answer a failure with an empty list.
///
/// Evidence is optional by design: a proposal with no feed items is a proposal
/// with no reading attached, while a proposal that never got made because comms
/// was down is a decision lost to an unrelated service.
///
/// Only `id`, `title`, `url` and `day` are read, and only from an item comms
/// classifies at or below `c1`. Four fields is a bound on VOLUME; the bound the
/// doctrine asks for is on CLASS, and it is not this call site's to choose:
/// `comms_feed_items.data_class` admits all four classes and comms serves
/// `POST /feed/{id}/data-class` so a human can raise one (capabilities/comms/src/
/// server/main.rs:324). A title raised to c2 because it names a person, copied
/// into a `c1` decision row, is a de-escalation no human asked for --
/// `content_item::admit_reclassification` (libs/content-item/src/lib.rs:393)
/// admits that only from a human with a rationale.
///
/// It fails CLOSED: an item whose class comms does not state is dropped, not
/// assumed public. `GET /feed` does not carry `data_class` today, so this
/// currently drops every item and proposals are minted with no feed evidence,
/// which the design already names as an acceptable state ("a proposal with no
/// feed items is a proposal with no reading attached"). The one-line comms change
/// that restores the evidence is recorded for the merge reviewer; guessing a
/// class here is the alternative, and a guessed class is the defect.
fn read_feed_items(base_url: &str) -> Vec<finance::decision::FeedEvidence> {
    if base_url.is_empty() {
        return Vec::new();
    }
    let Ok(client) = finance::price::blocking_client() else {
        return Vec::new();
    };
    let mut request = client.get(format!("{}/feed", base_url.trim_end_matches('/')));
    if let Some(header) = sjel_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header("Authorization", header);
    }
    let Ok(body) = request.send().and_then(reqwest::blocking::Response::text) else {
        return Vec::new();
    };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&body) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item_is_quotable(item))
        .filter_map(|item| {
            Some(finance::decision::FeedEvidence {
                id: item.get("id")?.as_str()?.to_string(),
                title: item.get("title")?.as_str()?.to_string(),
                url: item.get("url")?.as_str()?.to_string(),
                day: item.get("day")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// May this item's title and URL be copied into a `c1` row?
///
/// Only when comms states a class and that class is no stricter than the row's
/// own. `class_rank` is the single home of the ordering
/// (libs/content-item/src/lib.rs:350); comparing the strings here would be a
/// second copy of the vocabulary.
fn item_is_quotable(item: &Value) -> bool {
    let Some(stated) = item.get("data_class").and_then(Value::as_str) else {
        return false;
    };
    let (Some(stated), Some(row)) = (
        content_item::class_rank(stated),
        content_item::class_rank(finance::decision::DATA_CLASS),
    ) else {
        return false;
    };
    stated <= row
}

/// The per-trip slice, so a caller does not download the whole projection to read
/// one object.
///
/// No start/end query, deliberately: a caller passing a range would silently get
/// a partial trip total, and a partial total that looks whole is worse than no
/// route. A trip with no allocated rows answers 200 with zeros, not 404 -- an
/// empty state is the caller's to render.
async fn trip_spending(State(state): State<AppState>, Path(id): Path<String>) -> ApiResponse {
    let database_path = state.database_path.clone();
    // The configured reporting currency, never a literal: the response labels the
    // four figures with the currency they were filtered on, and a caller that
    // relabels them with a plan's own currency would print a wrong unit the first
    // time a plan is not in this one.
    let currency = state
        .targets
        .as_deref()
        .map(|targets| targets.currency.clone())
        .unwrap_or_else(|| DEFAULT_CURRENCY.to_string());
    match tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let store = FinanceStore::open(&database_path).map_err(|error| error.to_string())?;
        let rows = store
            .transaction_projection()
            .map_err(|error| error.to_string())?;
        let filter = AnalyticsFilter {
            currency: Some(currency.clone()),
            ..AnalyticsFilter::default()
        };
        let summaries = analytics::trip_spending(&rows, &filter);
        let summary = summaries
            .into_iter()
            .find(|summary| summary.trip_id == id)
            .unwrap_or(analytics::TripSpendingSummary {
                trip_id: id.clone(),
                personal_spending_cents: 0,
                gross_cash_outflow_cents: 0,
                reimbursed_cents: 0,
                outstanding_cents: 0,
                expense_posting_count: 0,
            });
        let mut value = serde_json::to_value(&summary).map_err(|error| error.to_string())?;
        if let Some(object) = value.as_object_mut() {
            object.insert("currency".into(), json!(currency));
        }
        Ok(value)
    })
    .await
    {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error.to_string()),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::load();
    let port = config.port;
    sjel_server::serve_local("finance", port, build_router(state_from(config))).await;
}

/// This capability's name, for the origin guard's env var
/// (`SJEL_FINANCE_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "finance";

fn state_from(config: Config) -> AppState {
    AppState {
        database_path: Arc::new(config.database_path),
        obsidian: config.obsidian,
        journal: config.journal,
        commitments: Arc::new(config.commitments),
        planning: Arc::new(config.planning),
        csv_mappings: Arc::new(config.csv_mappings),
        investment_csv_mappings: Arc::new(config.investment_csv_mappings),
        investment_snapshot: config.investment_snapshot,
        balance_snapshot: config.balance_snapshot,
        journal_write: Arc::new(std::sync::Mutex::new(())),
        projection_write: Arc::new(std::sync::Mutex::new(())),
        balance_write: Arc::new(std::sync::Mutex::new(())),
        instruments: Arc::new(config.instruments),
        targets: config.targets.map(Arc::new),
        comms_base_url: Arc::new(config.comms_base_url),
        overlay_root: config.decisions_root.map(Arc::new),
    }
}

/// The wired router, so a test can drive the real thing rather than a handler.
///
/// `GET /api/dashboard` and `GET /api/portfolio` are the operator's money —
/// balances, holdings, every transaction the ledger carries — and
/// `CorsLayer::permissive()` made them readable by any page open in their
/// browser. Four writes here take no request body (`/api/ledger/rebuild`,
/// `/api/import/obsidian`, `/api/writeback`, and `/api/decisions/run`, whose
/// `Option<Json<_>>` accepts a missing one), so a cross-site *simple* POST
/// reaches them with no preflight for CORS to refuse. Refusing the request,
/// which is what this guard does, is what closes that.
///
/// The origin guard sits below every route on purpose: axum wraps only the
/// routes registered BEFORE a `.layer()` call (axum 0.7
/// `src/docs/routing/layer.md`), so a route appended under it would silently
/// lose the refusal.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/subscriptions", get(list_subscriptions))
        .route("/api/subscriptions/burn", get(burn))
        .route("/api/subscriptions/{id}/price", post(append_price))
        .route("/api/subscriptions/{id}/state", post(append_state))
        .route("/api/import/obsidian/scan", get(scan_vault))
        .route("/api/import/obsidian", post(import_vault))
        .route("/api/writeback", post(writeback))
        .route("/api/import/csv/preview", post(preview_csv))
        .route("/api/import/csv", post(import_csv))
        .route("/api/import/csv/mappings", get(list_csv_mappings))
        .route(
            "/api/import/investments/mappings",
            get(list_investment_mappings),
        )
        .route("/api/import/investments/preview", post(preview_investments))
        .route("/api/import/investments/confirm", post(confirm_investments))
        .route("/api/import/candidates", get(list_candidates))
        .route(
            "/api/import/candidates/confirm-batch",
            post(confirm_candidates_batch),
        )
        .route(
            "/api/import/candidates/reclassify-batch",
            post(reclassify_candidates_batch),
        )
        .route("/api/import/candidates/{id}/review", post(review_candidate))
        .route(
            "/api/import/candidates/{id}/reconcile-transfer",
            post(reconcile_transfer),
        )
        .route(
            "/api/import/candidates/{id}/allocation",
            post(allocate_expense),
        )
        .route(
            "/api/import/candidates/{id}/reimbursement",
            post(link_reimbursement),
        )
        .route("/api/balance-snapshot", post(update_balance_snapshot))
        .route("/api/ledger/check", get(check_ledger))
        .route("/api/ledger/rebuild", post(rebuild_ledger))
        .route("/api/dashboard", get(dashboard_projection))
        .route("/api/portfolio", get(portfolio))
        .route("/api/prices/status", get(prices_status))
        .route("/__axon/freshness", get(freshness))
        .route("/api/decisions", get(list_decisions))
        .route("/api/decisions/run", post(run_decisions))
        .route("/api/decisions/{id}/verdict", post(record_verdict))
        .route("/api/trips/{id}/spending", get(trip_spending))
        // ADD NEW ROUTES ABOVE THIS LINE. Below it they lose the origin guard.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance::analytics::{TransactionKind, TransactionRow};
    use finance::import::{AmountSign, CsvDateFormat, CsvRowPolicy};
    use finance::investment::{ReviewedHoldingsSnapshot, ReviewedHoldingsSource};
    use finance::planning::ExpectedCoverage;

    fn synthetic_csv_mapping() -> CsvMapping {
        CsvMapping {
            delimiter: ';',
            decimal_separator: ',',
            date_column: "Date".into(),
            amount_column: "Amount".into(),
            description_column: "Description".into(),
            categorization_columns: Vec::new(),
            reference_column: Some("Reference".into()),
            currency_column: Some("Currency".into()),
            default_currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            default_outflow_account: "expenses:uncategorized".into(),
            default_inflow_account: "income:uncategorized".into(),
            categorization_rules: Vec::new(),
            row_filter: None,
            amount_sign: AmountSign::AsProvided,
            amount_rounding: finance::import::AmountRounding::Reject,
            date_formats: vec![
                CsvDateFormat::IsoYearMonthDay,
                CsvDateFormat::DayMonthYearDots,
            ],
            row_policy: CsvRowPolicy::Strict,
            location_columns: None,
        }
    }

    #[test]
    fn source_freshness_distinguishes_current_stale_and_missing() {
        assert_eq!(source_age_days(Some("2026-08-01"), "2026-08-11"), Some(10));
        assert_eq!(
            source_freshness_status(Some("2026-08-01"), "2026-08-11", 14),
            "current"
        );
        assert_eq!(
            source_freshness_status(Some("2026-07-01"), "2026-08-11", 14),
            "stale"
        );
        assert_eq!(source_freshness_status(None, "2026-08-11", 14), "missing");
    }

    #[test]
    fn configured_sources_are_checked_independently() {
        let rows = vec![TransactionRow {
            id: "synthetic".into(),
            date: "2026-08-01".into(),
            description: "Synthetic".into(),
            kind: TransactionKind::Expense,
            account: "liabilities:card:synthetic".into(),
            category: "expenses:food".into(),
            amount_cents: 100,
            currency: "EUR".into(),
            source_id: None,
            purpose: None,
            trip_id: None,
            cash_amount_cents: 100,
            shared_cents: 0,
            reimbursement_for: None,
        }];
        let investment = ReviewedHoldingsSnapshot {
            schema_version: 2,
            snapshot_id: "synthetic".into(),
            reviewed_at: "2026-07-01".into(),
            coverage: HoldingsCoverage::Partial,
            holdings: Vec::new(),
            sources: vec![ReviewedHoldingsSource {
                source_key: "synthetic-broker".into(),
                snapshot_id: "synthetic-source".into(),
                reviewed_at: "2026-07-01".into(),
                coverage: HoldingsCoverage::Partial,
            }],
        };
        let config = PlanningConfig {
            source_expectations: vec![
                SourceExpectation::Transactions {
                    id: "card".into(),
                    label: "Synthetic card".into(),
                    account_prefixes: vec!["liabilities:card:synthetic".into()],
                    freshness_days: Some(14),
                    coverage: ExpectedCoverage::Complete,
                },
                SourceExpectation::Holdings {
                    id: "broker".into(),
                    label: "Synthetic broker".into(),
                    source_key: "synthetic-broker".into(),
                    freshness_days: Some(14),
                    coverage: ExpectedCoverage::Complete,
                },
                SourceExpectation::Transactions {
                    id: "missing".into(),
                    label: "Missing source".into(),
                    account_prefixes: vec!["assets:missing".into()],
                    freshness_days: None,
                    coverage: ExpectedCoverage::Partial,
                },
            ],
            ..PlanningConfig::default()
        };

        let sources = expected_source_freshness(&rows, Some(&investment), &config, "2026-08-11");
        assert_eq!(sources[0].freshness, "current");
        assert_eq!(sources[0].coverage, "complete");
        assert_eq!(sources[1].freshness, "stale");
        assert_eq!(sources[1].coverage, "partial");
        assert_eq!(sources[2].freshness, "missing");
        assert_eq!(sources[2].coverage, "missing");
    }

    #[test]
    fn a_price_append_needs_an_auditable_reason() {
        let price = PricePoint {
            valid_from: "2026-10-01".into(),
            amount_cents: 10_000,
            currency: "EUR".into(),
            cycle: finance::BillingCycle::Monthly,
            plan: Some("Max".into()),
            reason: String::new(),
        };
        assert_eq!(
            validate_price(&price),
            Err("reason is required for an append-only price point".into())
        );
    }

    /// The manifest is data a caller reads to learn the surface; a served route
    /// missing from it is invisible.
    ///
    /// This replaced a hand-typed array of 27 paths. That array could only assert
    /// that each path it listed was in ROUTES -- it could not see a route the
    /// router serves that nobody declared, which is the failure it existed to
    /// catch. `undeclared_routes` reads this file's own source, so adding a
    /// `.route()` without a manifest entry fails here (ISA PLC-1). If it fails,
    /// declare the route; do not weaken the assertion.
    #[test]
    fn every_route_the_router_serves_is_declared() {
        assert!(
            route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES).is_empty(),
            "served but undeclared: {:?}",
            route_manifest::undeclared_routes(include_str!("server.rs"), ROUTES)
        );
    }

    #[test]
    fn batch_reclassification_accepts_only_reviewed_expense_targets() {
        assert!(is_reviewed_expense_account("expenses:food:groceries"));
        assert!(!is_reviewed_expense_account("expenses:uncategorized"));
        assert!(!is_reviewed_expense_account("expenses:food:uncategorized"));
        assert!(!is_reviewed_expense_account("income:salary"));
        assert!(!is_reviewed_expense_account("assets:bank:checking"));
    }

    /// A state with nothing configured, pointed at one scratch database. Every
    /// field the verdict route reads is here; the rest are the empty defaults the
    /// other handler tests already use.
    fn state_on(database: std::path::PathBuf) -> AppState {
        AppState {
            database_path: Arc::new(database),
            obsidian: None,
            journal: None,
            commitments: Arc::new(Vec::new()),
            planning: Arc::new(PlanningConfig::default()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            balance_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
            balance_write: Arc::new(std::sync::Mutex::new(())),
            instruments: Arc::new(Vec::new()),
            targets: None,
            comms_base_url: Arc::new(String::new()),
            overlay_root: None,
        }
    }

    fn scratch_database(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("finance-server-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("{name}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        path
    }

    /// The three refusals that happen before anything is written, and the one
    /// that is NOT a 409: a body whose `expected_proposal_id` disagrees with the
    /// path is a malformed request from one client, not evidence that the numbers
    /// moved. Both fields come from the same caller, so the comparison can only
    /// ever catch the caller contradicting itself -- the gate that catches moved
    /// numbers is the recompute inside the closure.
    #[tokio::test]
    async fn a_verdict_is_refused_before_it_is_recorded() {
        let database = scratch_database("verdict-gate");
        let mismatched = record_verdict(
            State(state_on(database.clone())),
            Path("rebalance:instrument:SYN-A:0011".into()),
            Json(VerdictRequest {
                expected_proposal_id: "rebalance:instrument:SYN-A:0022".into(),
                verdict: "accepted".into(),
                note: String::new(),
            }),
        )
        .await;
        assert_eq!(mismatched.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            mismatched.1 .0["error"],
            "expected_proposal_id must be the id in the path"
        );

        let unnamed = record_verdict(
            State(state_on(database.clone())),
            Path("rebalance:instrument:SYN-A:0011".into()),
            Json(VerdictRequest {
                expected_proposal_id: "rebalance:instrument:SYN-A:0011".into(),
                verdict: "maybe".into(),
                note: "synthetic".into(),
            }),
        )
        .await;
        assert_eq!(unnamed.0, StatusCode::UNPROCESSABLE_ENTITY);

        let unexplained = record_verdict(
            State(state_on(database.clone())),
            Path("rebalance:instrument:SYN-A:0011".into()),
            Json(VerdictRequest {
                expected_proposal_id: "rebalance:instrument:SYN-A:0011".into(),
                verdict: "rejected".into(),
                note: "   ".into(),
            }),
        )
        .await;
        assert_eq!(unexplained.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            unexplained.1 .0["error"],
            "a note is required for a rejected proposal"
        );

        // And an id the ledger does not carry is a 404 rather than a recorded
        // verdict against nothing.
        let unknown = record_verdict(
            State(state_on(database)),
            Path("rebalance:instrument:SYN-A:0011".into()),
            Json(VerdictRequest {
                expected_proposal_id: "rebalance:instrument:SYN-A:0011".into(),
                verdict: "accepted".into(),
                note: String::new(),
            }),
        )
        .await;
        assert_eq!(unknown.0, StatusCode::NOT_FOUND);
        assert_eq!(unknown.1 .0["error"], "no proposal with that id");
    }

    /// The class bound on feed evidence is on the CLASS, not on the field count.
    /// An item comms does not classify is dropped rather than assumed public, and
    /// an item comms classifies above the row's own `c1` is dropped whatever its
    /// four fields say.
    #[test]
    fn only_an_item_comms_classifies_at_or_below_the_row_is_quotable() {
        let at_class = json!({ "id": "a", "title": "t", "url": "u", "day": "2026-09-05",
                               "data_class": "c1" });
        let public = json!({ "id": "b", "data_class": "c0" });
        let stricter = json!({ "id": "c", "data_class": "c2" });
        let unstated = json!({ "id": "d", "title": "t", "url": "u", "day": "2026-09-05" });
        let unknown = json!({ "id": "e", "data_class": "confidential" });
        assert!(item_is_quotable(&at_class));
        assert!(item_is_quotable(&public));
        assert!(!item_is_quotable(&stricter));
        assert!(!item_is_quotable(&unstated));
        assert!(!item_is_quotable(&unknown));
    }

    #[tokio::test]
    async fn transaction_csv_preview_is_summary_only_and_staging_requires_its_identity() {
        let content = "Date;Amount;Description;Reference;Currency\n2026-08-09;-12,34;Synthetic service;one;EUR\n";
        let (status, Json(body)) = preview_csv(Json(CsvPreviewRequest {
            content: content.into(),
            mapping: synthetic_csv_mapping(),
        }))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["candidate_count"], 1);
        assert_eq!(body["preserved_repetitions"], 0);
        assert!(body.get("candidates").is_none());
        assert!(body.get("description").is_none());

        let state = AppState {
            database_path: Arc::new(PathBuf::new()),
            obsidian: None,
            journal: None,
            commitments: Arc::new(Vec::new()),
            planning: Arc::new(PlanningConfig::default()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            balance_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
            balance_write: Arc::new(std::sync::Mutex::new(())),
            instruments: Arc::new(Vec::new()),
            targets: None,
            comms_base_url: Arc::new(String::new()),
            overlay_root: None,
        };
        let (status, Json(body)) = import_csv(
            State(state),
            Json(CsvImportRequest {
                content: content.into(),
                mapping: synthetic_csv_mapping(),
                expected_preview_id: "changed-preview".into(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "CSV preview changed before staging");
    }

    #[tokio::test]
    async fn csv_mapping_endpoint_returns_private_profiles_without_mutating_them() {
        let profile = CsvMappingProfile {
            label: "Synthetic semicolon export".into(),
            mapping: synthetic_csv_mapping(),
        };
        let state = AppState {
            database_path: Arc::new(PathBuf::new()),
            obsidian: None,
            journal: None,
            commitments: Arc::new(Vec::new()),
            planning: Arc::new(PlanningConfig::default()),
            csv_mappings: Arc::new(vec![profile.clone()]),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            balance_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
            balance_write: Arc::new(std::sync::Mutex::new(())),
            instruments: Arc::new(Vec::new()),
            targets: None,
            comms_base_url: Arc::new(String::new()),
            overlay_root: None,
        };

        let Json(returned) = list_csv_mappings(State(state)).await;
        assert_eq!(returned, vec![profile]);
    }

    #[tokio::test]
    async fn investment_mapping_endpoint_returns_private_profiles_without_mutating_them() {
        let profile = InvestmentCsvMappingProfile {
            source_key: "synthetic-broker".into(),
            label: "Synthetic activity export".into(),
            coverage: HoldingsCoverage::Partial,
            mapping: InvestmentCsvMapping {
                delimiter: ';',
                decimal_separator: ',',
                date_column: "Date".into(),
                instrument_column: "Instrument".into(),
                quantity_column: "Quantity".into(),
                activity_type_column: None,
                position_activity_values: Vec::new(),
                non_position_activity_values: Vec::new(),
                reference_column: Some("Reference".into()),
                price_column: Some("Price".into()),
                currency_column: Some("Currency".into()),
                default_currency: "EUR".into(),
                instrument_aliases: Default::default(),
            },
        };
        let state = AppState {
            database_path: Arc::new(PathBuf::new()),
            obsidian: None,
            journal: None,
            commitments: Arc::new(Vec::new()),
            planning: Arc::new(PlanningConfig::default()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(vec![profile.clone()]),
            investment_snapshot: None,
            balance_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
            balance_write: Arc::new(std::sync::Mutex::new(())),
            instruments: Arc::new(Vec::new()),
            targets: None,
            comms_base_url: Arc::new(String::new()),
            overlay_root: None,
        };

        let Json(returned) = list_investment_mappings(State(state)).await;
        assert_eq!(returned, vec![profile]);
    }

    #[tokio::test]
    async fn investment_confirmation_requires_a_private_snapshot_path() {
        let state = AppState {
            database_path: Arc::new(PathBuf::new()),
            obsidian: None,
            journal: None,
            commitments: Arc::new(Vec::new()),
            planning: Arc::new(PlanningConfig::default()),
            csv_mappings: Arc::new(Vec::new()),
            investment_csv_mappings: Arc::new(Vec::new()),
            investment_snapshot: None,
            balance_snapshot: None,
            journal_write: Arc::new(std::sync::Mutex::new(())),
            projection_write: Arc::new(std::sync::Mutex::new(())),
            balance_write: Arc::new(std::sync::Mutex::new(())),
            instruments: Arc::new(Vec::new()),
            targets: None,
            comms_base_url: Arc::new(String::new()),
            overlay_root: None,
        };
        let request = InvestmentConfirmRequest {
            content: String::new(),
            mapping: InvestmentCsvMapping {
                delimiter: ';',
                decimal_separator: ',',
                date_column: "Date".into(),
                instrument_column: "Instrument".into(),
                quantity_column: "Quantity".into(),
                activity_type_column: None,
                position_activity_values: Vec::new(),
                non_position_activity_values: Vec::new(),
                reference_column: None,
                price_column: None,
                currency_column: None,
                default_currency: "EUR".into(),
                instrument_aliases: Default::default(),
            },
            source_key: "synthetic-broker".into(),
            expected_snapshot_id: String::new(),
            coverage: HoldingsCoverage::Complete,
        };

        let (status, _) = confirm_investments(State(state), Json(request)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }
}

/// The router-level proof that `libs/sjel-server`'s predicate tests cannot give:
/// a route registered BELOW the `.layer()` call passes every test of
/// `origin_allowed_by` and still answers a hostile page.
///
/// `/routes` is the control rather than `/api/dashboard`, because every data
/// handler here opens the deployment's SQLite file and a test must not. The
/// refusal is asserted on the data routes, where the guard answers before the
/// handler runs and nothing is opened.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn answer(method: &str, path: &str, origin: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        build_router(state_from(Config::load()))
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .expect("the router answers")
            .status()
    }

    /// The three POSTs in this list take no request body, so a hostile page can
    /// send them as *simple* requests — no preflight, nothing for a CORS policy
    /// to refuse. They are the reason the guard refuses the request rather than
    /// merely withholding a response header.
    #[tokio::test]
    async fn a_foreign_origin_can_neither_read_the_ledger_nor_drive_a_write() {
        for (method, path) in [
            ("GET", "/api/dashboard"),
            ("GET", "/api/portfolio"),
            ("GET", "/api/subscriptions"),
            ("POST", "/api/ledger/rebuild"),
            ("POST", "/api/writeback"),
            ("POST", "/api/import/obsidian"),
        ] {
            assert_eq!(
                answer(method, path, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{method} {path} answered a foreign origin — it is registered below the guard layer"
            );
        }
    }

    /// The other half. 200 from `/routes` is a handler answering.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_still_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                answer("GET", "/routes", origin).await,
                StatusCode::OK,
                "the guard refused a caller it must admit: {origin:?}"
            );
        }
    }
}
