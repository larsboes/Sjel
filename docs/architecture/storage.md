# Storage architecture

Capabilities that use the shared relational store open the same SQLite file and own distinct
table prefixes. Other state, such as mounted media or a vault, retains its own storage contract.
The [store capability](../../capabilities/store/README.md) owns the shared file's backup contract;
[`sjel-store`](../../libs/sjel-store/README.md) owns connection and migration behavior.

## Database specifications

[`sjel-config`](../../libs/sjel-config/src/lib.rs) resolves `SJEL_DB_PATH`, then
`<overlay>/data/axon/axon.db`, with a temporary scratch file as its fallback when no overlay
resolves. [`sjel-store`](../../libs/sjel-store/src/lib.rs) applies these settings to each pooled
connection:

```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
```

SQLite admits one writer at a time. Capabilities open their own connections; there is no separate
writer service.

## Transaction rules

### Immediate write locking

Use `sjel_store::write_transaction(&mut conn)` for a transaction that reads and then writes.
It starts `BEGIN IMMEDIATE`, taking the writer lock before the read. A deferred transaction can
fail immediately when it upgrades a read lock, even with a busy timeout configured.

Migrations run once per database file and table prefix in each process, through
`sjel_store::migrate_once`. The migration transaction also takes the write lock first.

## Embedded replicas

A device can read from a local copy of a remote Turso primary instead of the shared file. The copy
holds the primary's write-ahead log, so reads are local and writes are forwarded to the primary.
[`sjel-turso`](../../libs/sjel-turso/README.md) owns the open call, the first pull before the DDL,
and the background interval. The replica engine is a default feature rather than an opt-in one,
because following a primary has to be configuration: an opt-in feature meant editing a Cargo.toml per
capability, which is the one thing a machine's setup cannot do for itself.

Freshness is a configured promise, not a guarantee. A capability that publishes health reports the
replica's `seconds_since_success` against its interval, and a surface that must be exact calls
`sync_now` before reading. One file is either the shared local database or a replica, never both,
and a replica file that already holds local writes is refused rather than converted.

`sjel setup plan turso` is where a deployment finds out what it still needs: the URL, the
credential, and which capabilities would restart to read them. It writes nothing; `apply --confirm`
records the settings in the private overlay and restarts the followers.

The settings reach a process because `tools/service-runner.sh` passes the overlay's deployment
settings into everything it starts. A deployment-wide setting has one home, and a capability's own
`[capability.<n>] env` entry overrides one name when a machine needs that.

## Path indirection & state mounts

Resolve deployment paths through the owning configuration API. Machine state mounts are declared
in the active overlay's `config/machine.toml`; see the
[machine schema](../../schemas/machine.toml.example).

The shared store uses `backup_sqlite_online`: `tools/backup.sh` takes a live SQLite `.backup`
snapshot. Do not copy an open database file without its SQLite snapshot protocol. Container-owned
SQLite files use the separate cold-backup contract described in the
[service schema](../../schemas/service.toml.example).

## Backups before migrations

No data migration starts without a tested 3-2-1 backup. A command being reversible in source
control does not make its state mutation reversible.

`tools/backup.sh <capability>` writes a self-describing archive, ships it under a partial name,
verifies the remote byte count before renaming it, applies the capability's retention policy, and
records the archive size and SHA-256 in the private receipt. The receipt also records whether
retention ran. A shipment receipt proves delivery, not recovery.

For a recovery rehearsal, run `tools/backup.sh --no-prune <capability>`. This mode ships and
verifies a new archive but does not list or delete older archives. A capability that declares
`backup_sqlite` stays held while all declared host paths and the cold database copy are staged and
the staged copy passes `integrity_check`; it resumes before compression or network access. Failure
or interruption after the hold triggers a resume attempt and exits without shipping the staged
snapshot. A capability that declares `backup_sqlite_online` instead is never held: its database is
copied open, through `sqlite3 .backup`, and the archive records how many tables and rows the copy
held so a restore can check that they came back. That form is correct only where every reader is a
host process on this machine, which is the condition SQLite's WAL states and a container behind a
virtiofs mount does not meet.

Retrieve that new archive into a private scratch location, compare its SHA-256 with the receipt,
then run `tools/restore.sh <capability> <archive> --receipt <receipt.json>`. The explicit receipt
makes the command reject the wrong capability, archive name, byte count or SHA-256 before
extraction. Restore defaults to a new `/tmp` directory and refuses the Sjel checkout or active
overlay as a destination. It separates the recovery stages explicitly:

1. **Retrieve:** copy the named archive from the backup target without applying it.
2. **Extract:** validate archive structure and capability identity, then extract into isolation.
3. **Restore:** expand container-path archives with their recorded modes.
4. **Verify:** require declared roots, run SQLite integrity checking on every declared database,
   and compare an online copy's table and row counts against the numbers the archive recorded.
5. **Clean up:** inspect and then remove the retained scratch tree manually. The tool never
   removes the extracted evidence.

The rehearsal ends in that isolated scratch tree. It never applies recovered files to the live
overlay or starts a restored service against live paths. Any live replacement is a separate,
explicitly approved migration after the recovery evidence has been reviewed.

For a container-path backup, the nested tar remains the ownership and mode authority. Inspect it
with `tar -tvzf <restore-dir>/container-*.tar.gz`; the convenience extraction preserves modes, but
an unprivileged host user cannot re-apply a container's numeric ownership. Apply that archive only
inside the intended disposable or stopped container after the verification pass.

Archives created before the embedded `axon-backup.toml` format require `--allow-legacy`. That mode
uses the standard archive name and current manifest as weaker identity evidence, so it is for a
known private receipt, not an arbitrary file. Actual restore timestamps, digests, application-level
queries and verdicts stay in the private overlay; only a redacted pass/fail result belongs in a
public issue.

Security continues after install: egress, access and agent touches remain observable, while the
logs themselves stay in the private overlay.
