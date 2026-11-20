---
id: storage-archive-backup
title: "Protect the unsorted archive"
state: done
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
evidence:
  - "The isolated restore of the operator's no-prune archive backup matched the receipt and every source path, file byte and link target."
  - "On 2026-11-20 the owner ruled the external drive alone sufficient for the archive; recurring second-drive enrollment is declined, and a second connected drive stays optional."
  - "All 11 staging directories left by failed runs were removed on 2026-11-20; the staging directory is empty."
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
- The archive and the `backup-target` volume are on the same external drive, so the archive
  cannot enroll in the existing recurring contract. The `archive` capability is not enabled and
  has no scheduled attempts in `sjel backup runs`.
- On 2026-11-20 the owner accepted the external drive as sufficient, with no recurring backup.
  A drive failure leaves only the one-off restore-tested copy.
- The 11 staging directories from failed runs between 2026-09-30 and 2026-11-19 were removed.
