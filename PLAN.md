# Sjel plan

The one ordered plan for Sjel. It orders the work by priority and names where each area stands.
It is not a task list and it holds no decisions. What Sjel is and how it is shaped is in
[`website/content/product.md`](website/content/product.md). Each area has one file in
[`docs/plans/`](docs/plans/). Concrete, testable work can also be a GitHub Issue.

## Priority 0: concrete failures in existing use

Repair verified data-loss, restore, authorization and publication defects before dependent work.
Keep existing protections in force. An entire agent or security roadmap is not a Priority 0 gate.
Backup and restore checks belong to [`storage.md`](docs/plans/storage.md); verification failures
belong to [`tooling.md`](docs/plans/tooling.md). Diagnose a stale receipt before choosing a fix.

## Priority 1: unify and reduce maintenance

Code unification, cleanup, precision and less repeated work precede agent features. The
[maintenance review](research/plan-maintenance-review.md) records the evidence and candidate
tools. The first four items build the shape in the [product page](website/content/product.md).
Finish them before any other framework or surface is introduced.

1. One registry. A capability declares its object types, properties with data class, links and
   actions in its manifest, and `route-manifest` carries a response schema for every read route.
   `projects` is the first capability. Done when its MCP descriptions and TypeScript types are
   generated from the declaration ([`data.md`](docs/plans/data.md)).
2. One folder of Markdown files for prose. The files are the authority for their text, and Sjel
   indexes them
   ([`storage.md`](docs/plans/storage.md#one-vault-for-prose)).
3. Derived views from a web catalog. The GPUI and NSH pages are frozen. Done when an object list
   and an object view for `projects` render from the registry in a browser, with no page code for
   that capability. By 2026-12-09 either the catalog or the native pages are marked for removal
   ([`gpui.md` §11](docs/plans/gpui.md#views-from-the-web-catalog)).
4. Audit every capability against the build rule in the
   [product page](website/content/product.md#what-sjel-builds-ingests-and-links-to): build,
   ingest or link out. A capability that fails the rule shrinks to a connector or a link.
5. Reconcile plans with source: remove shipped detail, repair stale paths, keep one owner per
   open item, and distinguish implemented work from outstanding live acceptance.
6. Finish shared API contracts and remove remaining client copies and adapters
   ([`restructure.md`](docs/plans/restructure.md)). Verify existing API crates before extracting
   more. Directory renames and workspace consolidation need a measured benefit.
7. Reuse existing UI primitives, readers and platform controls; remove superseded code after
   equivalent behavior is verified ([`gpui.md`](docs/plans/gpui.md)).
8. Make routine maintenance automatic: reuse installed dependency scanners, evaluate an upstream
   skill detector, and reduce repeated review through evidence tied to content and policy
   ([`tooling.md`](docs/plans/tooling.md)). Scanner adoption does not waive current admission rules.
9. Remove unused dependencies and confirmed duplication, then shorten owner documentation and
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
  [`agent.md`](docs/plans/agent.md). Saved and live views, which a model composes, follow the
  derived views of Priority 1 ([`gpui.md` §11](docs/plans/gpui.md#views-from-the-web-catalog)).
  Existing containment remains;
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
| A question that needs three sources is answered from one place | One real question a week, logged with the sources it joined | No manual lookup in a source tool |
| A mail that concerns a project appears on that project | Replay of one week of mail against `projects` | No manual step |

## Decisions

This plan holds none. A decision is stated in the document that owns its subject, in the present
tense: the [product page](website/content/product.md) for what Sjel is, a capability's README for
that capability, the area plan for work that is still open. Git records when it changed.

## Rules for this plan

- One file per area. A new file earns its place only when no area owns the work.
- A shipped work package shrinks to one line with its commit.
- One owner per fact. Link to the owner instead of restating it.
- History lives in Git. A date appears only on a measurement or a deadline.
