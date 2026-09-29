# store

The one SQLite file every capability's tables live in. Nine capabilities open it directly, so
cross-domain joins are a single connection; nothing runs here, and the manifest exists so one
owner declares how the file is backed up. Why a shared store exists at all is below; how a
capability talks to it is `libs/sjel-store/README.md`.

## Verdict

**Adopt, one file, `rusqlite` with the `bundled` feature.** The licence and the measurements
behind the choice are `upstreams.toml` `[rusqlite]`; the version requirement is `Cargo.toml`'s,
and Dependabot moves it. Bundled rather than the host's
libsqlite3, so the deployment does not depend on whichever SQLite macOS or an Alpine image happens
to ship — the failure that avoids is a build that works on the developer's machine and silently
loses a function on the host.

**One file, not one per capability.** Cross-schema joins within one database are a single
connection with no `dblink` or foreign-data-wrapper machinery. A file per capability would have
bought migration isolation Axon already gets from table prefixes, and paid for it with exactly the
correlation queries the shared store exists for. The prefix IS the old schema: `comms.feed_items`
became `comms_feed_items`, one namespace, one file.

**Postgres held this role until 2026-08-27** (PRD Q45, `upstreams.toml` `[postgres]`). What the
retirement removed was cost — a container, a managed volume, a password, a watchdog, a logical-dump
backup contract and a CI service container — for a database taking 0.30 commits/s whose 462 of 464
columns were SQLite-native. What it kept is this section: the reason for one shared store rather
than one per capability is the product thesis, not the server.

## Why this shape: cross-domain correlation

The differentiator over an existing travel-search engine, an existing event scout, or an ad-hoc
"ask an AI to search" is **persistent cross-domain correlation**: events, cheap travel, the
operator's own availability and (later) people all scored against one interest profile and
queryable *against each other*, with a memory of what has already been seen and judged. If this
system does not correlate across domains, there is no reason to have built it instead of a flight
search, an event site and a chat tab.

**What the phase labels in the source mean.** Several doc-comments across `scouting` and `transit`
name a phase; the numbering is scope, not status, and it is defined only here:

| Phase | Scope |
|---|---|
| 1 | Scouting memory: `status` (`new`/`dismissed`/`saved`) on `opportunities`, plus a `source_state` per-adapter cursor so re-runs skip already-judged items |
| 2 | The shared store: scouting off its own SQLite file, `transit` with its own `trips`/`trip_legs` tables, its fare search wired in as a scored scouting source |
| 3 | Fuzzy/triggered trip-search sessions (`transit plan`), built on Phase 2's tables |
| 4 | The `calendar` capability: availability windows, scoped rhythms, events, day views — and the feasibility verdicts that join it against `scouting_opportunities` |
| 5 | People as time-windowed entities, plus a suggestion engine over everything above |

**What the correlation layer has to answer.** These are the acceptance criteria the design is
judged against, not a roadmap:

1. A constant background scan of events and cheap travel, continuously scored against an interest
   profile, with dismissed items staying dismissed.
2. A triggered fuzzy trip search — "in September I feel like a trip" — as soft destination
   expansion plus date-window sampling into a persistent session.
3. Re-runs that skip already-judged items rather than rescoring the world.
4. Every scored opportunity carrying a feasibility verdict from the operator's calendar:
   `free` / `needs-travel-day` / `conflicts`.
5. Cost as a cross-cutting score dimension, so "cheap" means the same thing for a flight, an event
   and (later) a couch to sleep on.
6. People as time-windowed entities — a friend's *residence* window and *availability* window are
   structurally the same shape as a `calendar_entries` row, which is why Phase 4 came before
   Phase 5 rather than after it.
7. Trip overviews that assemble the above into what to do and who to invite.

**Where the join itself lives is still open**, and it is the first decision the correlation layer
forces: a SQL view across prefixes, or a dedicated module. The calendar README's "Correlation
contract" section specifies the verdict protocol; the query shape and volume are now known, so the
call is no longer hypothetical. Whether a `correlations` table is worth having stays deferred until
Phase 4's query patterns have actually been exercised.

## Considered and declined

**Surfacing `transit`'s trip-search journeys back into `scouting_opportunities`.** It would make
trip results show up in the backlog alongside events, which sounds like a free win. It is not: it
needs a scouting-side adapter reading from transit sessions, which reverses the established
dependency direction (`scouting → transit`, never the reverse). The direction is a correlation-layer
design decision, and it belongs to whoever builds that layer — not a side effect of wanting one more
row type in the backlog view.

## Layout

| Path | Purpose |
|---|---|
| `<overlay>/data/axon/axon.db` | the database. Resolved by `sjel_config::database_path()`; `SJEL_DB_PATH` overrides it, and is the one variable that moves a deployment |
| `<overlay>/data/axon/axon.db-wal`, `-shm` | SQLite's own sidecars. WAL mode is set on every connection open (`libs/sjel-store`), not stored in this repo |
| `<overlay>/data/axon/axon-local-*.lock` | cross-process admission locks, one file per inference backend (`capabilities/comms/src/local_gate.rs`). Not state, and deliberately not backed up |
| `service.toml` | `kind = "data"`: the backup contract, and the only manifest that owns this file |

There is no env template and no password. A file has no connection string, which is most of the
point: the DSN, its redaction helper and the six `*_TEST_DATABASE_URL` variables all went with the
server.

## Commands

```bash
sqlite3 "${SJEL_DB_PATH:-$SJEL_OVERLAY_ROOT/data/axon/axon.db}" '.tables'
tools/backup.sh store            # live `sqlite3 .backup`, no service held down
tools/restore.sh store <archive> # integrity_check + table/row counts, into an isolated dir
```

`tools/service-runner.sh start store` refuses by name: `kind = "data"` declares a file, and the
capabilities that read it are the processes.

On this machine, `store` ships to a local iCloud Drive directory. The backup job first checks
its new archive's local SHA-256 and asks macOS whether that specific item finished uploading.
Before retention removes anything, it downloads the preceding archive if needed, verifies its
original receipt and runs `restore.sh` in isolation. Only then does it remove archives past the
14-archive limit. Removing an offloaded item deletes it from iCloud too; it is not a way to free
local space. `--no-prune` skips the older-archive check and deletion, but still requires the new
upload. A failed check leaves the archives in place and makes the backup run fail. Immutable
per-archive receipts under `<overlay>/backup/receipts/history/store/` survive newer runs replacing
the latest receipt.

The iCloud upload flag is the operating system's acknowledgement, not a cloud-side restore test.
When the folder is not pinned with Finder's “Keep Downloaded”, macOS may offload old archives.
The backup job will download the preceding one on demand; offline recovery from an offloaded
archive is impossible. The removable Extreme copy is manual and outside this scheduled contract.

## Gotchas

- **Backup is a live `.backup`, never a raw file copy and never a cold one.** Raw-copying a
  database with an open WAL yields a torn snapshot. A cold copy would mean stopping nine
  capabilities to read one file, which is an outage, not a backup. `sqlite3 .backup` is correct
  here for the reason it was *wrong* for vaultwarden on 2026-07-25 (`tools/backup.sh`): SQLite's
  WAL coordinates readers and writers through shared memory and requires every connection on one
  host, and every reader of this file is a host process with no virtiofs mount in between.
- **The backup contract names a path, and `SJEL_DB_PATH` can move the file out from under it.**
  A deployment that sets it must update `backup_sqlite_online` too. `tools/backup.sh` refuses to
  run on a missing source rather than shipping an archive of nothing, so the mistake is loud.
- **Timestamps are TEXT in one canonical 29-character format** (`sjel_store::NOW`). Two widths in
  one column stop `ORDER BY` being time order; `libs/sjel-store/README.md` has the format and the
  translation table, and every capability writes through that constant rather than its own.
- **One writer at a time.** WAL plus `busy_timeout` is what makes nine processes on one file safe
  at the measured write rate. A transaction that reads and then writes takes the write lock up
  front (`TransactionBehavior::Immediate`) — a deferred one answers a failed upgrade with
  `SQLITE_BUSY`, which `busy_timeout` deliberately does not retry.
- **Nothing here is enabled per machine except by name.** `store` belongs in the overlay's
  `machine.toml` `capabilities` list, in the slot `postgres` used to hold, so the backup surface
  finds its contract in the registry.
