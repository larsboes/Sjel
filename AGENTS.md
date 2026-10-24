# Sjel agent bootstrap

Load the `sjel` skill (`Packs/harness/skills/sjel/SKILL.md`) for any Sjel operation or repository
change. It routes to the rules for placement, documentation, verification, data boundaries, and
commits. For operations, run `sjel help` or `sjel search <task>` before you browse files.

Shared documentation and planning rules live in [CONTRIBUTING.md](CONTRIBUTING.md#documentation).
Keep this file as the bootstrap; link to the owning rule instead of duplicating it here.

Before editing:

1. Run `git status --short --branch` and `tools/doctor`.
2. Derive capabilities, ports, health, and open claims from Sjel tools, the owning `ISA.md`, and
   live metadata. Do not trust remembered prose.

Always:

- Work directly on `main` until the official release. Do not create or switch branches unless
  asked.
- Other agent sessions edit this checkout at the same time. Never stash, revert, or overwrite
  their changes. Touch and stage only the files of your task.
- Keep private values, machine state, and secrets in the overlay or the OS keychain. Never
  generate, expose, or change a secret without explicit authorization.
- Keep comments and config comments short: no dates, measurements, decisions or history. The
  commit message carries them. Rules: `Packs/harness/skills/sjel/references/on-documentation.md`.
- Write a durable rule or decision into the repository: code, the skill that owns it, `ISA.md`, or
  this file. Never into agent memory. On this machine, the untracked `.claude/settings.json` sets
  `autoMemoryEnabled: false`.
- Format replies for low cognitive load, per the `attention-control` skill: the result and its
  verification first, numbered chunks, exact counts and paths, and manual steps in their own
  block with a time estimate.

## graphify

`graphify-out/` holds a knowledge graph of this repository. For a codebase question, run
`graphify query "<question>"` first, `graphify path "<A>" "<B>"` for a relationship, and
`graphify explain "<concept>"` for one concept. Dirty `graphify-out/` files are normal. After a
code change, run `graphify update .`.

## Current Priority: GPUI Universal App & Standalone iOS

Active engineering focus is on unifying and porting the native GPUI client ([`apps/gpui`](apps/gpui/README.md))
across platforms (macOS, iOS, and subsequent Linux/Windows targets):

1. **Shared cross-platform core and UI**: Extract and share views, navigation, editor blocks, and
   data adapters between desktop and mobile rather than maintaining diverged mobile stubs.
2. **Standalone and offline-first iOS capability**: The iPhone app must function autonomously without
   requiring an active connection to the Mac host.
   - On-device local store (embedded SQLite/LibSQL or local cache in the app sandbox).
   - Local note viewing and editing, task management, and capture.
   - Offline queue with bi-directional delta synchronization whenever the Mac LAN listener is discovered.
   - Graceful degradation: local search and cached reads remain responsive when disconnected.
3. **The dashboard is the second consumer, and it retires** (decided 2026-10-22). Port each
   dashboard page to GPUI, then improve it there. A rule the two clients share goes into a
   capability or `apps/gpui/core`, never only into `dashboard/`. Do not start new feature work
   in the dashboard. It retires when GPUI covers its pages.
   See [`apps/gpui/PORTING_PLAN.md`](apps/gpui/PORTING_PLAN.md) for the active route migration matrix,
   work packages for concurrent sessions, and the UI/UX polish roadmap.

