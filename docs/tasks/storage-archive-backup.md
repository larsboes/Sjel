---
id: storage-archive-backup
title: "Protect the unsorted archive"
state: verify
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
blocked_by: "The managed environment denied staging a protected path and removing partial staging. A complete backup and isolated restore require an authorized operator context."
---

# Protect the unsorted archive

## Outcome

Protect the unsorted archive through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Give the whole archive an independent backup contract and pass an isolated restore.
- Resolve staging/cleanup failures; a nonzero backup is not accepted.
- Prove coverage of excluded material and avoid relying on a second copy on the same physical drive.

## Verification

- `tools/backup.test.sh` passed with synthetic data. It checks same-filesystem refusal, dangling
  and directory symlink metadata, staging cleanup, isolated restore, and changed-file detection.
  The host lacks `/dev/full`, so the short-write case did not run.
- The archive has a private destination on a separate device. The live run failed during staging
  and published no archive or new receipt. The previous receipt points to the source device.
- Full source coverage, a clean live staging teardown, and recurring backup enrollment remain open.
