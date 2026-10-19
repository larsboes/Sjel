# Calendar architecture & contracts

Calendar owns temporal entries, bounded recurring rhythms, planning context, and reviewed Google
imports and exports. Start with the [capability README](../../capabilities/calendar/README.md) for
its purpose; use the [model](../../capabilities/calendar/src/model.rs),
[store](../../capabilities/calendar/src/store.rs), and
[HTTP routes](../../capabilities/calendar/src/server.rs) for implementation details.

## 1. Temporal data model

The default SQLite table prefix is `calendar`; the store implementation accepts an explicit
prefix for isolated uses such as tests.

| Table | Contract |
| --- | --- |
| `calendar_entries` | Date-only all-day intervals or timed local wall-clock intervals, with exclusive ends. `kind` and `commitment` are separate axes. |
| `calendar_rhythms` | Weekday rules with required inclusive `valid_from` and `valid_until` bounds. |
| `calendar_contexts` | Notices with an inclusive validity window. These inform planning without occupying time and are independent of entry deletion. |
| `calendar_google_exports` | Per-entry export opt-in, target calendar, remote event ID, and last push time. Deleting an entry cascades its ledger row. |
| `calendar_trip_materializations` | Idempotence ledger linking calendar entries to plans created through the Trips API. |

Entries can span days. Validation requires an end after the start; it does not require stored
entries to split at midnight. The presentation layer can split their display across days.
Kinds remain extensible tokens. Commitment is a closed set: `possible`, `planned`, `committed`.

## 2. Materialized forward-only rhythms

[`rhythm.rs`](../../capabilities/calendar/src/rhythm.rs) materializes matching dates from the
later of `valid_from` and today, through `valid_until` inclusive. There is no fixed 90-day
materialization horizon. Unique `(rhythm_id, starts_at)` slots make repeated materialization
idempotent.

Updating a rhythm replaces future linked instances and preserves past entries. Editing an
individual generated entry detaches it by clearing `rhythm_id`, so a subsequent rhythm update
does not overwrite that edit.

## 3. Conflict evaluation doctrine

The [correlation layer](../../capabilities/calendar/src/correlate.rs) returns evidence and a soft
feasibility verdict: `free`, `needs-travel-day`, or `conflicts`. It does not filter candidates out.
An empty day is available by default.

The [impact function](../../capabilities/calendar/src/correlate/feasibility.rs) takes the cheaper
of a kind's impact and its commitment ceiling. A `possible` entry cannot block a day; a `planned`
entry can cost a travel day; only `committed` entries can reach `conflicts`. Unknown kinds are
neutral. Planning context supplies information without becoming a time block.

## 4. Google Calendar synchronization

[`google_sync`](../../capabilities/calendar/src/google_sync.rs) separates transport from the
import/export decisions in [`google.rs`](../../capabilities/calendar/src/google.rs).

- Import review previews a bounded window, then re-fetches selected events and rejects changed
  revisions before writing non-blocking drafts. Imports deduplicate by Google event ID. An imported entry at `possible` may be refreshed; raising it to `planned` or
  `committed` adopts it, and the local version then wins over Google changes.
- Export requires an explicit per-entry opt-in ledger row. Subsequent pushes patch the recorded
  Google event instead of creating another. The ledger's target calendar wins over changed
  deployment configuration.
- Imported Google entries and linked rhythm instances cannot be exported. Exported events carry
  `extendedProperties.private.axon_entry_id` and `axon_kind` for traceability. Timed exports carry
  both the home timezone and an explicit offset; all-day exports keep exclusive date ends.

These are reviewed import and export operations, not a continuous incremental-sync-token
protocol. Open work belongs in [ISA.md](../../ISA.md).
