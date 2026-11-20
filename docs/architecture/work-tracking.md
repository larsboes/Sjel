# Work tracking

Repository Markdown owns planned work. GitHub Issues provide intake and discussion, not a second
task database. The website renders the same records that local validation and release preparation
read. Area plans route readers to canonical tasks and specifications.

## Records

| Path | Owns |
|---|---|
| `docs/tasks/<slug>.md` | One actionable task, acceptance and evidence |
| `docs/roadmap/<slug>.md` | One public outcome and its task membership |
| `docs/releases/vMAJOR.MINOR.PATCH.md` | Intended release scope, upgrade notes and required assets |
| `docs/releases/receipts/<version>.json` | Verified publication identity and asset availability |
| `docs/architecture/` | Contracts and design, not completion journals |

Task and outcome IDs are immutable and independent of filenames. `tools/work new` allocates
UUID-based task IDs without a shared counter. Hand-authored descriptive IDs are also valid;
the graph validator rejects duplicates across tasks and outcomes.

Tasks require `id`, `title`, `state`, `area`, `kind` and `priority`. Optional fields are
`depends_on`, `assignee`, `blocked_by`, `issue`, `spec`, `target_release`, `reason` and `evidence`.
Bodies contain `## Outcome` and `## Acceptance` with observable checks. Keep constraints only
when they affect implementation; link longer design through `spec`.

Outcomes require `id`, `title`, `summary`, `order` and `tasks`. Outcomes own membership; tasks
do not repeat parent pointers. A maintenance task need not belong to a public outcome.

Release records require `version`, `title`, `tasks` and `required_assets`. The body holds
user-visible changes, breaking changes, upgrade steps and known limitations. Required assets
are exact filenames, not a claim that every platform ships together.

The vocabulary and schema live in `tools/lib/work.ts`. Unknown fields, dangling references,
duplicate IDs, invalid states and dependency cycles fail validation.

## Lifecycle

| State | Meaning |
|---|---|
| `backlog` | Not selected or still needs specification |
| `ready` | Bounded acceptance; dependencies are done |
| `active` | Implementation is in progress; an assignee is required |
| `verify` | Implementation exists; required acceptance remains open |
| `done` | Acceptance passed and evidence is recorded |
| `cancelled` | Rejected, duplicate or superseded; a reason is required |

Blocking is a named reason, not a second lifecycle. Priorities retain the existing 0–4 order.
Done and cancelled records remain addressable but disappear from default task lists.
Implementation commits and unit tests do not close an outstanding live acceptance check.

Roadmap state derives from task state: planned, active or awaiting release. A cancelled task
never counts as completed. Published outcomes appear in a separate delivered view only when
all required tasks have delivery records. Reopened work returns to active planning; old release
receipts still describe what previously shipped.

## Commands

```sh
tools/work list                 # open tasks, ordered by priority
tools/work list --area gpui --state verify
tools/work list --all --json     # all task records
tools/work show <id>
tools/work new <area> <title>
tools/work check                # offline schema, graph and saved receipt checks
tools/work inventory            # every remaining plan section; no writes
tools/work inventory --json     # includes original section text
tools/work render --out <dir>   # roadmap, delivered, release and task-index pages
```

`--area` and `--state` can be combined. An explicit state also selects completed or cancelled
tasks when requested. `--all` removes the default open-task filter.

The static site build renders task detail through the existing documentation renderer.
`tools/work render` renders indexes and release views; it is not a replacement for `website/build`.
No planning command needs a running Sjel service. Normal builds do not call GitHub.

## Releases

Sjel uses one coordinated source version with explicit artifact coverage. Release acceptance is
bounded, not gated on completing the entire roadmap. The existing release line owns SemVer and
preview numbering; this tracking layer neither allocates nor changes versions.

1. Create a release record and select its tasks and required artifact filenames.
2. Complete acceptance, record evidence and commit the selected task and release records.
3. Run `tools/work release-check <version> [ref]`. It reads acceptance from the exact source ref,
   not uncommitted task edits. Run `tools/work notes <version>` for the release narrative.
4. Use the existing release and distribution tools. `tools/release` requires task acceptance even
   with its explicit CI-gate bypass; full-CI checks otherwise retain their existing behavior.
5. After approved publication, run `tools/work receipt <version>` and commit the receipt.
   `tools/work reconcile <version>` checks it against GitHub without changing files.

A source tag, draft release, accepted task and installed artifact are different states.
`target_release` expresses intent only. Publication receipts verify the public release, exact
local/remote tag identity, task acceptance at that tag and the required asset set. Existing
distribution metadata owns archive checksums; receipts do not replace artifact integrity checks.
Network failure means unverified. No command here tags, publishes, installs or rewrites history.

Delivered history requires receipts. Do not fabricate historical release membership or mark
existing source tags as published merely to populate the page.

## Writing

- Roadmap title: concrete feature or result, usually two to six words.
- Summary: one user-benefit sentence, normally no more than 30 words.
- Task: verb and bounded outcome, normally one to five independently rejectable checks.
- Task body: usually under 250 words; longer design belongs in a linked specification.
- Release notes: changes, breaking changes, upgrade steps and limitations; not a raw commit dump.

Remove repeated context, session chronology and shipped implementation inventories. Length
limits guide review; they never justify deleting safety or compatibility requirements.

Example: **Notes in Finder** — Find notes by title and folder, then open the source file.

## Migration and verification

When consolidating plans, give each open acceptance check a canonical destination or an explicit
rejection before removing its source. Keep durable constraints with their implemented owners or
specifications. Preserve compatibility anchors and leave completed records addressable. Git
retains removed prose. Issue reports link tasks rather than mirroring their bodies.

[The migration record](../tasks/work-tracking-migration.md) retains the reconciliation evidence;
[the verification task](../tasks/work-tracking-verification.md) owns remaining acceptance.

`tools/check-work.sh` runs in the repository gate registry. Model tests cover malformed records,
references, lifecycle and exact-source acceptance. Website tests cover empty states, escaping,
draft releases and separation of upcoming/delivered work. Browser layout, keyboard, theme and
publication reconciliation require their own evidence; unit tests do not establish them.

## References

Presentation references, read on 2026-11-18:
[Obsidian roadmap](https://obsidian.md/roadmap/) for short descriptions and release links;
[Vector releases](https://vector.dev/releases/) for explicit changes and upgrade guidance.
These references do not establish either project's task backend.
