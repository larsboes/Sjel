# Storage: Turso, and the phone without the Mac

The goal: every capability on one database engine, one database per capability or agent, and an
iPhone that reads and writes on its own and syncs when it finds the Mac. Sequenced after Lane A's
restructure in [`PLAN.md`](../../PLAN.md), except the storage work the phone needs for Lane A.

## 1. Facts (checked 2026-10-25)

- **One shared file.** Capabilities open `axon.db` through `sjel_config::database_path`.
- **Two engines.** 3 capabilities use `libs/sjel-turso` (libSQL): `projects`, `software`,
  `operator-profile`. 21 use `libs/sjel-store` (rusqlite). A binary links one engine, never both.
- **No embedded replicas.** `sjel-turso` dropped them on 2026-10-22
  (`libs/sjel-turso/README.md`, "Embedded replicas: removed"). They cost ten advisories through
  `libsql 0.10.0` (hyper 0.14, rustls 0.22), and they cannot open over a file that holds its own
  writes. `capabilities/sqld` still runs a primary that holds no data.
- **The phone's store.** A local libSQL database (`apps/gpui/core/src/store.rs`). Writes queue in
  `_offline_mutations`, and `core/src/sync.rs` pushes them through the Mac's existing routes and
  pulls changed records over the signed connection. This covers projects and calendar.
- **The phone's copy.** A read-only snapshot of other tables (`core/src/mirror.rs`), built by
  `sjel mirror build` and served by `GET /api/sjel-status/mirror`, checked by digest. Route R1 in
  [`research/offline-on-the-phone.md`](../../research/offline-on-the-phone.md).
- **A coupling to cut before any split.** `capabilities/sjel-status/src/status/search.rs` reads
  domain tables directly.

**Still to correct.** `capabilities/store/README.md` and `docs/architecture/storage.md` justify
one file with cross-schema joins. Only one statement in all capabilities spans two prefixes, a
migration helper. Fix both when the split starts.

## 2. Decisions

- **Target: one database per capability or agent** (2026-10-23). This supersedes "one database,
  for now" (2026-10-21). Service records stay behind their capability contracts.
- **Turso is a measured upgrade, not a feature tour** (2026-10-23). Adopt a feature only when it
  makes the system simpler or faster by a measurement.
- **Secrets stay in the OS keychain.** A database-backed secret store needs its own security
  decision: key custody, process access, replication, backups, output scrubbing.

## 3. Phone standalone

The phone works without the Mac for notes, tasks, capture and calendar today. Next:

1. **More writable domains.** Each needs an outbox replay path through its owner's routes, the
   way projects and calendar have one. People writes on the phone need an entities outbox first
   (Work Package 23 keeps the phone read-only).
2. **Search on the phone.** Local search over the store and the copy, so cached reads stay useful
   offline.
3. **Copy freshness.** The copy says when it was built. Decide which tables it carries as pages
   move to the phone.

Order (D3, 2026-10-25): inventory and wishlist first, then People.

## 4. Full migration

Proposed sequence. Each step leaves a working system.

1. **Choose the engine.** `libsql` (linked now) or the newer Turso Database crate (`turso`,
   `upstreams.toml` `[turso]`, not read yet). Check compatibility with existing files, recursive
   queries, vector search, concurrency and recovery on the chosen engine. API similarity proves
   nothing.
2. **Cut capabilities over one at a time.** The pattern is `operator-profile` and `projects`:
   schema, store, handlers, CLI and tests on `sjel-turso`, then `rusqlite` leaves the dependency
   graph.
3. **Split files.** `<overlay>/data/<cap>/<cap>.db`, with `ATTACH` for a read across owners.
   Cut the `sjel-status` search coupling first. Each file gets its own line in the backup
   contract, and `tools/restore.sh` proves each one.
4. **Sync last.** Before any replica or sync service: per-database identity, authorization,
   conflicts, revocation, freshness and data-class admission. Replicas come back only when the
   phone must write through one and libsql uses rustls 0.23.

Backups before every migration (CONTRIBUTING.md, "Backups before migrations").

## 5. Agent databases

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
