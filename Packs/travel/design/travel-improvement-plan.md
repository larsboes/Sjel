# Travel system improvement plan — execution

Single working file for the travel-system work. Source brainstorm and full reasoning:
`travel-system-brainstorm-2026-08-11.md` (same folder). Started 2026-08-11.

<!-- The em dashes below separate a file reference from what is wrong at it, which is the
     form every capability README in the Axon repo already uses. -->

Decisions already settled by Lars (2026-08-11):

- Dated pre-trip obligations: **calendar owns them**, not tasks.
- Buying: **inverted flow**. He books in the browser, pastes the link or drops the ticket,
  Axon parses it into a booking record. Axon never buys.
- Travel mode: **discovery matters more than it looks**. The model-driven ideas are not
  cut for being discovery-shaped.
- Agent surface: **HTTP-only eventually**. The bun CLI is a stepping stone; the
  composition eventually moves into a capability.

Repo note: the worktree had unrelated in-progress `finance` changes when this started
(`capabilities/finance/src/planning.rs` and friends, untracked and modified). They are
left untouched, and every commit here names only travel paths.

## Do first

- [x] **1. `axon capability call` learns PUT/PATCH/DELETE and stops eating error bodies**
      `axon:114-141`. Only `get` and `post` existed, both `curl -sf`. Half of trips'
      lifecycle was unreachable and every `{"error": …}` body was discarded before the
      caller saw it.
- [x] **2. `transit /api/trips` stops returning `{count, trips: []}` with HTTP 200**
      `capabilities/transit/src/server.rs`. Real read via `list_trips(session_id, limit)`,
      with `count`/`returned`/`truncated` so a bounded read says what it left behind.
- [x] **3. Split-ticket segments record which train they were priced for**
      `capabilities/transit/src/hafas.rs` and `travel.rs`. `train_match` per segment,
      `savings` now optional, chain-level `confidence`, and failed fare lookups counted
      instead of silently routed around.
- [x] **4. Typed plan-item payload variants**
      `schemas/trip-plan.schema.json` plus `capabilities/trips/src/store.rs`.

## Verification standard

Nothing is checked off on "should work". Each item needs one of: a passing test that
fails without the change, real command output, or a live HTTP response.

## Progress log

### 1. axon capability call — done 2026-08-11

Added `put`, `patch` and `delete` arms; swapped `curl -sf` for `curl -s --fail-with-body`
(installed curl is 8.7.1, well past the 7.76 minimum). Non-zero exit unchanged. All three
usage strings updated.

Verified live against the running trips capability:

- invalid POST now returns the deserialize error body with exit 22, where it previously
  returned nothing
- full create → PATCH → DELETE → confirm-gone lifecycle runs through the CLI; PATCH and
  DELETE both previously died at `axon: method must be get or post`

### 2. transit /api/trips — done 2026-08-11

`list_session_trips` now delegates to `list_trips(session_id: Option<&str>, limit:
Option<i64>)`; `count` delegates to `count_trips(session_id)`. `LIMIT $2` with a NULL
parameter is Postgres' `LIMIT ALL`, so the unbounded read stays a parameter value rather
than a second SQL string. Handler takes `?session_id=` and `?limit=` (default 100, clamped
to 500) and returns full trips with legs.

- new test `list_trips_sees_manual_and_session_trips_and_bounds_the_read`: manual and
  session trips in one ranking, filter still narrows, bounded read leaves `count` intact,
  and `list_session_trips` still agrees with the new path
- live: 5 real trips with legs where the endpoint previously returned `[]` with HTTP 200;
  `?limit=1` reports `returned: 1, truncated: true`

### 3. Split-ticket honesty — done 2026-08-11

Deviation from the brainstorm, deliberately: it specified `exact|partial|unknown`, but
"priced for a train sharing nothing with your journey" is a known mismatch, not an unknown.
Added a fourth value, `different`, so the case that costs money is not filed under
"could not tell".

`SplitResult` gained `confidence`, `unpriced_pairs`, `queried_pairs`; `segments` became
`Vec<SplitSegment>` carrying `train_match` and `expected_trains`; `savings` became
`Option<f64>`. `extract_section_spans` maps the direct journey's trains onto stop indices
as a second pass, because `extract_stops` deduplicates by station id and a transfer stop is
pushed before the train it departs on is known.

- three new tests: train-match classification including wrong-order, worst-case chain
  confidence, and the null-savings distinction. 41 transit tests pass, up from 38
- live HAFAS, Bonn → Frankfurt on 2026-08-25: `train_match: exact` against
  `expected_trains: ["66","1513"]`, and **3 of 6 fare lookups returned nothing** — the
  condition the old code hid completely
- dashboard updated to match, browser-verified: the chain warning renders with the
  lookup-failure count, and the booking link is withheld entirely when confidence is `low`

### 4. Typed plan-item payloads — done 2026-08-11

The pre-audit changed the design. `event` turned out to have three producers with three
different payload shapes (scouting opportunity, whole search result, calendar anchor), so
declaring a shape for `event` would have broken two working buttons. Declared variants only
where there is exactly one shape to promise: `transport` (`{mode, journey}`) and the new
`option_set` (`{query, options, observed_at?}`). Everything else stays permissive and the
schema says why.

- four new tests, including one that replays all five existing dashboard write payloads
  field for field so a narrowed variant cannot silently break the travel page
- live: valid `transport` accepted; `transport` missing `journey` rejected with
  `payload for item_type 'transport' requires the field 'journey' (required: mode, journey)`
  — visible only because item 1 stopped discarding error bodies; `option_set` accepted
  through the widened CHECK constraint; arbitrary `event` payload still accepted

### Docs corrected in the same pass

`capabilities/transit/README.md` called `/api/trips` a stub in two places and had no
account of what a split chain does not promise. `capabilities/trips/README.md` described
the payload as unconstrained. Both now match the code.

## Second pass (2026-08-11, same day)

Lars: finish the plan fully, commit the finance WIP too. Nine more items, each verified
live and committed on its own.

- [x] **finance planning** committed as its own change (`d19908c`), 111 tests green.
      It was intended work, not stray edits.
- [x] **Booking records + ticket extraction over HTTP** (`af3c8bb`). Running the parser
      against a realistic confirmation for the first time found two bugs in it: `ok` was
      the literal `true`, so a parse with no legs reported success; and `FROM_TO_RE`
      accepted `From|Ab|Start|Origin` but not the German `Von`, while `Nach` was already
      accepted. German rail confirmations are the parser's whole subject, so the one
      label it could not read was the one it most needed.
- [x] **Calendar syncs committed travel back** (`4dd4adf`). A fortnight that read as one
      free window became two with the booked day carved out. The system could propose a
      trip on top of a trip it created.
- [x] **`tools/travel.ts`** (`14cd3a1`). Verified against the page: same 11 candidates,
      same order, same states.
- [x] **Conditional writes, budget, day column, place identity** (`55650c2`). The places
      projection found three real collisions on its first run, including
      `obsidian-place:berlin` vs `place:berlin` — the same city imported from the vault
      and typed by hand, which the 75 km match has been treating as two destinations.
- [x] **Punctuality keeps its histogram** (`ef918d9`). Six minutes was never a considered
      threshold, it was the one that got a column.
- [x] **Ungoverned cloud path closed** (`08b44df`). `embed`/`rerank` reached an https
      endpoint with no policy check at all. Costs nothing today; every configured
      embedding role is loopback.
- [x] **Four rejected seams written down** (`0fa0478`) and a **Packs/travel skill**
      (`7ffb66d`), both authored by subagents and re-verified by hand. The decision
      record turned up its own finding: the redactor rewrites an ISO date to `[phone]`
      and an EVA code to `[number]` while passing every proper noun through, which is
      the inverse of what a pseudonymized derivative should do.
- [x] **Fare observation time and the trip outcome record** (`369723c`).
- [x] **Derived request schemas in `/routes`** (`636c6f0`). His stated priority.

## Third pass: xberg, and the ride reader (2026-08-11)

Lars picked xberg over Dolphin and Apple Vision, then asked for it fully.

**The spike changed the answer.** A generated DB confirmation with the journey in a table
parsed to two legs running from "Bahnhof" to "Bahnhof Zug Gleis" — the header row — with
`ok: true` and nothing missing. Worse than the morning's empty-legs case, because legs
existed. xberg was expected to fix it and does not: it recovers a key-value block as a
Markdown table and leaves the journey table flat.

So the work split in two:

- [x] **xberg adopted for the image path only** (`76e4c2f`), which builtin can never serve.
      `document_backend` is overlay config; unknown values fall back to builtin loudly.
      Two things the spike caught that no reading would have: its plain-text mode reorders
      a PDF, putting a journey table's dates twenty lines from its stations, so extraction
      parses from Markdown; and the default OCR language turns German umlauts into noise
      (`Züge` → `Ztige`), so `ocr_language` defaults to `deu`. Its JSON envelope is
      `{result:{content}}`, not the `{content}` its own docs show.
- [x] **The parser now reads row shape, not labels.** date · time · station · time ·
      station · train. That shape survives every reader tested; the layout survives none.
      Verified live: PDF, PNG and .eml all yield the two correct legs under both backends.
- [x] **`punctuality ride`** (`02de39e`). One train's actual stops on one day, from the
      columns ingest was discarding. Two traps found, both returning confident wrong
      answers rather than errors: the time columns are timestamps not strings (a string
      downcast made every query return zero stops, which looks exactly like "no such
      ride"), and DB writes the same train as `ICE 611`, `ICE611` and `611`.
- [x] **Re-ingest widened** to 2025-12..2026-07. 470,782 cells from 117M rows. The
      histogram equality holds: `at_least_minutes: 6` reproduces `share_late_6` to 1e-7,
      and a four-minute buffer at Bonn Hbf for a 10:00 ICE now reads 76%.

The OCR route from the original roadmap is closed by this, differently than specced: no
Apple Vision route, no `foundation-models` change.

### Rest of the plan, same session

- [x] **HAFAS realtime and cancellation fields** (`e9908f5`). The roadmap called live
      verification "unbounded calendar time waiting to catch a live disruption". It took an
      hour: ICE 619 scheduled 00:20 running 00:50, ICE 22 scheduled 23:44 running 00:05.
      The bigger find is that the old `or_else(istzeit)` was **dead code** — bahn.de serves
      `echtzeit`, so the fallback never fired and every journey carried its scheduled time
      as though it were actual. Captured from a live response rather than guessed, which is
      the only reason it was caught.
- [x] **NL-intent draft** (`306b37c`). `trips draft-intent "<sentence>"`, local rung only,
      persists nothing. Two measured model failures shaped it: asked for "somewhere warm in
      October" with no year it answered **2023**, twice; and it lists `dates` as unresolved
      almost every time *including when it has just returned correct ones*. My first rule
      trusted that self-report and threw away good dates. Neither is trusted now.
- [x] **Foreign-language scouting adapter: probed and rejected** (`20e3f42`). The probe
      found something that generalises well past this adapter: **a quoted span existing in
      a document does not verify the claim attached to it.** Five events passed the naive
      check with real page headings and five sequential invented dates. Requiring the quote
      to support the *date* took it from 5/5 to 0/5. The same weakness sits in
      `cloud-content-analysis`'s `source_text` requirement today.

## Not done, and why

*Refreshed 2026-08-19. Four of the five items below closed between 08-12 and 08-19;
what replaced them is smaller and different in kind.*

- **The grounding verifier** — CLOSED 2026-08-12 (`5ae2361`). For date claims it needed
  no model at all: the quote either carries the claimed day and month or it does not.
- **Transfer survival from ride-level history** — its gate held and then partly opened.
  Journey reliability (`f35d04e`, 2026-08-19) composes the histogram into a per-journey
  number, asking each transfer at its own buffer. What it still cannot do is the
  calibrated part, because that needs recorded outcomes. Still L1.
- **The dbweb train-type vocabulary mismatch** — NEW, and now the biggest one. punctuality
  keys on DB's open-data vocabulary (`RB`, `RE`, `ICE`); dbnav's `produktGattung` matches
  and dbweb's `kategorie` does not, so the same RE5 arrives as `DRB` with no cell. Every
  regional leg on dbweb silently loses its `delay_risk_score`, and has since long before
  the reliability work exposed it. Left unmapped rather than guessed at.
- **The parquet reader has no test** — NEW, found while bumping arrow-rs to 59.2.0. The
  29 unit tests cover time maths and never open a file. Needs a committed miniature; the
  real monthly files are private and 100+ MB.
- **Cancellation flag, end to end.** Unchanged: the keys are captured from a real section
  and fixture-tested, but no cancelled train has appeared in a response yet.
- **Obsidian export.** Unchanged, still gated on L1 producing something worth exporting.
- **Turning `bodies_without_schemas` into a build gate.** Unchanged: fourteen routes with
  undeclared bodies, and failing the build on all of them today directs nothing.
- **L1, one outcome record.** Not code, and now the gate on two things rather than one:
  L2's calibration, and the independence assumption reliability states rather than fixes.
