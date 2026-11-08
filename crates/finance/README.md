# finance

Journal-backed cash flow, reviewed bank imports and subscriptions with
append-only price and state history.

## Surface

### CLI (`finance-cli`)

```bash
finance-cli prices fetch                   # fetch daily quotes from configured market providers
finance-cli subscriptions audit            # audit currency declarations, cycle settings, and burn consistency
finance-cli decisions export               # export decision ledger history when server is offline
```

### HTTP API

The finance service binds loopback on `SJEL_FINANCE_PORT` (default 8090) with authenticated routes:

- `GET /health`, `GET /ready` — service status and database connectivity.
- `GET /api/dashboard` — composite dashboard projection (cash flow, recurring spending, savings rate).
- `GET /api/subscriptions` — subscription registry with calculated current price and status.
- `GET /api/subscriptions/burn?at=YYYY-MM-DD` — annualized and monthly burn totals in EUR.
- `POST /api/subscriptions/:id/price` — append an auditable price change point.
- `POST /api/subscriptions/:id/state` — append an auditable state transition (`considering`, `trial`, `active`, `paused`, `cancelled`).
- `POST /api/writeback` — write rendered series copies into `<overlay>/data/finance/subscriptions/`.
- `GET /api/import/csv/mappings` — list configured CSV import profiles from private configuration.
- `POST /api/import/csv/preview` — validate raw CSV uploads against mapping rules.
- `POST /api/import/csv` — stage validated CSV transactions into review candidates.
- `GET /api/import/candidates` — list unconfirmed transaction candidates.
- `POST /api/import/candidates/:id/review` — confirm or reject a staged transaction candidate.
- `POST /api/import/candidates/reclassify-batch` — batch update categories for matching unconfirmed expenses.
- `POST /api/import/candidates/:id/allocation` — assign personal/shared cost split or purpose tag.
- `POST /api/import/mail-candidate` — stage candidate transaction from `comms` mail receipt detection.
- `GET /api/portfolio` — aggregate investment holdings with drift against target allocations.
- `GET /api/prices/status` — instrument quote freshness and provider fetch health.
- `GET /api/decisions` — list open and resolved investment rebalancing proposals.
- `POST /api/decisions/:id/verdict` — record an operator decision on an active proposal.
- `GET /api/trips/:id/spending` — transaction actuals filtered to a specific trip window.

## Why this shape: Plaintext journal with projection indexing

Accounting core logic is centered on a double-entry plaintext journal:

1. **Human-readable single source of truth.** The canonical ledger is an `hledger`-syntax journal file
   under version control. Plaintext git history preserves the complete audit trail and guarantees that
   records remain readable independent of database state.
2. **Disposable SQLite projection.** The shared database holds rebuildable projections
   (`finance_transaction_projection`, `finance_holding_projection`). If projection tables drift or corrupt,
   re-parsing the journal via `src/journal.rs` regenerates all historical metrics identically.
3. **In-process parsing.** `src/journal.rs` parses journal files natively in Rust without shelling out
   to external interpreters or CLI binaries, ensuring fast startup and cross-platform consistency.

## Why this shape: Append-only subscription series

Subscriptions model recurring commitments as historical series rather than mutable properties:

1. **No mutable price or status columns.** Subscription tables omit mutable scalar columns for current price
   or current status. The active rate is calculated via `price_at()` over the price points series, and current
   commitment status is derived from the state changes series.
2. **Historical drift detection.** Retaining dated price points enables exact cumulative cost calculations
   since inception and surfaces unexpected vendor price increases.
3. **Audit durability.** State transitions (`considering → trial → active ⇄ paused → cancelled`) record
   historical dates and rationale, preserving subscription lifecycle context across multiple years.

## Why this shape: Staged import and candidate review

Data intake is strictly decoupled from ledger mutation:

1. **Two-phase ingest.** Raw bank CSVs and email receipt extractions are staged into hash-addressed
   `TransactionCandidate` records. Staging is idempotent; re-importing identical files does not create duplicates.
2. **Explicit confirmation required.** No staged candidate posts to the journal automatically. The operator
   reviews proposed accounts, categories, and personal/shared splits before committing transactions.
3. **Atomic posting.** Review confirmation appends balanced postings to the canonical journal and updates the
   SQLite projection in a single atomic operation.

## Why this shape: Append-only decision ledger

Portfolio allocation proposals and rebalancing logic (`src/decision.rs`) follow an append-only event stream:

1. **Deterministic rule proposals.** Drift exceeding configured asset-class or instrument bands emits
   rule-based recommendations (`src/decision.rs`). Rung 1 rules determine triggers; risk analytics (`src/risk.rs`)
   provide supporting evidence (volatility, covariance) but never initiate triggers directly.
2. **Immutable proposal events.** Decisions record proposal events (`accepted`, `rejected`, `superseded`,
   `reinstated`) in `finance_decision_events`. A human verdict permanently closes a proposal; automated runs
   cannot reopen a row an operator answered.
3. **State consistency gates.** Submitting a verdict (`POST /api/decisions/:id/verdict`) re-evaluates rules
   against current market data; if positions or prices shifted during review, the endpoint rejects the submission
   with HTTP 409, preventing stale decisions from committing.

## Why this shape: Reviewed investment ideas

Ideas and their edits live in finance storage, keeping one history for the API and app.
Research matches ticker, name and keywords before a model writes a cited digest. A model
may propose entry or exit, but the proposal carries its model, sources and reference price,
requires a human verdict, and never places a trade. The separate deterministic allocation
engine remains in `src/decision.rs`.

Price outcomes after 30 and 90 days make the proposal's record inspectable; an unanswered
review or missing price is not evidence of success. Public Reddit RSS supplies research
without storing an OAuth credential. Subscription notes remain Sjel-managed through projects.

## Why this shape: Symbolic accounts and cross-capability joins

Accounting schemas protect privacy and integrate across workspace domains:

1. **Symbolic account paths.** Accounts use generic organizational identifiers (`assets:bank:checking`) rather
   than real-world bank names, account numbers, or IBANs, keeping financial identifiers out of repository history.
2. **Tag-based trip joins.** Expenses incurred during travel carry an `sjel-trip-id` tag pointing to the Trips
   capability plan ID (`src/analytics.rs`). Transactions remain classified under their natural spending category
   (such as food or transit) while allowing Trips views to extract exact trip window actuals.
3. **Integer arithmetic.** Monetary values are represented in integer cents. Fractional currency conversions
   round half away from zero to avoid floating-point drift against bank statement totals.

## Considered and declined

- **Automatic journal posting without human review.** Declined: automated bank imports risk silent misclassifications
  and duplicate postings; all candidates require explicit operator approval.
- **Mutable subscription price and status columns.** Declined: overwriting current prices erases historical
  cost curves and conceals vendor price creep.
- **Lot-level tax and capital gains calculation.** Declined: complex tax accounting requires specialized
  broker trade execution data; capability scope is focused on cash flow, savings rate, and portfolio allocation.
- **Storing real account numbers or IBANs in journal files.** Declined: plaintext journals under git must not
  permanently record sensitive banking identifiers.
- **External CLI accounting binaries.** Declined: relying on external shell-outs introduces environment
  dependencies and IPC overhead; journal parsing executes in-process in Rust.
