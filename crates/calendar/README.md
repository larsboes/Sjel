# calendar

Sjel's personal calendar layer: the source of truth for what time means on any given day — availability windows for travel, on‑site work, remote‑work blocks, busy periods, events you're attending, rhythms that materialize those blocks, and (later) day‑planning detail.

## Surface

The `calendar` capability manages scheduled time, recurring rhythms, planning contexts, and travel availability:

- HTTP API: `calendar-server` (`src/server.rs`), bound to loopback, default port **8087**:
  - `GET /api/entries?from=YYYY-MM-DD&to=YYYY-MM-DD` — list calendar entries in half-open window.
  - `POST /api/entries` — create manual or external calendar entry.
  - `GET /api/entries/:id` — retrieve entry by ID.
  - `PATCH /api/entries/:id` — update entry kind, title, time, location, notes, or commitment.
  - `DELETE /api/entries/:id` — remove entry and any associated export ledger records.
  - `GET /api/rhythms` — list active recurring rhythms.
  - `POST /api/rhythms` — create rhythm and materialize future entry instances.
  - `PATCH /api/rhythms/:id` — update rhythm schedule and refresh instances.
  - `DELETE /api/rhythms/:id` — delete rhythm, optionally retaining materialized instances detached.
  - `POST /api/feasibility` — evaluate feasibility of candidate dates against existing commitments.
  - `GET /api/feasible-windows?from=YYYY-MM-DD&to=YYYY-MM-DD&min_days=N` — compute viable travel spans.
  - `GET /api/contexts?from=YYYY-MM-DD&to=YYYY-MM-DD` — retrieve active planning contexts.
  - `POST /api/contexts` — register planning context notice (`uncertainty`, `transition`, etc.).
  - `PATCH /api/contexts/:id` & `DELETE /api/contexts/:id` — modify or remove planning contexts.
  - `GET /api/trip-drafts` — clustered travel candidate drafts grouped by place and time proximity.
  - `POST /api/trip-drafts/:id/plan` — materialize reviewed trip draft into Trips plan.
  - `GET /api/google/review` — preview incoming Google Calendar events against local drafts.
  - `POST /api/google/import` — adopt selected Google Calendar events into Sjel entries.
  - `GET /api/google/export-queue` — inspect entries queued for outbound export.
  - `POST /api/google/export` — push opted-in entries to Google Calendar.
  - `GET /content/:source/:id` — unified content contract reader projection.
  - `GET /health` & `GET /ready` — service health and readiness endpoints.

## Why this shape: Orthogonal kind and commitment axes

An entry has two independent axes: `kind` defines what it is (`busy`, `work_onsite`, `work_remote`, `away`, `event`, `nightlife`, `deadline`, `travel_ok`), while `commitment` defines whether it is actually happening (`possible`, `planned`, `committed`) in `src/model.rs` and `src/store.rs`. Conflating status into kind would prevent expressing tentative travel or confirmed appointments cleanly. In `src/correlate/events.rs`, candidate impact is calculated as the minimum of the kind ceiling and commitment ceiling, ensuring that `possible` entries never block a travel window.

## Why this shape: Naive local storage with real timezone offsets at boundaries

Calendar entries are stored using naive local wall-clock timestamps (`YYYY-MM-DD` for all-day spans, or `YYYY-MM-DDTHH:MM:SS` for timed spans) in `src/model.rs` and `src/store.rs`. External protocols such as Google Calendar represent instants with RFC 3339 UTC offsets. Boundary translations in `src/zone.rs` resolve the exact offset in effect at each local instant rather than storing floating UTC timestamps that drift across DST boundaries or daylight saving legislative changes.

## Why this shape: Forward-only idempotent rhythm materialization

Recurring commitments (rhythms) materialize forward onto concrete calendar entries rather than being expanded dynamically on every query (`src/rhythm.rs`, `src/store.rs`). Unique partial indexes on `(rhythm_id, starts_at)` guarantee idempotent upserts. Storing materialized entries preserves individual edits, allows one-off overrides, and decouples query latency from recurrence complexity.

## Why this shape: Explicit opt-in export ledger

Synchronizing personal entries outward to external calendar systems requires an explicit opt-in row in `calendar.google_exports` (`src/google_sync/export.rs`). Entries are never exported autonomously. Deleting a calendar entry cascades to delete its export record, ensuring complete control over externally published schedule data.

## Why this shape: Time-bounded planning context decoupled from time blocks

Planning contexts represent situational constraints such as moving house, thesis writing, or exam periods (`src/model.rs`, `src/store.rs`). They inform opportunity discovery and Home overview rankings over an inclusive date span without creating artificial busy blocks or preventing travel during free hours.

## Verification

Run the calendar test suite:

```bash
cargo test -p sjel-calendar
```

## Considered and declined

- **Dynamic query-time recurrence expansion**: Declined. Evaluating recurring rules at query time introduces complexity and prevents manual overrides on individual instances.
- **Storing UTC instants for local events**: Declined. Appointments and events are planned in local wall-clock time; storing UTC creates unwanted shifts during daylight saving transitions.
- **Autonomous two-way Google Calendar synchronization**: Declined. Third-party calendar changes must be reviewed as drafts before adoption, and local entries require explicit export opt-in.
- **Merging commitment tiers into kind tokens**: Declined. Kind defines activity semantics, while commitment defines availability impact.
