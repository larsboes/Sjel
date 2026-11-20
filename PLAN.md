# Sjel plan

[Canonical tasks](docs/tasks/index.md) own acceptance, state, priority and evidence. Run
`tools/work list` for open work. [Roadmap outcomes](docs/roadmap/index.md) group public benefits;
[release records](docs/releases/index.md) own bounded scope and verified delivery. GitHub Issues
provide intake and discussion, not a second task database.

[Area indexes](docs/plans/index.md) link tasks and longer specifications. Completed work leaves
active plans; Git history retains it without commit breadcrumbs.

## Priority 0: concrete failures in existing use

Repair verified data-loss, restore, authorization and publication defects before dependent work.
Diagnose a stale receipt before choosing a fix. An entire agent/security roadmap is not a failure
gate. [Storage](docs/plans/storage.md) and [Tooling](docs/plans/tooling.md) link the relevant tasks.

## Priority 1: unify and reduce maintenance

Finish existing contracts and acceptance before introducing another framework or surface:

- [Installed note-path search](docs/tasks/storage-note-search.md).
- [Derived views and shared UI controls](docs/plans/gpui.md#derived-views).
- [Capability boundaries](docs/plans/product.md).
- [Work-tracking acceptance](docs/tasks/work-tracking-verification.md).
- [Shared API contracts and maintenance](docs/plans/restructure.md).

Directory/workspace changes require measured benefit. Scanner adoption does not waive admission
rules. Fewer lines alone do not establish better boundaries or equivalent behavior.

## Priority 2: finish daily workflows

- Mac Documents, Mail, Calendar and Travel: [GPUI](docs/plans/gpui.md).
- Restore proof and domain sync contracts: [Storage](docs/plans/storage.md).
- Import routing, idempotency and review: [Data](docs/plans/data.md).
- Existing-result validation: [Transit](docs/plans/transit.md), [Finance](docs/plans/finance.md).
- Fresh-user CLI installation: [preview acceptance](docs/tasks/cli-preview-acceptance.md).

Compilation and unit tests do not close live acceptance. Engine migration requires compatibility
evidence; public release and publication checks remain separate from source completion.

## Priority 3: agent features and expansion

[Agent](docs/plans/agent.md), [Containment](docs/plans/containment.md),
[Packs](docs/plans/packs.md), [Learning](docs/plans/learning.md), [Homes](docs/plans/homes.md),
[Website](docs/plans/website.md) and [Posts](docs/plans/posts.md).

Existing containment remains in force. Current integrity defects can rise in priority; new
features do not. [Soundscape's context join](docs/tasks/soundscape-context.md) follows this tier.

## Priority 4: deferred iPhone work

iPhone-specific implementation and physical-device acceptance follow Priorities 1–3, including
standalone sync and [Scanning](docs/plans/scanning.md). Mac work and shared contracts retain their
own priority. Implemented phone paths keep their outstanding device checks.

## Success measures

| Measure | Probe | Threshold |
|---|---|---|
| Other people's data leaves only as trusted_view to a c2 provider | comms egress-log --audit | 0 c2 violations |
| A trip reaches bookable legs | One real trip | Under 30 minutes |
| One question joins three sources | One real question weekly, with sources logged | No manual source-tool lookup |
| Project-relevant mail reaches its project | Replay one week of mail | No manual step |

<a id="decisions"></a>

## Ownership

The [product page](docs/product.md) owns scope. The [documentation rules](docs/architecture/engineering.md#documentation)
assign each fact to its owner. Deferral preserves acceptance checks without authorizing a
replacement architecture.
