# media

Exact-byte ingest gate and integrity ledger for media libraries on mounted volumes, plus a private local attachment library. It indexes files by SHA-256, verifies mirrors, records ingest dispositions and previews reviewed organization mappings. Local attachments retain originals and source filenames without content extraction or cloud calls. It does not provide a backup contract, browse photographs or delete library content.

The technical manual is [`website/content/capabilities/media.md`](../../website/content/capabilities/media.md). It documents the ledger, CLI, HTTP surface, safety constraints and isolated acceptance probes.

## Why this shape: bytes and locations are distinct

A digest identifies file bytes, not a pathname. The ledger therefore stores one file identity and separate locations, allowing duplicate copies and mirrors without losing path information. Exact hashing governs duplicate admission; metadata and similarity can inform review but do not replace byte identity.

Mounted-volume HTTP routes remain read-only. The operator-only attachment contract accepts raw bytes into `<overlay>/data/media/attachments/<sha256>` and records metadata and observed filenames in `media_attachments` and `media_attachment_sources`. Originals are verified and atomically published without replacing existing paths; duplicate uploads retain source filenames but store no second original. This local library is independent of the external-drive library and requires no mounted volume.

File-moving and organization commands remain explicit CLI operations with approval inputs, and reclaim moves approved files to quarantine rather than deleting them.

## Run

Use `media help` for the CLI. Start the service with `media serve`; a private overlay is required for attachment storage. The listener requires an operator credential and admits no agent credentials. `POST /api/attachments?filename=<name>&sha256=<digest>` accepts at most 128 MiB of raw bytes; `GET /api/attachments/{sha256}` returns verified metadata, not file contents. Import uses this local HTTP contract; it never accesses media storage directly.

For a single original outside an import folder, use `media attach --file /absolute/path/screenshot.png`. This owner CLI stores the file directly in the same overlay attachment library and reports `{attachment, created}`. It requires a stable regular source, refuses symlink paths, and leaves the source untouched. No running HTTP service is needed.

Apple binary-property-list payloads beginning with `bplist00` are refused as unsupported placeholders, including when named `.jpeg`, `.heic` or `.mov`. Intake does not claim general media decoding or usability verification. Genuine components of a `.pvt` live-photo directory can be imported individually; this is not bundle reconstruction.

For isolated builds and tests, use `tools/cargo-hermetic test -p sjel-media -p sjel-import --locked`.

## Considered and declined

- **Intake is not a backup.** The shared store backs up attachment metadata, not original bytes.
  Local attachment blobs currently have no backup contract. Retain source originals; intake must
  not be used as permission to delete them.
- **No automatic deletion or organization.** Destructive or moving operations require explicit CLI approval, and quarantine remains separate from deletion.
- **No perceptual duplicate verdict.** Similarity can provide review candidates but cannot decide whether bytes are duplicates.
