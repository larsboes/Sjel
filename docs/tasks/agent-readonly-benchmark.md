---
id: "agent-readonly-benchmark"
title: "Repeat the read-only agent benchmark"
state: "backlog"
area: "agent"
kind: "investigation"
priority: 3
issue: "https://github.com/larsboes/Sjel/issues/241"
---

# Repeat the read-only agent benchmark

## Outcome

Repeat the read-only agent benchmark with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- Human-check expected claims and source-line references for 6–10 fixed code-reading tasks, including the three session-file ending cases.
- Match model/source/prompt/context/output cap and enforced read-only tool policy in Pi and Sjel; record unmatched harness differences instead of calling the comparison controlled.
- Repeat each task at least twice; record tool-call success, factual/format correctness, writes attempted, wall time, model/runtime versions and parameters.
- Record thermal/memory only when measured; keep the runner small, retain limitations in existing assistant-harness research and leave deployment inference roles unchanged.
