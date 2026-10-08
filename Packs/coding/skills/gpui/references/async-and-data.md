# Async work and Sjel data boundaries

## Contents
- UI-thread rules
- GPUI executor and external runtimes
- Sjel service integration
- Loading, error, and cancellation states

## UI-thread rules

Never block rendering or input callbacks on HTTP, disk, database, subprocess, or expensive computation. Capture owned inputs, run asynchronous or blocking work on an appropriate executor, then update the GPUI entity and notify it. Keep cancellation and window/entity teardown in mind: a late result should not resurrect a closed view or overwrite newer state.

## GPUI executor and external runtimes

GPUI provides an executor integrated with its platform event loop. Verify its API at the pinned revision. An integrated GPUI executor is not automatically a Tokio runtime: libraries such as `reqwest` with Tokio timers need a Tokio runtime context. The Sjel evaluation prototype encountered this distinction and currently runs its reqwest check on a dedicated thread with a current-thread Tokio runtime, returning the result through a oneshot and a GPUI entity update. Reuse that pattern only while it fits the workload; for larger workloads, choose and document a single runtime boundary rather than creating a runtime per request.

Avoid holding synchronous locks across `.await`. Keep channels bounded for long-lived streams, and handle task errors as visible UI state rather than panics.

## Sjel service integration

The GPUI app is a desktop client, not a replacement for the web dashboard, mobile client, service registry, or HTTP boundary. Prefer existing typed API contracts for capability data. Respect the auth mode of each endpoint: do not put bearer tokens in source, logs, command-line arguments, or ordinary view state. Do not read private overlay files to bypass an API boundary. If no authorized client contract exists, select a read-only slice that does not cross that boundary and record the limitation.

The prototype's `GET /health` endpoint is public liveness only. Capability state currently comes from the local `sjel capability list` command, which owns registry/probe semantics. Review endpoint auth before replacing that path with HTTP.

## Loading, error, and cancellation states

Represent at least idle, loading, success, empty, and error states for user-triggered requests. Prevent duplicate work when the same action is already running. Preserve existing good data when a refresh fails if that helps the user distinguish stale data from missing data; show when data was last refreshed.
