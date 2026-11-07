# Sjel plan

The one ordered plan for Sjel. It orders the work by priority, names where each area stands, and
records the decisions that shape it. It is not a task list. Each area has one file in
[`docs/plans/`](docs/plans/). Concrete, testable work can also be a GitHub Issue. A decision moves
to its owner's README once it is built.

## Priority 0: agent containment by provider

Claude Code, Codex, and pi on OpenAI read Sjel fully. A coding agent on any other provider
runs in a sandbox: one project directory, a network allow-list, and the demo node as its only
Sjel. Plan:
[`containment.md`](docs/plans/containment.md).

## Priority 1: the phone app, used every day

The GPUI app on the iPhone is the daily tool for six things, each with a UI that is good to use:

| Area | Phone today | Next | Plan |
|---|---|---|---|
| The Sjel agent | Sessions, prompts, steering, approvals and stop on the right edge. A session has one of three access tiers, and every command is blocked, asked about or allowed by a rule set. Not tested on a device. | Live validation on a phone, then the Block card | [`agent.md`](docs/plans/agent.md) |
| Other agents (Claude Code, Codex, Pi) | Terminal on the Mac behind Face ID, opens a login shell. Not tested on a device. | Live validation, then a preset and directory picker | [`agent.md`](docs/plans/agent.md#6-terminal-from-the-phone) |
| Travel | Tab with trips and a native map | Itinerary timeline, map polish, offline-ready mark, fare alerts on a device | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Documents | Nothing (reader planned) | The native reader, offline, from the hash-verified original (#245) | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Mail | Nothing (Feed is Mac-only) | Read and triage mail on the phone | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |
| Calendar | Tab, reads and writes the replica | A time grid and quick rescheduling | [`gpui.md`](docs/plans/gpui.md#2-phone-priority-areas) |

Order inside Priority 1:

1. The agent's tiers and command policy.
2. Live validation of the agent and the terminal on a phone.
3. The UI stack ([`gpui.md` §11](docs/plans/gpui.md#11-ui-stack)): the pins, GPUI Fast on the
   Mac, then the NSH trial, which builds the phone's Documents reader.
4. Mail on the phone, then the new tab bar, both on the stack the trial chose.
5. Travel and Calendar UI.

Work that Priority 1 needs from other areas (storage for offline reads, a missing write path)
belongs to Priority 1.

## Priority 2: one app, a smaller repository

1. Done: the dashboard, its desktop shell and plugins are deleted. The device checks that were
   gates are open checks ([`gpui.md`](docs/plans/gpui.md#3-what-retiring-the-dashboard-still-takes)).
2. Restructure into one workspace with flat `crates/`, and sweep comments, TOML and READMEs in
   the same pass ([`restructure.md`](docs/plans/restructure.md)).
3. The agent's plugins, the Pi port and the measured comparison with Pi
   ([`agent.md`](docs/plans/agent.md)).

## Priority 3: everything else

- Full Turso migration, the rest of the standalone phone, and backups that prove a restore
  ([`storage.md`](docs/plans/storage.md)). The stalled backup timer (§5 item 1) is not deferred:
  it is fixed as soon as it is understood.
- Measured transit reliability, starting with the Köln–Bonn corridor, and collection that works
  while the Mac is closed ([`transit.md`](docs/plans/transit.md)).
- Pack deployment at user, project and account level, and its defects
  ([`packs.md`](docs/plans/packs.md)).
- RoomPlan device validation and the shared capture pipeline ([`scanning.md`](docs/plans/scanning.md)).
- Imports, formats, the settings surface, adaptive features and the domain backlog
  ([`data.md`](docs/plans/data.md)).
- Investment ideas, price triggers, research routed from comms and model proposals
  ([`finance.md`](docs/plans/finance.md)).
- Learning in the vault with `teach`, `practice` and claim notes, then practice on the phone
  ([`learning.md`](docs/plans/learning.md)).
- UI polish for areas outside Priority 1, and the public site with screenshots, animations, docs
  and a description of the app ([`website.md`](docs/plans/website.md)).
- A fresh-user Mac and iPhone install ([#235](https://github.com/larsboes/Sjel/issues/235)).

## Continuous

- Tooling gaps agents hit while working: [`tooling.md`](docs/plans/tooling.md).

## Success measures

Each needs a command or log query that shows its current value.

| Measure | Probe | Threshold |
|---|---|---|
| Data about other people leaves the machine only as `trusted_view` to a `c2` provider | `comms egress-log --audit` | 0 C2 violations |
| A trip is planned fast | One real trip, from request to bookable legs | Under 30 minutes |
| One surface replaces several apps | Log each time a replaced app is opened for a covered task, for 7 days | 0 entries |

## Decisions

Dated decisions that span areas. Area decisions are in each area plan.

| Decision | Date |
|---|---|
| The dashboard retires. GPUI is the one app. | 2026-10-22 |
| GPUI Fast renders the desktop. The phone runs a trial of NSH, with `gpui-mobile` as the bridge until it decides ([`gpui.md` §11](docs/plans/gpui.md#11-ui-stack)). | 2026-11-07 |
| Target one database per capability or agent. | 2026-10-23 |
| Knowledge stays in the vault until moving it hurts. | 2026-10-23 |
| The model provider sets an agent's reach. Untrusted providers run sandboxed, before Priority 1. | 2026-10-31 |
| Anthropic and OpenAI harnesses read Sjel fully; every other provider gets the demo node only. | 2026-11-04 |
| Priority order as above: the phone app first. | 2026-10-25 |
| One workspace, flat `crates/`, one API crate per capability, third-party software in `services/`, a hybrid process model later. Directory name equals package name. The `sjel-` prefix stays only on binaries a user types. | 2026-10-25 |
| Pantry stock is a domain inside `inventory`. | 2026-10-25 |
| The phone writes inventory and wishlist next, then People. | 2026-10-25 |
| `ISA.md` is retired. Its open items moved to the area plans and to issues #245–247. The file is a pointer until no session links to it. | 2026-10-25 |
| The Mac is not an always-on host. Collection and alerts run on a host independent of it. | 2026-10-27 |
| `c2` reaches the `trusted` ring (Claude, GPT) as `trusted_view` with full names. No harness reads raw overlay data. The data classes stay `c0`–`c3`. | 2026-11-03 |
| The always-on host is a home server, a Mac mini. It runs collection and alerts, and a large local model for `c2` and `c3` work. Until it is bought, the Mac stands in. | 2026-11-03 |
| Sjel never drives a consumer subscription itself. A subscription reaches Sjel through MCP when the user starts it; Sjel's own model calls use API keys. | 2026-11-03 |
| Local compilation, Clippy and test suites wait while the runtime profile is On the go; one command may carry `SJEL_ALLOW_LOCAL_BUILD=1`. | 2026-11-07 |
| Every change goes straight to `main` until this plan is done. Then `main` is tagged `v0.1.0`, and branches, worktrees and PRs start. | 2026-10-31 |

## Rules for this plan

- One file per area. A new file earns its place only when no area owns the work.
- A shipped work package shrinks to one line with its commit.
- Dates mark decisions. History lives in Git.
