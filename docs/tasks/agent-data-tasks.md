---
id: "agent-data-tasks"
title: "Run one approved proposal-only data task"
state: "backlog"
area: "agent"
kind: "feature"
priority: 3
depends_on: ["agent-run-supervisor","agent-client-mac"]
spec: "docs/architecture/agent-sessions.md"
---

# Run one approved proposal-only data task

## Outcome

Run one approved proposal-only data task through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Use sjel tier by default; a Claude/Codex alternative must prove file/shell tools disabled and read only through sjel-mcp.
- Represent the task in the existing proposal queue with prompt, read capabilities, allowed proposal routes and limits; obtain approval before its first run.
- Write only owner proposals, never final records; an Ask write immediately fails the step as blocked without a pending queue.
- Pass egress audit with zero violations, stop at supervisor limits and review proposals on the Mac.
