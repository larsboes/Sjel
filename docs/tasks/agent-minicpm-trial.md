---
id: "agent-minicpm-trial"
title: "Test MiniCPM structured tool compatibility"
state: "backlog"
area: "agent"
kind: "investigation"
priority: 3
issue: "https://github.com/larsboes/Sjel/issues/240"
---

# Test MiniCPM structured tool compatibility

## Outcome

Test MiniCPM structured tool compatibility with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- Confirm the exact approved MiniCPM5-2B MLX checkpoint loads through oMLX and answers the same bounded read-only Qwen baseline prompt/tool set.
- Require structured tool calls rather than raw XML markup and a correct explanation; measure latency and report thermal/memory only when measured.
- Leave deployed roles unchanged and reject adoption if parser compatibility or quality fails.
