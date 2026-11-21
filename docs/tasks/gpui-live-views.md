---
id: "gpui-live-views"
title: "Render live views in the agent panel"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 1
depends_on: ["gpui-saved-views","agent-client-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Render live views in the agent panel

## Outcome

Render live views in the agent panel through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Mount validated live view documents in the existing agent panel without executing model code.
- Preserve source/read-route identity, data classes and owner-mode proposals through updates and failure states.

## Evidence

- Wired `Row::LiveView(Box<ValidatedA2uiView>)` and `"view_document"` stream event handling into `AgentRun` (`apps/gpui/core/src/agent.rs`) and rendered validated views in `AgentSeat` (`apps/gpui/ui/src/agent_seat.rs`) using Ely `Card`, `MetricStrip`, `MetricTile`, `DescriptionList`, `DataTable`, `Badge`, and `Button` with data-class and read-route badges and owner-mode proposal actions.
- Verified in `sjel_gpui_ui::agent_seat::tests::live_and_saved_a2ui_views_mount_reload_and_queue_owner_mode_proposals`.
- Unrun checks: live agent-capability streaming acceptance in the running desktop and phone apps.
