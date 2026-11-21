---
id: "agent-policy-acceptance"
title: "Accept live command-policy boundaries"
state: "done"
area: "agent"
kind: "maintenance"
priority: 3
spec: "docs/architecture/agent-sessions.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept live command-policy boundaries

## Outcome

Accept live command-policy boundaries through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Run the shared Pi/Rust command and write cases, including wrapper/compound evasions; show the selected rule ID and distinguish one-call Block override from exact-argument Ask grants.
- Reload the attended Pi session, verify checkpoint advisories do not trigger a steering loop, and measure real approval frequency for the project policy.
- Measure a week of use and approvals/calls over a month; set a prompt-rate threshold only after measurement.
- In a fresh session, verify the configuration floor refuses secrets reads; cloud derivatives contain only necessary task fields. A classifier is not a sandbox.

## Evidence

- The focused Bun suite (`tools/command-policy*.test.ts`, `tools/pack-extensions.test.ts`, `tools/pi-local-adaptations.test.ts`) passes: 100 passed, 2 skipped. The local Git grant tolerates `GIT_EDITOR`, `GIT_SEQUENCE_EDITOR`, `GIT_PAGER` and `GIT_TERMINAL_PROMPT` in the session environment and refuses every other `GIT_` variable. An isolated Pi session allows routine checkout writes and local Git, refuses sibling writes and pushes, and permits recursive cleanup only in session scratch. Legacy grant entries cannot widen this scope.
- Folder names alone protect only hook directories, so ordinary `hooks`, `prompts`, `agents` and `extensions` source folders stay routine. Checkout discovery keeps `GIT_CEILING_DIRECTORIES`. A `vault_exec` call that injects keys gets no package or home fast path. A Git failure warns and retries instead of dropping the project grant without notice.
- A fresh configured Pi 1.6.0 session preserved the previous request on all three continuations. OpenAI reported 14,848, 16,384, and 18,176 cached tokens as tool results accumulated.
- After an attended reload, the live policy log recorded an in-checkout `mkdir -p` no-op as `R4 / allow` and an outside-checkout no-op as `A1 / allow-once`. The latter ran only after explicit approval.
- Live checkpoint behavior, process-inspection integration, and longer-term approval measurements remain unverified.
