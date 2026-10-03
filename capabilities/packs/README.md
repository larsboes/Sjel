
# packs

What Sjel has deployed into each agent harness on this machine, and the proof that it did.

This capability owns the deployment ledgers under `~/.local/state/axon/pack-deployments/`
— one JSON file per harness, each recording every deployed unit, the Pack source it came
from, and the digest that was installed. It owns nothing else. The deployers stay in
`tools/`, where operator machinery belongs (`CONTRIBUTING.md#three-architectural-nouns`).

## Why this is a capability and not only a directory of tools

The domain is bounded and it has state and external systems: four agent harnesses, each
with its own skill root, its own install marker and its own idea of what a deployed skill
is. `tools/lib/harness-registry.ts` enumerates them; `tools/harnesses` asks all of them
the same question at once.

What was actually missing was an owner. `schemas/service.toml.example` puts it plainly for
`kind = "data"`: a file with several readers still needs exactly one owner for the one
question a file's owner has to answer — how is it backed up. These ledgers have six
readers (four `packs-*` adapters, `tools/doctor`, `tools/harnesses`) and, until this
manifest, no owner and no backup.

Losing a ledger does not lose a skill. The copies stay on disk. It loses the proof that
Sjel put them there, and that is worse than it sounds: every deployed skill then reads as
an unowned collision, each Pack has to be re-adopted by hand, and `adopt` refuses anything
that is not byte-identical to its source — so a copy that had drifted cannot be reclaimed
at all.

## What it deliberately does not do

- **It runs nothing on a schedule.** A timed *deploy* would overwrite a harness copy while
  somebody is editing it, which is the one failure the whole drift design exists to
  prevent. A timed read-only *check* would be safe, and is not built until something needs
  it.
- **It serves no HTTP of its own.** `sjel-status` answers `GET /api/sjel-status/packs` by
  shelling `tools/harnesses status --json`, the same way it already shells
  `tools/capability.sh registry`, `tools/repos`, `tools/backup.sh` and
  `tools/service-runner.sh`. A second server for a JSON passthrough would be a port and a
  process to keep alive for nothing.
- **It holds no copy of the engine.** `tools/lib/pack-deploy.ts` owns the ledger format and
  the mutating half — deploy, sync, remove, adopt, and the lock they hold — and
  `tools/sjel-cli/src/harnesses/` reads the same ledger for `tools/harnesses list|status|drift`
  and the doctor's Pack sections. One reader per verb, read verbs first (2026-10-02): a
  capability that re-implemented either half would be a second engine and a migration of
  thirty-odd call sites toward an interface nobody has specified.

## Operating it

```bash
tools/harnesses list                     # which harnesses exist, and which are installed here
tools/harnesses status [<pack>] [--json] # the matrix, and what sits unowned at each destination
tools/harnesses drift [<pack>] [--diff]  # per-file detail
tools/harnesses sync <pack>|--all        # one-way Sjel -> harness
tools/harnesses promote <skill> --pack <p>   # a harness skill Sjel does not own
tools/harnesses accept <pack> <skill>        # an edit to a skill Sjel already owns
```

The direction rule is in `Packs/harness/skills/harness-sync/SKILL.md`: sync is one-way and
destructive at the destination by design, and both moves back into Sjel are manual because
an edit made inside a harness is a decision.

## Concurrency

Every mutating verb holds `<ledger>.lock` for its duration (`tools/lib/pack-deploy.ts`,
`withStateLock`). `writeState` was always atomic — a temp file and a rename — but that was
never the race: each mutator reads the whole ledger once and writes it back one or more
times, so two overlapping runs each held a snapshot taken before the other's writes and
the last one to finish erased the other's rows. The lock is re-entrant, because
`activateProfile` calls `deployPack` and `removePack`, and it is stolen from a holder
whose pid is gone so that one killed process cannot wedge the tool.

## Why this shape: the flip condition

Cut this manifest the day the ledgers stop being state worth keeping — if a future deployer
derives ownership from the destination itself rather than from a recorded digest, the file
it protects is gone and so is the reason for the row.

Promote it to a process capability with its own port and panel only when `sjel-status`
starts needing logic that is specific to Packs rather than a passthrough. Until then, a
process here would be a port, a binary, a health check and a dashboard mount bought for a
JSON file that another service already hands over for free.
