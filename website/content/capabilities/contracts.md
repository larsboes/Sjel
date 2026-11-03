# Freshness, health & sweep contracts

Use separate signals for a listening process, usable dependencies, and arriving data. Their
contracts are declared in [service manifests](manifest.md), with the complete field reference in
[`schemas/service.toml.example`](../../schemas/service.toml.example).

## Freshness contracts

Freshness uses flat manifest fields:

```toml
freshness_advise_hours = "12"
freshness_stale_hours = "24"
```

A declaring service answers `GET /__sjel/freshness` with
`{"last_arrival_at": <epoch seconds>}`, or `null` for that field when nothing has arrived.
The pre-rename `GET /__axon/freshness` answers the same body for one release, because sjel-status
polls every capability and a capability built before the 2026-10-20 rename cannot answer a path
it has never heard of; `libs/sjel-server/src/auth.rs` owns both names.
[`doctor`](../../tools/sjel-cli/src/doctor/checks.rs) reads this endpoint and evaluates the
declared thresholds. The timestamp measures producer arrival, including a successful sweep that
finds nothing new. It is not the newest item's publication timestamp: a quiet source is different
from a stopped collector. There is no `[freshness]` table or arbitrary SQL query in this contract.

## Sweep invariants

### 1. Distinct run vs. success

Comms stores per-source telemetry in
[`source_state.rs`](../../capabilities/comms/src/store/source_state.rs).
`record_sweep_success` updates run and success timestamps, records the pass's counts, and clears
the failure streak. `record_sweep_failure` updates run and failure timestamps and increments the
streak without replacing the last success. These are Comms methods, not a mandatory API that
every capability implements.

### 2. Error classification

Comms uses short error labels such as `auth`, `quota`, `network`, and `store`. Keep provider
messages out of this telemetry: they can contain request URLs or mail subjects. A source's last
successful pass and its consecutive failure count answer different diagnostic questions.

### 3. Health probe standard

`health_path` declares the liveness route that the runner polls after starting a process.
`ready_path` optionally declares a dependency-aware readiness route; `sjel-status` prefers it when
judging availability. The runner does not wait for readiness at startup. Consult the capability's
route contract for its response body rather than assuming every probe returns the same JSON.
