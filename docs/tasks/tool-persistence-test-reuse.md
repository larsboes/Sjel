---
id: tool-persistence-test-reuse
title: "Reuse the Cargo test binary for persistence checks"
state: verify
area: tooling
kind: maintenance
priority: 3
spec: docs/operations/doctor.md
evidence:
  - "The sjel-cli suite passes the isolated persistence fixture, and the planner tests select the crate for changes to the launcher, unit owner script, and manifest inputs."
---

# Reuse the Cargo test binary for persistence checks

## Outcome

The required Cargo job checks persistence with the binary it already builds. Repository gates
and the shell suite do not compile a second CLI or run unrelated diagnostics for this fixture.

## Acceptance

- The fixture retains coverage for enabled, orphaned, unmanaged, and script-owned units.
- Edits to the persistence implementation, launchers, and relevant manifests select the Cargo test.
- A hosted run shows the fixture in `cargo test (hermetic)` and no separate persistence test in
  `repo gates` or `bun test`. Compare job and workflow timings separately.
