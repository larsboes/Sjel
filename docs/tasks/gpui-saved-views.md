---
id: "gpui-saved-views"
title: "Reload saved views without a model"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 1
depends_on: ["task-336bf8f4-9597-436e-84bb-ef6ff052ce59"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Reload saved views without a model

## Outcome

Reload saved views without a model through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Compose a validated view once and reload it after app restart without a model call.
- Retain schema/version/route validation, classified read admission and proposal-only mutations.

## Evidence

- Implemented `SavedA2uiView` in `apps/gpui/core/src/a2ui.rs` and `AgentRun::{save_view, reload_saved_view}` in `apps/gpui/core/src/agent.rs`, re-running schema version, catalog ID, read-route admission, data-class ceiling (`DataClass::C0..C3`), and proposal-only intent validation on reload without a model call.
- Verified in `sjel_gpui_core::a2ui::tests::adoption_gate_synthetic_list_detail_form_saves_and_reloads_without_model` and `sjel_gpui_ui::agent_seat::tests::live_and_saved_a2ui_views_mount_reload_and_queue_owner_mode_proposals`.
- Unrun checks: live desktop and iPhone device acceptance across app restart.
