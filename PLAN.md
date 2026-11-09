# Sjel plan

The one ordered plan for Sjel. It orders the work by priority, names where each area stands, and
records the decisions that shape it. It is not a task list. Each area has one file in
[`docs/plans/`](docs/plans/). Concrete, testable work can also be a GitHub Issue. A decision moves
to its owner's README once it is built.

## Priority 0: concrete failures in existing use

Repair verified data-loss, restore, authorization and publication defects before dependent work.
Keep existing protections in force. An entire agent or security roadmap is not a Priority 0 gate.
Backup and restore checks belong to [`storage.md`](docs/plans/storage.md); verification failures
belong to [`tooling.md`](docs/plans/tooling.md). Diagnose a stale receipt before choosing a fix.

## Priority 1: unify and reduce maintenance

Decided 2026-11-08: code unification, cleanup, precision and less repeated work precede agent
features. The [maintenance review](research/plan-maintenance-review.md) records the evidence and
candidate tools. Finish existing seams before introducing another framework.

1. Reconcile plans with source: remove shipped detail, repair stale paths, keep one owner per
   open item, and distinguish implemented work from outstanding live acceptance.
2. Finish shared API contracts and remove remaining client copies and adapters
   ([`restructure.md`](docs/plans/restructure.md)). Verify existing API crates before extracting
   more. Directory renames and workspace consolidation need a measured benefit.
3. Reuse existing UI primitives, readers and platform controls; remove superseded code after
   equivalent behavior is verified ([`gpui.md`](docs/plans/gpui.md)).
4. Make routine maintenance automatic: reuse installed dependency scanners, evaluate an upstream
   skill detector, and reduce repeated review through evidence tied to content and policy
   ([`tooling.md`](docs/plans/tooling.md)). Scanner adoption does not waive current admission rules.
5. Remove unused dependencies and confirmed duplication, then shorten owner documentation and
   comments. Preserve domain boundaries and public contracts; fewer lines alone is insufficient.

## Priority 2: finish daily workflows

Complete existing behavior and its acceptance checks before expanding the surface:

| Area | Next boundary | Plan |
|---|---|---|
| Documents, Mail, Calendar and Travel | Mac and phone acceptance, missing writes and offline correctness | [`gpui.md`](docs/plans/gpui.md) |
| Storage | Restore proof and domain sync contracts; engine migration only with compatibility evidence | [`storage.md`](docs/plans/storage.md) |
| Imports and domain data | Correct routing, idempotency and review of existing pipelines | [`data.md`](docs/plans/data.md) |
| Transit and Finance | Validate existing results before more collection or model features | [`transit.md`](docs/plans/transit.md), [`finance.md`](docs/plans/finance.md) |
| Physical capture and installation | RoomPlan device acceptance; fresh-user installation | [`scanning.md`](docs/plans/scanning.md), [#235](https://github.com/larsboes/Sjel/issues/235) |

Unification required to finish these workflows stays in Priority 1. A live check remains open
even when compilation and unit tests pass.

## Priority 3: agent features and expansion

- Agent UI, terminal conveniences, plugins, Pi comparison, retrieval and optimization loops:
  [`agent.md`](docs/plans/agent.md). Generated views follow the catalog in
  [`gpui.md` §11](docs/plans/gpui.md#generated-views-priority-3). Existing containment remains;
  additional harness support follows [`containment.md`](docs/plans/containment.md).
- Pack deployment levels and account synchronization: [`packs.md`](docs/plans/packs.md).
  Defects in current installation integrity can rise to Priority 0 or 1.
- New learning surfaces and automation: [`learning.md`](docs/plans/learning.md).
- New home integrations: [`homes.md`](docs/plans/homes.md).
- Website polish and media: [`website.md`](docs/plans/website.md).

The priority order here supersedes older area-local ordering. Deferred work retains its
acceptance check; deferral does not authorize a replacement architecture.

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
| GPUI Fast renders the Mac and NSH renders the phone. `gpui-mobile` is removed (`b59879d0`). No Mac page moves to NSH until a Sjel-owned NSH facade exists and `bench-scroll` measures NSH Mail inside GPUI against GPUI Mail on Sjel data. If it matches, the target is one NSH screen per area. Generated UI is a declarative catalog that Sjel's components render, built in Priority 3. Model-written TSX packages and HTML do not run. Supersedes the NSH trial decision of 2026-11-07 ([`gpui.md` §11](docs/plans/gpui.md#11-ui-stack), [research](research/ui-stack.md#2026-11-09-the-long-term-stack-and-how-generated-ui-runs)). | 2026-11-09 |
| Generic tables (`DataTable`), rich-text/Markdown readers (`TextView`), and date/time pickers (`Calendar`, `DatePicker`, `TimeField`) use the installed `gpui-kit` and `nsh-base` primitives styled through `apps/gpui/ui/src/primitives`, heavy media uses native OS views (`PDFKit`, `MapLibre`), and custom layout code stays reserved for Sjel-specific surfaces ([`gpui.md` §7](docs/plans/gpui.md#7-ui-and-ux-roadmap)). | 2026-11-08 |
| Target one database per capability or agent. | 2026-10-23 |
| The Obsidian vault holds learning only: `teach`, `practice`, claim notes and subject knowledge. Every other note, fact and link source moves into Sjel data; the inventory is in [`storage.md`](docs/plans/storage.md#vault-holds-learning-only). Supersedes "knowledge stays in the vault until moving it hurts" (2026-10-23). | 2026-11-08 |
| Sjel data holds only what no live source holds. A fact about Sjel lives in the repository. Subject knowledge lives in the vault. Status lives in the owning record (a task, a calendar entry, a ledger row), never in note prose. Delete outdated notes instead of archiving them ([`storage.md`](docs/plans/storage.md#one-home-per-fact)). | 2026-11-08 |
| The model provider sets an agent's reach. Untrusted providers run sandboxed. The former roadmap gate is superseded by the maintenance-first order above. | 2026-11-08 |
| Anthropic and OpenAI harnesses read Sjel fully; every other provider gets the demo node only. | 2026-11-04 |
| Maintenance and code unification first, daily workflows next, agent features and expansion later. Supersedes the phone-agent-first order. | 2026-11-08 |
| One workspace, flat `crates/`, one API crate per capability, third-party software in `services/`, a hybrid process model later. Directory name equals package name. The `sjel-` prefix stays only on binaries a user types. | 2026-10-25 |
| Pantry stock is a domain inside `inventory`. | 2026-10-25 |
| The phone writes inventory and wishlist next, then People. | 2026-10-25 |
| `ISA.md` is retired. Its open items moved to the area plans and to issues #245–247. The file is a pointer until no session links to it. | 2026-10-25 |
| The Mac is not an always-on host. Collection and alerts run on a host independent of it. | 2026-10-27 |
| `c2` reaches the `trusted` ring (Claude, GPT) as `trusted_view` with full names. No harness reads raw overlay data. The data classes stay `c0`–`c3`. | 2026-11-03 |
| The always-on host is a home server, a Mac mini. It runs collection and alerts, and a large local model for `c2` and `c3` work. Until it is bought, the Mac stands in. | 2026-11-03 |
| Sjel never drives a consumer subscription itself. A subscription reaches Sjel through MCP when the user starts it; Sjel's own model calls use API keys. | 2026-11-03 |
| Local compilation, Clippy and test suites wait while the runtime profile is On the go; one command may carry `SJEL_ALLOW_LOCAL_BUILD=1`. | 2026-11-07 |
| Every change goes straight to `main` until this plan is done; branches, worktrees and PRs start afterward. Release readiness is separate: CLI-first previews use successive `v0.0.x` tags, and `v0.1.0` requires bounded installation, update and recovery acceptance rather than completion of this plan ([`tooling.md`](docs/plans/tooling.md#cli-installation-and-release-delivery)). A `v0.1.0` tag does not activate the branch workflow. This decision takes precedence over tag-triggered workflow wording in `AGENTS.md`, `CONTRIBUTING.md` and `Packs/harness/skills/sjel/references/on-changes.md`; the checkout policy changes only when this plan is done. | 2026-11-09 |

## Rules for this plan

- One file per area. A new file earns its place only when no area owns the work.
- A shipped work package shrinks to one line with its commit.
- Dates mark decisions. History lives in Git.
