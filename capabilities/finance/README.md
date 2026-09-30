# finance

Journal-backed cash flow, reviewed bank imports and subscriptions with
append-only price and state history.

## Why a capability

`src/journal.rs` is the accounting engine. The private plaintext journal is
canonical, the store holds a disposable index, and this capability owns the review
path and the product above both.

The split is not a compromise between build and adopt. A plaintext journal under
git satisfies the knowledge-boundary's V1 and an index rebuilt from it satisfies
V2, so choosing the storage format did the boundary work rather than a rule someone
has to remember. What gets built here is the layer nothing off the shelf does well.
Sjel owns the typed CSV adapter, duplicate detection, explicit review and journal
write because those are product policy, not accounting-engine policy. Price feeds
are still not built here: `pricehist` already emits `P` directives.

Until 2026-08-28 this paragraph also said that return math was not built here
because `hledger roi` already computes IRR and TWR. PRD Q50 retired that claim
along with the executable. Nothing in Sjel ever called `hledger roi`, so no IRR
or TWR figure was ever computed for this capability by anything — the sentence
described an engine's feature list rather than this system's behaviour. Return
math remains unbuilt, which is now stated as the absence it is.

## What exists today

- `AccountingEngine` defines two operations, `check` and `transactions`, without
  exposing journal syntax. `JournalEngine` implements both against
  `src/journal.rs`, in this process. The trait declared register, balance, budget,
  cash-flow and ROI reports until PRD Q50 (2026-08-28); measured then, production
  called none of the five, so they retired with the shell-out that was their only
  reason for existing.
- A configurable CSV adapter handles column names, delimiters, decimal marks,
  currencies and symbolic source accounts. It emits SHA-256-addressed
  `TransactionCandidate` values and discards the raw rows.
- A separate investment activity preview maps signed quantities, source references
  and optional exact-decimal unit prices. Private profiles can explicitly classify
  position-changing and non-position activity values; an unclassified nonzero
  quantity fails closed rather than inflating holdings. Preview is read-only;
  explicit confirmation re-runs the adapter and atomically stores only aggregate
  holdings in a private snapshot. Instrument aliases remain private mapping data.
- Candidates stay pending until the local UI confirms or rejects them. Confirmation
  validates the prospective journal, appends once, and atomically rebuilds the
  transaction projection. A retry cannot duplicate the posting. Confirmed
  uncategorized expenses can later be grouped locally by description and explicitly
  batch-reclassified; the selected journal postings are validated and replaced once.
- A confirmed expense can then receive a reviewed purpose and personal/shared split.
  The personal share remains on the expense account; money fronted for others posts
  to `assets:receivable:shared`. A linked repayment settles that receivable and is
  never projected as income or negative spending.
- `/finance` has Overview, Planning, Transactions, Investments and Subscriptions.
  Personal result, external cash movement, category composition, purpose/trip
  summaries, the table and the constrained flow explorer all use the same Rust
  projection. Investments loads on demand and puts the decision inbox above the
  position table, so the tab opens on the question rather than on a valuation.
  Transactions starts with largest-first categorization review and a trip-first
  allocation workspace: Trips owns the plan and dates, while Finance loads that
  window and reviews each transaction's personal share. Internal transfers are
  excluded by default.
- Planning uses medians from complete months, private behavior rules and dated
  commitments to project monthly spending and savings. Exceptional trip spending is
  excluded from the recurring forecast. Reviewed balances and holdings add liquidity,
  runway and concentration; partial snapshots remain visibly partial.
- Subscription anomalies, card break-even calculations and loyalty values share that
  planning response. Provider terms need dated source links. Eligible spend can come
  from reviewed journal postings, but benefit use and point value remain explicit
  private assumptions, so an incomplete comparison stays provisional.
- Subscription prices and states remain append-only, with conflict-safe Obsidian
  writeback through `libs/markdown-root`.

## A subscription is not a row with a price

Every tool in this space stores the price as one mutable number. That number
answers "what am I paying now" and nothing else. It cannot say what this has cost
since it started, and it cannot notice a provider raising the price, because the
moment the new figure is written the old one is gone.

So a subscription carries two append-only series:

- **Price points.** A change appends a row with its date and reason. Two rows give
  true cost since inception and a drift signal; one mutable number gives neither.
- **State changes.** `considering → trial → active ⇄ paused → cancelled`, each with
  a date and a note, so "when did I pause this and why" survives the year.

The table shape makes that structural. There is no `price` column on `subscriptions` and
no `status` column, because a column is a thing that can be updated. What the
current price *is* comes from `price_at()` over the series. There is no cached
total either: a stored figure is a second source of truth that goes stale silently.

`trial` counts as billing even at a price of zero. The price series says what it
costs and the state says whether you are on the hook, which is the case that makes
collapsing the two into one field wrong.

## Ownership across the vault boundary

| Statement | Owner | Written by |
|---|---|---|
| Why I pay for this, value check, alternatives | the vault note's prose | the human |
| Price history, state history, computed burn | the shared store | this capability |
| Current price, monthly equivalent, drift | the vault note, marked region | this capability, regenerable |
| The price and state SERIES, row by row | the vault note, same marked region | this capability, **not** regenerable |
| Confirmed postings | private plaintext journal | this capability, after explicit review |
| Import candidates and transaction projection | the shared store | this capability, rebuildable |
| Spending behavior, forecast adjustments and personal card/loyalty values | private `config/finance.json` | the human |
| Reviewed aggregate holdings | private configured snapshot | this capability, after explicit review |
| Holdings dashboard projection | the shared store | this capability, rebuildable from the private snapshot |
| Baseline, forecast, runway and decision results | API response only | this capability, rebuilt from reviewed private state |

Writeback goes through `libs/markdown-root`'s region writer, which preserves every
byte outside the markers and refuses to overwrite a region a human edited. Nothing
here opens a file for writing by any other path. A conflict is reported and
counted, never resolved: the response names each conflicting note so the operator
can look at it.

The third row is the one that is not like the others. Everything else the region
holds can be recomputed from the store; the series is where the store's numbers come
FROM. PRD Q47 (2026-08-27) counted `finance_price_points` and `finance_state_changes`
among the 512 irreplaceable rows in the database, because a price is observed once, on
the day it changed, and nothing can reconstruct it afterwards. Region version 2
(2026-08-28) therefore renders both series as tables under the current-state callout —
a summary of a series is not a copy of it.

The region is regenerated by `POST /api/writeback` and by nothing else: no schedule,
and the one caller is the Finance page's writeback button. That makes the safety copy
as fresh as the last press, which is the honest state of it — a scheduled writeback
would be a second trigger to keep in agreement, and this capability is `autostart`, so
the button is always reachable.

### When the note is not there

Measured 2026-08-28: it is not, for any of them. The 2026-08-23 vault reorganisation
(vault commit `ba60231`, ruled in PRD §5.5) moved all 37 finance notes out to
`<overlay>/data/finance/vault-notes/`, on the grounds that they are entity rows rather
than things a human wrote sentences into. So all seven live subscriptions point at a
note the vault no longer has, the region writer had nothing to write into, and the
series was in no file at all.

PRD Q31 rules exactly this: write a region into the human's note when one exists, and
create a projected file only when none does. `export_projections` is the second branch —
one whole generated file per note-less subscription under `Resources/Sjel/Subscriptions/`,
carrying the same `render_block` body so the two paths cannot drift into two shapes of
one figure.

The projection deletes itself the moment a note for its subject appears in the vault,
and the region in that note takes over. Two homes for one number is the failure this
boundary exists to prevent, so the writeback never leaves both in place.

Frontmatter seeds a subscription and is then not re-read for those fields. A
re-import would otherwise throw away every price change recorded since, because the
single cost figure in the note was only ever a starting point.

## EUR is declared, and a price that is not EUR is refused rather than relabelled

PRD Q103 (2026-09-09) makes EUR the currency every single total is stated in. The ruling
had to be written because subscription notes carried four spellings of one fact, and a
note that declared `currency: USD` could carry a block Sjel had written as `EUR / month`.

`obsidian::read_price` settles the spellings. The first key present wins and the rest are
recorded as shadowed, never merged:

| Key | Currency | Cycle |
|---|---|---|
| `cost` | `currency:`, else EUR | `billing_cycle:`, else monthly |
| `cost_eur` *(deprecated)* | EUR; a differing `currency:` is a contradiction | `billing_cycle:`, else monthly |
| `price_eur` *(deprecated)* | EUR; a differing `currency:` is a contradiction | `billing_cycle:` is **required** |
| `yearly_cost_eur` *(deprecated)* | EUR | yearly; a differing `billing_cycle:` is a contradiction |

`price_eur` is the one that must not default. It appears on purchase-decision notes,
which carry an annual fee and often no `billing_cycle:` at all, so a monthly default
turns a 240 EUR annual card fee into 240 EUR a month.

`money::to_eur` does the conversion and refuses when no published rate covers the pair.
Nothing is assumed: `finance_fx_rates` was empty when the ruling was made, because
`price::run_named` took its FX targets from the holdings snapshot alone and every reviewed
holding was already EUR, so the `ecb` provider had never been handed a target.
`price::fx_targets` now adds every currency a subscription is priced in, so
`finance-cli prices fetch` records the rate a USD subscription needs. The burn
`GET /api/subscriptions/burn` returns under `eur` therefore contains only amounts that
are EUR or were converted with a dated, sourced rate; anything else is itemised in
`eur.not_convertible` so a short total is never a silent one.

`finance-cli subscriptions audit` is the read-only view of all three checks — unreadable
price keys, a note whose currency differs from the price point in force, and a burn that
cannot be completed. It exits 1 on any finding.

## HTTP surface

On the manifest-declared port. `GET /routes` serves the full manifest.

- `GET /health` · `GET /ready` (liveness and a reachable database, judged separately)
- `GET /api/subscriptions`
- `GET /api/subscriptions/burn?at=YYYY-MM-DD`
- `POST /api/subscriptions/:id/price` · `POST /api/subscriptions/:id/state`; both append
  idempotently, and the response says whether a new history point was created
- `GET /api/import/obsidian/scan` · `POST /api/import/obsidian`
- `POST /api/writeback`
- `GET /api/import/csv/mappings`
- `POST /api/import/csv/preview` · `POST /api/import/csv` · `GET /api/import/candidates`
- `GET /api/import/investments/mappings` · `POST /api/import/investments/preview`
- `POST /api/import/investments/confirm`
- `POST /api/import/candidates/:id/review`
- `POST /api/import/candidates/reclassify-batch`
- `POST /api/import/candidates/:id/allocation`
- `POST /api/import/candidates/:id/reimbursement`
- `GET /api/ledger/check` · `POST /api/ledger/rebuild`
- `GET /api/dashboard?start=&end=&account=&category=&currency=`
- `GET /api/portfolio?currency=EUR` — positions with price freshness, share in basis
  points and drift against the configured targets
- `GET /api/prices/status` — per-instrument freshness, the newest fetch attempts and the
  last status per provider
- `GET /api/decisions?status=open|accepted|rejected|superseded|all`
- `POST /api/decisions/run` — recompute proposals and reconcile them against the ledger
- `POST /api/decisions/:id/verdict` — record a human verdict
- `GET /api/trips/:id/spending` — one trip's actuals, instead of the whole projection

The dashboard response includes source freshness and the planning report. Neither is
stored as a second source of truth.

## A price is an observation, never a correction

PRD Q81 (2026-09-05) rules the providers; `capabilities/finance-prices/README.md` is the
job that runs them. `finance_prices` holds what a source said an instrument was worth on a
day, and nothing else. Three consequences follow, and each is a rule the code enforces
rather than a convention:

**A fetched price is never written back into the reviewed holdings snapshot.**
`validate_source_snapshot` recomputes the file's content hash over `latest_unit_price`,
so a quote written there would make the snapshot refuse itself on the next read. The
market price is layered over the reviewed one at read time and every position says which
of the two valued it, in `value_basis`.

**A return series comes from one source per instrument.** Measured 2026-09-05: one
`broker` observation in EUR sitting inside 502 `yahoo` closes in USD produced a −87% day,
a +666% day and an annualised portfolio volatility of 152%. Two sources are two price
scales — a different currency, a different adjustment basis — so a switch between them is
a fabricated return. `risk.rs` uses the source with the most observations for each
instrument, and the consequence is stated rather than hidden: an instrument priced only by
`broker` grows one point per run and will sit at `insufficient_history` for a long time.

**A market price in another currency does not value the position.** It falls back to the
reviewed price and names the mismatch. Converting silently would hide two provenance facts
— the quote's date and the rate's date — behind one number.

`finance_prices` is NOT `finance_price_points`, which is Sjel's own subscription pricing
history. Market data and what Sjel pays for a streaming service are two different series
that happen to share a word.

## What cannot be computed here

There is no lot and no cost basis anywhere in this capability, and adding one is a
different feature with a different import. So `change_since_review` is the difference
between a position's market value and its value at the latest reviewed broker activity
price. **It is not a return and it is not P&L**, and the UI does not call it either.

## Two rungs: the rules propose, the model explains

PRD Q82 (2026-09-05) rules this shape; Principle 1 made structural rather than
documentary. Rung 1 is three rules in `src/decision.rs`. Drift beyond an allocation's band
emits `rebalance` — for an instrument target **and** for an asset-class target, which a
live fixture proved was not the same code path: the first pass walked positions only, and
a 60/40 policy showing a 3,407 bp drift minted nothing. A median monthly result above the
configured floor emits `contribute`. A lump-sum renewal dated in the month a contribution
is proposed for emits `review`, because a yearly charge reaches the median as one twelfth
of itself while the cash leaves whole. Every proposal carries `rung = "rule"` and
`every_proposal_names_its_rung_and_none_is_model` is the test that keeps it so.

Rung 2 is `src/risk.rs`: annualised volatility, pairwise correlation, portfolio volatility
and long-only minimum-variance and max-Sharpe weights over a hand-rolled Cholesky. It is
attached to a proposal as evidence and **never** as a trigger. Its floors are refusals
rather than warnings — below 120 daily observations per instrument, or 60 overlapping
dates per pair, the answer names the actual count and the figure is `null`. Never a zero:
a zero volatility claims a price never moved and a zero correlation claims an independence
nobody measured. A singular covariance is reported, never regularised: the Cholesky's
non-positive pivot *is* the guard, so the solver and the refusal are one code path.

`sell` is accepted by the ledger's CHECK and minted by no rule. A machine proposing the
sale of a real position is a different order of claim from proposing a rebalance.

## The decision ledger appends; it never updates

PRD Q80 (2026-09-05) rules this table. Its rows are **c1** — the §6.1 definition governs,
and a proposal about my own allocation names no third party. `data_class` carries **no
CHECK**, deliberately: the vocabulary belongs to `libs/content-item` and has been renamed
once already, and a constraint over somebody else's vocabulary is a table rebuild waiting to
happen. `content_item::valid()` runs at every write site instead. `kind` and `rung` do carry
CHECKs, because those are closed sets this capability owns — and the rebuild one of them
cost is recorded two paragraphs below.

`finance_decisions` holds the proposal and `finance_decision_events` holds everything
afterwards — the verdict, the outcome, the supersession, the reinstatement — as appended
rows. There is no mutable `verdict` column, deliberately: a column is a thing that can be
updated, and one UPDATEd verdict loses the date the call was actually made, which is the
only fact that makes "what did I decide, and did it work" answerable a year later.

**Nothing here is deleted either, including a supersession.** A run that no longer
produces a proposal appends `superseded`; a later run that produces it again appends
`reinstated` beside it, and the LATER of that pair is the proposal's state. A `verdict`
outranks both and closes the row for good, because a human answered those exact numbers
and re-asking would be the ledger forgetting. The reason this matters in practice: the
proposal id is a hash over bucketed numbers, so a drift that leaves its band and comes back
into the same bucket re-mints an id the ledger already carries. Reading the supersession by
presence rather than by recency left such a proposal unreachable for ever — no later run
can mint a different id for it — while the run reported success. Keeping both assertions
means "this left the inbox on the 5th and came back on the 6th" is still readable a year
later, which is the whole reason this table has no mutable column.

Widening the `event` CHECK to admit `reinstated` costs a table rebuild on a file that
already carries the three-value shape, and `FinanceStore::run_migration` performs it once,
behind a probe of the installed DDL and idempotent on re-run. That price is why an earlier
form of the repair deleted the row instead; it is the honest cost of an append-only ledger
and the migration pays it.

Accepting a proposal records a decision and moves no money: no journal entry, no holdings
snapshot, no order.

**A verdict is not accepted on trust.** `POST /api/decisions/:id/verdict` re-runs the rules
inside its own blocking closure and answers **409 with the current proposal id** unless the
run still mints the id being answered, so a stale inbox cannot record an answer to numbers
that have moved. That recompute is handed an empty feed list: the id hashes kind, subject
and buckets and never the evidence, so comms being unreachable may not refuse a verdict.

Evidence itself fails closed. `item_is_quotable` copies a feed item's title and URL into a
c1 row only when comms states a class no stricter than the row's own, and drops every item
whose class is unstated. `GET /comms/feed` did not state one until 2026-09-06, so the
decision inbox shipped with **no feed evidence at all**; the list now carries `data_class`
and the block fills as items arrive. That was a stated, accepted state, never an oversight.

The human-readable copy is written on the WRITE path, not by a CLI verb. The verdict
handler re-renders `<overlay>/data/finance/decisions/YYYY-MM.md` in the same request that
appends the event, whole-file through a temp-plus-rename at mode 0600. A human verdict and
its note is the one fact here that no re-import and no re-run reproduces.
`finance-cli decisions export` is the copy you can take when the server is down — never
the only writer.

The destination is `SJEL_FINANCE_DECISIONS_ROOT` if it is set and the overlay root
otherwise. The override is not a convenience: `SJEL_DB_PATH` isolates the database and
nothing else, so a verification run that only overrides the database still writes month
files into the owner's overlay and a subscriptions projection into their vault. Redirect
those two with `SJEL_FINANCE_DECISIONS_ROOT` and `SJEL_FINANCE_OBSIDIAN_ROOT`; overriding
`SJEL_PERSONAL_ROOT` instead would redirect the config read as well, so the run would be
against a configuration that is not the one being tested.

## Configuration

The thirteen tables live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `finance`, so they are
`finance_subscriptions`, `finance_price_points`, `finance_state_changes`,
`finance_transaction_candidates`, `finance_transaction_projection`,
`finance_holding_projection`, `finance_holding_projection_state`,
`finance_holding_projection_sources`, `finance_prices`, `finance_fx_rates`,
`finance_price_fetches`, `finance_decisions` and `finance_decision_events`
(`libs/sjel-store/README.md`). PRD Q45
(2026-08-27) moved them there from a Postgres schema, and the path is a deployment
fact rather than a capability one: `$SJEL_FINANCE_DATABASE_URL` is gone, because a
file per capability would drop the join `capabilities/places` builds its spend layer
on. Vault location from
the overlay's `config/finance.json`, or `SJEL_FINANCE_OBSIDIAN_ROOT` for
development. `journal` lives in that same private file and can also be set with
`SJEL_FINANCE_JOURNAL`. `schemas/finance.json.example` documents the
shape without carrying private deployment values. Named `csv_mappings` also live in
that private file; the loopback API supplies them to the local review UI, where the
operator can still edit every field before staging. A mapping explicitly declares
amount direction, accepted date formats, and whether every row must match the header
or rows without transaction fields may be counted and ignored. Preview returns only
quality counts and an identity token; staging recomputes the CSV and requires that
unchanged token. Stable-reference duplicates are counted within one export. When
the source has no reference, repeated normalized rows are preserved with
deterministic occurrence identities, so legitimate repetition and overlapping-export
idempotency both survive in the candidate store.
Named `investment_csv_mappings` supply the corresponding preview-only adapter. The
stable source key and source identifier to symbolic commodity mapping belong there
rather than in Sjel. `investment_snapshot` names the private canonical collection
written after review. Reconfirming one source replaces only that source; Overview
derives its aggregate and review coverage from every confirmed source. A provider
without an export can use a privately authored current-position CSV with one dated
row per open position. Raw CSV rows and mapping values are never written to the
canonical collection. When one instrument spans sources, its activity price is
suppressed rather than selecting an arbitrary source. Other prices shown in Overview
are latest activity prices, not live quotes or a claim about current market value.

Optional `planning` configuration controls baseline length, cash-buffer target,
source-freshness thresholds, category behavior rules and dated adjustments. Private
source expectations can track each symbolic transaction account or reviewed holdings
source independently, so one recent import cannot hide another stale source. A rule
classifies what normally recurs; a trip purpose remains exceptional even when its
category is otherwise recurring. Historical categories replaced by commitment or
subscription series are removed once before those dated series are added, so they do
not count twice. Fixed costs without a replacement series stay in the forecast.

Card options carry fee, reward-rate, FX-cost and dated source evidence. When private
account prefixes are configured, eligible spend is derived from reviewed expense
postings for the trailing twelve months. Personal benefit values, FX spend, point
valuations and loyalty balances are never inferred. They remain private inputs, and
the response marks a decision provisional until usage and sources have been reviewed.

Transaction source accounts are balance accounts. A reviewed card purchase therefore
posts from a liability to an expense, while a settlement posts between balance accounts.
The latter projects as a transfer and is excluded from spending unless transfers are
explicitly requested.

## Two representation choices

**Money is integer cents.** A monthly burn is a sum of divisions, since a yearly
plan contributes a twelfth, and floating point produces figures that disagree with
the bank by a cent for reasons nobody can reconstruct later. Conversion rounds half
away from zero rather than truncating, because truncation loses a cent per
subscription per month and the total drifts below reality.

**Dates are ISO-8601 strings.** They sort lexicographically, which is the only
operation performed on them. `trips` set the precedent, and a date library would be
a dependency bought for `<=`.

Weekly converts at 52/12, not four weeks a month. Four-week months are eleven
months of the year, and the error runs toward under-reporting what you spend.

## Ledger and engine boundary

The account tree, trip tag and public conventions are fixed and validated as
[`schemas/finance-journal.example`](../../schemas/finance-journal.example), because
the shape of the tree is a foreclosing call and writing it down after the importer
exists means writing it around the importer's accidents.

Two decisions in there worth naming:

**Accounts are symbolic.** `assets:bank:checking`, never an IBAN. Git history is
permanent and the knowledge-boundary requires that forgetting stay possible, so the
mapping from a symbolic name to a real account lives in Vaultwarden. This holds for
comments and commit messages too.

**A trip is context, not a category branch.** A trip expense is also a food expense.
Duplicating the category tree under a trip prefix would make "what did I spend on
food this year" answerable only by remembering to union two subtrees. The
`axon-trip-id` tag is the join, read in `src/analytics.rs` when the projection is
built. Finance stores only the opaque Trips
plan identifier; the dashboard asks Trips for the current title. Purpose is a
separate `axon-purpose` tag, and ownership is represented by postings, so category,
reason and whose money was spent remain independently queryable.

**The format is hledger's; the engine is not.** Until PRD Q50 (2026-08-28)
`service.toml` declared `ledger = "hledger"`, `toolchain.toml` made every machine
enabling Finance install the pinned 1.52.1 executable, and this capability shelled
out to it. `src/journal.rs` reads the journal in-process now, so Finance needs no
host tool and the GPL question the separate executable raised does not arise.

What survived is the file. The journal is still plaintext hledger syntax, still
under git, still diffable, and still opens in hledger on any machine that has it —
an operator's choice rather than a requirement. `schemas/finance-journal.example`
is the published contract (Principle 8). This is why the swap changed no journal
byte and no `source-id`, which `tests/live_journal.rs` and the fixture-parity test
in `src/journal.rs` both hold to. `upstreams.toml [hledger]` keeps the dated
verdict.

## Related tools and why this is not them

| Tool | Good at | Relationship |
|---|---|---|
| [Actual Budget](https://actualbudget.org) | Envelope budgeting, fast local-first UI | Rejected as core. Its automatic German bank sync ran through GoCardless Bank Account Data, which stopped accepting new accounts in July 2025 |
| [Firefly III](https://firefly-iii.org) | A serious rule engine and a real REST API | PHP with its own database. A second store inside Sjel, and a ledger that is not git-diffable, so agent writes stop being reviewable |
| [Ghostfolio](https://ghostfol.io) | Portfolio math, price feeds, allocation | A candidate for the investment half later. It owns valuation well and models a subscription's history not at all |
| [hledger](https://hledger.org) | Double-entry, commodities, reports and `roi` | Its journal FORMAT is what Sjel writes and reads. The executable was adopted as the engine behind `AccountingEngine` until PRD Q50 (2026-08-28) and is retired; `src/journal.rs` parses the file in-process. Never the importer, never bundled |
