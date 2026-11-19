---
id: "agent-plugins"
title: "Expose tier-gated agent plugins"
state: "backlog"
area: "agent"
kind: "feature"
priority: 3
depends_on: ["agent-tier-boundary"]
spec: "docs/architecture/agent-sessions.md"
---

# Expose tier-gated agent plugins

## Outcome

Expose tier-gated agent plugins through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Expose web, subagents, mcp, skills and rust through capability plugins only within each tier's allowed set.
- Port all pi-web-access providers, page curation and PDF extraction; DuckDuckGo remains the keyless default.
- Compare subagent files and inference-key/provider behavior with Pi; decide whether questions support is needed rather than importing it silently.
