---
id: storage-data-root
title: "Separate personal data from configuration"
state: backlog
area: storage
kind: feature
priority: 1
spec: docs/plans/storage.md
---

# Separate personal data from configuration

## Outcome

Separate personal data from configuration through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Add a configurable data root outside Git repositories.
- Migrate each owner with verified database/file copies and a backup; preserve records and private configuration.
- Audit tracked overlay data privately; untracking does not remove prior history.

## Verification

- `sjel_config::owner_data_dir` resolves `SJEL_DATA_ROOT_<OWNER>` from the environment or
  `deployment.env` to `<root>/data/<owner>`. A relative or missing root is an error, not the
  overlay (`crates/sjel-config/src/lib.rs`).
- Interior uses it for flats, layouts and the default assets directory. A synthetic test with a
  mock Inventory listener loaded the flat and archived a layout under the external root, left the
  overlay bytes unchanged, refused an unmounted root with the key and path in the error, and read
  the overlay again after the key was removed (`crates/interior/tests/data_root.rs`).
- `tools/backup.sh` takes the same key as the source root. Synthetic cases refuse an unavailable
  root and a disagreeing `backup_source_root`, archive no overlay copy, and restore in isolation
  with matching bytes and modes (`tools/backup.test.sh`).

## Remaining

- Interior declares no live backup contract yet. Add one before its cutover.
- Not tested: the installed Interior service and CLI against a real external volume.
- CV: per-save recovery outside Git, then the same rehearsal.
