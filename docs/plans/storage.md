# Storage: Turso, backups, and the phone without the Mac

The goal: every capability and the phone on one database engine, one database file per
capability or agent, a phone that reads and writes on its own and syncs when it finds the Mac, and
backups that prove they can be restored. Priority 3 in [`PLAN.md`](../../PLAN.md), except the
storage work Priority 1 needs for the phone.

## Rotation

`sjel setup rotate turso` replaces the primary's signing key in one act. This sqld holds no data
and has no client a person cannot reprovision, so there is no staged key to promote and nothing to
roll back. The journal it writes *is* the inhibit marker that `sjel setup apply turso` and the
managed `sqld` starting verbs consult.

- **Authorization** is a terminal on stdin and stderr, `--confirm`, and a typed phrase. An agent
  session is refused outright.
- **Preflight** refuses a primary answering on the manifest port, installed and loaded boot
  persistence, and a missing or unpinned `sqld`. Nothing is started, installed or enabled.
- **Order**: the journal is written and fsynced first; the old-key probe token is minted while the
  old key is still active and its signature checked; the previous private half is retained at
  `turso-rotation-previous`; the new key reaches the Keychain before the public key file, so
  `signing_key` never signs with a key sqld rejects; the replacement is journalled, then proven.
- **The proof** is a disposable `sqld` on a loopback port it reserves, over a temporary database,
  with the new public key: `probe_rotation` requires a real `SELECT 1` for a new-key token and
  explicit 401/403 for the old-key and absent ones.
- **Failure** keeps the marker, exits nonzero and never restores the old key. The retained private
  half is deleted after the proof, and the report says plainly when it survives.
- `sjel setup rotate recover` reports the journal and the public key file. It reads files only and
  clears nothing.

Open: the proof establishes key mechanics, not the live primary; the persistence answer is
`persistence-status` alone, so a directly started `sqld` is caught only by the port check; and
reprovisioning clients — this machine's own recorded token included — stays a separate command
(`sjel setup apply turso --remint`).

Not run: an end-to-end rotation on a live deployment, the live Keychain read and delete path, a
power-loss test at each durable write, and any check of a started primary.

## 1. Facts (checked 2026-10-25)

- **One shared file.** Capabilities open `<overlay>/data/sjel/sjel.db` through
  `sjel_config::database_path`.
- **Two engines.** 3 capabilities use `libs/sjel-turso` (libSQL): `projects`, `software`,
  `operator-profile`. 21 use `libs/sjel-store` (rusqlite). A binary links one engine, never both.
  The phone's store (`apps/gpui/core/src/store.rs`) is libSQL too.
- **No embedded replicas.** `sjel-turso` dropped them on 2026-10-22 (`libs/sjel-turso/README.md`,
  "Embedded replicas: removed"). That also removed the ten advisories they pulled in: the CI
  Security job passes on `main` (runs for `8ebd667d`, `6ee15b19`, `b6bf56c8`).
- **Left over from the replica path.** `services/sqld` runs a primary that holds no data and
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
  1. `crates/sjel-status/src/status/search.rs` (the finder's server search).
  2. `apps/gpui/core/src/data/repo.rs`: the desktop app reads `projects_records` and
     `calendar_entries` from `sjel.db` directly.
  3. `sjel mirror build`, which copies tables out of the shared file.

**Docs to correct when the split starts.** `services/store/README.md` and
`website/content/architecture/storage.md` justify one file with cross-schema joins. Only one statement in all
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

### Vault holds learning only

Decided 2026-11-08 (`PLAN.md`, Decisions). The vault keeps learning notes: `teach`,
`practice`, claim notes and subject knowledge, plus the Obsidian plugins that render them. Everything else that reads or
writes the vault moves to Sjel data or retires. Counted 2026-11-08 with `git grep -i -E
"\bvault\b|obsidian"`: about 300 lines in 40 Markdown files and 108 code files outside
`crates/vault/`.

State on 2026-11-08, after a pass over code and overlay config:

| Component | State |
|---|---|
| `finance`, `trips` | Done before this decision: subscription notes live in `projects` (2026-10-20), plan copies in `<overlay>/data/trips/plans/` (`crates/trips/src/projection.rs:13`). Stale comments in `plan_search.rs` fixed. |
| Overlay config | Done: scouting's two vault sources and the dead `obsidian` blocks in trips, finance and interior removed. `knowledge.toml` and the vault state mount stay for Learning. |
| `README.md`, `CONTRIBUTING.md`, `Packs/knowledge`, `ARCHITECTURE.md` | Done: they describe the vault as learning only. |
| `crates/comms` (`markdown_links.rs`, `server/markdown_links.rs`) | Done: the scanner, its two routes and the `markdown_link_sources` config key are named for Markdown rather than the vault. No link source is configured, so it still reads nothing. |
| `crates/scouting` (`sources/markdown_notes.rs`) | Done, rename only: file, type, factory key (`markdown-notes`), README and example config. The opportunity id namespace and the stored `obsidian_markdown` source keep their values, because rows already carry them. |
| `crates/scouting` (`vault_linker.rs`) | Open, rename only. It matches opportunities against any configured events directory; the name still says vault. Found in the same pass and not in the count above. |
| Relevance profiles (`comms-relevance-shadow.json`) | Done: the three dead `profile_path`s are gone and the frozen `text_variants` remain. Restoring personal lenses stays `operator-profile`'s call. |
| `obsidian` as a record source in `entities`, `places`, `scouting`, `trips` | Keep. It records where historic rows came from. |

## 3. Phone standalone

The phone works without the Mac for notes, tasks, capture and calendar today. Unpaired, Mail and
Documents show a card that names the Mac as their source (`376a85ed`); Agent and People already
did.

The custom outbox stays. Turso's sync engine (`turso` crate, feature `sync`, read at `2709799`,
0.9.0-pre.2) replicates the whole database file: push replays local changes as row upserts
(`sync/engine/src/database_replay_generator.rs`), so a phone write would reach the Mac's tables
without passing the owning capability, against the conflict rule below. Its self-hosted server
(`tursodb --sync-server`) has no authentication and is for local and development use
(`docs/sql-reference/cli/command-line-options.mdx`), while the phone talks only over the signed
device client. Partial sync is marked experimental. Not measured, and not needed for this
answer: opening the store file, recursive CTEs, vector queries, crash recovery, an iOS build.

### A sync contract per domain

Today each writable domain replays through individual routes, with rules spread over
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

#### State per part

| Part | Calendar | Projects |
|---|---|---|
| Pull | Done: `GET /api/entries/changes` with tombstones (`crates/calendar/src/store.rs`, `changes_since`) | Lists every live project, then reads records whose `updated_at` moved, then sweeps (`core/src/sync.rs`, `pull`) |
| Push | `POST`, `PATCH` and `DELETE` on `/api/entries`; a 404 on `PATCH` re-creates the entry | Dashboard routes; a create is detected by matching refusal text (`Reply::missing`) |
| Idempotency | None. A create that reached the Mac but whose answer was lost is sent again and makes a duplicate. | None, same risk |
| Order and refusal | Done for both: one refused record blocks only itself, and `_sync_refusals` keeps the reason (#244, closed) | |
| Pending pull | Done for both: `dirty_keys` skips records with an unsent write | |

The calendar cursor is a second-resolution stamp that is unique only within one process
(`now_text`, `LAST_STAMP`). A second process that writes entries, or a transaction that commits
after a pull read a higher stamp, can produce a change below the phone's cursor, which the phone
then never pulls. Not observed; this is an inference from the code.

#### Steps

Each step is one commit with its own check.

1. **Contract types.** `Changes<T> { cursor, upserted, deleted }`, `Tombstone`, and the
   idempotency header name, in `crates/sync-contract`, linked by the capabilities and
   `apps/gpui/core` (S1). Done when calendar's feed serializes through it and its feed test passes
   unchanged.
2. **Sequence cursor.** Each owner gets a change log with an `INTEGER PRIMARY KEY` sequence,
   written in the same transaction as the record and its tombstone. The feed answers
   `seq > since`. Calendar first, keeping the old stamp cursor readable for one release.
   Done when a test with two store handles writing in the same second loses no change.
3. **Idempotent push.** The phone mints a key when it queues a write (new column on
   `_offline_mutations`) and sends it on every replay. The server keeps a ledger of
   key → answer and returns the stored answer on a repeat (S2). Done when a test sends
   a create, drops the answer, replays it, and finds one record on the Mac.
4. **Projects feed.** `GET /api/changes` in `crates/projects` with tombstones on delete and
   archive. The phone's projects pull switches to it, and the list-and-sweep pull and
   `Reply::missing` are deleted. Done when the projects sync tests pass against the feed and a
   delete on the Mac removes the record on the phone.
5. **One domain shape on the phone.** `core/src/sync.rs` runs every domain through one
   `push`, `pull` and `apply` shape; a new domain adds a table, a feed path and its write
   routes. Captures stay as they are: they are create-only. Done when calendar and projects run
   through it and the file has no per-domain pull loop.
6. **First pairing.** A test where a phone that never paired holds records, pairs, and syncs:
   every record exists on the Mac once and stays on the phone.
7. **Inventory and wishlist on the phone** (D3): the phone store's tables, the feed and
   idempotency in `crates/inventory`, and the phone page (route 11 in
   [`gpui.md`](gpui.md#1-route-matrix), planned). Done when an item added offline reaches
   the Mac once on reconnect and an item changed on the Mac reaches the phone.
8. **People** after Work Package 23 gives entities an outbox.

#### Decisions

- **S1. Contract types** live in one crate, `crates/sync-contract`, the pattern
  `crates/roomplan-contract` set (2026-11-06). A copy per API crate would drift
  ([`restructure.md`](restructure.md#3-layout)).
- **S2. Idempotency ledger** is one table per capability database, written in the same
  transaction as the write, so a write and its ledger row cannot disagree (2026-11-06). Keys
  older than 30 days are dropped; a phone offline longer than that replays without protection.

### Also open

- **Task fields on the phone.** The replay sends a task's done, priority and due only
  (`apps/gpui/core/src/sync.rs`, `replay`), so the phone refuses a status, context or blocked-by
  write (Work Package 30 in [`gpui.md`](gpui.md#open)). They join in step 5's domain shape.
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
   in §3. Answered: it does not (§3).
2. **Remove the replica leftovers** unless step 1 brings replicas back: stop and delete
   `services/sqld`, `libs/libsql-auth`, `tools/sjel-cli/src/setup/turso.rs`, the
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
   only for migrations and one-off reports. When the last capability has moved, `sjel.db` is
   archived, not deleted.
6. **Sync last.** Before any replica or sync service: per-database identity, authorization,
   conflicts, revocation, freshness and data-class admission.
   Sync is not inference: rows are encrypted on the device before they leave it, with the key in
   the Secure Enclave or keychain, so a relay that cannot read them (for example a Cloudflare
   Worker) may carry every class, `c3` included. Data-class admission then applies only to
   plaintext. Open: whether the sync engine read in step 1 supports client-side encryption, or
   whether Sjel encrypts the change log itself.

## 5. Backups

`crates/backup` owns the contracts. The timer runs inside `sjel-status` and starts a
target once its last successful backup is older than its interval (`crates/backup/src/policy.rs`).

Open:

1. **Silent stalls.** On 2026-10-25 six targets were 1.2 days old against a 1-day interval, while
   the `sjel-status` log showed "policy started 1 run(s)" and no receipt changed after
   2026-10-24 11:53Z. A manual `tools/backup-all.sh` the same day completed and wrote to
   `/Volumes/Extreme/Backup/sjel/`, an external drive. First hypothesis to check: the scheduled
   runs started while that drive was not mounted. Either way, a started run must end in a receipt
   or an attempt record, and a missing target volume must show as a failed attempt.
   Found 2026-11-03: the `backup-target` timer has been off (`interval_hours` NULL) since
   2026-10-22 15:12Z, so finance, store, knowledge-base, packs, devices and projects only get
   manual runs. `tools/doctor` now names an off timer, and the timer is back on at 24h
   (2026-11-03). Open: why it was turned off.
2. **A CLI for the runs.** `sjel backup policy` shows and sets the timer. Open:
   `sjel backup status|runs` over `GET /api/sjel-status/backups` and `/backup/runs` (see
   [`tooling.md`](tooling.md)).
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
| [Code indexing](https://docs.turso.tech/guides/code-indexing) | Incremental symbol and chunk index | Only if it fills a measured gap next to `rg` and `sjel search`. |
| [Content caching](https://docs.turso.tech/guides/content-caching) | Skip rereading unchanged files | Measure tokens saved. A cache never skips a security or freshness check. |
| [Secrets vault](https://docs.turso.tech/guides/secrets-vault) | Secret metadata and access audit | Values stay in the keychain (§2). |

The retrieval results that bear on these are in [`agent.md`](agent.md#9-context-and-retrieval).

## History

The 2026-10-21 proposal (phases, the `sqld` topology, the token path, the replica proofs) is in
Git: `git show 3ecf0093:PLAN.md`, section "Historical storage proposal".
