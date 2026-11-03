# media

Exact-byte ingest gate and integrity ledger for media libraries on mounted volumes. It indexes files by SHA-256, verifies mirrors, records ingest dispositions and previews reviewed organization mappings. It does not provide a backup contract, browse photographs or delete library content.

The technical manual is [`website/content/capabilities/media.md`](../../website/content/capabilities/media.md). It documents the ledger, CLI, HTTP surface, safety constraints and isolated acceptance probes.

## Why this shape: bytes and locations are distinct

A digest identifies file bytes, not a pathname. The ledger therefore stores one file identity and separate locations, allowing duplicate copies and mirrors without losing path information. Exact hashing governs duplicate admission; metadata and similarity can inform review but do not replace byte identity.

The HTTP surface is read-only. File-moving and organization commands remain explicit CLI operations with approval inputs, and reclaim moves approved files to quarantine rather than deleting them.

## Run

Use `media help` for the CLI. Start the read-only service with `media serve`.

For isolated builds and tests, use `tools/cargo-hermetic test -p sjel-media --locked`.

## Considered and declined

- **No backup contract.** The shared store owns the database backup contract; media does not declare a duplicate one.
- **No automatic deletion or organization.** Destructive or moving operations require explicit CLI approval, and quarantine remains separate from deletion.
- **No perceptual duplicate verdict.** Similarity can provide review candidates but cannot decide whether bytes are duplicates.
