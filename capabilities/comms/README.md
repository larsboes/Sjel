# comms

General observed-information intake for Sjel, plus reviewed mail triage.

## Surface

The `comms` capability provides general observation intake across news, media, repositories, research papers, and reviewed mail triage:

- CLI binary: `comms` (`src/main.rs`):
  - `comms sweep [--limit N] [--dry-run]` — perform read-only inbox sweep and propose mail triage.
  - `comms ingest <url>` — ingest and extract content from any supported public URL.
  - `comms feed [--stream news|media] [--days N] [--include-dismissed]` — query stored feed items.
  - `comms keep <id> | dismiss <id>` — update triage state; optionally export distilled markdown note.
  - `comms summarize --pending` — generate summaries for items lacking digests.
  - `comms clips [--limit N]` — query captured Sjel Clipper documents via local Unix socket.
- HTTP API: `comms-server` (`src/server/main.rs`), bound to loopback, default port **8083**:
  - `GET /feed` — list feed items with assigned trust classes.
  - `GET /feed/:id` — feed item details, relevance matches, and evaluation factors.
  - `POST /feed/:id/status` — update item status (`new`, `keeper`, `dismissed`).
  - `POST /ingest` — ingest URL and extract content.
  - `GET /content/:source/:id` — unified content item contract across feed and mail.
  - `POST /content/:source/:id/digest` — generate or refine local summary.
  - `POST /content/:source/:id/diagram` — render validated Mermaid diagram.
  - `POST /content/:source/:id/chart` — extract grounded data table.
  - `GET /triage` — query mail triage proposals, streams, and authenticity verdicts.
  - `POST /triage/sweep` — execute bounded inbox sweep without mutating Gmail state.
  - `POST /triage/:id/gmail` — execute reviewed action (`archive`, `trash`, `restore`).
  - `POST /triage/:id/status` — update local proposal triage state.
  - `GET /health` & `GET /ready` — service health and readiness probes.

## Why this shape: Read-only mailbox sweep with explicit local mutations

Mail intake is strictly read-only by default (`src/intake.rs`, `src/rules.rs`). Background sweeps fetch thread headers and snippets to propose triage streams (`aktiv`, `issue`, `feed`, `werbung`, `belege`, `steuern`, `sonstiges`) without altering Gmail state.

Mutations against Gmail (`archive`, `trash`) require explicit operator authorization and are recorded as durable local intent jobs (`comms_gmail_action_jobs` in `src/store.rs`). Permanent deletion, arbitrary label application, and outbound message sending are absent by design.

Triage classifications and stream overrides are stored locally and preserved across repeat scans.

## Why this shape: Tiered privacy redaction before persistent storage

Every ingested item is assigned an inspectable data trust class (`src/cloud_derivative.rs`): `c0` Public, `c1` Mine, `c2` Others, or `c3` Secret.

Private metadata (`c2`/`c3`) is redacted deterministically before database insertion (`src/intake.rs`), replacing contact names, email addresses, phone numbers, IBANs, and credentials with typed placeholders.

Content marked `c3` is mechanically excluded from local model prompts (`content_item::local_prompt_allowed`). External cloud inference is restricted strictly to reviewed, hash-approved pseudonymized derivatives of `c0`/`c1` items; `c2` and `c3` content never crosses external network boundaries.

## Why this shape: Sized local digests over retained message bodies

Summaries persist as derived artifacts in `content_digests` (`src/digest.rs`), decoupled from raw source items.

Mail message bodies fetched for summarization or event extraction are parsed in memory, evaluated against local models, and dropped immediately. Raw message bodies and attachments are never written to disk.

Structured depth ladders (`libs/summarize`) adapt digest detail to content length, avoiding speculative generation or hallucinations on brief or empty items.

## Why this shape: Grounded event and receipt extraction for downstream plans

Downstream integrations with Calendar (`src/mail_events.rs`) and Finance require verbatim textual evidence quotes from the source text before staging proposals.

Candidate dates without confirmed, valid time ranges default safely to non-blocking all-day calendar entries.

Extracted purchases must match configured import accounts and remain in review staging until explicitly confirmed by the operator before reaching the journal.

## Why this shape: Unified local inference with deterministic lexical fallback

Embedding, sequence classification, and local generation consolidate under authenticated local inference endpoints (`src/relevance.rs`).

Multi-stage relevance scoring pairs fast multilingual embedding retrieval with cross-encoder reranking, maintaining a labelled lexical fallback during inference downtime.

## Why this shape: Hardened public URL ingestion over unrestricted fetches

URL ingestion (`src/media.rs`) validates protocols (`http`/`https`) and inspects resolved destination IP addresses.

Loopback, private (RFC 1918), link-local, and cloud metadata addresses are rejected before socket creation, preventing SSRF into internal machine services.

## Verification

```bash
cargo test -p sjel-comms --lib store::db_tests::
cargo run -p sjel-comms --bin comms -- --help
cargo run -p sjel-comms --bin comms-server
```

## Considered and declined

- **Autonomous cloud mail classification**: Declined. Privacy doctrine requires local processing, deterministic redaction, and bounded model evaluation.
- **Direct permanent deletion or automated sending**: Declined. Sjel is a personal intake companion and reader, never an autonomous mailbox mutator.
- **Retaining complete message bodies in the database**: Declined. Message bodies are transient; mail is distilled into structured items and digests.
- **Per-capability SQLite databases**: Declined. Cross-domain joins across places, finance, and calendar require a shared database.
- **Unrestricted network fetching**: Declined. Loopback and private network destinations are blocked at the ingestion boundary to prevent internal service compromise.
