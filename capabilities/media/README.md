# media

An exact-byte ingest gate and integrity ledger for a library on mounted volumes, with a read-only preview of reviewed organization mappings and a mirror it both builds and verifies. It answers whether a SHA-256 digest is known, where indexed bytes live, whether indexed mirrors still match, and which collections have reviewed destinations. It does not provide a backup contract, delete library content, browse photographs, or offer an HTTP/UI surface. The organization preview has no apply action.

## Run it

The user-facing `capabilities/media/media` launcher builds only the `media` Rust binary on first use and honors `CARGO_TARGET_DIR`. With the capabilities directory on `PATH`, invoke it as `media`.

```sh
media volume-id --root <mounted-volume>/Media/Library
media index --root <mounted-volume>/Media/Library --uuid <registered-volume-uuid>
media audit --root <mounted-volume>/Media/Library --uuid <registered-volume-uuid> --sample 100
media status
media classify --digest <sha256>
media ingest --staging <batch> --library <mounted-volume>/Media/Library --uuid <registered-volume-uuid>
media ingest --staging <batch> --library <mounted-volume>/Media/Library --uuid <registered-volume-uuid> --apply
media verify-mirror --left-root <library-a> --left-uuid <uuid-a> --right-root <library-b> --right-uuid <uuid-b>
media mirror --from <library-a> --from-uuid <uuid-a> --to <library-b> --to-uuid <uuid-b> [--path <rel>]… [--exclude <rel>]… [--consume <prefix>]… [--journal <path>] [--settled-for <seconds>] [--apply | --check]
media sync --from <library-a> --from-uuid <uuid-a> --to <library-b> --to-uuid <uuid-b> [--path <rel>]… [--exclude <rel>]… [--consume <prefix>]… [--journal <path>] [--settled-for <seconds>]
media reclaim --from <library-a> --from-uuid <uuid-a> --to <library-b> --to-uuid <uuid-b> [--path <rel>]… [--exclude <rel>]… [--list <path>] [--quarantine <path>] [--journal <path>] [--apply]
media duplicates --uuid <uuid> [--legacy <prefix>] [--resolve-inside <prefix> --root <path> [--metadata]] [--list <path>]
media supersede --root <path> --uuid <uuid> --list <path> --quarantine <path> --journal <path> [--apply]
media reconcile --root <path> --uuid <uuid> [--apply]
media relabel --from <dir> --to <dir> --uuid <uuid> --journal <path> [--library <path>] [--plan <path>] [--apply]
media organize --structure <file> [--only <collection>]… [--apply --journal <path>]
```

Four verbs act on the library rather than report on it, and each carries its own guard. `organize` moves whole collections by rename within one volume, refusing any mapping not marked reviewed and aborting **every** move if one conflicts. `duplicates` reports one row per digest with two or more locations, plus look-alike groups (same basename and size, different bytes) which are never removal candidates. `supersede` acts on an approved removal list and re-hashes every row immediately before it renames the file into quarantine — nothing is deleted, so reclaiming the space is a separate declared step. `relabel` places a flat pile by capture day, resolving a name collision with the library's own `~2` disambiguator rather than overwriting, and `--plan` restricts it to exactly the rows an approved TSV names.

`verify-mirror` asks two different questions and the flag chooses which. By default it groups by digest and answers *"is every digest on both volumes"*. With `--paths` it compares the two as **trees** and answers *"do they match"* — reporting a path only on one side, or one holding different bytes there. The difference is load-bearing and was measured: the digest mode reports a healthy mirror while every byte sits at a different path on the other volume, which is precisely the state `mirror` was in after the library was reorganised. Both modes first walk each mounted volume and refuse if any on-disk path is not indexed; run `media index` before verifying. This catches additions since the last index, but it is not an atomic snapshot and cannot prevent files changing during verification. The default meaning is unchanged, so `--paths` adds the question rather than replacing an answer.

`mirror` is the verb that builds what `verify-mirror` judges, and it reads as little as it can. It plans from the ledger: a source or destination path whose recorded size and mtime have not moved is trusted without being read, and a destination path with **no** record is read rather than assumed equal. A run without `--apply` writes nothing at all — no media, no moved path, and no ledger row — so recording a digest stays `index`'s job and a dry run cannot quietly index as a side effect. `--check` is a scheduler-friendly read-only verdict: it exits 1 when copies or moves are pending, destination-only paths remain, a source subtree is deferred, or a failure occurs; the JSON still includes counts and path samples. The default quiet window is 300 seconds; `--exclude` remains available for a known active path. `--consume PREFIX` declares a destination-side path whose bytes may be *moved* into place instead of copied, which is what makes retiring a pre-sort snapshot cost no data movement; the move is journalled before it happens and reverses by swapping two paths. A destination holding files but no index is **refused**, not read: measured at ~47 minutes for this library, because a USB SSD sustains 113 MB/s and parallelism makes it worse, not better — 112.7 MB/s on one thread against 67.9 on eight. A destination path the source does not hold is reported and never removed; reclaiming it is a separate declared step, so `mirror` deletes nothing. `--path` narrows a run.

For the enforced manual workflow, `media sync` indexes both UUID-checked volumes, applies the mirror, and then verifies the trees with `verify-mirror --paths`. It does not quarantine or delete destination-only paths. It skips the final tree verification when a recent source subtree was deferred and exits non-zero unless the completed run verifies cleanly. Sync remains an operator-invoked command; this decision does not install a schedule.

`reclaim` is the other half of that pair, and it exists because "the mirror is faithful" and "the mirror is tidy" are different claims. A mirror can be complete and still carry every path a supersede or a rename left behind — `mirror` held 1,532 such paths, 71.7 GiB, whose bytes had all moved to a new name on the other volume. It removes a path **only** when the digest is present at a source path that is on disk *right now*; bytes that exist nowhere else are reported and left alone, because that case is the entire value of a second copy. It plans from what the destination **holds**, not from what the ledger says should be there: a destination file whose recorded size and mtime still match is answered from its row without being read, and anything the ledger cannot answer is hashed — the rule `mirror` uses. That distinction is not cosmetic. Planning only from ledger rows made the verb blind to a tree the ledger did not describe and silent about it: measured 2026-10-02, `mirror/Inbox` (193 GB) and both `_quarantine` batches (33.7 GB) each answered `candidates: 0, complete: true, issues: []` while holding files, because the Inbox was never a library root and a quarantined path has its row dropped by design. `planned_from`, `from_ledger` and `hashed` report what the plan had to read, and an unreadable destination file sets `complete: false` so a partial plan cannot pass as a full answer. `--apply` needs an approved `--list`, re-hashes every row immediately before moving it, quarantines rather than deletes, and writes the journal row before the move so the run reverses by swapping two paths.

`reconcile` is the one verb that removes a ledger row. `index` deliberately keeps a row for a path it did not find, because an undeclared disappearance has to stay visible — but nothing could resolve one either, so a rename left a permanent false absence. `reconcile` drops a gone location **only** when its digest is also recorded at another path present on this volume. Bytes that are nowhere leave the row in place, so a library root that exists while its volume does not reconciles to nothing rather than to a mass deletion. Two consequences are load-bearing. The index must be current first: a move's new path is not recorded until `index` runs, so reconciling earlier would find no witness and read a rename as a disappearance, and an unindexed path therefore refuses the run. And a rename and the deletion of one of two identical copies are indistinguishable afterwards — both leave one path gone and the same bytes elsewhere — so the report names `survives_at` rather than claiming which happened. `relabel --library` drops its own rows for the same reason, so a tool-driven in-library move needs no reconcile pass.

Use `--db <scratch.sqlite>` for an isolated test. Without it, `sjel_config::database_path` selects the shared store; `SJEL_DB_PATH` isolates **only the database**, not the volumes. Every verb that reads or writes the ledger — `index`, `status`, `audit`, `classify`, `duplicates`, `supersede`, `relabel`, `reconcile`, `mirror` and `reclaim` — opens the store and therefore creates the `media_` tables on first use; `preview` and `organize` do not open it at all. Before **any** first command on a real shared database, follow [backups before migrations](../../CONTRIBUTING.md#backups-before-migrations): a tested 3-2-1 backup and recovery rehearsal of `store`. The acceptance probes always use a temporary database and disposable trees; they do not initialize the operator's store or change library bytes.

Start by indexing every mounted library *only after that backup*. Record each volume's UUID while it is mounted (check `media volume-id --root PATH` against the OS volume listing). Pass the **previously recorded** UUID on every index, audit and ingest. Do not derive the expected UUID from a library path *after* a drive disappears: that path may now be on the host filesystem. The CLI compares the expected UUID to the mounted filesystem **before opening the database** and refuses a mismatch. `index` also prints the observed UUID. An unchanged (size, mtime) location is skipped on a second index; `audit --sample N` re-hashes N stride-selected locations, while `--sample 0` checks paths and counts without hashes. The digest and row counts are different: equal files at two paths mean two locations and one file. The index does not silently remove records for absent files or replace a digest when an indexed path changes bytes; it refuses the changed path for investigation. `audit` names missing and unindexed paths, and `reconcile` is how such a row is resolved — no other verb removes one.

Two intake routes remain supported. A producer can stage an export and use `media ingest` when per-file imported, duplicate, resumed or refused dispositions are required. An export written directly into the library can be adopted with `media index`; that records its current locations and digests, but does not create historical ingest dispositions. Run `index` before mirror verification; both modes refuse paths on disk that are not yet indexed.

The staging contract is:

```text
<batch>/originals/             export tree, never edited by import
<batch>/export-manifest.json   {"source":"...", "tool":"...", "options":{...}}
<batch>/staging-hashes.tsv     atomically replaced: SHA-256, size, mtime, JSON-escaped relative path
```

`--apply` is required to write library bytes or ledger decisions. An optional `refusals` array in the export manifest has `path`, `reason`, and `evidence` for each deliberately withheld file. These decisions are recorded as `refused` items on apply. The dry run hashes and atomically replaces only `staging-hashes.tsv` (apart from the first-open database migration noted above). A symlink at that path is refused, and a pre-existing hard link is replaced without writing its linked target. For *explicit* cleanup, `--apply --prune` removes `originals/` only after **all** imports verify and no duplicate, failure, or refusal remains. A digest lookup alone does not prove an indexed duplicate is still readable: leaving the staged copy intact avoids deleting its potentially last surviving bytes. It never removes the manifest, hashes, or library files. Review the report before requesting prune. An unmounted or wrong volume UUID refuses index, audit, or ingest; verify-mirror instead reports `availability: absent` with no missing-file discrepancies and a successful availability exit status.

Ingest hashes the staging tree, checks digests against the index, invokes ExifTool once for the tree's `DateTimeOriginal` CSV, chooses EXIF → filename date → mtime → `_undated`, then copies the new bytes to a temporary sibling of the library while checking the digest in the copy stream. It reserves the provisional path, links a collision-free name (`~2`, `~3`, …) into `by-date/YYYY-MM/`, syncs the destination directory, re-reads the destination, and marks it verified only if size and SHA-256 agree. It never overwrites a path. Same-batch repeated digests are classified as duplicates after the first verified import. The source export remains intact unless explicit prune passes verification. A batch containing duplicates requires a separate operator decision to remove staging; this tool will not do it on the strength of its lookup. A prior verified import from *this same staging path* is `resumed`, not a duplicate; an unverified copy is re-read on retry. An orphaned temporary copy or a crash after path reservation but before verification still needs operator inspection; F1 does **not** promise atomicity across SQLite and the filesystem.

## Preview organization without changing files

```sh
media preview --structure <private-organization-draft.json>
media preview --structure <private-organization-draft.json> --metadata
```

For a source checkout whose installed release binary predates this verb, use `tools/cargo-hermetic run -p sjel-media --locked -- preview --structure <private-organization-draft.json> [--metadata]`. This builds an isolated debug binary rather than replacing an installed release binary.

Both commands emit JSON and bypass the ledger entirely: no database open or migration, directory creation, hashing, file moves, or changes to the input draft. They require an existing source and archive on the previously recorded volume UUID. Symlinks and filesystem crossings are not followed. Traversal failures or a source changing during the metadata pass make the report incomplete; unresolved human choices are reported separately.

The draft defines categories and explicit collection mappings. It is not a rule that guesses a category for every file. A minimal example, using illustrative paths and a UUID placeholder:

```json
{
  "draft_version": 1,
  "source_root": "/mounted-volume/Inbox/Photos and Videos",
  "archive_root": "/mounted-volume/Media/Library",
  "expected_volume_uuid": "PREVIOUSLY_RECORDED_UUID",
  "organization": {
    "categories": [{"name": "Trips"}, {"name": "Events"}]
  },
  "mappings": [{
    "source_collection": "2024-08-Trip",
    "proposed_destination_relative": "Trips/2024/2024-08-Trip",
    "status": "destination_reviewed"
  }],
  "execution": {"file_moves_authorized": false}
}
```

`destination_reviewed` offers a destination, not permission to execute it. A mapping with status `provisional_event_membership_needs_review` stays unresolved, even if it contains a candidate destination. Unmapped collections and loose files also remain unresolved. Category names and destinations must be bounded relative components; existing destinations and unsafe ancestors are conflicts, not overwrite invitations. Annotation fields from the reviewed draft can remain alongside these inputs.

Without `--metadata`, the preview reads filesystem metadata only. With it, ExifTool reads selected date and location tags from recognized media files in explicitly mapped collections only. Unmapped collections receive filesystem inventory, not metadata inspection. Each batch allows at most 128 files, 30 seconds and 2 MiB of output; ExifTool user configuration is disabled. The report retains date-field provenance, distinguishes container-only evidence, and flags calendar-day disagreements or dates outside a collection's date label. It does not silently convert uncertain timestamps between timezones, split an event across months, or substitute filesystem modification times for missing capture dates. Missing ExifTool or incomplete extraction is an explicit failure of metadata coverage.

Location output is tag presence only, never coordinate values. A normal pass does not establish that no deeper timed metadata exists. Names and paths can still be private, so the JSON is not a public fixture. No model, reverse-geocoding service, or network request is part of this command. It is a preparation tool, not an integrity audit, move journal, rollback system, or photo manager.

## Why this shape: files and locations

A digest identifies bytes, not a pathname. `media_files` has one row per digest, while `media_locations` has a row per `(volume UUID, relative path)` referencing it. A file can occur at several paths on either volume; a mirror can carry it on both. A `(digest, volume)` primary key would discard multiple legitimate paths on one volume. `media_ingests` and `media_ingest_items` preserve the producer's manifest, source path, disposition, reason, and verification status. These tables belong to the `media_` prefix in the shared SQLite file. `capabilities/store/service.toml` owns its existing `backup_sqlite_online` contract; media declares no duplicate backup contract.

## Why this shape: volume UUID and exact bytes

Erasing a drive can change its `/dev/diskN` number within one boot. The volume UUID, obtained from the mounted filesystem on macOS or `findmnt` on Linux, survives device renumbering; the label is only display text. A mountpoint left as an empty host directory has a different UUID and is reported absent, not interpreted as thousands of deleted files.

The 2026-09-29 merge measured four nonmatches among 17,583 same-name candidate pairs, and 4,943 recovered rows with **zero matches against the pre-merge library**. Those rows contain 4,918 distinct SHA-256 digests: the other 25 are repeats *inside the recovered set*. Today's library has absorbed the recovered material, so today's live lookup correctly reports it present. Exact hashing is authoritative for duplicate admission. Similarity and a future perceptual hash may offer a human review candidate, never an automatic import or delete. F4 has not begun; neither a perceptual parser nor review tables exist. ExifTool supplies date hints for ingest and bounded date/location-tag evidence for organization preview; metadata cannot make a duplicate verdict. The `[sha2]` and `[exiftool]` provenance verdicts are in `upstreams.toml`.

## Verification and limits

For isolated builds and tests, use `tools/cargo-hermetic test -p sjel-media --locked`. After an isolated debug build, run `python3 capabilities/media/acceptance/preview.py <media-bin>` for the preview's no-write, option-refusal, metadata-privacy and changing-entry checks. Run the existing ingest/index probes separately: `python3 capabilities/media/acceptance/scratch.py <media-bin>` and `python3 capabilities/media/acceptance/labelled.py <media-bin> <private-audit-trail-dir> [<library-root>]`. The synthetic scratch probe compares index counts to a separate `os.walk` implementation and checks idempotence, digest-only admission, a name/size collision, a recorded refusal, interrupted verification, a missing mirror and corruption detection. The historical script uses a **temporary** media schema seeded with historical positive digests, and compares bulk `media classify` output to SQLite queries independently. It checks all 4,943 recovered rows, all four known nonmatches, ten positive controls and both exception ledgers. Supplying a library root also re-hashes any canonical targets still accessible and reports how many of the four were unavailable; unavailable paths are not passing live comparisons.

`verify-mirror` hashes every indexed path on both mounted volumes and emits a discrepancy per affected digest. Building a mirror is a separate verb, `mirror`; the two are deliberately not one command. A verified mirror at one instant does not protect against both volumes failing, later corruption, or files that were never indexed. F0's full live row-count/hash probe and F2's full-volume verification require an explicitly backed-up shared store, full indexing, and long reads; neither is implied by the scratch tests. The live run status and open probes remain in [ISA.md](ISA.md).

## Considered and declined

- A `service.toml` for the multi-verb CLI: `tools/capability.sh` accepts a capability directory without a manifest; `tools/doctor` checks enabled directories, and `tools/check-manifest-integrity.sh` checks only declared `requires`. `kind = "data"` requires its own backup contract, which this capability does not own. `kind = "process"` without a schedule needs a port; this CLI has neither. The vault CLI preceded its server manifest in the same way. F3 may add a server manifest later.
- A scheduled re-index: no justified cadence, and a manifest job needs a single command. Manual indexing and explicit audits are safer until that requirement is measured.
- EXIF parsing in Rust and perceptual deduplication now: both duplicate already owned work, and neither decides byte equality. No new parser crate is introduced.
