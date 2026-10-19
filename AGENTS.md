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
