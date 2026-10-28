# Storage: Turso, backups, and the phone without the Mac

The goal: every capability and the phone on one database engine, one database file per
capability or agent, a phone that reads and writes on its own and syncs when it finds the Mac, and
backups that prove they can be restored. Priority 3 in [`PLAN.md`](../../PLAN.md), except the
storage work Priority 1 needs for the phone.

## 1. Facts (checked 2026-10-25)

- **One shared file.** Capabilities open `<overlay>/data/axon/axon.db` through
  `sjel_config::database_path`.
- **Two engines.** 3 capabilities use `libs/sjel-turso` (libSQL): `projects`, `software`,
  `operator-profile`. 21 use `libs/sjel-store` (rusqlite). A binary links one engine, never both.
  The phone's store (`apps/gpui/core/src/store.rs`) is libSQL too.
- **No embedded replicas.** `sjel-turso` dropped them on 2026-10-22 (`libs/sjel-turso/README.md`,
  "Embedded replicas: removed"). That also removed the ten advisories they pulled in: the CI
  Security job passes on `main` (runs for `8ebd667d`, `6ee15b19`, `b6bf56c8`).
- **Left over from the replica path.** `capabilities/sqld` runs a primary that holds no data and
  is backed up daily. `libs/libsql-auth`, `tools/sjel-cli/src/setup/turso.rs` and the
  `SJEL_TURSO_URL` handling in `tools/sjel-cli/src/runner.rs` and `apps/gpui/core/src/setup.rs`
  serve only that primary.
- **The phone's writes.** Writes queue in `_offline_mutations`. `core/src/sync.rs` replays them in
  order through the Mac's existing routes and pulls records whose `updated_at` moved. A record
  with an unsent local write is skipped on pull. This covers projects and calendar.
- **The phone's copy.** A read-only snapshot of `PHONE_SET` (`tools/sjel-cli/src/mirror.rs`:
  projects, calendar, entities, places), built by `sjel mirror build`, served by
  `GET /api/sjel-status/mirror` and checked by digest. Comms, documents and finance are left out on
  purpose.
- **Three readers that go around the owners.** Each reads another capability's tables in the
  shared file, and each breaks when the file splits:
  1. `capabilities/sjel-status/src/status/search.rs` (the finder's server search).
  2. `apps/gpui/core/src/data/repo.rs`: the desktop app reads `projects_records` and
     `calendar_entries` from `axon.db` directly.
  3. `sjel mirror build`, which copies tables out of the shared file.

**Docs to correct when the split starts.** `capabilities/store/README.md` and
`docs/architecture/storage.md` justify one file with cross-schema joins. Only one statement in all
capabilities spans two table prefixes, and it is a migration helper.

## 2. Decisions

- **One database per capability or agent** (2026-10-23). This supersedes "one database, for now"
  (2026-10-21). Service records stay behind their capability contracts.
- **Turso is a measured upgrade, not a feature tour** (2026-10-23). Adopt a feature only when it
  makes the system simpler or faster by a measurement.
- **Secrets stay in the OS keychain.** A database-backed secret store needs its own security
  decision: key custody, process access, replication, backups, output scrubbing.
- **Data on the phone** follows the rules in [`gpui.md`](gpui.md#what-may-live-on-the-phone):
  online first, offline only what the user chose, no second writer (2026-10-25).
- **Personal data is outside Git repositories** (2026-10-26). Private configuration remains in
  the overlay; databases, capability files and unsorted archives move to a separate local data
  root. The knowledge-only vault remains a separate directory under the developer workspace, with
  its own fresh Git history and no remote until configured. Only selected data is kept on the
  phone. External-drive backups may be unencrypted for now.

### Local data and knowledge-vault migration

The existing overlay contains both configuration and data. A separate data root is not yet a
runtime contract. On 2026-10-28 the vault pointer, state mount and backup source were switched to
the knowledge-only copy. A backup of that copy and a backup of the original vault both passed
isolated restores. The original vault and its history remain in place. The new vault lacks the
contract required by the Obsidian skill; service reload, Obsidian rendering, link resolution and
complete preservation of non-knowledge files remain to be verified before migration is complete.
On 2026-10-28, `vault compare` found 174 newly unresolved link occurrences across 1,156
unchanged destination notes; 64 explicitly target `Atlas/Media/` and 121 occur in frontmatter.
Those categories overlap. The comparison excludes changed or ambiguous notes; none of the 1,156
destination notes was excluded in this run. Account for the broken references before cutover is
complete.
The machine overlay's Comms projection and two missing link sources no longer point at the old
vault. Five missing relevance-profile paths were cleared, so relevance has no personal lenses
until replacements are configured. All five names were found in two archive copies with matching
bytes, but archive membership does not establish that those lenses are current; they were not
reactivated. Reconcile them with the `operator-profile` owner and the Focus import before restoring
relevance. `operator-profile` was not present in the enabled capability registry in this check.
Interior now uses its existing overlay data fallback instead
of a missing vault assets path. Stale vault roots in Trips and Finance configuration were cleared;
their current config loaders do not use those roots. Runtime reload and Interior asset resolution
remain unverified. Historical backup configs and retired LifeOS files still contain old paths.
The obsolete Notability-to-vault helper now refuses to run rather than writing into the retired
vault; its source files remain for the Sjel import work in `data.md`.
`tools/service-runner.sh status` reported both Vault and Scouting as stopped while their health
endpoints answered. Their running process ownership and loaded configuration must be established
before a restart; health alone does not prove the configured vault changed.
An rsync checksum dry run on 2026-10-28 compared the original vault against the existing archive
snapshot, excluding `Knowledge/`, application state, Git history and secret-pattern paths. It
reported 87,091 bytes of source files needing transfer. This is a partial comparison, not evidence
that the snapshot contains every non-knowledge file; the full original-vault backup remains intact.
An additive archive intake on 2026-10-28 copied 3,091 files from the original vault, excluding
`Knowledge/`, Git, Obsidian and Smart Environment state, symlinks and secret-pattern paths. An
rsync checksum dry run reported zero bytes needing transfer within that scope. The excluded
material remains in the original vault and its prior full backup; it has not been reconciled.
The intake tool's independent checksum comparison found 3,091 files on each side, zero missing,
extra or changed files, four excluded source entries, and no included symlinks or special entries.
The four excluded entries are the root knowledge, Git, Obsidian and Smart Environment trees;
their contents were not inspected by this comparison.
The archive capability was enabled for a backup attempt, then disabled after staging failed on
filesystem-denied paths and cleanup reported permission errors. The expanded archive has no
verified new backup; the earlier archive backup and original-vault backup remain in place.
The filtered intake alone was streamed to a separate archive on 2026-10-28 and restored in
isolation. On 2026-10-28, the restored tree matched all 3,091 source files by normalized path and
SHA-256, with no missing, extra or changed files. The backup command exited nonzero because
staging cleanup failed. The intake and its backup are on
the same external drive and do not protect against loss of that drive. The earlier statement
about the expanded archive refers to the whole archive, not this filtered intake.

1. Add a configurable data root outside both repositories; migrate each owner without changing
   its records, verify database and file copies, and keep the overlay for private configuration.
2. Give the unsorted archive an independent backup contract and prove an isolated restore. Do
   not treat an offline phone copy as the archive backup. Audit tracked overlay data privately;
   removal from Git tracking alone does not remove prior history.
3. Preserve the knowledge tree, required Bases and assets in the separate vault. Resolve or
   explicitly account for links to material moving to Sjel. Compare source and destination with
   Unicode-normalized paths; do not use a deleting sync to resolve normalization differences.
4. Review Obsidian settings and verify Bases, embeds and links in Obsidian. Update the vault
   pointer, state mount and backup source together only after a new-vault backup and restore pass.
   Keep the original vault until cutover is verified. Fail plugin deployment closed if vault
   resolution fails.
5. Fresh Git history starts without a remote; review files and ignore rules before the first
   commit or remote. The retained original history can contain non-knowledge material.

## 3. Phone standalone

The phone works without the Mac for notes, tasks, capture and calendar today.

### A sync contract per domain

Today each writable domain replays through routes built for the dashboard, with rules spread over
`core/src/sync.rs`. Every new writable domain uses one contract instead, declared in its
capability's API crate ([`restructure.md`](restructure.md#3-layout)):

| Part | Rule |
|---|---|
| Pull | `GET /api/<cap>/changes?since=<cursor>` returns changed and deleted records with a new cursor. |
| Push | Each queued write carries an idempotency key. A replay of the same key changes nothing. |
| Order | Writes to one record replay in the order they were made. The queue stops at the first refusal for that record, not for the whole queue (#244). |
| Conflict | The Mac's owner decides. A refused write stays on the phone with its reason, and the user resolves it. |
| Pull while a write is pending | The record keeps the phone's version until its write is accepted or refused. |

Projects and calendar move to this contract when it exists. Order of new domains (D3,
2026-10-25): inventory and wishlist, then People (needs an entities outbox, Work Package 23).

### Also open

- **Search on the phone** over the store and the copy, so cached reads stay useful offline.
- **Copy contents.** Which tables the copy carries as pages move to the phone, under the data
  rules above.

## 4. Full migration

Each step leaves a working system. Backups before every step (CONTRIBUTING.md, "Backups before
migrations").

1. **Choose one engine for the Mac and the phone.** `libsql` (linked now) or the Turso Database
   crate (`turso`, 0.8.3 on 2026-10-15, `upstreams.toml` `[turso]` is `watch` and NOT READ). Read
   it first, then test on copies of real data: opening an existing file unchanged, the recursive
   queries in `projects`, `json_each` in `calendar`, vector search (`F32_BLOB`,
   `vector_top_k`), concurrent writers, crash recovery, and an iOS build. Unverified until read:
   whether the `turso` crate brings an offline sync engine that could replace the custom outbox
   in §3.
2. **Remove the replica leftovers** unless step 1 brings replicas back: stop and delete
   `capabilities/sqld`, `libs/libsql-auth`, `tools/sjel-cli/src/setup/turso.rs`, the
   `SJEL_TURSO_URL` handling, `tools/install-sqld.sh`, and the `[libsql-server]` and `[sqld]`
   entries. One fewer daemon and one fewer daily backup.
3. **Cut the three bypassing readers** (§1). The finder's search asks each capability's search
   route. The desktop `Repo` reads through the capability API crates. `sjel mirror build` asks
   each owner for an export.
4. **Cut capabilities over to the engine, one per commit.** Order by phone need: `inventory`
   first (D3), then `entities`, `calendar`, then the rest. The pattern is `operator-profile` and
   `projects`: schema, store, handlers, CLI and tests on `sjel-turso`, then `rusqlite` leaves that
   crate's dependency graph.
5. **Split files.** Each capability writes `<overlay>/data/<cap>/<cap>.db`. One writer per file
   ends `SQLITE_BUSY` between capabilities without depending on MVCC. Schema versions use
   `PRAGMA user_version` per file. A read across owners goes through the owner's API. `ATTACH` is
   only for migrations and one-off reports. When the last capability has moved, `axon.db` is
   archived, not deleted.
6. **Sync last.** Before any replica or sync service: per-database identity, authorization,
   conflicts, revocation, freshness and data-class admission.

## 5. Backups

`capabilities/backup` owns the contracts. The timer runs inside `sjel-status` and starts a
target once its last successful backup is older than its interval (`capabilities/backup/src/policy.rs`).

Open:

1. **Silent stalls.** On 2026-10-25 six targets were 1.2 days old against a 1-day interval, while
   the `sjel-status` log showed "policy started 1 run(s)" and no receipt changed after
   2026-10-24 11:53Z. A manual `tools/backup-all.sh` the same day completed and wrote to
   `/Volumes/Extreme/Backup/sjel/`, an external drive. First hypothesis to check: the scheduled
   runs started while that drive was not mounted. Either way, a started run must end in a receipt
   or an attempt record, and a missing target volume must show as a failed attempt.
2. **A CLI for the runs.** `sjel backup status|runs` over `GET /api/sjel-status/backups` and
   `/backup/runs` (see [`tooling.md`](tooling.md)).
3. **Per-kind proof.** A target kind counts as verified only when its own restore passes. Only
   the local kind is rehearsed. iCloud and ssh are unproven.
4. **Documents on the external SSD.** Originals are local. Done when a restore from the selected
   SSD returns a document's bytes and metadata together.
5. **One contract per database file** after the split (§4 step 5), each proven by
   `tools/restore.sh` in isolation.
6. **The phone.** Data that exists only on the phone (queued writes, kept documents) is covered
   by the iOS device backup only. Decide whether that is enough.
7. **A fresh primary with no data.** `backup.sh` fails on a declared database that has never
   existed (seen with `sqld`). It should report "nothing to back up yet". Moot if §4 step 2
   removes `sqld`.

## 6. Agent databases

Candidates for isolated per-agent databases. None is approved because Turso documents a recipe.

| Candidate | Use | Gate |
|---|---|---|
| [AgentFS](https://docs.turso.tech/agentfs/introduction) | Per-agent files, KV state, tool-call trace, copy-on-write workspaces | Beta. Check maturity, licence, isolation and backup fit. MCP exposure only through an allowlist. |
| [AI memory](https://docs.turso.tech/guides/ai-memory) | Durable agent memories with correction history | Measure usefulness and stale-memory correction against current context sources. |
| [Code indexing](https://docs.turso.tech/guides/code-indexing) | Incremental symbol and chunk index | Only if it fills a measured gap next to Graphify. |
| [Content caching](https://docs.turso.tech/guides/content-caching) | Skip rereading unchanged files | Measure tokens saved. A cache never skips a security or freshness check. |
| [Secrets vault](https://docs.turso.tech/guides/secrets-vault) | Secret metadata and access audit | Values stay in the keychain (§2). |

The retrieval results that bear on these are in [`agent.md`](agent.md#9-context-and-retrieval).

## History

The 2026-10-21 proposal (phases, the `sqld` topology, the token path, the replica proofs) is in
Git: `git show 3ecf0093:PLAN.md`, section "Historical storage proposal".
