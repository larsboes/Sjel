# Architecture Plan: Controlled Migration from SQLite to Turso (LibSQL)

> **Context**: Sjel currently persists all capability state into a single SQLite database (`axon.db`, ~273 MB, 108 tables) managed synchronously via `rusqlite` in WAL mode (PRD Q45). As the Zed GPUI desktop app evolves, background agents scale, and mobile shells connect, SQLite's single-writer limitation (`SQLITE_BUSY`) and lack of native embedded vector search create friction. This plan lays out a safe, stepwise migration to Turso / LibSQL with strict backup guarantees, zero data loss, and seamless GPUI integration.

---

## 1. High-Level Architecture & System Boundaries

Turso/LibSQL serves as the multi-tenant, concurrent, embedded storage engine backing the Sjel core daemon, desktop GPUI shell, and remote clients.

```mermaid
flowchart TD
    subgraph Clients["Clients & Presentation"]
        GPUI["Zed GPUI Desktop App<br/>(macOS Native, 120 FPS)"]
        Mobile["iOS Mobile Client<br/>(Tailscale RPC / SSE)"]
        Dashboard["SvelteKit Dashboard<br/>(Web Admin)"]
    end

    subgraph Daemon["Sjel Core Daemon"]
        IPC["Local Unix Domain Socket / IPC"]
        HTTP["Axum HTTP / REST API"]
        EventBus["Internal Event Stream"]
    end

    subgraph Capabilities["Capability Engines"]
        Calendar["calendar"]
        Comms["comms"]
        Places["places"]
        Projects["projects"]
        Scouting["scouting"]
    end

    subgraph Storage["Turso / LibSQL Engine"]
        Engine["LibSQL Storage Core"]
        MVCC["Concurrent MVCC Engine<br/>(BEGIN CONCURRENT)"]
        Vector["Native Vector Index<br/>(Vector Embeddings)"]
        LocalFile[("Local Database File<br/>axon.db")]
        CloudSync["Optional Remote Replica Sync<br/>(Turso Cloud / sqld)"]
    end

    GPUI <-->|Domain Socket IPC| IPC
    Mobile <-->|Tailscale Tunnel| HTTP
    Dashboard <-->|HTTP / REST| HTTP

    IPC --> Daemon
    HTTP --> Daemon
    Daemon --> Capabilities

    Capabilities --> Engine
    Engine --> MVCC
    Engine --> Vector
    Engine --> LocalFile
    Engine -.-> CloudSync
```

---

## 2. Why Turso / LibSQL (Key Advantages & Measured Delta)

| Feature | Current SQLite (`rusqlite`) | Target Turso (`libsql`) | Architectural Value for Sjel |
|---|---|---|---|
| **Concurrency** | Single writer in WAL; locks entire DB | MVCC / `BEGIN CONCURRENT` | GPUI UI actions, subagents, and comms ingestion never block each other with `SQLITE_BUSY`. |
| **Vector Search** | External vector cache / files | Native embedded vector data types & indexes | In-DB vector search for `scouting`, `comms`, and knowledge notes without external vector DB dependencies. |
| **Edge & Replicas** | Single static file on local host | Embedded replicas (local reads, sync writes) | Zero-latency local reads for GPUI with automated background synchronization across multiple devices. |
| **Format Safety** | Strict SQLite C format | Byte-level SQLite file format compatible | Direct in-place reading of existing `axon.db`; rollback to standard SQLite remains trivial. |
| **Runtime API** | Synchronous blocking (`r2d2`) | Native Tokio `async` + sync local mode | Matches Tokio-based Axum capabilities and async GPUI task runners natively. |

---

## 3. Strict Backup & Recovery Invariant (Recovery Ladder)

No migration step may proceed without verifying the 4-tier recovery ladder.

```mermaid
sequenceDiagram
    autonumber
    participant Live as Live axon.db
    participant Backup as tools/backup.sh
    participant Target as iCloud Backup Target
    participant Verifier as tools/restore.sh
    participant Sandbox as Isolated Sandbox (/tmp)

    Live->>Backup: 1. Online sqlite3 .backup snapshot
    Backup->>Backup: 2. Record SHA-256 + Table Count + Row Count
    Backup->>Target: 3. Ship compressed tarball + immutable receipt
    Note over Backup,Target: Raw static clone taken: axon.db.pre-turso-backup
    Target->>Verifier: 4. Read archive & receipt for validation
    Verifier->>Sandbox: 5. Unpack outer archive in isolation
    Sandbox->>Sandbox: 6. PRAGMA integrity_check
    Sandbox->>Sandbox: 7. Validate Table Count (99+) & Row Count (639k+)
    Sandbox-->>Verifier: 8. Verify exact match (0 live state touched)
```

### Verification Commands

1. **Take automated backup & receipt**:
   ```bash
   tools/backup.sh store
   ```
2. **Verify backup in isolation**:
   ```bash
   tools/restore.sh store "$HOME/Library/Mobile Documents/com~apple~CloudDocs/Backup/store/$(ls -t "$HOME/Library/Mobile Documents/com~apple~CloudDocs/Backup/store" | head -1)" --receipt "$SJEL_PERSONAL_ROOT/backup/receipts/store.json"
   ```
3. **Take cold raw safety clone**:
   ```bash
   cp "$SJEL_PERSONAL_ROOT/data/axon/axon.db" "$SJEL_PERSONAL_ROOT/data/axon/axon.db.pre-turso-backup"
   ```

---

## 4. Phased Rollout Roadmap

```mermaid
flowchart LR
    P1["Phase 1: Verification & Benchmarking<br/>• Full store backup<br/>• Cold clone<br/>• Isolated LibSQL probe"] --> P2["Phase 2: Upstreams & LibSQL Adapter<br/>• upstreams.toml audit verdict<br/>• sjel-storage / trait wrapper<br/>• Query test matrix"]
    P2 --> P3["Phase 3: GPUI & Daemon IPC Boundary<br/>• Unix socket daemon layer<br/>• Non-blocking GPUI client<br/>• Event notifications"]
    P3 --> P4["Phase 4: Pilot Capability Migration<br/>• Migrate places & scouting<br/>• Vector search test<br/>• Multi-writer stress test"]
    P4 --> P5["Phase 5: Full Cutover & Manifest Update<br/>• Update store manifest<br/>• Update tools/backup.sh<br/>• Archive legacy rusqlite"]
```

### Phase 1: Preflight Verification & Isolated Compatibility Spike
- [x] Run `tools/backup.sh store` and verify with `tools/restore.sh store`.
- [x] Create cold snapshot `axon.db.pre-turso-backup`.
- [x] Write an isolated scratch probe (`scratch/libsql_compat.rs`) opening a copy of `axon.db` using `libsql`.
- [x] Verify execution of complex queries: `json_each(?3)` predicates in `calendar`, recursive CTEs in `projects`, and spatial joins in `places`.

### Phase 2: Upstreams Gate & Storage Abstraction
- [x] Add `[libsql]` upstream entry with formal review in `upstreams.toml`.
- [x] Provide unified storage adapter in `libs/sjel-turso` (supporting LibSQL async concurrency, PRAGMAs, and once-per-process migration tracking).
- [x] Add embedded replicas to that adapter: a remote URL, auth token and pull interval from `SJEL_TURSO_URL` / `SJEL_TURSO_AUTH_TOKEN` / `SJEL_TURSO_SYNC_INTERVAL_SECS`, a first pull before the DDL, a background loop, and `Store::sync_stats()` for a freshness surface. The engine is a default feature (2026-10-21) so following a primary is configuration rather than a Cargo.toml edit, and `sjel setup plan|apply|verify turso` is the setup path: the shell renders the plan, the CLI writes and restarts. No live primary existed to pull from, so the pull path itself is unverified.
- [x] Verify compilation across all targets (`cargo check --workspace` and `cargo test -p sjel-turso`).

#### Open decision (2026-10-21): what the replica path is for

Measured while re-checking the setup work, and left open rather than decided:

1. **It cannot be switched on over the shared database.** `projects` and `operator-profile`
   both open `sjel_config::database_path()`, which is the `axon.db` twenty capability modules
   share, and LibSQL refuses to turn a file that holds its own writes into a replica
   (`libs/sjel-turso/src/replica.rs`, *"remove the file to start a fresh copy"*). So a primary
   pasted into `/setup` fails at open time, or requires deleting this machine's database. The
   per-process registry guard cannot see the other eighteen processes writing the same file.
   *Correction to the line above: "the pull path is unverified" understates it — with this
   wiring it is unreachable.*
2. **It costs ten unpatchable advisories.** Every finding left in `Cargo.lock` — h2 0.3.27,
   rustls 0.22.4 twice, rustls-webpki 0.102.8 five times (RUSTSEC-2026-0104, CVSS 7.5), bincode
   1.3.3, libsql-sqlite3-parser — is reached through `libsql 0.10.0`, the newest release, which
   pins hyper 0.14 / tonic 0.11 / hyper-rustls 0.25. `osv-scanner.toml` cannot waive a known
   vulnerability, so the Security job stays red for as long as the sync feature is in the
   workspace. Removing the feature is the only local fix; making it non-default is not, because
   `Cargo.lock` lists optional dependencies too.
3. **A phone replica is buildable.** `sjel-turso` checks clean for `aarch64-apple-ios`
   (45 s cold, 2026-10-21), and a phone starts empty, so LibSQL's refusal does not apply to it.
   What it needs is a primary that is not Turso Cloud: `sqld` is neither on Homebrew nor on
   crates.io, so a local primary means building it from Turso's repository and supervising a
   second daemon.
4. **The tailnet listener already covers "one more device".** A paired phone drives this Mac's
   data over Tailscale (8443). What a replica would add is *offline* reads and reads while this
   Mac is asleep — nothing else.

The decision is open because the cheap move and the interesting move disagree: deleting the
path greens Security today and forecloses the phone-in-the-bag case, while keeping it means a
permanently red scanner until libsql moves to hyper 1 / rustls 0.23. Whichever way it goes, the
prerequisite for any replica — this device's or another's — is the same: `axon.db` stops being
one shared file, and that split is the work, not the sync loop.


### Phase 3: GPUI Desktop App & Daemon Boundary
- [x] Integrate direct non-blocking LibSQL local database connector in `apps/gpui`: one handle per process, opened on first read and shared by every `Repo` clone, with the shell as its fallback.
- [x] Wire direct non-blocking queries for projects, tasks, calendar entries, project records, and notes (`Repo::fetch_projects`, `Repo::fetch_tasks`, `Repo::fetch_calendar`, `Repo::fetch_project_records`, `Repo::fetch_record`, `data::read_repo`) in `apps/gpui`.
- [x] Decouple high-throughput reads from HTTP daemon serialization while keeping writes routed through the authenticated shell.
- [ ] Implement reactive change notification subscriptions over `/api/sjel-status/events`.
- [x] Keep GPUI independent of heavy file locks (LibSQL non-blocking WAL/MVCC).

### Phase 4: Pilot Capability Migration (Vector & Concurrency)
- [x] Run concurrent write stress tests against scouting schema via `sjel-turso` (`libs/sjel-turso/tests/multi_writer_concurrency.rs`).
- [x] Confirm zero `SQLITE_BUSY` errors under load across 50 simultaneous inserts in 0.02s.
- [x] Enable and verify LibSQL native vector embedding indexing (`libsql_vector_idx`, `vector_distance_cos`) with top-k cosine similarity ranking.
- [x] Resolve C symbol isolation: established strict separation between `libs/sjel-store` (`rusqlite`) and `libs/sjel-turso` (`libsql`) to prevent linker duplicate symbols across targets.
- [x] Cut one capability over end to end as the pattern: `operator-profile` (2026-10-21) — schema, store, HTTP handlers, CLI and tests on `sjel-turso`, `rusqlite` gone from its dependency graph, and the existing row read back unchanged from the live database. `Store` in `libs/sjel-turso` is the shape the remaining capabilities follow; `places` and `scouting` are still on `rusqlite`.
- [x] Second cutover on the same pattern, at real size: `projects` (2026-10-21) — 1221 rows re-read through LibSQL and compared byte for byte against the rendered second copy with no drift, every `spawn_blocking` site in its handlers replaced by an awaited store call, and a router test covering create/read/save/stale-save/remove. It is also the table `apps/gpui` reads directly, so writer and reader now share one engine.

### Phase 5: Production Promotion & Backup Contract Evolution
- [x] Update `capabilities/store/README.md` and doctrine recording LibSQL adoption, non-blocking MVCC, and native vector capabilities.
- [x] Verify `tools/backup.sh` and `tools/restore.sh` isolated recovery ladder (107 tables, 640,464 rows) operates identically on LibSQL format.
- [x] Keep Sjel self-model (`self.json`) and service manifests (`service.toml`) validated and current.

---

## 6. Topology decision (2026-10-21): this Mac is the primary, the phone is a replica

Decided by the owner after the replica path was found unreachable over the shared `axon.db`.
The shape, and what each hop is for:

```
   phone (iOS, Tauri app)          this Mac (Lars-Mac)                 later: home server
   embedded replica   ──pull/write──▶  sqld  ◀──embedded replica──  capabilities  ──▶  sqld (primary)
   over tailnet                       primary                        over loopback
```

1. **This Mac runs `sqld`** (libsql-server) as the primary for the data that must reach the
   phone. Measured 2026-10-21: `libsql-server-v0.24.32` ships a prebuilt
   `libsql-server-aarch64-apple-darwin` binary whose `.sha256` matches the published digest, and
   `sqld --version` runs here. It carries `--auth-jwt-key-file` (Ed25519 public key; clients
   present JWTs), `--http-listen-addr`, `--grpc-listen-addr`, and `--primary-grpc-url` — so the
   same binary can later run as a *replica* of a home-server primary, which is what makes the
   last move a configuration change rather than a rewrite.
2. **Capabilities that serve phone data hold embedded replicas** of that primary over loopback:
   reads local, writes forwarded to `sqld`, pulled back on the interval. This is libSQL's own
   model; the alternative — keeping `axon.db` authoritative and writing a job that pushes a
   subset somewhere — is a custom replication protocol with its own correctness risk, which is
   the thing `libsql` exists to avoid.
3. **The phone holds an embedded replica.** `sjel-turso` checks for `aarch64-apple-ios`
   (45 s cold, 2026-10-21) and the Tauri app already declares an iOS target
   (`dashboard/src-tauri/tauri.conf.json` → `bundle.iOS`, minimum 16.0, team 52Q4FQ5XN8,
   `gen/apple` present). Reads are local and work offline; writes go to the Mac when it is
   reachable, which is what an embedded replica means.
4. **Reachability is the tailnet, not the internet.** This machine is `100.116.104.41`
   (`Lars-Mac`) on the tailnet; a primary bound there is reachable by the owner's devices only.
   `sjel doctor`'s tailnet identity gate reports no `tailscale serve` HTTPS handler, so nothing
   injects an identity header — `sqld`'s own JWT check is the access control for this port, and
   it must be configured before the port is opened to the tailnet.
5. **A later home server changes one setting.** Its `sqld` becomes the primary; this Mac and the
   phone keep their replicas and point `SJEL_TURSO_URL` at it. Nothing in the storage layer
   changes, which is the whole reason to build it this way now.

### Proven 2026-10-21: the pull path works

The `libsql 0.10.0` crate and the `sqld 0.24.32` binary — a 2025-02 release train — do sync. A
scratch primary on loopback (`sqld --db-path … --http-listen-addr 127.0.0.1:8880`) plus
`libs/sjel-turso/tests/live_primary.rs` showed, in 0.24 s: a write through replica A reaching the
primary, a pull bringing it back, and a second replica that had never seen the row finding it on
its first pull — the pull that runs before the DDL. `sync_stats()` reported `syncs >= 1`,
`failures = 0`, and the primary's own SQLite file
(`<db-path>/dbs/default/data`) held the table the forwarded DDL created.

The test reads the deployment's own `SJEL_TURSO_URL` / `SJEL_TURSO_AUTH_TOKEN`, so it skips with
a printed reason when no primary is named — a machine with none does not fail a suite it cannot
satisfy — and becomes a real check the moment `sjel setup` records a primary. What it does not
cover yet: JWT authentication (`sqld --auth-jwt-key-file`), which the phone's replica will need
before the port is reachable from the tailnet.

### Also proven 2026-10-21: the token path, both directions

`sqld --auth-jwt-key-file` plus `sjel_turso::PrimaryKey` (`libs/sjel-turso/src/auth.rs`) is the
authenticated form of the same thing. Against a scratch primary started with a generated public
key, the HTTP API answered:

| credential | `POST /v2/pipeline` |
| --- | --- |
| none | 401 |
| a token minted by `PrimaryKey`, claims `{"a":"rw","exp":…}` | 200 |
| a token signed by a different key | 401 |

and the replica test passed in 0.21 s with that token in `SJEL_TURSO_AUTH_TOKEN`.

The claims are `sqld`'s, copied from `libsql-server` rather than guessed at: `jsonwebtoken`
decodes with `Algorithm::EdDSA` only, `a` is `"ro"` or `"rw"`, `id` names a namespace and may be
absent on a `--db-path` server, and `p` (fine-grained scopes) is unused here. The private half is
a PKCS#8 document that belongs in the OS keychain; `sqld` gets only the 32-byte public key,
base64url without padding.

**One gap this exposed, worth fixing before an operator meets it:** `libsql`'s client reports a
refused credential as `Timeout performing handshake with primary`. Wrong, absent and revoked all
look like a network problem, and the primary logs nothing at INFO. Whoever wires `sjel setup
verify` has to say "the primary refused this token" itself, because the client will not.

### What this commits us to, stated plainly
- **The `axon.db` split is on the critical path.** One file cannot be a replica and a local
  database, and `axon.db` is opened by twenty capability modules. Only the capabilities whose
  data the phone needs move to the primary; the rest may stay local, but they must stop sharing
  that one file first.
- **The ten libsql advisories stay until libsql moves to hyper 1 / rustls 0.23**, so CI's
  `Security` job stays red. Accepted as the price of this topology, recorded in the open-decision
  block above rather than waived in `osv-scanner.toml`.
- **`sqld` becomes supervised software**, not a one-off: a pinned version with its digest, an
  `upstreams.toml` entry with a review, a start/stop path Sjel owns, and a place in the backup
  contract. A primary that nobody backs up is worse than no primary.
- **Token minting is Sjel's job.** `sqld` authenticates JWTs signed by a key it holds the public
  half of; the private key belongs in the OS keychain, and `sjel setup` is the surface that
  mints a token, writes `SJEL_TURSO_URL` / `SJEL_TURSO_AUTH_TOKEN`, and verifies with a real
  pull — the verification that has never once run.
