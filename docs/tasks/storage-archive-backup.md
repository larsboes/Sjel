---
id: storage-archive-backup
title: "Protect the unsorted archive"
state: verify
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
blocked_by: "Staging from earlier failed backups remains in protected storage. Recurring backup enrollment has not been approved or verified."
---

# Protect the unsorted archive

## Outcome

Protect the unsorted archive through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Give the whole archive an independent backup contract and pass an isolated restore.
- Resolve staging/cleanup failures; a nonzero backup is not accepted.
- Prove coverage of excluded material and avoid relying on a second copy on the same physical drive.

## Verification

- Fixtures cover same-filesystem refusal, dangling and directory symlinks, canonical Unicode path
  equivalence, normalization collisions, changed file bytes, and changed link targets.
- The operator's no-prune backup completed on a separate physical device and left no staging
  from the successful run. The isolated restore matched the receipt and verified every source
  path, file byte, and symbolic link target. The disposable restore copy was removed.
- Older protected staging remains untouched. Scheduler state could not be verified from the
  managed environment. No schedule was changed.
