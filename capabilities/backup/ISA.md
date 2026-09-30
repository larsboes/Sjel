# ISA · backup

Capability-local claims, their falsifiers, and the plan this capability is being rebuilt to. The
repo-wide `ISA.md` does not own this work.

## Problem

**The timing of every backup on this machine is a source change.** `capabilities/backup/service.toml`
declares `schedule = "24h"`, so the interval lives in a tracked manifest: an operator cannot turn
backups off, cannot choose a different interval, and cannot see the schedule as a decision. The
schedule is also the only thing that makes the manifest valid, because a `kind = "process"` without
a `schedule` needs a `port` that does not exist — which determines how the timing can leave the
manifest, not whether automatic backups should stop.

**The target is a single role, chosen in the private overlay, with no per-run choice.** `backup_target`
resolves one target id for the whole machine. There is no way to name a target for one run, no record
of which target was verified and when, and no refusal when the target is not attached.

**A failed run can still read green.** `tools/doctor`'s Backups section reads the latest receipt;
`tools/backup.sh` writes that receipt only after a run lands. Measured 2026-09-29: store's iCloud
target had been failing to upload for days (`CKErrorDomain:1`, 5 pending uploads), two gated runs
exited non-zero, and doctor still reported `store — backed up 0.0d ago`. The one signal meant to
notice a stale backup was reading the last success and calling it current.

That measurement is what makes the gap expensive rather than cosmetic: the machine held **no
verified copy reaching offsite**, and every surface that could have said so was quoting a receipt
from the last time it worked.

## Vision

**Steerable backup management.** A backup happens when the operator asks for one, or on the
interval the operator sets. Targets are declared data with a kind; a run names its target; every run
and every verification is a row; and the timing is a stored policy the panel writes (off, 24h, or
another interval) rather than a manifest edit. Automation is a per-target policy — the same switch
the UI writes — not a second mechanism.

`tools/backup.sh` and `tools/restore.sh` keep moving and checking the bytes. What is missing is the
surface that owns *when*, *where*, and *what happened*.

## Out of Scope

- **Moving bytes.** The mechanism stays in `tools/`. This capability drives it and records it.
- **Heuristic target selection.** "Back up to whatever looks available" is how a backup lands in the
  wrong place. The operator names the target; the machine only refuses a target it can prove is not
  there.
- **An encrypted offsite repository (restic).** `tools/backup.sh --stream` already exists to pipe an
  archive into one, and `toolchain.toml` already names restic. It is a future *target kind*, not part
  of this rebuild.
- **Backing up anything that does not declare a `backup_target`.** The set stays derived.

## Principles

1. **Manual by default, and never silently periodic.** Every timing that runs without a person is a
   stored policy with a visible state and an off switch.
2. **A target counts only after a rehearsal.** Produce, retrieve, checksum, restore in isolation.
   Until that passes, the target is listed as unverified and is not offered for a real run.
3. **A finished run is not a checked backup.** The check reads the artifact, not the exit status.
4. **A failed attempt is a visible fact, not an absence.** A run that fails writes a row, and the
   surfaces read it. Silence must never be the way a failure is represented.
5. **Verification is per target kind.** A folder, a removable disk and a remote host cannot share
   one definition of "verified".
6. **The mechanism stays in `tools/`; the capability owns records, policy and the surface.**

## Constraints

- **Portable shell.** `tools/` stays bash 3.2-safe with argv arrays, per `CONTRIBUTING.md`. New
  mechanism code does not belong here, and neither does a new runtime.
- **The shared store.** Records live under the `backup_` prefix in the store every capability opens;
  `libs/sjel-store` owns migrations. This capability owns a table prefix, not a file.
- **The HTTP surface is an adapter.** `libs/sjel-server` for the process, `panel_port` /
  `panel_path` for the dashboard mount, one panel per capability.
- **Full Disk Access is a real requirement, not a manifest checkbox.** The vault's contract reads
  Obsidian's iCloud container, which a process without FDA cannot see. The scheduled job needed the
  signed launcher for exactly this reason; whatever replaces it must state how it gets there
  (`tools/fda-launcher/install`).
- **A run can be long.** A store archive is ~47 MB, the knowledge-base archive is 4.9 GB. Triggering
  is asynchronous: start, then poll. A request that blocks for minutes is not a panel.

## Goal

For each capability with a backup contract and each declared target, answer: **when did a run last
land, what did the artifact check say, and is the target verified** — then let the operator trigger a
run, choose the target, and set the timing.

## Features

Each is a claim, and names the probe that would falsify it.

### F0 · The declared schedule keeps running while the surface is built

**Claim.** Automatic backups continue on the declared 24h interval until the panel can own the
timing, and the handover leaves no gap in coverage.

> *Probe:* `tools/doctor`'s Scheduled producers reports `backup` with a recent production and
> `launchctl list` holds the unit, through every commit of steps 2 and 3. Falsified by a commit that
> leaves the manifest without a valid `schedule` or `port`, or by a machine where the unit is
> installed but no run follows.

**Claim.** The handover is one structural change, not a change to the operator's interval. The
manifest keeps a declared `schedule` until the process has a `port`; after that the interval is
stored policy (F4) and the manifest declares neither.

> *Probe:* at the end of step 4, the manifest declares `port` and no `schedule`, and changing the
> interval in the panel changes a row, not a file. Falsified by an interval that still requires a
> manifest edit.

### F1 · Run records

**Claim.** Every triggered run has a row carrying its capability, target, start, exit status and the
archive's recorded digest, including runs that fail.

> *Probe:* trigger a run against a deliberately failing target; assert the row exists and names the
> failure. Falsified by a failure that leaves no record — the hole this ISA opens with.

### F2 · Target registry

**Claim.** Targets are declared data with a kind, and a target that is not present is refused before
a run starts rather than discovered part-way through.

> *Probe:* declare a target on a volume that is not attached; assert the surface refuses by name and
> no partial archive is written. Falsified by a run that fails after writing bytes.

### F3 · Per-kind verification

**Claim.** Each target kind counts as verified only when its own check passes, and an unverified
target is never presented as verified.

| Target kind | Verified only when |
|---|---|
| iCloud folder | the item reports uploaded, then a preceding archive restores in isolation |
| removable (SSD) | the volume UUID matches the recorded one, then checksum and isolated restore |
| remote host (ssh) | the byte count matches on the target, then retrieve, checksum and isolated restore |

> *Probe:* one rehearsal per kind: backup, retrieve, checksum, `tools/restore.sh` in isolation, then
> assert the recorded verdict. Falsified by a target offered as verified that has no rehearsal row.

### F4 · Timing policy

**Claim.** The interval is stored policy, settable from the panel (`off`, the declared 24h, or
another interval), and changing it is not a source change.

> *Probe:* set `24h`, assert a run occurs inside the interval; set `off`, assert none occurs; assert
> no manifest changed. Falsified by a timing that only a manifest edit can express.

### F5 · Media live probes *(blocked on F3)*

**Claim.** The `media` capability's live F0 index, F1 ingest and F2 mirror verification run only
after the active target has a rehearsal row.

> *Probe:* the rehearsal row exists; then index both volumes by recorded UUID, audit by sample,
> ingest a staging batch, and verify the mirror. Falsified by a migration into the shared store with
> no verified target.

## Not yet specified

- **Concurrency and cancellation.** Two runs against one target at once, and what a stop does to a
  half-written archive. `tools/backup.sh` refuses a same-second destination collision today; that is
  not a job model.
- **Run-record retention.** How long a failure history is kept, and whether it is pruned at all.
- **A `restic`/offsite target kind.** Named above as future work with nothing built.

## Test Strategy

Follow `tools/backup.test.sh`: a synthetic overlay with a fixture checkout, mocked `ssh`, `docker`,
`date` and `sqlite3`, and assertions on ordering (hold before copy, resume before network) rather
than on timing. Every target kind gets its own rehearsal script, and a refusal is asserted to leave
the previous state intact — the pattern the iCloud gate's tests already use.

The mechanism's suites stay the acceptance gate for the mechanism: `tools/backup.test.sh`,
`tools/restore.test.sh`, `tools/backup-archive-guard.test.sh`, `tools/backup-receipts.test.sh`,
`tools/icloud-store-backup.test.py`, `tools/doctor.test.ts`.

## Decisions

**2026-09-29 — manual-first, and timing becomes stored policy.** The operator decides *when*;
the interval is stored policy the panel writes, starting from the declared 24h. The fixed manifest
schedule is the bootstrap, not the destination. Evidence: the operator asked for a manual flow with
UI-managed timing, and the fixed schedule cannot be turned off or re-timed without a source change.

**2026-09-29 — the declared 24h schedule stays until the panel exists.** A first attempt removed the
manifest outright, since a `kind = "process"` manifest needs `schedule` or `port` and the schedule
was only ever a placeholder. The operator stopped it, correctly: deleting the manifest also deleted
the only automatic backup on the machine, and the intent was to make the timing customizable, not to
remove the feature. The manifest and its LaunchAgent were restored in the same session
(`tools/service-runner.sh install-persistence backup`), and the schedule runs unchanged until step 4
moves the interval into stored policy and gives the manifest its `port`.

**2026-09-29 — v1 targets: the iCloud folder and the Extreme SSD, with the home server declared.**
The iCloud folder and the removable SSD are the two that exist here. The home server is listed as a
placeholder so a real target can be added without redesign; it is not configured, and the earlier Pi
copies were deleted at the operator's request.

**2026-09-29 — the manifest enters step 2 before the schedule leaves it.** A `kind = "process"`
manifest needs `schedule` or `port`. The manifest therefore keeps its declared schedule while the
process is built, and gives it up only in the commit that adds both the `port` and the stored policy
that replaces it. No commit leaves this capability without either a valid timer or a running process.

**2026-09-29 — the surface is sjel-status, and this capability is a library.** The plan said a new
process with its own port and panel. Reading the tree first showed `sjel-status` already serves
`GET /api/sjel-status/backups` and `POST /api/sjel-status/capabilities/{name}/backup`, already opens
the shared store, and already links a capability's store module (`capabilities/devices`). A second
process would therefore have been a second trigger and a second truth for one job. So the tables,
the runner, the verifiers and the policy loop live here, and sjel-status links this crate and serves
the routes. `capabilities/backup` keeps no `service.toml`: a library needs no port.

**2026-09-29 — the mechanism does not move.** `tools/backup.sh` already handles ssh, local and stream
targets, and its iCloud gate already implements the one per-kind check that matters most here.
Rewriting it in Rust would trade a tested mechanism for an untested one.

## Anti-claims

- **Not a backup implementation.** It drives `tools/backup.sh` and records what happened.
- **Not unattended by default.** The interval is a stored policy the operator owns, and `off` is one
  of its values; nothing in this capability invents a cadence of its own.
- **Not a claim that every target is verified.** Only a target with a passed rehearsal is offered.
- **Not encryption.** An archive on an unencrypted removable disk is readable by whoever holds the
  disk; that is an accepted, stated trade for the offline copy.

## Plan

**Step 1 — the plan, recorded (this commit).** `ISA.md` states the problem, the claims and the
phases. Nothing about the running backup changes: the manifest, its `schedule = "24h"` and the
LaunchAgent stay exactly as they are. Deliverable: a recorded plan, and a timer still firing.

**Step 2 — the surface. DONE (2026-09-29).** `backup_` tables in the shared store, and routes on
sjel-status: trigger with an optional target, `/backup/targets`, `/backup/runs`, `/backup/policy`,
`/backup/verify`, and `attempt` on `/backups`. Records every attempt, including failures (F1). The
declared schedule still runs, so coverage never broke. Verified live against the real target with a
scratch database: the rehearsal hashed and restored a real archive, and the policy loop started two
runs and recorded one landing and one failing with its reason.

**Step 3 — targets and their verifiers. DONE for the local kind (2026-09-29).** Targets are read
from `tools/backup-all.sh --list` and `--targets-json`, so the derivation stays in one place; a run
names one; a target keeps the verdict of its last rehearsal; `unchecked` is what an `ssh` target
answers, with the reason. Deliverable met for local: `verify` refuses what it cannot prove.

**Step 4 — timing policy. DONE (2026-09-29).** The interval is stored policy with `off` as `null`,
writable and readable from the surface, and the loop that acts on it runs inside sjel-status. The F0
handover was made in the order that keeps coverage: the policy row was set to 24h first, then the
manifest and its LaunchAgent were removed. One timer owns the interval now, and it is the one an
operator can change.

**Step 5 — media live probes.** F0 index and audit, F1 ingest once a staging export exists, F2 full
mirror verification plus a real unmount (F5). Deliverable: the media ISA's live claims, moved from
"untested" only where a probe passed.

## Probe record (2026-09-29)

- **F1 passes live, and is the claim this work exists for.** With a scratch database and a
  scratch overlay narrowed to two small contracts, the policy loop started two runs:
  `packs` landed `packs-20260930T054422Z.tar.gz` (exit 0, archive identity recorded) and
  `finance` recorded `exit=1` with `backup.sh: declared backup path is missing` — a failure that
  before this left only a log line. The attempt marker was then read by `tools/doctor`, which
  reports a failed attempt as a finding even while the receipt's age still looks fresh.
- **F3 passes for the local kind.** `POST /backup/verify` hashed `finance-20260929T210314Z.tar.gz`
  against its recorded receipt and restored it in isolation: verdict `verified`. An `ssh` target
  answers `unchecked` with the reason, which is the honest value, not a claim.
- **F2 passes.** `GET /backup/targets` resolves the declared target from the overlay
  (`kind=local`, `present=true`, five declaring capabilities) and re-reads it on every call, so a
  capability that starts declaring a contract appears without a restart.
- **F4 passes, and F0 with it.** The interval round-trips (`24` → stored → `null` for off), `0` is
  refused by name, and an unknown target is refused with 404. The declared schedule and its
  LaunchAgent are gone; the stored 24h policy is what fires, and the first sweep under it recorded
  `finance exit=0` with its archive, `store exit=1` **with the archive it produced and the upload
  gate's reason**, and `knowledge-base` still running. That store row is the whole point of this
  work: the bytes landed, the verification failed, and the failure is a record with a log rather
  than an absence.
- **Two defects the live run found, both fixed:** the rehearsal selected the first declared
  capability rather than one whose archive is actually at the target, and `restore.sh` requires the
  destination's *parent* to exist, so the scratch directory is created first.
- **Not built: the dashboard page.** The surface is JSON plus sjel-status's existing panel; a
  picker and an interval control in `dashboard/` are the remaining UI work, and the API they need
  is in place.
- **Not built: an offsite or encrypted target kind.** `tools/backup.sh --stream` exists to pipe an
  archive into one; nothing consumes it yet.

### A mistake worth recording

Testing the surface used a scratch overlay whose `config/*.toml` were **symlinks** to the live
overlay's. Editing "the scratch copy" therefore rewrote the live `machine.toml` (enabled set) and
the live `systems.local.toml` (the backup target's path), which made `doctor` report a real archive
as missing. Both were restored — `machine.toml` from git, the untracked `systems.local.toml` by
hand — and the receipt a test run had overwritten was reconstructed from the archive it names.
The lesson is specific and cheap to apply: a scratch overlay must **copy** the files it intends to
edit, never symlink them.
