# scouting

A generic opportunity-discovery pipeline: scrape a source (events, CFPs/scholarships, ...), normalize into one shared `Opportunity` type, score it against a configurable "interest profile" via embeddings/cosine similarity, rank, persist, and optionally cross-reference against existing notes.

## Surface

- CLI binary: `scout` (`src/main.rs`). Scans declared sources, scores candidates, prints ranked backlogs, manages triage status, and promotes approved events to calendar.
- HTTP service: `scout-server` (`src/server.rs`), defaulting to port `8084`:
  - `GET /health` — service status and database ping.
  - `GET /discover?adapter=<id>` — scan declared source and run full pipeline.
  - `GET /sources` — list configured and proposed sources.
  - `POST /sources/proposed` — record discovered source candidate into triage inbox.
  - `POST /sources/proposed/:id/dismiss` — dismiss candidate source.
  - `GET /opportunities?include_dismissed=true` — query scored backlog and event routes.
  - `POST /opportunities/:id/status` — update human triage state (`new`, `saved`, `dismissed`).

## Why this shape: Declared event sources over heuristic discovery

What runs is declared. Nothing polls unless declared in overlay configuration (`src/config.rs`). Sjel does not probe unannounced endpoints, and no automated scan promotes itself into a running producer. Explicit declaration keeps network egress, token usage, and background execution bounded.

Candidate sources discovered during secondary workflows land in `proposed_sources` (`src/store.rs`) with provenance details. Candidates stay inert until an operator promotes them into `sources[]`. Discovery earns an inbox proposal, never an automatic recurring fetch.

Adapter types are a closed enum handled in `src/sources/mod.rs` (`markdown-notes`, `rss`, `luma-calendar`, `splash-hub`). Format support is small and typed; compile-time match arms eliminate dynamic plugin loading overhead.

## Why this shape: Decoupled interest profiles and opportunity storage

An interest profile is a consumer input representing operator criteria, separate from scraped event data and raw vault documents. Decoupling profile paths from opportunity note trees (`src/score.rs`) allows multiple sources to score against one profile without duplicating files across directories.

Embedding vectors are generated via `src/embed.rs` through `crates/inference`, writing `telos_vectors.json` caches keyed by producer identity to prevent cross-model cache poisoning. When inference backends are absent, deterministic hash embeddings maintain local testability.

Opportunities persist in the shared SQLite database (`src/store.rs`) under the `scouting_` table prefix (`scouting_opportunities`, `scouting_links`, `scouting_source_state`, `scouting_proposed_sources`). A unified database preserves relational joins between opportunities and travel options (`crates/transit`).

## Why this shape: Deterministic event routing before calendar promotion

Relevance scoring measures whether an opportunity matters; geographic event routing (`src/event_route.rs`) determines where it belongs (`local`, `travel_candidate`, `online`, `unresolved`).

Physical event classification computes great-circle distance only when both operator home coordinates and venue coordinates are complete and valid. Incomplete coordinates fall back to country tokens and timezone prefixes rather than defaulting to `0,0` or city string heuristics.

Calendar promotion (`src/calendar_promote.rs`) upserts only `saved` opportunities classified as `local` or `online` into `crates/calendar` via idempotent `PUT /api/entries/external`. Wall-clock conversion uses explicit civil arithmetic in `src/localtime.rs` without guessing timezones. Non-local opportunities remain in the store as travel candidates for trip planning.

## Why this shape: Persistent triage status and idempotent upserts

Discovery scans re-fetch current listings, but operator triage decisions are durable. When re-fetching an existing opportunity, `upsert` in `src/store.rs` refreshes score, metadata, and timestamps while preserving existing `status` (`new`, `saved`, `dismissed`).

Execution state tracking (`scouting_source_state`) is keyed per declared source identifier rather than per adapter type, ensuring distinct poll cursors for independent feeds and calendars.

## Why this shape: Fail-closed scholarship qualification before scoring

Scholarship discovery enforces a strict frontmatter qualification contract in `src/sources/markdown_notes.rs`. An opportunity note is excluded from actionable ranking unless it declares active status, `eligibility: eligible`, `employment_compatible: yes`, a valid deadline, and a canonical URL.

Ineligible, incomplete, or incompatible notes remain in the knowledge base for audit history and are never surfaced by semantic fit scoring alone.

## Verification

```bash
cargo test -p sjel-scouting
cargo run -p sjel-scouting --bin scout -- --list-sources
cargo run -p sjel-scouting --bin scout -- --adapter euro_hackathons --no-store --limit 5
cargo run -p sjel-scouting --bin scout-server
```

## Considered and declined

- **Collaborative filtering recommendation engines (Gorse)**: Declined. Multi-user feedback matrices and Redis/MySQL service topologies introduce unnecessary machinery for a single-operator static profile scorer.
- **Unlicensed deal-flow tools (`troyriverabusiness/scout`)**: Declined as direct dependencies due to absent license terms. Architectural patterns (adapter trait and vector cosine scoring) were implemented cleanly in-tree.
- **Per-capability SQLite files**: Declined. Isolated SQLite files break cross-domain relational joins between opportunities and transit itineraries.
- **Heuristic background discovery promotion**: Declined. Discovered hubs or feeds require operator review before recurring network polling is enabled.
- **Dynamic plugin loading**: Declined. Native compile-time adapters provide type safety, straightforward error reporting, and zero runtime loading complexity.
