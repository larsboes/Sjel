---
id: "gpui-agent-follow"
title: "Specify agent-driven navigation"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 3
depends_on: ["agent-client-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Specify agent-driven navigation

## Outcome

Specify agent-driven navigation through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Define how the step stream follows searches/pages/value proposals in the main column without allowing model code or direct writes.
- Use the existing finder/panel/run model; this remains a design task, not authority to replace workspace architecture.

## Evidence

- Defined `AgentFollowIntent` (`OpenTarget`, `FinderSearch`, `FocusProposal`) and `derive_follow_intent` in [`apps/gpui/core/src/agent.rs`](../../apps/gpui/core/src/agent.rs), deriving read-only search, page navigation, and owner-mode value-proposal focus intents from the step stream while refusing write or shell-execution tool calls.
- Added `AgentSeatEvent::Follow`, the `"Follow: On/Off"` toggle, and the `"Open in column"` follow bar in [`apps/gpui/ui/src/agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs).
- Verified with unit test `step_stream_derives_read_only_follow_intents_and_ignores_write_or_exec_calls` in [`apps/gpui/core/src/agent.rs`](../../apps/gpui/core/src/agent.rs).

## Unrun checks

- Live agent step-stream follow verification in the running desktop app window.
