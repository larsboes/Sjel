---
name: harness-sync
description: Answers where every Sjel Pack skill is deployed across the agent harnesses installed on this machine, what has drifted from its source, and which of the two reverse moves keeps an edit that was made inside a harness. Drives tools/harnesses (list, status, drift, sync, promote, accept). Use whenever the question involves deployed skills or agents — "is everything deployed", "sync my skills", "what drifted", "which harnesses have this skill", "bring this skill into Sjel", "I edited a skill in place", "why is this skill missing in pi", "deploy the pack" — and whenever a Pack, a skill or an agent file has just been added, renamed or edited in the Sjel repository. Do not use for writing the content of a skill, for choosing which skill to build next, or for shrinking always-on context files.
license: MIT
---

# harness-sync

Sjel is the source. Every harness holds a copy or a pointer, and the two drift.

One tool answers all of it: `tools/harnesses` in the Sjel repository. This skill is the judgment
around it — which direction a change should travel, and when the answer is not `sync`.

## The direction rule

```
Sjel Packs  ──(sync, routine, one-way, overwrites)──▶  harness
Sjel Packs  ◀──(promote / accept, manual, human decides)──  harness
```

Sync is the normal move and it is destructive at the destination by design. The reverse moves are
manual because an edit made inside a harness is a decision, and a tool that guessed would either
lose good work or import junk into the source of truth.

| Situation | Move |
|---|---|
| The Pack changed; the harness is behind | `sync` |
| A deployed copy was edited in place and the edit is worth keeping | `accept` |
| A deployed copy was edited in place and the edit is not worth keeping | `discard` — replaces the copy with the Pack's text and keeps the replaced copy beside the ledger. It lists what it replaces, adds and removes, and refuses until given `--force` |
| A skill exists in a harness, no Pack owns it, and it should be shared | `promote` |
| A skill exists in a harness and belongs to that harness alone | leave it; it shows as unowned every run, which is the honest state |

## Procedure

```
- [ ] 1. Ask which harnesses are installed
- [ ] 2. Read the matrix
- [ ] 3. Resolve drift before deploying anything
- [ ] 4. Move in one direction at a time
- [ ] 5. Re-read the matrix
```

**1 — Installed, not merely supported.** `tools/harnesses list`. A harness with an adapter is not a
harness on this machine, and deploying to an absent one leaves skills in a directory nothing reads.
Every verb defaults to installed harnesses only; `--all-harnesses` overrides it and needs a reason.

**2 — Read the matrix.** `tools/harnesses status [<pack>]`. One row per Pack skill, one column per
harness: `·` current, `o` outdated, `D` drifted, `M` missing, blank not deployed. The dashboard's
**Packs** page is the same matrix, read through `sjel-status` — and its three buttons
(`sync`/`deploy`/`remove`) are this skill's `sync` and the per-harness `deploy`, offered where the
defect is. Fixing a cell from the page is the same move as running the command: the page shells the
same launcher, so its refusals are the tool's own words. A `D` cell has no button there, because
`sync` refuses drift and the fix is the decision below, not a click.

Under it, each harness lists what sits at its destination that no Pack owns:

| Kind | What it means | What to do |
|---|---|---|
| `copy` | An ordinary directory nobody claims | A `promote` candidate. Read it first. |
| `external` | Carries `.git`, `.graphify_version` or similar | **Never promote.** Another installer owns it and re-syncs from its own remote; a copy in Sjel is a fork that will silently pin it. Record it in `upstreams.toml` instead. |
| `symlink` | Points into a product install | **Never promote.** The target is the install; an upgrade updates it. |

**3 — Resolve drift first.** `tools/harnesses drift [<pack>] [--diff]` names the differing files and
prints the diff. `sync` refuses to run against a drifted copy — that refusal is the feature. Two verbs
clear the state, and each is a decision:

- `accept` keeps the edit: it imports the destination's text into the Pack, so review `git diff`
  afterwards and revert what should not have been imported.
- `discard` drops the edit: it writes the Pack's text to the destination and keeps the replaced copy
  under `~/.local/state/axon/pack-deployments/discarded/`. Run it without `--force` first; it lists
  the files it would replace, add and remove, and refuses until given `--force`. The Pack is never
  written.

**4 — One direction at a time.** Do not run `accept` and `sync` for the same pack in one pass. Accept
first, review `git diff`, then deploy outward.

**5 — Re-read.** The matrix after the move is the receipt. `accept` re-records the ledger itself, so
a row still showing `D` afterwards means the copy and the source still differ.

## What each harness actually does with a Pack

- **Materialized** (Claude Code, Codex, opencode, GitHub Copilot CLI): the adapter copies the
  skill and records a digest. Drift is possible and detectable. Claude Code additionally takes a
  Pack's `agents/` directory as one unit at `~/.claude/agents/<pack>`.
- **Registry** (pi): the harness reads the Pack source in place through a path list in its own
  settings. There is no copy, so there is no drift — the only defect is a registered path that no
  longer exists, which `status` reports as `missing`. `promote` and `accept` are meaningless there
  and refuse. pi additionally auto-discovers `~/.agents/skills` and every `.agents/skills` up to
  the repository root, so a skill placed there by anybody else is loaded too. `status` lists those
  as `~` discovered; a *system-wide* one that no Pack owns is a policy violation, not a promote
  candidate, because the same name can also be registered and pi keeps the first. `tools/skills
  audit` names them, and `tools/skills add` installs a non-Pack skill project-scoped instead.

## When the hooks fire

Two hooks call `tools/pack-drift-hook`. They report; they never block.

- **At session start**, a list of drifted copies. Something edited a deployed skill since the last
  session. Run `drift --diff` before assuming it was a mistake.
- **When a SKILL.md under a harness skill root is written**, a note that the file is a deployed copy
  and not the source. Stop editing there. Edit the Pack and `sync`, or finish the edit and `accept`
  it.

## Error handling

- **`already owned by Pack 'X'`** — that destination belongs to another Pack. Rename, or remove the
  other Pack's copy first.
- **`is a symlink: another installer owns that skill`** — correct refusal. See the table above.
- **A Pack is deployed to a harness that is no longer installed.** The copies are inert, not
  harmful. Remove them with that harness's own CLI, or leave them for a reinstall — but say which,
  because an unexplained skill root is what made this tool necessary.
- **A skill should exist in every harness and only one has it.** That is a deploy, not a sync:
  `tools/packs-<harness> deploy <pack>`, then re-read the matrix.
