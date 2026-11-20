---
id: storage-archive-backup
title: "Protect the unsorted archive"
state: verify
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
blocked_by: "Owner decisions: a recurring target on a physical drive other than the archive drive, and removal of the staging left by failed runs."
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
- The archive and the `backup-target` volume are on the same external drive, so the archive
  cannot enroll in the existing recurring contract. The `archive` capability is not enabled and
  has no scheduled attempts in `sjel backup runs`.
- The backup staging directory holds 11 directories from failed runs between 2026-09-30 and
  2026-11-19: five archive runs, four knowledge-base runs, one store run and one cleanup probe.
