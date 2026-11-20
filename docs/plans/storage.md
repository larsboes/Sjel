# Storage and sync

Canonical open work lives in the linked tasks. Database and backup contracts remain with their
owners; the [storage architecture](../architecture/storage.md) describes the current shared-file
system, not an already-completed split.

## Decisions

- Move to one database per capability. Use Turso WAL for the Inventory pilot.
- Capabilities own writes and cross-owner reads. No whole-file phone replication bypasses owners.
- Private configuration stays in the overlay; personal data belongs outside Git repositories.
- Knowledge prose stays in the learning vault; Projects owns project prose; typed facts stay in owners.
- The phone follows [chosen offline data](gpui.md#what-may-live-on-the-phone), with no second writer.
- Secrets stay in the OS keychain. A secret database needs its own security decision.

## 1. Migration acceptance

- [Accept live key rotation](../tasks/storage-rotation-acceptance.md).
- [Separate personal data from configuration](../tasks/storage-data-root.md).
- [Protect the unsorted archive](../tasks/storage-archive-backup.md).
- [Accept knowledge-vault cutover](../tasks/storage-knowledge-cutover.md).
- [Reconcile personal relevance lenses](../tasks/storage-personal-lenses.md).
- [Accept installed note-path search](../tasks/storage-note-search.md).

<a id="one-vault-for-prose"></a>

<a id="one-home-per-fact"></a>

Repository facts live in code, canonical tasks, architecture or owning guidance. Vault prose does
not duplicate task status. Git history and capability backups retain removed material. Historical
source identifiers remain provenance, not an active vault dependency.

## 3. Phone standalone

- [Share domain sync contracts](../tasks/sync-contract.md).
- [Make change cursors durable](../tasks/sync-sequence.md).
- [Replay phone writes exactly once](../tasks/sync-idempotency.md).
- [Add the Projects changes feed](../tasks/sync-projects-feed.md).
- [Unify phone domain replay](../tasks/sync-domain-shape.md).
- [Preserve records during first pairing](../tasks/sync-first-pair.md).
- [Sync Inventory and wishlist](../tasks/sync-inventory.md).
- [Sync People proposals](../tasks/sync-people.md).
- [Search chosen offline records](../tasks/storage-phone-search.md).

### A sync contract per domain

Pull returns changed/deleted records and a cursor. Push carries a persisted idempotency key;
per-capability ledgers share the write transaction and retain keys 30 days. Writes preserve order
per record. A refusal blocks that record, not the queue; it remains visible with a reason.
Pending local writes are protected from pull. Captures remain create-only.

## 4. Full migration

- [Remove unused replica infrastructure](../tasks/storage-remove-replica.md).
- [Route reads through owners](../tasks/storage-owner-reads.md).
- [Split capability databases](../tasks/storage-files.md).
- [Specify encrypted sync admission](../tasks/storage-sync-security.md).

Backups precede each migration. ATTACH is for migration/reporting only. Per-file schema versions
use PRAGMA user_version. Archive the old shared database after all owners have moved.

## 5. Backups

- [Explain scheduled backup starts](../tasks/backup-triggers.md).
- [Prove each backup target kind](../tasks/backup-target-kinds.md).
- [Back up each owner database](../tasks/backup-split-files.md).
- [Choose phone-only data protection](../tasks/backup-phone-decision.md).
- [Handle a never-created database](../tasks/backup-empty-primary.md).

## 6. Agent databases

- [Evaluate isolated agent storage](../tasks/storage-agent-candidates.md).

Agent state is isolated; authoritative records stay with owners. Graph/vector indexes are
rebuildable projections carrying source, revision, observation time, data class and access scope.
Authorization applies at each expansion; c3 never enters an index.
