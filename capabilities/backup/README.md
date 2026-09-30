# backup

Every capability's backup contract: the run, the surface that manages it, and the record of what
each attempt did.

`tools/backup-all.sh` asks the registry which capabilities declare `backup_target` and runs
`tools/backup.sh` for each. The set is derived, never typed: a capability is backed up because its
manifest says it has something to back up.

It runs every contract even when one fails, and exits non-zero if any did. Stopping at the first
failure would let one broken capability cancel the others' backups — the shape of outage that ends
with two weeks of nothing.

## Shape: a library here, the surface in sjel-status

This directory holds a **library**. `capabilities/sjel-status` is the **surface**, because it
already listed backup age and already triggered a backup: a second process would have been a second
trigger and a second truth for one job. The capability owns the tables, the runner, the verifiers
and the timing policy; sjel-status links it the way it already links `capabilities/devices`.

| Route (on sjel-status) | What it answers |
|---|---|
| `GET /api/sjel-status/backups` | age from the receipt, **plus `attempt`** — what the last run actually did |
| `POST /api/sjel-status/capabilities/{name}/backup` | run one capability, optionally against a named `{ "target" }` |
| `GET /api/sjel-status/backup/targets` | declared targets: kind, coordinates, presence, last rehearsal verdict, interval |
| `GET /api/sjel-status/backup/runs?limit=N` | every attempt, newest first, failures included |
| `POST /api/sjel-status/backup/policy` | `{ "target", "interval_hours" }` — set, change, or `null` for off |
| `POST /api/sjel-status/backup/verify` | `{ "target" }` — hash the newest recorded archive and restore it in isolation |

Four things the receipts could not carry, and this does:

- **A failed run is a row.** `tools/backup.sh` writes a receipt only after a run lands, so a failed
  run left the previous receipt in place and `doctor` read its age as fresh. Every attempt is now
  recorded with its exit status, reason and log, and a failure also writes
  `<overlay>/backup/receipts/attempts/<capability>.json` — removed the moment a run succeeds — so an
  offline check can see it.
- **The target is a per-run choice.** `tools/backup.sh --target <id>` outranks both the manifest and
  the machine override.
- **A target is verified or it is not.** `verify` hashes the newest archive the producer recorded
  against its own receipt and restores it in isolation, per kind and honestly: a local path proves
  its bytes are here, while an `ssh` target answers `unchecked` with the reason rather than a claim.
- **The interval is data.** `backup_policy.interval_hours` is written from the surface, where `null`
  is `off`.

## The declared schedule is the bootstrap, not the destination

`capabilities/backup/service.toml`'s `schedule = "24h"` still runs, unchanged. An earlier attempt
deleted the manifest outright — a `kind = "process"` manifest needs `schedule` or `port`, and the
schedule was only ever a placeholder — which also deleted the only automatic backup on the machine.
That was the wrong reading of "make the timing customizable" and it was reverted the same session
(`tools/service-runner.sh install-persistence backup`).

`ISA.md` F0 states the handover: the manifest keeps its `schedule` until the process has a `port`,
and the interval then lives in `backup_policy` where the surface writes it. Until that commit
lands, the timer keeps firing and the policy table is inert.

## Why this is a capability and not a LaunchAgent

It was a LaunchAgent, hand-written, for weeks. `tools/doctor` reported it as an orphan on every
run — a unit no manifest owned, so nothing versioned its schedule and a rebuilt machine would have
come back without it, silently.

Doctor's advice was `remove-persistence`, which would have deleted the only scheduled backup on
the machine. **The check was right about the shape and wrong about the remedy.** An orphan unit is
a declaration gap, and a gap can be closed from either end; here the missing end was the manifest.

That unit also named one capability, `backup.sh store`, while three declare a contract — so the
vault and finance were outside the schedule that was supposed to protect everything.

## Why this shape: a finished run is not a checked backup

Every one of this capability's historical failures was something reporting success without looking
at the artifact. The schedule reported success while backing up nothing for 8.3 days, because
`bun` was missing from launchd's PATH and an empty derived set read as "no contracts declared".
The vault's first archive shipped 704 MB, verified its byte count on the target, wrote a receipt
and could never have been extracted. The destination's eviction detector searched for a
placeholder form the file provider stopped writing years ago, and answered 0 against three evicted
archives. None of those is a subtle bug. Each is a tool trusting its own report.

So the run and the check are separate, and the check reads the artifact:

- `tools/doctor`'s **Backups** section reads the receipt for its age against the capability's own
  `backup_advise_days` / `backup_stale_days`, then resolves the target the *receipt* names and
  looks at the archive: present, right byte count, and not a cloud placeholder. It stats, never
  reads — one of these archives is 4 GB and evicted, and opening it would download it.
- `tools/doctor`'s **Scheduled producers** section asks whether the timer behind this capability
  fired at all, and what its last run exited. A `schedule` job has no supervisor, so it cannot be
  "down"; it stops, and nothing else notices.
- `tools/backup.sh` counts evicted archives at a `kind = "local"` destination on every run, in both
  forms a provider writes them — the legacy `.icloud` placeholder and the modern `SF_DATALESS`
  flag on the file's own name.

For `store` on this machine's iCloud Drive target, `backup.sh` now has a separate gate. It waits
for the new item's uploaded status, then, when retention is due, restores the preceding archive
from its immutable receipt before removing older uploaded items. A cloud-only item is downloaded
for that check and may be offloaded again later. This costs roughly one store-archive download
per daily pruning run after the 14-archive limit is reached. The upload flag alone is not a
recovery test; doctor remains offline and reports an offloaded latest archive as requiring a
network restore rather than telling the operator to pin the directory. Other capabilities keep
their existing local and SSH contracts; a 4 GB archive is not downloaded by implication.

**The named limit.** An archive on an `ssh` target is not verified. Reaching it costs a round trip
and an unlocked vault agent, and doctor is offline by contract — so that row says it was not
checked, and why, rather than passing over it. The prevention half has a limit too: macOS records
Finder's "Keep Downloaded" as an extended attribute a tool can read and no CLI can set, so `pin the
destination` stays an operator action for other iCloud-backed capabilities. For the store-only
cloud gate, the operator may remove that pin in Finder; the code does not evict files itself.
