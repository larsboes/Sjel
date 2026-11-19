---
id: "agent-gemma-trial"
title: "Measure the Gemma edge-model candidate"
state: "backlog"
area: "agent"
kind: "investigation"
priority: 3
issue: "https://github.com/larsboes/Sjel/issues/239"
---

# Measure the Gemma edge-model candidate

## Outcome

Measure the Gemma edge-model candidate with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- Confirm the exact approved Gemma 4 E2B MLX checkpoint loads through oMLX; use the same bounded read-only Qwen baseline prompt/tool loop.
- Measure parsed tool calls, explanation correctness and latency; memory/temperature claims require measurement.
- Leave deployment inference configuration unchanged and reject adoption if heat/quality does not improve; smaller effective parameter count alone proves nothing.
