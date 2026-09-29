# media

An exact-byte ingest gate and integrity ledger for a library on mounted volumes. It answers whether a SHA-256 digest is known, where indexed bytes live, and whether indexed mirrors still match. It does **not** make copies, provide a backup, delete a library file, manage photographs, or offer an HTTP/UI surface.

## Run it

The user-facing `capabilities/media/media` launcher builds only the `media` Rust binary on first use and honors `CARGO_TARGET_DIR`. With the capabilities directory on `PATH`, invoke it as `media`.

```sh
media index --root <mounted-volume>/Media/Library
media audit --root <mounted-volume>/Media/Library --sample 100
media status
media classify --digest <sha256>
media ingest --staging <batch> --library <mounted-volume>/Media/Library
media ingest --staging <batch> --library <mounted-volume>/Media/Library --apply
media verify-mirror --left-root <library-a> --left-uuid <uuid-a> --right-root <library-b> --right-uuid <uuid-b>
```

Use `--db <scratch.sqlite>` for an isolated test. Without it, `sjel_config::database_path` selects the shared store; `SJEL_DB_PATH` isolates **only the database**, not the volumes. `index`, `status`, `audit`, and `classify` open the store and therefore create the `media_` tables on first use. Before **any** first command on a real shared database, follow [backups before migrations](../../CONTRIBUTING.md#backups-before-migrations): a tested 3-2-1 backup and recovery rehearsal of `store`. The acceptance probes always use a temporary database and disposable trees; they do not initialize the operator's store or change library bytes.

Start by indexing every mounted library *only after that backup*. `index` prints its filesystem UUID. An unchanged (size, mtime) location is skipped on a second index; `audit --sample N` re-hashes N stride-selected locations, while `--sample 0` checks paths and counts without hashes. The digest and row counts are different: equal files at two paths mean two locations and one file. The index does not silently remove records for absent files or replace a digest when an indexed path changes bytes; it refuses the changed path for investigation. `audit` names missing and unindexed paths.

The staging contract is:

```text
<batch>/originals/             export tree, never edited by import
<batch>/export-manifest.json   {"source":"...", "tool":"...", "options":{...}}
<batch>/staging-hashes.tsv     emitted: SHA-256, size, mtime, JSON-escaped relative path
```

`--apply` is required to write library bytes or ledger decisions. An optional `refusals` array in the export manifest has `path`, `reason`, and `evidence` for each deliberately withheld file. These decisions are recorded as `refused` items on apply. The dry run hashes and writes only `staging-hashes.tsv`. For *explicit* cleanup, `--apply --prune` removes `originals/` only after **all** imports verify and no duplicate, failure, or refusal remains. A digest lookup alone does not prove an indexed duplicate is still readable: leaving the staged copy intact avoids deleting its potentially last surviving bytes. It never removes the manifest, hashes, or library files. Review the report before requesting prune. An unmounted or wrong volume UUID refuses index, audit, or ingest; verify-mirror instead reports `availability: absent` with no missing-file discrepancies and a successful availability exit status.

Ingest hashes the staging tree, checks digests against the index, invokes ExifTool once for the tree's `DateTimeOriginal` CSV, chooses EXIF → filename date → mtime → `_undated`, then copies the new bytes to a temporary sibling of the library while checking the digest in the copy stream. It reserves the provisional path, links a collision-free name (`~2`, `~3`, …) into `by-date/YYYY-MM/`, re-reads the destination, and marks it verified only if size and SHA-256 agree. It never overwrites a path. Same-batch repeated digests are classified as duplicates after the first verified import. The source export remains intact unless explicit prune passes verification. A batch containing duplicates requires a separate operator decision to remove staging; this tool will not do it on the strength of its lookup. A prior verified import from *this same staging path* is `resumed`, not a duplicate; an unverified copy is re-read on retry. An orphaned temporary copy or a crash after path reservation but before verification still needs operator inspection; F1 does **not** promise atomicity across SQLite and the filesystem.

## Why this shape: files and locations

A digest identifies bytes, not a pathname. `media_files` has one row per digest, while `media_locations` has a row per `(volume UUID, relative path)` referencing it. A file can occur at several paths on either volume; a mirror can carry it on both. A `(digest, volume)` primary key would discard multiple legitimate paths on one volume. `media_ingests` and `media_ingest_items` preserve the producer's manifest, source path, disposition, reason, and verification status. These tables belong to the `media_` prefix in the shared SQLite file. `capabilities/store/service.toml` owns its existing `backup_sqlite_online` contract; media declares no duplicate backup contract.

## Why this shape: volume UUID and exact bytes

Erasing a drive can change its `/dev/diskN` number within one boot. The volume UUID, obtained from the mounted filesystem on macOS or `findmnt` on Linux, survives device renumbering; the label is only display text. A mountpoint left as an empty host directory has a different UUID and is reported absent, not interpreted as thousands of deleted files.

The 2026-09-29 merge measured four nonmatches among 17,583 same-name candidate pairs, and 4,943 recovered rows with **zero matches against the pre-merge library**. Those rows contain 4,918 distinct SHA-256 digests: the other 25 are repeats *inside the recovered set*. Today's library has absorbed the recovered material, so today's live lookup correctly reports it present. Exact hashing is authoritative for duplicate admission. Similarity and a future perceptual hash may offer a human review candidate, never an automatic import or delete. F4 has not begun; neither a perceptual parser nor review tables exist. ExifTool is adopted solely for the date hint; metadata cannot make a duplicate verdict. The `[sha2]` and `[exiftool]` provenance verdicts are in `upstreams.toml`.

## Verification and limits

Run `cargo test -p media --locked`, then `python3 capabilities/media/acceptance/scratch.py <media-bin>` and `python3 capabilities/media/acceptance/labelled.py <media-bin> <private-audit-trail-dir> [<library-root>]`. The synthetic scratch probe compares index counts to a separate `os.walk` implementation and checks idempotence, digest-only admission, a name/size collision, a recorded refusal, interrupted verification, a missing mirror and corruption detection. The historical script uses a **temporary** media schema seeded with historical positive digests, and compares bulk `media classify` output to SQLite queries independently. It checks all 4,943 recovered rows, all four known nonmatches, ten positive controls and both exception ledgers. Supplying a library root also re-hashes any canonical targets still accessible and reports how many of the four were unavailable; unavailable paths are not passing live comparisons.

`verify-mirror` hashes every indexed path on both mounted volumes and emits a discrepancy per affected digest. It has no automatic mirror creation. A verified mirror at one instant does not protect against both volumes failing, later corruption, or files that were never indexed. F0's full live row-count/hash probe and F2's full-volume verification require an explicitly backed-up shared store, full indexing, and long reads; neither is implied by the scratch tests. The live run status and open probes remain in [ISA.md](ISA.md).

## Considered and declined

- A `service.toml` for the multi-verb CLI: `tools/capability.sh` accepts a capability directory without a manifest; `tools/doctor.ts` checks enabled directories, and `tools/check-manifest-integrity.sh` checks only declared `requires`. `kind = "data"` requires its own backup contract, which this capability does not own. `kind = "process"` without a schedule needs a port; this CLI has neither. The vault CLI preceded its server manifest in the same way. F3 may add a server manifest later.
- A scheduled re-index: no justified cadence, and a manifest job needs a single command. Manual indexing and explicit audits are safer until that requirement is measured.
- EXIF parsing in Rust and perceptual deduplication now: both duplicate already owned work, and neither decides byte equality. No new parser crate is introduced.
