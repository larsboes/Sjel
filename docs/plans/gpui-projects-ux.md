# Projects, Tasks, and Notes UI/UX Overhaul

This plan defines the UI/UX overhaul for Projects, Tasks, Notes, and their supporting charts in Sjel's native apps (`apps/gpui/ui/src/projects/` and `apps/gpui/ui/src/nsh_projects.rs`). It builds on the `ui-craftsmanship` rules, the [native UI contract](../architecture/native-ui-boundaries.md), and Sjel's `ely-gpui-component` fork (`apps/gpui/Cargo.toml`).

Every chart in this plan uses data Sjel already stores (statuses, due dates, priorities, `blocked_by` edges, linked capability records, and note counts). Per [Accept native chart interactions](../tasks/gpui-chart-acceptance.md), historical completion velocity is excluded until capability event timestamps exist.

## 1. Charts for Projects and Tasks (`ely::charts`)

`apps/gpui/ui/src/projects/board.rs` currently uses `GanttChart` and `ProgressChart`. The overhaul extends Projects and Tasks with the remaining `crate::ely::charts` primitives already proven in Finance, Systems, Inventory, and Feed, upgrading the `ely-gpui-component` fork where interaction or layout hooks are missing:

1. **Due-date pressure strip (`CalendarHeatmap` / `BarChart`) in `AllTasks`:** Render a compact 14–30 day strip of due task counts above the task table. Clicking a day filters the list to that date's due items.
2. **Project health & scope (`BulletChart`) in `Hubs`:** Pair or replace plain progress bars with a `BulletChart` showing `done`, `active`, and `blocked` counts against total tasks so stalled projects read differently from healthy ones.
3. **Portfolio distribution (`Treemap`) in `Hubs`:** Size project tiles by open task count (or linked spend/items) and tone by status or staleness to expose where attention is concentrated.
4. **Urgency × Priority/Blockers matrix (`ScatterChart`) in `Tasks`:** Plot tasks by days until due against priority or downstream blocker count; clicking a region filters the task table.

## 2. Project Hubs and Cross-Capability Joins

`ProjectSection` currently exposes `Note`, `Tasks`, `Notes`, `MailReview`, and `InventoryLinks`.

1. **Subject-led project header:** Lead each project with one compact identity block: status chip, next due milestone, tabular open/blocked counts, and one primary action (`+ Task` or `Open Focus`). Secondary controls move down the explicitness ladder.
2. **Unified Touches strip:** Show every linked capability (Mail threads, Calendar events, Finance spend/budgets, Inventory items, Documents/Clips) in one join bar on the project overview, opening the target record in place in the inspector.
3. **Inline Finance and Inventory readiness:** When a project links Finance budgets or Inventory items, show a `BulletChart` (budget vs. actual spend) or readiness count (`acquired / total`) in the project header.

## 3. Task Focus, Triage, and Dependencies

1. **Blocker-chain and Unblockers view:** Surface `blocked_by` relationships with an **Unblockers** filter/sort and visual highlighting in `task_panel.rs` so completing a root task highlights the downstream tasks it unlocks.
2. **Keyboard-first triage mode:** Support `J`/`K` row navigation with single-key triage actions (status, priority, due date shortcuts for today/next week, project assignment) in `AllTasks`, keeping rare or destructive edits in the side panel.
3. **Dimmed inactive rows and stale indicator:** Keep done and cancelled tasks dimmed in place when toggled visible, and show a subtle age chip on tasks stuck in `in_progress` or `blocked`.
4. **Pinned Next Action promotion:** Allow pinning 1–3 tasks per project as *Next* so `Hubs` and `Today` (`nsh_today.rs`) surface the immediate next step without opening the full project.

## 4. Notes Capture, Structure, and Linking

1. **Split-pane notes workspace:** On wide Mac windows, pair the note list (left) with the live editor/preview (right) and a collapsible backlinks/joins rail instead of full-page context switches.
2. **Promote note line to task:** Offer a hover/focus action on note lines or checklist items (and in `scratchpad.rs`) to promote an item to a task while retaining a backlink to the source note.
3. **Auto-surfaced related notes:** When inspecting a task, mail thread, or calendar event, list notes that reference that project, person, or linked record in the side panel.

## 5. Question-Driven Views, Guided Review, and Dual-Surface Parity

1. **Question-driven saved views:** Complement `Hubs | AllTasks | AllNotes` with decision-oriented views: **Needs me today**, **Blocked on others** (joined with Mail/People), and **Drifting** (stale projects or overdue tasks).
2. **Morning / Weekly guided review:** Provide a single keyboard-driven review flow stepping through overdue tasks, stale projects, unfiled `MailReview` / `scratchpad` items, and the 7-day Calendar + Task load.
3. **Mac vs. iPhone parity (`Rule 5`):** Keep the Mac app optimized for dense tables, `ely::charts`, and side-panel editing. Phone `nsh_projects.rs` polish is parked while the phone UI migrates to shared GPUI ([Move the phone UI to GPUI](../tasks/gpui-phone-renderer.md)).

## Related tasks

- [Accept task and project controls](../tasks/gpui-project-tasks.md).
- [Accept native chart interactions](../tasks/gpui-chart-acceptance.md).
- [Reload saved views without a model](../tasks/gpui-saved-views.md).
- [Make note rename and saves lossless](../tasks/gpui-note-rename.md).
