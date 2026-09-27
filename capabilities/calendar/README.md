# calendar

Axon's personal calendar layer: the source of truth for what time means on any
given day — availability windows for travel, on‑site work, remote‑work
blocks, busy periods, events you're attending, rhythms that materialize those
blocks, and (later) day‑planning detail. Open by default: an empty day carries
no meaning, and absence is not a block.

## Status

**Phase B complete (core schema + usable month dashboard); Phase F's week and
day views built; Phase C's Feed verdict surface is live; Phase E's Google
account connection, reviewed draft import, and explicit export review are live.**
The store tables, HTTP CRUD, rhythm materialization, and the dashboard
month/week/day workspace are built and tested. Feasibility verdicts and feasible
travel windows are computed and served over HTTP, with unit coverage in
`src/correlate.rs`. Feed's Discover view batches dated opportunities through
the verdict endpoint and renders the resulting evidence as a soft explanation;
it never hides a candidate for a calendar conflict. Handing a window to a fare
search is still a command the operator runs, not something the system does on
its own. Phase A's Luma import is live: `scout --promote-calendar` upserts saved
Luma events as `source = luma` entries, verified end to end against this service.

The Google sync (Phase E) is code‑complete, fixture‑tested and connected to the
configured account. The dashboard starts with a 30-day, read-only review: it
shows exact-title/time duplicate candidates, requires explicit selection, then
re-fetches each selection and rejects a changed Google revision before writing a
non-blocking Axon draft. The Calendar workspace also has a 90-day draft inbox:
adopting an entry picks its kind and raises it to `planned`, while removing one
deletes only Axon's copy. Every path that needs a token still fails loudly and
by name (which key, which file, which setup step) instead of returning an empty
success; the setup steps are in § Phases > E. No event clustering, no travel-day
view. This README carries the full build plan — it's the working document until
the capability is fully built out. Each phase below has a checklist; items
without a check are not started.

### Built

- [x] Store tables (entries, rhythms, bounded planning contexts, indexes, dedupe)
- [x] HTTP CRUD for entries, rhythms, and bounded planning contexts
- [x] Rhythm materialization (forward‑only, idempotent)
- [x] Dashboard month‑grid view — inspect/edit entries, paint timeframes, create rhythms
- [x] Dashboard week and day views — hour columns, all‑day band, create by clicking an hour
- [x] Feasibility verdicts for dated candidates, with the entries that drove them
- [x] Feasible travel windows, in the day shape `transit plan --dates` consumes
- [x] Google import as drafts, deduped on the Google event id, Axon‑wins on collision
- [x] Dashboard Google import review — bounded preview, duplicate warnings, explicit selection, revision-checked draft import
- [x] Google export gated on a per‑entry opt‑in ledger
- [x] Real offset handling both ways, DST boundaries included (`src/zone.rs`)
- [x] Commitment axis — `possible`/`planned`/`committed`, orthogonal to kind, with the three tiers rendered and clickable in the workspace
- [x] Trip drafts — events grouped by place and time proximity (`GET /api/trip-drafts`)
- [x] Time-bounded planning context — editable notices that affect Home ranking without blocking calendar time
- [x] Home composition — upcoming calendar commitments, open calendar choices, planning context, location clusters, and source overview
- [x] Feed's Discover view shows each dated candidate's soft verdict and evidence
- [x] Materialising a reviewed trip draft into a `trips.plan` through the
      public Trips API, with an idempotence ledger and dashboard confirmation
- [ ] Dashboard travel‑day view (Phase F — not started)

### Dashboard surface

The calendar workspace lives at `/calendar` and is a first‑class primary nav entry
("Kalender", between Home and Feed). It provides:

- **Three views — Monat, Woche, Tag** — behind one segmented switcher. All three
  share a single anchor date, so switching keeps the same date in view; only the
  Previous/Today/Next navigation moves it, by month, by week or by day. The service window
  is exactly what is on screen (`from` inclusive, `to` exclusive), so switching
  view reloads the range rather than over‑fetching a month.
- A **month grid**: compact, colored entry chips per day by kind (busy → red,
  work_onsite → blue, work_remote → teal, away → amber, event → purple,
  travel_ok → green); out‑of‑month days dimmed; today highlighted
- **Week and day views** sharing one time grid: 24 hour rows, one column per day,
  timed entries positioned and sized by wall‑clock time and split side by side
  where they overlap. All‑day entries pin to a band above the hours and span the
  days they cover. Rhythm‑derived blocks are hatched with a dashed edge —
  "what my rhythm says" reads differently from "a one‑off I painted". A line
  marks the current time on today's column; days outside the anchor month are
  dimmed. The view opens on the first entry of the range, or on the working
  morning when it is empty. An empty day stays empty — absence is not a block.
- **Entry creation/edit** — clicking an entry chip opens it in any view; clicking
  an empty hour opens the same form prefilled as a timed entry in that hour
  (ends stay exclusive: 09:00 opens 09:00–10:00), clicking the all‑day band
  prefills an all‑day entry for that day, and in the month grid dragging across
  days prefills the inclusive range. Kind picker, title, all‑day/timed toggle,
  inclusive calendar dates, location and notes are editable; deleting asks for
  confirmation, and rhythm-derived overrides explain that they detach from the
  rhythm. Clicking a date in the week header jumps to that day's view.
- **Planning context** — the info panel above the calendar shows only notices
  whose validity overlaps the visible range. Contexts have a type, title,
  explanation, and inclusive validity window; they can be created, edited, and
  deleted in place. They inform Home and ranking but never occupy time.
- **Rhythm creation** — kind select, title, weekday picker, start/end times,
  validity window; on save the server materializes future instances and the
  grid refreshes
- **Active rhythms list** — compact accordion below the grid showing each
  rhythm's kind dot, title, weekday schedule and time

Built in `dashboard/src/routes/calendar/+page.svelte` and
`dashboard/src/lib/calendar/` (MonthGrid, TimeGrid, EntryForm, RhythmForm,
types/constants). The date math and placement live in `types.ts` — `timedSpan`
and `allDaySpan` turn an entry into a half‑open interval against one day or one
window, and a single `packLanes` handles both the horizontal splitting of
overlapping timed blocks and the vertical stacking of the all‑day band.
The proxy auto‑discovers calendar's service via `tools/capability.sh registry`.
The manifest's API-only proxy mode keeps `/calendar` owned by this workspace and
forwards only `/calendar/api` to the service (restart the dev server after
enabling the capability).

## Data model

### `calendar.entries`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:entry:…` |
| `kind` | TEXT | See `KNOWN_KINDS` in model.rs |
| `title` | TEXT | |
| `starts_at` | TEXT | Naive local instant: `YYYY-MM-DD` (all‑day) or `YYYY-MM-DDTHH:MM:SS` |
| `ends_at` | TEXT | Exclusive (all‑day: next day; timed: after start) |
| `all_day` | INTEGER | 0 or 1 |
| `location` | TEXT? | |
| `notes` | TEXT? | |
| `source` | TEXT | `manual` / `rhythm` / `feed` / `comms` / `scouting` / `luma` / `google` |
| `external_id` | TEXT? | Dedupe key per source |
| `rhythm_id` | TEXT? → rhythms(id) ON DELETE SET NULL | |
| `payload` | TEXT (JSON) | Inert provider evidence |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

Unique indexes: `(source, external_id)` partial where non‑null, `(rhythm_id,
starts_at)` partial where non‑null (materialization idempotency).

### `calendar.contexts`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:context:…` |
| `kind` | TEXT | `uncertainty` / `transition` / `preference` / `planning_gap` / `note` |
| `title` | TEXT | Reader-facing summary |
| `details` | TEXT? | Why the context matters |
| `valid_from`, `valid_until` | TEXT | Inclusive `YYYY-MM-DD` window |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

Contexts are queried by overlap with the visible range. They stay separate from
entries deliberately: an uncertain colloquium window or a move period should
shape recommendations without pretending every day is busy.

### `calendar.rhythms`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | `cal:rhythm:…` |
| `kind` | TEXT | |
| `title` | TEXT | |
| `location` | TEXT? | |
| `byweekday` | TEXT | CSV: `"tu,we,th"` |
| `start_time`, `end_time` | TEXT? | `HH:MM`; both or neither (all‑day) |
| `valid_from`, `valid_until` | TEXT | `YYYY-MM-DD` |
| `active` | INTEGER | |
| `created_at`, `updated_at` | TEXT | Epoch seconds |

### `calendar.google_exports`

The per‑entry export opt‑in (Phase E). A row **is** the opt‑in — the table
starts empty, so nothing exports by default, and opting out deletes the row.

| Column | Type | Notes |
|---|---|---|
| `entry_id` | TEXT PK → entries(id) ON DELETE CASCADE | Deleting the entry withdraws the opt‑in |
| `google_calendar_id` | TEXT | Recorded per row, so a later config change cannot relocate an event already pushed |
| `google_event_id` | TEXT? | `NULL` until the first push; afterwards what makes the next push an update, not a duplicate |
| `pushed_at`, `created_at` | TEXT | Epoch seconds |

### Kinds (extensible, not a CHECK constraint)

`busy`, `work_onsite`, `work_remote`, `away`, `event`, `nightlife`, `deadline`, `travel_ok`,
`draft` are the well‑known kinds. Kinds are stored and validated as machine‑safe tokens
(`[a-z0-9_]+`, 1‑40 chars) — new kinds for day‑planning blocks (Phase F) land
without a migration. The correlation layer (Phase C) maps known kinds onto
feasibility verdicts; unknown kinds are treated as neutral.

`draft` is the one kind that carries state, not meaning: it is what an
imported Google event lands as until the operator adopts it. See § Drafts.

### Sources

`manual`, `rhythm`, `feed`, `comms`, `scouting`, `luma`, `web`, `google`. Same free‑text approach.

### Reviewed Comms proposals

A completed, explicitly run Comms analysis can hand one resolved date or dated action to
Calendar through the existing external-entry upsert. The reader requires a separate click for
each candidate. Calendar stores it with `source = comms`, `commitment = possible`, and a stable
content-derived external id, so retries update rather than duplicate and the proposal blocks no
time. Dated actions use the neutral `deadline` kind; extracted event dates use `event`.

The payload follows
[`calendar-proposal-provenance-v1`](../../schemas/calendar-proposal-provenance.schema.json): it
retains the Comms item/job reference, trust class, importance rationale, and only the bounded
evidence already returned by the reviewed analysis. It contains no provider endpoint, key, or
original mail body. The ordinary Calendar proposal rail remains the only adoption surface.

### Commitment: how binding an entry is

> **Why a second axis and not a `draft` kind**

`kind` says *what* an entry is. `commitment` says *whether it is happening*:
`possible`, `planned`, `committed`. The two are orthogonal, because a holiday
can be an idea or a booked flight and an event can be a bookmark or a paid
ticket.

This used to be spelled `kind = "draft"`, and this section used to argue for
that. The argument had a stated cost — "a draft cannot simultaneously carry the
kind Google suggests", so an out‑of‑office could not be an `away` — and it only
ever covered the Google case. Everything scouting promoted landed as a plain
`event`, which `correlate` read as a hard block, so an Impact Lab in Atlanta
nobody was going to attend deleted two days from August.

What survives from the old argument is its third point, and it survives
unchanged: **the dedupe key outlives adoption.** `(source, external_id)` is
what the partial unique index enforces, and raising the commitment changes
neither field, so the next import still recognises the entry.

Three consequences worth knowing:

1. **Adoption is raising the commitment**, not re‑kinding. Correcting the kind
   Google guessed is a correction, not a takeover, and `google::decide` treats
   it as such.
2. **`possible` can never block a day.** `correlate::impact` is
   `min(kind_ceiling, commitment_ceiling)`, so the commitment caps what an
   entry is allowed to cost whatever its kind.
3. **The column is a closed set with a CHECK constraint**, unlike kinds. An
   unknown kind has a safe reading; an unknown commitment has none, because
   deciding how hard a day is blocked is the entire job of the field.

The migration defaults to `possible`, not `committed`: a bug in that direction
hands out a free day you can check, rather than a silently blocked one you
never see.

### What Phase E settled

Phase E was named as the forcing function for real offsets, and it forced this:
**the offset becomes real at the boundary, and the store stays naive.**

Google emits genuine RFC 3339 (`2026-08-14T18:00:00+02:00`). `src/zone.rs`
converts it, in both directions, applying the offset *in effect at that
instant* rather than a fixed one:

- **Import** — Google's offset‑bearing instant → naive wall time in the
  operator's `home_timezone`, which is what lands in `starts_at`/`ends_at`.
- **Export** — the stored naive wall time → RFC 3339 with the offset that wall
  time actually had, plus the zone name, which is what Google's `dateTime`
  wants.
- **Nothing is lost.** The original offset‑bearing strings and Google's own
  `timeZone` sit byte‑for‑byte in `payload`, so a read path that later needs
  true instants can recover them without a re‑import.

That is exactly the plan this section carried before Phase E existed — store the
original in `payload`, layer normalization where a read path needs it — and it
survived contact. The alternative, offsets in the columns, was not close: the
window query compares `starts_at`/`ends_at` lexicographically in SQL,
`correlate::instant_minutes` parses them as naive, and the dashboard's
`types.ts` does its own date math on them. An offset suffix breaks all three.
Making instants first‑class is a v2 with a `timezone` column and a migration,
not a sync feature.

**Home timezone is configuration with no default.** `home_timezone` in the
overlay's `calendar.json`; both sync runs refuse until it is set. A wrong zone
writes every imported event an hour or more off, silently and plausibly, which
is worse than a refusal — the same call `scouting` made for the Luma promotion.

**The two DST edges, named.** These are pinned by tests in `src/zone.rs` and
`src/google.rs`, not left to be discovered:

| | Spring forward (29 Mar 2026, 01:00 UTC) | Autumn back (25 Oct 2026, 01:00 UTC) |
|---|---|---|
| What the wall clock does | skips 02:00–03:00 | replays 02:00–03:00 |
| Importing an *instant* inside it | fine — 00:59Z → 01:59, 01:00Z → 03:00 | fine, but two instants an hour apart both read 02:30 |
| Importing an event *spanning* it | fine — one real hour reads as two | **refused, by name**: one real hour reads as zero, and a zero‑length entry is not storable |
| Exporting a wall time inside it | **refused, by name**: no clock in this zone ever showed it, so there is no offset to stamp | resolved to the *first* occurrence (summer offset) — a stated convention, because there is no fact of the matter |

Both refusals are loud and specific, never bottoming out in the generic
"ends_at must be after starts_at". They are the honest edge of a naive model,
and naming them is the price of keeping it.

**Rhythms** store start/end as wall `HH:MM` and materialize into naive local
times. A rhythm timed at `09:00` produces `2026-08-14T09:00:00` for every
instance — no DST skip, no ambiguity. V2 may introduce an explicit `timezone`
column.

**Ends are exclusive.** An all‑day entry spanning only 2026-08-14 stores
`starts_at = "2026-08-14"`, `ends_at = "2026-08-15"`. This makes window
overlap queries simple: `substr(starts_at,1,10) < $to AND substr(ends_at,1,10) >
$from`.

## Why this shape

### A calendar capability (not trips, not scouting)

Trip plans (`trips`) own itinerary state — transport, stays, activities —
after you decide to go somewhere. Availability is the *before* state: what
your schedule allows. Putting availability into `trips` would make trips own
the user's calendar, which is exactly the domain‑ownership argument the trips
README makes for why plan items don't live in `transit` or `scouting`.
`scouting` finds and scores opportunities; it doesn't own your time. A
calendar domain is a third axis, not a second store in an existing one.

### Kinds are data, not a constraint

The transit README documents a real bug where a CHECK constraint
(`trigger_reason`) needed a migration when a new valid value appeared. Kinds
are the same shape — extensible — and a CHECK constraint would create the same
migration tax on every new kind. Instead, the database stores TEXT, the Rust
type validates shape (token/empty/len), and a `KNOWN_KINDS` constant documents
what the hardcoded correlation knows about. A kind unknown to the correlation
layer is simply treated as neutral — it still stores, queries, and renders
fine. The constraint lives at the thin edge where a verdict is produced, not
at every insert.

### Materialized rhythms, forward‑only

Rhythms are materialized on create and update (and re‑materialized on
demand via `POST /api/rhythms/:id/materialize`). Materialized instances store
`rhythm_id`, so a reader can distinguish "this is what my rhythm says" from
"this is a one‑off I painted." Re‑materialization is **forward‑only**: it
affects dates ≥ today. Past entries stay as historical record — editing a
rhythm doesn't rewrite history. User overrides are respected because any
patch to a rhythm‑linked entry detaches it (`rhythm_id → NULL`), so the next
re‑materialize won't touch it.

### No hard filter on conflicts (soft verdicts)

Correlation annotates an opportunity with `free` / `needs-travel-day` /
`conflicts` — it never hides it. A good enough event is worth moving a remote
day for, and that decision belongs to the operator, not to an opaque filter.
The same rule shapes the windows endpoint: a window that costs a travel day is
still returned, with the days it would cost named on it.

### Calendar hands the days over; transit never asks for them

Constraining a fare search to feasible days could run either direction. It runs
this one: calendar computes the windows and publishes them, transit grew a
`plan --dates` flag that searches exactly the days it is given, and the caller
moves the list between them.

The other direction — transit calling calendar when it plans — would put a
second capability's availability rules inside a fare searcher, add a
`requires` edge to a capability transit works fine without, and make a fare
search fail or silently widen whenever the calendar service is down. Transit
already took its dates as flags; it now takes a better-chosen set of them and
still knows nothing about why those days and not others. Calendar owns
availability, transit owns fares, and the only thing crossing between them is a
list of dates.

Concretely, the hand‑off is a shell one‑liner rather than a code path:

```bash
transit plan --from <EVA> --destinations "München" --dates "$(
  curl -s '127.0.0.1:8087/api/windows?from=2026-08-01&to=2026-09-01&min_days=2' |
    jq -r '[.windows[].days[]] | join(",")'
)"
```

`--dates` is mutually exclusive with `--date-from`/`--date-to` (a silent
precedence rule between "all of September" and "these four days" would hide
which one ran), and `--max-queries` still caps the fan‑out — a month of feasible
days is thinned to the query budget the same way a window is sampled. The flag
lives in `capabilities/transit/src/main.rs`; nothing there names this
capability.

## HTTP surface

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | |
| GET | `/api/entries?from=&to=&kind=` | Window query, optional CSV kind filter |
| GET | `/api/proposals?from=&to=` | External, non-Google `possible` entries waiting for a Calendar decision; manual soft blocks are excluded, and so is anything already adopted under another key (see Duplicate suppression) |
| GET | `/api/content/calendar/:id` | The entry as [`content-item-v2`](../../schemas/content-item.schema.json), for the shared reader. Same path shape comms serves — see Content contract |
| POST | `/api/entries` | Create entry |
| GET | `/api/entries/:id` | |
| PUT | `/api/entries/external` | Idempotent provider contribution; requires `source` + `external_id` |
| PATCH | `/api/entries/:id` | Any patch to a rhythm‑linked entry detaches it |
| DELETE | `/api/entries/:id` | |
| GET | `/api/rhythms` | |
| POST | `/api/rhythms` | Creates + materializes future instances |
| GET | `/api/rhythms/:id` | |
| PATCH | `/api/rhythms/:id` | Re‑materializes future instances |
| DELETE | `/api/rhythms/:id`?delete_instances=true | |
| POST | `/api/rhythms/:id/materialize` | Idempotent re‑materialize |
| POST | `/api/verdicts` | Feasibility verdicts for a batch of dated candidates (see Correlation contract) |
| GET | `/api/windows?from=&to=&min_days=` | Runs of days travel is possible in; `from` inclusive, `to` exclusive |
| GET | `/api/trip-drafts?from=&to=&max_gap_days=` | Recomputed, place/time-clustered journey candidates; no write |
| POST | `/api/trip-drafts/materialize` | Explicitly create one Trips plan from named `entry_ids`; Calendar records an idempotence ledger only after Trips accepts it |
| POST | `/api/google/import` | Pull the configured Google calendar in as drafts. Body `{"dry_run": false}` |
| GET | `/api/google/drafts?from=&to=` | Google-source entries still at `possible`, the draft inbox's ownership contract |
| POST | `/api/google/import-preview` | Read-only, date-bounded candidate review. Body `{"from":"YYYY-MM-DD","to":"YYYY-MM-DD"}`; maximum 90 days |
| POST | `/api/google/import-selected` | Re-fetch and import only reviewed selections as drafts. Every selection carries its Google `updated` revision; a changed event returns 409 for re-review |
| POST | `/api/google/export` | Push the opted-in entries, and only those. Body `{"dry_run": false}` |
| GET | `/api/google/exports` | The export opt‑in ledger |
| PUT | `/api/entries/:id/google-export` | Opt one entry in. Body `{"google_calendar_id": …}` optional |
| DELETE | `/api/entries/:id/google-export` | Opt out. The Google event it already created is left alone |
| GET | `/api/markdown/sources` | The declared markdown event sources, with their resolved roots |
| POST | `/api/markdown/preview` | Read-only scan of one source. Body `{"source": …}`; returns the candidates **and** every refusal with its reason |
| POST | `/api/markdown/import` | Import an explicit selection. Body `{"source": …, "external_ids": [...]}`; re-scans first, so an id the notes no longer offer is a 400 naming it |

Confirming a draft is `PATCH /api/entries/:id {"kind": "event"}` and dismissing
one is `DELETE /api/entries/:id` — no new verbs, because re‑kinding already was
one. Listing what is waiting is `GET /api/entries?from=&to=&kind=draft`.

Both sync routes answer **400 with a named error** when the home timezone, the
calendar id or the credential is missing — never 200 with an empty report. A
selected import answers **409** when Google changed an event after the review;
the user must inspect the current version before Axon writes it.

## Content contract

An entry also renders as [`content-item-v2`](../../schemas/content-item.schema.json)
at `GET /api/content/calendar/:id`, built in `src/content.rs` — pure over one
`Entry`, so the store is untouched and nothing is written. `libs/content-item`
carries the argument for sharing the *reader* contract while the tables stay
apart.

Two rules this projection holds to:

- **It never ranks.** `relevance` and `evaluation` stay empty, and the schema
  enforces that for `source = calendar`. An entry is something the operator
  already decided about; `commitment` is its triage axis and it surfaces as
  `status`. Scouting's promotion no longer copies a score across either — see
  `calendar_promote::evidence_payload`.
- **`summary` and `notes` are different things.** `payload.about` says what the
  thing *is*; `notes` is why the operator cares, and no machine writes it.

`payload.links[]` (and `payload.url`) become the contract's shared `links[]`, so
the mail carrying a ticket, a Luma event page and a map pin all render through
one block rather than a per-source view.

The extension also carries `entry_source` and `rhythm_id`. Neither is display
data: they decide which *actions* a reader may honestly offer. An entry imported
from Google must not offer to export back to Google, and a rhythm instance is
never exported individually — without those two fields a reader would have to
guess, and would guess wrong for exactly the entries where it matters.

### Editing from the reader

The reader edits everything the calendar form does: kind, commitment, title,
location, note, dates, times, the all‑day toggle, the Google export opt‑in, and
delete. The date rules are **not** reimplemented there — `whenOf` / `whenPatch` /
`whenError` in `dashboard/src/lib/calendar/types.ts` are the single
implementation and the form was moved onto them at the same time.

That matters because the two representations disagree by design: **the store's
`ends_at` is exclusive, every form shows an inclusive end.** An all‑day entry
ending `2026-08-16` covers the 15th, and a surface that forgets the conversion
silently moves the entry by a day. One implementation, covered by
`dashboard/vite/calendar-when.test.ts`.

The path shape mirrors comms' `/content/:source/:id` on purpose: one contract
served under two URL shapes would reintroduce, one layer down, exactly the
duplication the contract removes. The dashboard resolves an item from its source
alone (`CONTENT_BASE` in `dashboard/src/lib/api.ts`).

## Duplicate suppression

Entry identity is `(source, external_id)`, which dedupes a source against
itself and nothing else. The same party scraped off the venue's own site and
imported from Google therefore lands as two rows sharing no key — one the
operator already adopted, one still `possible`, and the month grid drew both
while the inbox kept announcing the second as new.

`GET /api/proposals` filters those out at **read** time: a `possible` entry that
overlaps an already-decided entry (`commitment <> possible`) and carries the
same title once case, diacritics and punctuation are removed is not proposed.
Place is deliberately not part of the match — 13 of 62 events in the last live
Luma sweep carried none, which would exempt exactly the entries most likely to
arrive twice.

Read-time rather than a merge at ingest, because a merge would have to pick a
winner between two sources' versions of one event and could not be undone. This
leaves both rows intact, so a match the rule gets wrong costs a proposal that
returns the moment the rule is corrected — never data. The rule lives in
`correlate::is_same_event`, beside the key-based `is_same_thing` it backstops.

## Correlation contract

The correlation layer (effectively the first half of Phase 4 as `capabilities/store/README.md` defines it, which
originally described people/friends windows) is Phase C. Built in
`src/correlate.rs`: a stable facade over pure functions, so the verdict rules
are unit‑testable without a database and the store stays the only thing that
queries. Its modules follow the domain phases: `feasibility.rs` owns the impact
policy, `events.rs` owns identity and overlap evidence, `windows.rs` owns query
and feasible windows, and `trips.rs` owns journey clustering. Cross-phase
contract tests remain together in `test_suite.rs` beside those modules.

**Verdicts.** A candidate is a date range plus the caller's own id — a
`scouting.opportunity` id from Feed's Discover view. Every entry overlapping
that range contributes an impact, and the verdict is the worst of them:

| Entry kind | Impact | Why |
|---|---|---|
| `away`, `event` | `conflicts` | You are elsewhere, or already committed to something concrete |
| `work_onsite`, `busy` | `needs-travel-day` | Movable — go remote, or drop the block. Still offered, with the cost named |
| `work_remote`, `travel_ok` | `free` | Location‑flexible by definition, or the explicit yes |
| Google-source `possible` entry | `free` | An unadopted import. Neutral as policy, not by accident — see § Drafts |
| anything else | `free` | A kind this layer never heard of is neutral, never a block |

No overlapping entry at all is `free`. The neutral‑unknown rule is the other
half of "kinds are data, not a constraint" below: a Phase F day‑planning kind
lands without a migration *and* without silently blocking travel on every day it
covers.

**The candidate's own entry never conflicts with it.** Once promotion has run,
the calendar holds an `event` entry for that opportunity. Correlating the
opportunity would otherwise make every saved opportunity conflict with itself,
so an entry whose `external_id` equals the candidate id is excluded and reported
as `already_in_calendar` instead. That is the free clause's "or only
`travel_ok`/`event`"; a *different* confirmed event on the same day is still a
conflict, because you cannot attend both.

The match accepts either the entry's provider `external_id` or the originating
Scouting id in `payload.opportunity_id`. That preserves provider-level dedupe
while allowing Feed to ask with its namespaced opportunity id; either key
recognizes the candidate's own entry instead of reporting a self-conflict.

**A verdict carries its evidence.** Every overlapping entry comes back with its
id, kind, title, range and impact, strongest first — including the neutral ones,
which explain a day without driving it. That is what lets a UI say "you have
on‑site work that day" instead of showing a badge with no argument.

**Foreign timestamps are read as local wall time.** Providers emit
`2026-07-10T18:00:00.000Z` (Luma), `2026-10-23T00:00:00+00:00`
(euro‑hackathons) and naive `2026-08-01T08:00:00` (transit) for the same kind of
fact. The zone designator and any fractional seconds are dropped and the clock
face is taken as written — the single‑home‑timezone call the time model above
already makes. Nothing is converted *here*, deliberately: the callers of this
endpoint (Feed's Discover, `transit`) already hand in instants that were
produced in the home zone, so converting them again would move them. The path
that does convert is the Google import, which receives foreign offsets and runs
them through `src/zone.rs` — see § Time model > What Phase E settled. A
candidate with no end, or one whose provider repeated the same instant twice,
covers the whole start day: an event without an end is a day, not an instant.

**Feasible windows.** `GET /api/windows` groups a span into maximal runs of days
where travel is possible at all — every day whose verdict is not `conflicts`.
Each window carries its days, the ones inside it that would cost a travel day,
and the worst verdict in the run, so the soft cost stays visible instead of
being filtered out. `min_days` drops runs too short to be worth searching.

Phase C's third piece — clustering two or more events by place and time proximity — is
`correlate::cluster_trips`, served at `GET /api/trip-drafts?from&to`
(`max_gap_days` defaults to 5, `home` falls back to `home_city` in the config).
A draft is recomputed per request rather than stored, because it is a function
of entries that can all move. Turning one into a real `trips.plan` remains a
deliberate act: Calendar calls Trips' public HTTP API and records the result in
its own idempotence ledger; it never writes into Trips' store directly.

Sources write venue lines (`Nordwerk, Bonn`; `Beispiel Innovation Hub,
Musterdeich 15, 20097 Hamburg`), not bare cities, and two rules read them differently
on purpose:

- **The clustering key is the city**, via `city_of` — the segment carrying a
  postal code, else the last segment. You travel to a city and then walk, so
  making the venue part of the identity split one Hamburg journey into two
  places and left both below the two‑event floor. The draft is labelled by city;
  the venues stay on the member entries.
- **The home test reads the raw place, every comma‑separated segment of it.**
  Not the whole string, which never matched a venue line, and not a substring
  search, which would read `Moxy Köln/Bonn Flughafen` as home and cancel a real
  trip to Köln. Forgiving on purpose: a false home can only drop a day the
  operator was home for anyway, while a forgiving *grouping* key would merge two
  cities into one journey.

A segment ending in digits stays an address line, so a house number never
becomes a city. `city_of`'s limit is `Berlin, Germany`, which yields `Germany`;
it does not arise because Luma supplies `payload.city`, which `place_of` prefers
outright. If it ever does, record `payload.city` — do not teach `city_of`
geography.

Verdicts are soft — see "No hard filter on conflicts" above.

## Markdown event import

`src/markdown_import.rs`. A bounded migration path for the case calendar had no
answer to: an operator holding a directory of structured markdown event notes.
Without one, they either keep two editable stores of the same events forever, or
write into calendar's tables behind the capability's back. Both are worse than
an importer.

Notes are a **contributing** source in exactly the sense Google is. The import is
one-directional by construction: there is no API here that could be pointed at a
note store as a writer, and deciding a note has been superseded stays the
operator's call, made after they have seen the entries land.

### The three steps, which are three endpoints

Collapsing them would lose the middle one — see what is there, say what to
write, write exactly that.

1. `POST /api/markdown/preview` reads a declared source and **writes nothing**.
   It returns the entries it would write and every note it refused, each with
   its reason. A note that is not an event at all (a MOC, a template, prose in
   the same directory) is skipped silently; an *event* note that cannot be
   honoured is reported, because that one is a decision the operator has to see.
2. `POST /api/markdown/import` takes the ids from that review. There is no
   "import everything" flag: importing the whole preview means sending the whole
   preview's ids back, which keeps *I reviewed this* and *write it* the same act.
3. It re-scans before writing rather than trusting the caller's copy. The file
   is the source of truth and may have changed since the review; an id the fresh
   scan no longer offers is a 400 naming it, never a silent skip.

### Identity, and why a second run is safe

An imported entry is keyed `(source, external_id)` where `source` is the
declared source id and `external_id` is the note's path relative to the declared
root. Both halves are derived, neither is invented. Writes go through
`upsert_external_entry`, so the partial unique index already behind
`PUT /api/entries/external` makes a re-import an update. That is what makes the
import safe to re-run, which is what makes it safe to run at all.

### The mapping, and where it refuses

| Note | Entry |
|---|---|
| `type: event` | required — anything else is not a candidate |
| `summary` / `title`, else the filename | `title` |
| `start` (or `date`) | `starts_at` |
| `end`, inclusive | `ends_at`, **exclusive**: the day after |
| no `end` | a single day |
| `location` | `location` |
| `status: confirmed` / `completed` | `committed` |
| `status: planned` | `planned` |
| anything else, including absent and unknown | `possible` |

**Uncertain never blocks a day.** A note store's status vocabulary is not
calendar's commitment model, and only two of its values claim "this is
happening". `cancelled` needs no special case: it is neither, so it lands on
`possible` — visible as evidence, blocking nothing.

**Malformed times fail closed.** No start, an end before its start, a zero-length
timed entry, or one date-only side and one timed side: reported and skipped, not
guessed. An event silently placed on the wrong day is worse than one that
visibly did not import.

**The inclusive→exclusive end is the one value the import changes** rather than
carries, and it is the reason a one-day note (`start == end`) does not become a
zero-length entry. It has its own test.

**`payload` is inert evidence, and it is not the note.** It carries enough to
audit the mapping later: which source, which note, and the fields the mapping
consumed. Deliberately not the body. Copying the prose into a database column
would recreate the two-stores problem this exists to end.

### Containment

Roots and globs come from the operator's private config; nothing here hardcodes
a note-store path. Every read goes through
[`//libs/markdown-root`](../../libs/markdown-root/README.md): a pattern that is
absolute or contains `..` is refused before the filesystem is touched, and every
resolved file is proven inside the declared root *after* symlink resolution.

## Google sync contract

Phase E, built in `src/google.rs` (every rule, pure and fixture‑tested),
`src/google_sync.rs` (the stable facade), its `auth.rs`, `transport.rs`,
`import.rs`, and `export.rs` domain modules, and `src/zone.rs` (offsets).
Cross-phase review, duplicate, revision, import, and export tests live beside
them in `google_sync/test_suite.rs`. Google is a **contributing** calendar,
never a second source of truth.

### Import

`POST /api/google/import` pulls `events.list` for the configured calendar over
the window `[today − import_days_back, today + import_days_ahead)` and lands
each event as a draft.

- **`singleEvents=true`.** A recurring series is expanded into instances, each
  with its own stable id (`base…_20260819T063000Z`). That id is the
  `external_id`, so twelve instances are twelve entries and a re‑import updates
  those twelve rather than adding more. The series id is kept in `payload`.
- **Dedupe is the existing upsert.** `(source = "google", external_id)` through
  `PUT /api/entries/external`'s semantics and the partial unique index behind
  them. Running an import twice produces one entry per event; running it twice
  over an *unchanged* calendar writes nothing at all, because the mapping is
  byte‑stable and a no‑diff refresh is skipped.
- **All‑day events need no arithmetic.** Google's `end.date` is exclusive, which
  is this capability's convention too.
- **`payload` is the evidence, and it is inert.** Google's event verbatim: the
  original offset‑bearing instants, its `timeZone`, `iCalUID`, `htmlLink`,
  `transparency`, `eventType`, `recurringEventId`. Deliberately no "imported at"
  stamp — a timestamp would make every run a write.
- **No date is guessed.** An event this layer cannot read exactly — no end, both
  a `date` and a `dateTime`, an instant with no offset, a span across the autumn
  transition — is reported as skipped with its reason and left alone. The rest
  of the run still imports.

### Conflict policy: Axon wins

One rule decides it, and adoption is the signal (§ Drafts):

| Google says | Axon holds | What happens |
|---|---|---|
| an event | nothing | `create` — a new draft |
| an event | a `draft` | `refresh-draft`, or nothing if it did not move |
| an event | anything re‑kinded | **`keep-axon-version`** — no write; what Google now says is reported on the outcome |
| `cancelled` | a `draft` | `drop-draft` — the source withdrew something never adopted |
| `cancelled` | anything re‑kinded | **`keep-cancelled-axon-version`** — the entry stays, the operator is told |
| `cancelled` | nothing | `skip` |

A divergence is *reported*, never silently applied and never silently dropped.

### Export

Opt‑in per entry, and only per entry. `PUT /api/entries/:id/google-export`
writes the ledger row; `POST /api/google/export` pushes exactly the rows that
exist. There is no "export everything" path and no default‑on flag, because the
empty table is the default.

- **First push inserts, later pushes patch** — the ledger remembers the Google
  event id, so a second run updates instead of duplicating.
- **The ledger's calendar id wins over the current config**, so re‑pointing
  `google.calendar_id` cannot relocate an event that already lives somewhere.
- **Two entries may never be opted in**, refused at the boundary with a reason:
  one whose `source` is `google` (it would duplicate the event Google already
  owns, and the next import would pull the copy back as a third thing), and a
  materialized rhythm instance (push the rhythm's meaning as its own recurring
  event instead of shipping generated instances).
- **Opting out does not delete the Google event.** Deleting something off
  someone's calendar as the side effect of a toggle is not a call this
  capability makes.

### The credential

Three keys — `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`
— in a plain `KEY=value` file in the private overlay, the same shape
`capabilities/comms` uses. Default `$SJEL_PERSONAL_ROOT/config/calendar.env`,
overridable with `google.env_path`. Nothing is read from this repo, and no token
value is ever logged — not in an error, not in a report, not in a failed‑refresh
body (Google puts token material in some of those).

`calendar.events` is the scope; it covers reading and writing events, so one
grant serves both runs. `capabilities/comms/auth/get-refresh-token.ts` already
requests it, so pointing `google.env_path` at `comms.env` reuses that grant
instead of minting a second one.

**Absent or broken credentials fail loudly and by name**, never as a no‑op:
which key, which file, and what to do about it. An HTTP failure from Google
names the likely cause (401 → revoked, 403 → missing scope or unshared
calendar, 404 → wrong `calendar_id`, 429 → rate limit).

## Phases

### A — Luma live‑verify
- [x] Add an explicit “add to Calendar” action for dated Scouting
      opportunities in Feed > Discover; confirm in the calendar entry form and
      promote through an idempotent source/external-id upsert
- [x] Run the existing `capabilities/scouting` luma adapter against the calendars
      you actually track. Done 2026-07-30 — **the adapter did not work.** What
      broke and what fixed it is written up in `capabilities/scouting/README.md`
      § Verdict. One calendar is tracked so far (Claude Community Events);
      which others to track is still open, see below.
- [x] Decide per calendar: ICS feed (Luma offers per‑calendar ICS subscribe)
      vs. fixing the scrape adapter. **Not a mix, as it turned out — fix the
      scrape, for both surfaces.** Evidence below.
- [x] When a Luma event is saved in scouting, promote into a `calendar.entry`
      with `source = luma` and `event` kind — that completes the pipeline from
      discovery to availability annotation. `scout --promote-calendar`; see
      `capabilities/scouting/README.md` § Calendar promotion for the contract.

#### ICS vs. scrape: why scrape won

Luma offers both for a calendar and both work. They were compared on the one
axis this capability cares about — can an entry be created without guessing?

| | `GET /calendar/get-items` (JSON) | `GET /ics/get?entity=calendar&id=…` (ICS) |
|---|---|---|
| Dedupe key | `api_id` (`evt-…`), the id the adapter already mints | `UID` (`evt-…@events.lu.ma`) — recoverable, one parse further away |
| Schedule | UTC instant **plus** the event's IANA `timezone` | `DTSTART`/`DTEND` in UTC, no `TZID`, no `VTIMEZONE` |
| Location | structured `city`/`country`/`address` | one free‑text `LOCATION` line, `GEO` on most |
| Scope | future events only | the whole history — 290 VEVENTs for 62 future ones |
| Parser cost | the shape the adapter already deserializes | a new ICS parser (line unfolding, escaping, property params) |

The JSON endpoint returns the **same `entries[].event` object** as the discover
feed the adapter already read, so supporting it cost one URL and no new parsing.
ICS would have cost a parser and still delivered less: with no per‑event zone,
turning `DTSTART` into local wall time needs a geo→timezone lookup Axon does not
have — precisely the guessing this capability refuses. ICS stays the fallback
for a calendar `get-items` does not serve; none encountered yet.

#### Still open: which calendars to track

One is tracked (`claude-community` / `cal-TOpA5LAFfuDeFpu`, the public Claude
Community Events calendar — 62 future events, every one with usable schedule
metadata). Declare more as `luma-calendar` entries in `scouting.json`'s
`sources[]`; each needs the calendar's `cal-…` api id, since Luma publishes no
public slug lookup.

### B — Core capability + dashboard month-grid
- [x] Store tables (entries, rhythms, indexes, dedupe)
- [x] HTTP CRUD for entries and rhythms
- [x] Rhythm materialization (forward‑only, idempotent)
- [x] Dashboard month‑grid view — paint timeframes, create rhythms (see Dashboard surface above)
- [x] Entry edit/delete from the month grid (entry chip → edit dialog)
- [x] Drag‑to‑paint multi‑day blocks across the grid

### C — Correlation
- [x] Feasibility verdicts on dated candidates — kind→impact mapping, evidence,
      unknown kinds neutral, the candidate's own promoted entry excluded
- [x] Constrain transit fare search to feasible windows — calendar publishes the
      days, `transit plan --dates` searches exactly those (see the why‑block)
- [x] Verdict endpoint in the correlation layer — `POST /api/verdicts` (batch)
      and `GET /api/windows`
- [x] Discover renders the batch verdict as a soft badge with evidence; a
      conflict never hides the opportunity or blocks calendar promotion

### D — Combining + trip assembly
- [x] Geo/time clustering of saved events
- [x] Reviewed draft trip → `trips.plans` through Calendar's HTTP boundary
- [x] Materialised plan opens directly in the existing Travel workspace
- [ ] Dashboard Trips workspace renders combined trip

### E — Google Calendar sync (two‑way)
- [x] Import: new events tagged `source = google`, dedupe by external_id;
      imported events arrive as drafts you confirm/re‑kind/dismiss. See
      § Google sync contract and § Drafts.
- [x] Export: per‑event opt‑in → push to Google Calendar. The
      `calendar.google_exports` ledger is the opt‑in; nothing exports by default.
- [x] Conflict policy: Axon is the source of truth; sync resolves toward Axon.
      Implemented in `google::decide`, table in § Conflict policy.
- [x] Real offset handling, both directions, both DST edges — § Time model >
      What Phase E settled.
- [x] **Connect a real account.** OAuth refresh and a live, read-only preview
      against the configured account were verified on 2026-08-01. Import remains
      explicit; no bulk entries were written during that check.
- [x] Dashboard surface for a date-bounded Google import review: it explains
      existing entries and likely exact-title/time duplicates, then imports only
      explicit, revision-checked selections as drafts.
- [x] Dashboard draft inbox: 90-day Google-source/possible queue, with
      re-kind + planned adoption or Axon-only removal.
- [x] Per-entry Google export toggle plus a dry-run review and explicit push.
      The toggle writes only the opt-in ledger; the separate sync review is
      the only dashboard action that contacts Google.
- [ ] Watch/incremental sync (`syncToken`, push notifications). This build
      re‑lists the window every run, which is fine at one operator's volume and
      wrong at scale.

#### Connecting an account

The credential is a configured input, never something this capability acquires
on its own. In order:

1. **Reuse the grant you already have, if you have one.** `bw unlock`, then
   `bun capabilities/comms/auth/get-refresh-token.ts` already writes
   `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_REFRESH_TOKEN` into
   `axon-overlay/config/comms.env` **with the `calendar.events` scope
   included**. If that file exists and its token was minted after that scope was
   added, skip to step 4 and point `google.env_path` at it. If the token
   predates the scope, re-run the script once (`prompt=consent` is set, so it
   returns a fresh refresh token) — that also keeps Gmail working.
2. Otherwise, in Google Cloud Console: create (or reuse) a project, enable the
   **Google Calendar API**, and create an **OAuth 2.0 Client ID** of type
   *Desktop app*. Add yourself as a test user on the consent screen so the grant
   does not expire in seven days.
3. Mint a refresh token with `scope=https://www.googleapis.com/auth/calendar.events`
   and `access_type=offline&prompt=consent`, and write the three keys into
   `axon-overlay/config/calendar.env` as `KEY=value` lines, mode `600`. The
   comms bootstrap is the reference implementation.
4. Create `axon-overlay/config/calendar.json` from
   `capabilities/calendar/calendar.config.example.json`, setting at minimum
   `home_timezone` and `google.calendar_id` (`primary`, or a secondary
   calendar's `…@group.calendar.google.com` id), plus `google.env_path` if you
   reused `comms.env`.
5. Restart `calendar-server` and dry-run the import:
   `curl -s -X POST 127.0.0.1:8087/api/google/import -H 'content-type: application/json' -d '{"dry_run":true}'`.
   It reports what it *would* do and writes nothing. A misconfiguration comes
   back as a 400 naming the key and the file.
6. Run it for real (`"dry_run": false`), then look at what landed:
   `curl -s '127.0.0.1:8087/api/entries?from=…&to=…&kind=draft'`. Confirm one
   with `PATCH /api/entries/:id {"kind":"event"}`, dismiss one with `DELETE`.
7. Only if you want the other direction: opt a single entry in with
   `PUT /api/entries/:id/google-export`, dry-run `POST /api/google/export`, then
   run it. Nothing else is ever pushed.

### F — Week and day views
- [x] Week view: seven day columns over 24 hour rows, timed entries at their
      wall‑clock position, all‑day entries pinned to a band at the top
- [x] Day view: the same time grid at one column, taller hours, entry location
      on the chip; rhythm‑derived blocks distinguishable from one‑offs
- [x] View switcher sharing one anchor date across month/week/day
- [x] Create by clicking an empty hour or the all‑day band; edit by clicking a chip
- [ ] Travel‑day view: transit legs + event + buffer times
- [ ] General day‑planning blocks (the stuff that would overload Google Calendar)
- [ ] Not synced to Google — these are Axon‑only detail

## Done looks like

1. ✅ The operator can open a month‑grid view, see which days are open (empty),
   with painted availability windows, rhythm‑generated blocks, and imported
   events.
2. Scouting opportunities are annotated with a feasibility verdict that
   explains (not hides) conflicts: "This event is on 14 Aug; you have on‑site
   work that day — you'd need to take it remote or take a day off."
3. Saved events cluster into trip suggestions: "Two events in Munich in one
   week — want a combined trip 12–16 Aug?"
4. ✅ Google Calendar imports start as a bounded, reviewed draft selection;
   exports remain opt-in per event. The real account connection was verified
   read-only on 2026-08-01.
5. The day view on travel days shows the full picture: departure, connections,
   arrival, event window, buffer, return.

## Config

The five tables live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `calendar`, so they are
`calendar_entries`, `calendar_rhythms`, `calendar_contexts`,
`calendar_trip_materializations` and `calendar_google_exports`
(libs/sjel-store/README.md). PRD Q45 (2026-08-27) moved them there from a Postgres
schema.

Where the file lives is a deployment fact, not a capability one, so a `database_url` left
in `calendar.json` is ignored and `SJEL_CALENDAR_DATABASE_URL` is gone: a file per
capability would drop the cross-capability joins the shared instance existed for.

`SJEL_CALENDAR_PORT` or `SJEL_PORT` → 8087 (default)

Phases A–D need nothing else: all personalization is the data in the calendar
tables themselves. Phase E added the first config file, because a Google sync
cannot work without two personal values and refuses to guess either.

`$SJEL_CALENDAR_CONFIG` → `$SJEL_PERSONAL_ROOT/config/calendar.json` →
`capabilities/calendar/calendar.config.json` (local, gitignored). Template:
`calendar.config.example.json`.

| Key | Default | Notes |
|---|---|---|
| `home_timezone` | **none** | The zone every stored wall time is in. Both sync runs refuse until it is set |
| `google.calendar_id` | **none** | `primary`, or a secondary calendar's `…@group.calendar.google.com` id |
| `google.env_path` | `<overlay>/config/calendar.env` | `KEY=value` file with the three `GOOGLE_*` keys. May point at `comms.env` |
| `google.import_days_back` / `_ahead` | 7 / 120 | Import window, relative to today |
| `google.max_events` | 1000 | Hard bound on one import's paging |

No secret value belongs in `calendar.json` — only the path to the file that
holds them.
