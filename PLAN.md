# Sjel plan

The one ordered plan for Sjel. It orders the work by priority, names where each area stands, and
records the decisions that shape it. It is not a task list. Each area has one file in
[`docs/plans/`](docs/plans/). Concrete, testable work can also be a GitHub Issue. A decision moves
to its owner's README once it is built.

## Priority 1: the phone app, used every day

The GPUI app on the iPhone is the daily tool for six things, each with a UI that is good to use:

| Area | Phone today | Next | Plan |
|---|---|---|---|
| The Sjel agent | Sessions, prompts, steering, approvals and stop on the right edge. Not tested on a device. | Tiers and command policy, then live validation | [`agent.md`](docs/plans/agent.md) |
| Other agents (Claude Code, Codex, Pi) | Nothing | A native terminal: PTY sessions on the Mac in `capabilities/agent`, one terminal view for Mac and phone | [`agent.md`](docs/plans/agent.md#6-terminal-from-the-phone) |
| Travel | Tab with trips and a native map | Itinerary timeline, map polish, offline-ready mark, fare alerts on a device | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Documents | Nothing (reader planned) | The native reader, offline, from the hash-verified original (#245) | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Mail | Nothing (Feed is Mac-only) | Read and triage mail on the phone | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Calendar | Tab, reads and writes the replica | A time grid and quick rescheduling | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |

Order inside Priority 1:

1. The terminal and the agent's tiers and command policy.
2. Live validation of the agent and the terminal on a phone.
3. Documents and Mail on the phone, then the new tab bar.
4. Travel and Calendar UI.

Work that Priority 1 needs from other areas (storage for offline reads, a missing write path)
belongs to Priority 1.

## Priority 2: one app, a smaller repository

1. Finish the gates for deleting the dashboard, then delete the dashboard, Tauri and the demo
   ([`gpui.md`](docs/plans/gpui.md#3-what-retiring-the-dashboard-still-takes)).
2. Restructure into one workspace with flat `crates/`, and sweep comments, TOML and READMEs in
   the same pass ([`restructure.md`](docs/plans/restructure.md)).
3. The agent's plugins, the Pi port and the measured comparison with Pi
   ([`agent.md`](docs/plans/agent.md)).

## Priority 3: everything else

- Full Turso migration and the rest of the standalone phone ([`storage.md`](docs/plans/storage.md)).
- RoomPlan device validation and the shared capture pipeline ([`scanning.md`](docs/plans/scanning.md)).
- Imports, formats, the settings surface, adaptive features and the domain backlog
  ([`data.md`](docs/plans/data.md)).
- UI polish for areas outside Priority 1, and the public site with screenshots, animations, docs
  and a description of the app.
- A fresh-user Mac and iPhone install ([#235](https://github.com/larsboes/Sjel/issues/235)).

## Success measures

Each needs a command or log query that shows its current value.

| Measure | Probe | Threshold |
|---|---|---|
| No raw data about other people leaves the machine | `comms egress-log --audit` | 0 raw C2 violations |
| A trip is planned fast | One real trip, from request to bookable legs | Under 30 minutes |
| One surface replaces several apps | Log each time a replaced app is opened for a covered task, for 7 days | 0 entries |

## Decisions

Dated decisions that span areas. Area decisions are in each area plan.

| Decision | Date |
|---|---|
| The dashboard retires. GPUI is the one app. | 2026-10-22 |
| Target one database per capability or agent. | 2026-10-23 |
| Knowledge stays in the vault until moving it hurts. | 2026-10-23 |
| Priority order as above: the phone app first. | 2026-10-25 |
| One workspace, flat `crates/`, one API crate per capability, third-party software in `services/`, a hybrid process model later. Directory name equals package name. The `sjel-` prefix stays only on binaries a user types. | 2026-10-25 |
| Pantry stock is a domain inside `inventory`. | 2026-10-25 |
| The phone writes inventory and wishlist next, then People. | 2026-10-25 |
| `ISA.md` is retired. Its open items moved to the area plans and to issues #245–247. The file is a pointer until no session links to it. | 2026-10-25 |

## Rules for this plan

- One file per area. A new file earns its place only when no area owns the work.
- A shipped work package shrinks to one line with its commit.
- Dates mark decisions. History lives in Git.
