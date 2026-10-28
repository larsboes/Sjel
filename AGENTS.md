# Sjel agent bootstrap

Load the `sjel` skill (`Packs/harness/skills/sjel/SKILL.md`) for any Sjel operation or repository
change. It routes to the rules for placement, documentation, verification, data boundaries, and
commits. For operations, run `sjel help` or `sjel search <task>` before you browse files.

Shared documentation and planning rules live in [CONTRIBUTING.md](CONTRIBUTING.md#documentation).
Keep this file as the bootstrap. Link to the owning rule instead of duplicating it here.
Every harness reads this file, Claude Code included. Do not add a `CLAUDE.md` beside it.

Before editing:

1. Run `git status --short --branch` and `tools/doctor`.
2. Derive capabilities, ports, health, and open work from Sjel tools, the area plan in
   `docs/plans/`, and live metadata. Do not trust remembered prose.

Always:

- A small change goes straight to `main`. A work package gets its own branch and worktree
  (`sjel worktree new <area>-<slug>`) and a draft PR that lands as a squash merge. Merge only when
  the user said so. Rules: `Packs/harness/skills/sjel/references/on-changes.md`.
- Other agent sessions edit the primary checkout at the same time. Never stash, revert, or overwrite
  their changes. Touch and stage only the files of your task.
- Keep private values, machine state, and secrets in the overlay or the OS keychain. Never
  generate, expose, or change a secret without explicit authorization.
- Write every tracked document in a neutral open-source register, never as a conversation with
  an assistant. Rules: `Packs/harness/skills/sjel/references/on-documentation.md` (Register).
- Keep comments and config comments short: no dates, measurements, decisions or history. The
  commit message carries them. Rules: `Packs/harness/skills/sjel/references/on-documentation.md`.
- Use Sjel tooling first (`sjel`, `tools/`, capability routes). When a task needs a custom script,
  a manual workaround, or a raw command that the tooling does not cover, do one of two things.
  Build it into the tooling in the same change, or add an entry to
  [`docs/plans/tooling.md`](docs/plans/tooling.md) in its format. Do not leave the script only in
  `/tmp` or in the session.
- Write a durable rule or decision into the repository: code, the skill that owns it, the area
  plan in `docs/plans/`, or this file. Never into agent memory. The tracked `.claude/settings.json`
  sets `autoMemoryEnabled: false`. Machine-specific settings go in `.claude/settings.local.json`.
- Format replies for low cognitive load, per the `attention-control` skill: the result and its
  verification first, numbered chunks, exact counts and paths, and manual steps in their own
  block with a time estimate.

## graphify

`graphify-out/` holds a knowledge graph of this repository. For a codebase question, run
`graphify query "<question>"` first, `graphify path "<A>" "<B>"` for a relationship, and
`graphify explain "<concept>"` for one concept. Dirty `graphify-out/` files are normal. After a
code change, run `graphify update .`.

## Working through the plan

[`PLAN.md`](PLAN.md) orders the work by priority. Each area has one plan in
[`docs/plans/`](docs/plans/) with its state, specs and done-when checks. Rules:
`Packs/harness/skills/sjel/references/on-documentation.md` (Plans).

1. Take work from the highest priority in `PLAN.md` that no other session holds. Read its area
   plan before you edit code.
2. Build to the spec. If the spec is wrong or missing a decision, stop and ask. Do not decide a
   product question in code.
3. In the same change, update the area plan: shrink shipped work to one line with its commit,
   record what is still open, and state each check that did not run.
4. A tooling gap you hit goes to [`docs/plans/tooling.md`](docs/plans/tooling.md) (see Always).

Do not start new feature work in `dashboard/`. A rule both clients share goes into a capability
or `apps/gpui/core`.
