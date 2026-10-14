# Calendar Architecture & Contracts

Technical reference manual for Sjel's temporal scheduling, materialized rhythms, and external calendar synchronization.

```mermaid
flowchart TD
    subgraph Sources["Calendar Ingestion"]
        CommsProposal["Reviewed Mail Proposals"]
        MarkdownEvents["Markdown Notes / Frontmatter"]
        GoogleImport["Google Calendar API"]
    end

    subgraph Core["Core Capability (capabilities/calendar)"]
        Entries["calendar.entries\n(Single-day intervals)"]
        Rhythms["calendar.rhythms\n(Materialized forward-only)"]
        Contexts["calendar.contexts\n(Join keys: trips, projects)"]
        ConflictEngine["Soft Conflict Evaluator\n(Scoring, no hard blocks)"]
    end

    subgraph Sinks["Downstream Consumers"]
        GoogleExport["Google Calendar Export\n(Sjel wins on conflict)"]
        DashboardMonth["Dashboard MonthGrid / Day View"]
        TransitJoin["Transit & Trip Planner\n(Context queries)"]
    end

    Sources --> Entries
    Rhythms --> Entries
    Entries --> ConflictEngine
    ConflictEngine --> Contexts
    Contexts --> GoogleExport
    Contexts --> DashboardMonth
    Contexts --> TransitJoin
```

## 1. Temporal Data Model

The calendar data model splits into four core tables in SQLite:

| Table | Purpose | Concurrency & Integrity Invariants |
| :--- | :--- | :--- |
| `calendar.entries` | Scheduled events, tasks, and appointments. | Bounds enforced to single day intervals. Cross-midnight events split at `00:00`. |
| `calendar.rhythms` | Recurring habits and schedules. | Materialized forward-only (up to 90 days). Never backfilled retroactively. |
| `calendar.contexts` | Associations joining entries to trips, places, or projects. | Weak references; deleting an entry cascades contexts. |
| `calendar.google_exports` | Remote sync state tracking etags and remote event IDs. | Tracks external mutations to prevent sync loops. |

## 2. Materialized Forward-Only Rhythms

Rhythms (recurring events such as weekly syncs or gym sessions) follow strict materialization doctrine:

- **Forward-Only**: Rhythms materialize instances forward from the current execution timestamp up to a fixed horizon (default 90 days).
- **Zero Retroactive Mutation**: Modifying a rhythm never alters past materialized entries. Historical journal accuracy is strictly preserved.
- **Detached Instances**: Once materialized, editing an individual instance marks it as overridden without breaking the parent recurrence rule.

## 3. Conflict Evaluation Doctrine

Traditional calendars refuse overlapping events or force hard constraints. Sjel employs soft conflict scoring:

- **Verdicts Over Blockers**: Double-booking is permitted and flagged with a soft conflict verdict score (`none`, `soft`, `hard`).
- **Context-Aware Weighting**: Travel buffers and flight connections score higher conflict weights than flexible task deadlines.

## 4. Google Calendar Synchronization

Two-way synchronization runs via `capabilities/calendar/src/google_sync.rs`:

```mermaid
sequenceDiagram
    participant Google as Google Calendar API
    participant Sync as Sjel Calendar Sync
    participant Store as Sjel SQLite Store

    Google->>Sync: Fetch modified events (Incremental sync token)
    Sync->>Store: Reconcile entries (Sjel authoritative on conflict)
    Store->>Sync: Collect local export queue
    Sync->>Google: Push exported entries with private extension tags
```

- **Conflict Policy**: When an external edit conflicts with a local change, **Sjel wins**. Sjel local state overwrites Google Calendar.
- **Loop Prevention**: Outbound exports write a unique sync fingerprint into Google extended properties, preventing re-import.
- **Privacy Floor**: Entries marked `c3` (Secret) are never exported to external calendar providers.
