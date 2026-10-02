# ISA · media

Capability-local claims and their falsifiers. The repo-wide `ISA.md` does not own this work.

## Problem

Photos and video live on two external volumes — `Extreme` (canonical) and `INTENSO` (mirror) — as
a temporary NAS until a real one exists. Two facts are now measured rather than assumed:

**Duplicate detection has no memory.** The 2026-09-29 merge hashed 390 GB at ~85 MB/s to check
17,583 candidate pairs — 17,579 matches and 4 non-matches — in **75 minutes**. That cost
returns on the next question because nothing recorded the answer. Nothing in the repo can answer "is this file
already here" as a lookup.

**The cheap heuristic is wrong at a measurable rate.** Grouping by (size, basename) was wrong 4
times in 17,583: same name, same byte count, different bytes (four truncated MP4s, including
DJI clips, with invalid or missing `moov` atoms). And the `recover/` set — 4,943 files all named `[NNNNNN].jpg` — *looked* like a duplicate
pile and was **0% duplicates against the pre-merge library**: six years of photographs,
2006–2012, that no name-based view could distinguish from redundancy. The 4,943 rows contain
4,918 distinct digests, so "0%" never meant no duplicates *within* the recovered set.
Both classes of error were caught only by reading bytes.

**The next ingest has no safe path.** iCloud Photos is 86 GB / ~1,824 assets and is the next piece
of work. Exporting straight into the library risks re-importing 2006–2020 content already present,
and would produce no record of what was skipped or why.

## Vision

An **ingest gate and integrity ledger**. Staged candidates are hashed before admission; copying
checks the digest in the write stream and verification re-reads the destination. That is not one
read or free hashing: it trades bounded ingest-time work for a durable lookup on later
questions: *is this already here*, *which volumes hold it*, *is the mirror faithful*.

`capabilities/store` already backs the database up on a daily contract. Immich owns browsing,
faces, albums, ML search and thumbnails, and will exist when the real NAS does. What is needed now
is the thing Immich cannot be yet, and what makes the eventual Immich import idempotent: an
authoritative answer to whether these bytes already exist.

## Out of Scope

- **Albums, faces, ML search, thumbnails, a browsing UI.** Immich's, and building a tentative
  version makes a competitor to migrate away from instead of a gate that stays useful.
- **Any modification or deletion of library content.** The library is append-only under ingest.
- **Perceptual similarity as a decision.** It produces review candidates only.
- **Cloud model access to media.** No content leaves the machine; see Constraints.
- **Backups.** This capability verifies a mirror. It does not create redundancy, and does not own
  a backup contract — its state is in the shared store, which `store` owns.

## Principles

1. **Exact bytes decide duplicates, never similarity.** Field equality, not resemblance.
2. **Persist the admission hash.** Never re-hash the library merely to answer whether a candidate
   digest is known. Ingest still re-reads imported bytes to verify the copy; audits and mirror
   verification intentionally re-hash to detect damage.
3. **An absent volume is not a volume whose files are gone.** Availability is not liveness — the
   same distinction `capabilities/vault` draws between `/health` and `/ready`.
4. **Identity comes from an expected volume UUID, never the device node or the path alone.**
   Measured 2026-09-29: erasing INTENSO renumbered it `/dev/disk10` → `/dev/disk11` in the same
   boot. An unmounted path can exist on the host filesystem, so index, audit and ingest require a
   previously recorded `--uuid` and reject a mismatch before opening the store.
5. **A digest is not a location.** A mirror means one digest legitimately has two paths. Keying
   files by digest alone would collapse the mirror into a single row and make verification
   impossible.
6. **An applied refusal is recorded, not silent.** A file not imported gets a disposition and reason;
   a dry run writes only `staging-hashes.tsv`, not a database decision.

## Constraints

- **State lives in the shared store.** The capability owns a table prefix, not a file.
  `libs/sjel-store/README.md`: a capability opens the database, it does not own it.
- **`sha2` 0.11 is already a workspace dependency.** Any new crate needs an `upstreams.toml`
  entry (`url`, `verdict`, `license`, `why`) — `CONTRIBUTING.md#dependency-verdicts-and-provenance`.
- **EXIF is read by batch `exiftool` invocation, not a new parser crate.** exiftool 13.55 is
  installed. The hand-written JPEG parser of 2026-09-29 agreed with exiftool on 23 of 25 files with
  0 mismatches, but agreement on a sample is not a reason to keep a second implementation of a
  standard.
- **`service.toml` is single-line TOML** (`tools/lib/toml.sh`), schema in
  `schemas/service.toml.example`.
- **Capabilities parse argv by hand.** No `clap` in the workspace; `capabilities/vault/src/main.rs`
  is the model.
- **Data class `c1` (Mine).** The index stores digests, sizes and paths — no content — so nothing
  reaches a model. If a later phase embeds image content, `content_item::cloud_admission` gates it
  and `c3` is refused every local prompt too (`CONTRIBUTING.md#data-classes`).

## Goal

For any file, `media` answers which digests already exist and on which volumes, and what is new.
It writes to the library only through an explicit import, then reads the destination back before
marking it verified. A failed verification retains staging and the unverified disposition.

## Features

Each is a claim, and names the probe that would falsify it.

### F0 · The index

**Claim.** Every regular file under an indexed volume's library path has one location row; files
with equal SHA-256 share a file row. The location count, **not the distinct digest count**, equals
what `find` reports for that path. Symlinks and special files refuse the walk.

> *Probe:* `media audit --root PATH --uuid UUID --sample N` re-hashes N stride-selected files and reports
> discrepancies; `acceptance/scratch.py` independently counts disk files with `os.walk`.
> Falsified by a location-count difference or sampled digest disagreement. An unsampled byte
> change with forged size and mtime is not detected until a full audit or mirror verify.

**Claim.** Indexing is idempotent — indexing the same tree twice adds no rows and changes no
digests.

> *Probe:* index a scratch tree, snapshot row counts and digests, index again, compare. Falsified
> by any difference.

**Claim.** A digest present on two volumes produces two `media_locations` rows and one
`media_files` row.

> *Probe:* index a fixture tree present on both volumes; assert 1 file row and 2 location rows.
> Falsified by 2 file rows (Principle 5 broken) or 1 location row (the mirror collapsed).

### F1 · The ingest gate

**Claim.** Ingesting a fresh directory whose digests already exist imports zero files and records
duplicate dispositions; re-running the *same* staging directory instead reports verified imports
as `resumed`, not new duplicates.

> *Probe:* stage a copy of an indexed fixture with a new staging identity and apply; assert 0
> imports and one duplicate item per file. Re-run an applied staging directory and assert 0 new
> imports, `resumed` for its verified import. The 4,943 recovered rows are now in the live library:
> their **historical** zero matches cannot be retested by querying today's library.

**Claim.** Ingest imports exactly those files whose digests are absent, and no others.

> *Probe:* a fixture staging directory with `n` new and `m` duplicate files yields exactly `n`
> imports, and the `n` digests are the ones that were absent.

**Claim.** Ingest never overwrites or deletes: a name collision between a new file and a different
existing digest leaves both present and the existing digest unchanged.

> *Probe:* ingest a file whose name is taken by a different digest; assert both paths exist and the
> pre-existing file's digest is byte-identical to before.

**Claim.** Staging is not pruned until verification has passed; a batch with a duplicate is not
pruned automatically because a ledger lookup does not prove the indexed copy still exists.

> *Probe:* inject a failure after import but before verify; assert the staging directory is intact
> and no row says `verified`.

**Claim.** Every regular, UTF-8-named file *considered by an applied run that completed the staging
hash pass* has a recorded disposition and reason. A staging hash or filesystem walk failure stops
before an applied run exists; full per-file failure capture remains unproven.

> *Probe:* assert `imported + duplicates + refused + failed + resumed == files considered`, on
> an applied run containing each disposition; inspect reason fields in `media_ingest_items`.

### F2 · Mirror verification

**Claim.** For every digest with a location on both volumes, the files are byte-identical.

> *Probe:* `media verify-mirror` reports 0 discrepancies across the full mirror (317 GB,
> 2026-09-29). Falsified by any digest mismatch, size mismatch, or a location recorded on a volume
> where no file exists.

**Claim.** A volume that is not mounted is reported as absent, not as N missing files.

> *Probe:* run `verify-mirror` against a missing or UUID-mismatched scratch mount; assert
> `availability: absent`, `checked: 0`, no file discrepancies and exit 0. On the real mirror,
> unmount and repeat (not run yet). A directory surviving unmount on another filesystem must not
> be mistaken for the registered volume (Principle 3).

### F3 · HTTP surface and dashboard panel *(not started)*

**Claim.** A panel exists whose numbers are read from the capability's own HTTP surface.

> *Probe:* `tools/capability.sh` registry entry plus `panel_port`/`panel_path`; the dashboard mounts
> it and the displayed totals equal `media status` (already JSON by default).

### F4 · Perceptual layer — advisory *(not started)*

**Claim.** No perceptual result can import, delete or modify anything without an explicit
`media_reviews` verdict row.

> *Probe:* run the perceptual pass; assert zero rows in any import or delete ledger attributable to
> it, and that every candidate pair has a review row.

### F5 · Human-reviewed organization preview

A category-first archive can use chronological trip folders without forcing every other category into the same hierarchy. The private draft owns category names and reviewed collection mappings; the public tool owns validation and measurement. This slice does not change the existing importer or its `by-date` convention.

- [x] MED-P1 — `media preview --structure FILE` reports reviewed destinations, unresolved collections and conflicts without opening the shared store or changing the source, archive or draft. Falsifier: an isolated CLI probe creates a database or changes a fixture tree.
- [x] MED-P2 — source and archive must match the expected volume UUID; symlinks, traversal and unsafe destinations are refused or blocked. Falsifier: a fixture escapes its root or a wrong-volume live preview proceeds.
- [x] MED-P3 — optional metadata inspection preserves date-field evidence and exposes location-tag presence, not coordinates. Missing metadata remains missing; provisional event membership does not become an approved move. Falsifier: synthetic fixtures emit a coordinate, silently prefer a 2026 container date over a retained 2016 recording date, or turn a provisional mapping into a reviewed destination.
- [x] MED-P4 — incomplete extraction or changed source entries is visible and returns a failing completeness verdict. Falsifier: failed, missing or duplicate metadata rows, or newly added/removed files during extraction, produce a complete report.

**2026-09-30 evidence.** The isolated crate run passed 18 tests and strict all-target clippy. The black-box `acceptance/preview.py` probe passed no-write checks, unsafe/repeated-option and wrong-volume refusals, mapped-only metadata scope, coordinate/diagnostic withholding, missing/duplicate/unexpected rows and added/removed entries during extraction. Existing `acceptance/scratch.py` also passed. A local preview on the registered archive volume found 14 collections, 2 reviewed proposals, 1 provisional collection and 11 unmapped collections, plus 1 loose file. The mapped-only pass read 1,134 videos, reproducing prior date-field/month evidence with zero extraction errors or standard location-tag presence. Independent before/after entry-stat snapshots of both roots and a draft-content comparison were unchanged; no scratch store or overlay was created. This is not a byte-integrity audit, proof of an atomic snapshot, or claim that deeper timed location metadata is absent. Reports and the real draft remain private. No apply action, model or production-binary replacement was performed.

**Later 2026-09-30 recheck — paused, not cleared.** A repeat using the final source returned a complete CLI report, but an independent combined assertion comparing source/archive entry-stat snapshots and exact draft bytes failed. The failing subcondition and cause were not isolated, and the failed run's snapshots were not persisted. Earlier passing probes remain evidence of those runs, not clearance of this later warning. Work is paused at the principal's request. The private organization draft records the handoff; resumption requires permission and instrumented, separately reported comparisons before attributing a change or claiming consistency. No move/apply command was run.

### F6 · Applied organisation, removal and reconciliation

F5 previews destinations; this slice acts on them, and the ledger is kept in step with the volume afterwards. Nothing here copies bytes the drives already hold: `organize` and `relabel` are same-volume renames, and `supersede` renames into quarantine rather than deleting.

- [x] MED-P5 — `organize` moves whole collections by rename only, refusing a mapping that was not reviewed, and aborts **every** move if one conflicts. Falsifier: an unreviewed collection moves, or a re-run after its own success refuses the run it already applied.
- [x] MED-P6 — `duplicates` reports one row per digest with two or more locations and never treats a look-alike as a removal candidate; `supersede` re-hashes every row immediately before moving and refuses the whole list if one row is unproven. Falsifier: a row moves after its bytes changed, or a group whose copies all sit under the declared prefix yields removals.
- [x] MED-P7 — `relabel` places a flat pile by capture day and resolves a name collision instead of overwriting; `--plan` moves exactly the rows an approved TSV names. Falsifier: a plan that does not describe the disk moves the rows that happen to fit.
- [x] MED-P8 — a path change is dropped from the ledger, so a rename is never reported as an absence, and a disappearance that is not a rename stays visible. Falsifier: `audit` reports a moved file as `unindexed absence`, or a row whose bytes are nowhere gets dropped.

**2026-10-01 evidence — MED-P8 was found by a live run, not by review.** 13 `.lrv` proxies were moved from `To Sort/` into `Trips/2026/2026-05-Oberstdorf/` with `relabel --plan`. `main.rs` assumed a relabel source is always outside the library — true for `Inbox/`, false here — so no index row was dropped. One defect, two false findings from the same 13 rows: 13 `unindexed absence` disagreements in `audit`, and 13 phantom duplicate groups in `duplicates`, each naming the same file at its old and new path. `media reconcile` was added as the resolution and `relabel` now drops its own rows. After the fix the live library reports `disk 16,790 = indexed 16,790`, 0 disagreements on an 80-file sample, and 1 duplicate group — a genuine pair inside `To Sort/`.

**The `reconcile` guard, and its precondition.** A gone location is dropped only when its digest is recorded at another path *present on this volume*; bytes that are nowhere leave the row in place, so a mounted-but-empty library root reconciles to nothing rather than to a mass deletion. The precondition is that the index is current. A moved file's new path is not recorded until `index` runs, so reconciling first would find no witness and read a rename as a disappearance; an unindexed path therefore refuses the whole run. This was measured by a test that failed before the guard existed, not assumed.

**A rename and a hand-deletion are indistinguishable afterwards.** Both leave one path gone and the same bytes elsewhere, and the digest is the only judge available. `reconcile` reports `survives_at` rather than claiming which of the two happened.

**Corpus of record for this slice:** `relabel` moved 198 `Inbox-old` files (56.45 GB), `organize` moved 17 collections, and `supersede` applied five approved removal lists — 1,353 then 322 then 24 rows — each row re-hashed at the moment of removal with 0 refused.

## Not yet specified

- **Per-file pre-classification failures.** A symlink, unreadable byte stream, or non-UTF-8
  path currently refuses the entire preflight before an ingest run exists. Probe: stage one such
  input among regular files, apply, and inspect whether every path has a durable disposition;
  it does not yet, so do not describe the stronger F1 claim as passing.
- **Periodic re-indexing.** A schedule is not needed now (see Decisions); if data freshness
  becomes a measured requirement, decide the job boundary and interval before adding one.
- **Interrupted multi-file ingest and orphan handling.** A prior unverified final copy can be
  verified and resumed by staging path and digest. A crash between path reservation and link,
  or an orphan in `.media-incoming`, still needs manual inspection. Probe: kill the process at
  each filesystem/SQLite boundary and inspect for an unverified final file before claiming full
  crash safety. The destination path is reserved in SQLite before linking, the copied bytes and
  destination directory are synced before verification is recorded, but a temporary copy left
  behind by an interrupted run still needs explicit cleanup. Sudden-power-loss durability was
  not measured by a scratch test.

## Test Strategy

**The 2026-09-29 run left labelled ground truth, and it should be the acceptance fixture.** This is
unusual and worth exploiting: these answers are already known to be right.

| Artefact | Ground truth |
|---|---|
| `verify-redundant.tsv` | 17,583 pairs, **4 known non-duplicates** |
| `recover-dedup.tsv` | 4,943 rows, 4,918 distinct digests; **0 matches against the pre-merge library** |
| `guard-exceptions.tsv` | 4 truncated MP4s, ffprobe evidence |
| `guard3-exceptions.tsv` | 29 game assets, not personal media |

Two acceptance assertions follow directly, and both are falsifiable without touching the library:

1. **A tool that reports any of the 4,943 recovered rows as a duplicate of the *pre-merge*
   library is wrong.** They were hashed against it and matched nothing. They have since been
   integrated, so the same lookup on today's library should return present.
2. **A tool that treats any of the 4 pairs in `verify-redundant.tsv` as duplicates is wrong.** Same
   name, same size, different bytes.

Acceptance scripts belong in `capabilities/media/acceptance/`, following
`capabilities/vault/acceptance/`. Per the vault precedent, tests check counts by running a second
implementation at the same moment rather than against a stored number.

## Probe record (2026-09-29, updated 2026-09-30)

- **F0 live passes (2026-09-30):** Following verified `store` backup run 220, full live index
  ran on the operator's shared store (`axon.db`): 15,761 files under
  `/Volumes/Extreme/Media/Library` hashed and indexed into `media_locations` across 15,490
  distinct `media_files` digests with 0 discrepancies (`{"uuid":"667F5744-21B3-43F3-8794-26365ABAB67B"}`).
  Idempotence verified: immediate second index pass ran in 0.5s with `hashed: 0`. Live sample
  audit (`media audit --sample 100`) re-read and hashed 100 stride-selected files with 0
  disagreements.
- **F1 passes:** scratch acceptance verifies regular file ingest, deduplication, disposition
  conservation, and failure injection resumption. `acceptance/labelled.py` verified all 4,943
  historical recovered rows absent against pre-merge cohort, all 4 corrupt pairs distinct, and
  10 positive controls present. The live canonical comparison on `/Volumes/Extreme/Media/Library`
  resolves all relocations and is **4/4 accessible and verified** (all four canonical target files
  exist and differ from the corrupt copies). Live iCloud ingest, true per-file preflight failures
  and boundary crash safety remain untested until staged.
- **F2 partial:** unit probe and scratch CLI probe pass. Live absent mirror probe verified:
  `media verify-mirror` against absent INTENSO reports `availability: absent`, `checked: 0`, and
  0 discrepancies with exit 0. Full 317 GB live mirror verification waits for INTENSO reconnection.

## Anti-claims

- **Not a photo manager.** No album, no face, no thumbnail, no search, no browsing UI.
- **Not a backup.** It verifies that a mirror is faithful. It does not create redundancy and must
  not be described as protecting anything.
- **Not a deduplicator that deletes.** It reports duplicates. Deleting is a separate, declared
  action with its own guard, as the 2026-09-29 `guard-exceptions.tsv` pattern established.
- **Not Immich, and not a stepping stone to be thrown away.** It is the ingest gate that stays
  useful after Immich arrives, because it is what makes the Immich import idempotent.

## Decisions

**2026-09-30 — prepare organization through a read-only preview, not automatic sorting.** The principal chose explicit collection mappings and category-specific browsing: chronology inside Trips, meaningful collections elsewhere. Files and companion files remain together. Dates and GPS availability are evidence for review, not event identity or move permission. Rust provides the repeatable preview; local-model advice is a separate, unvalidated option. No model dependency or apply action is added to this slice.

**2026-09-29 — exact hashing is authoritative, perceptual is advisory.** Evidence: a name-and-size
heuristic was wrong 4 in 17,583; the recovered set looked like duplicates and was 0% duplicates.
The vault's `Documents.md` doctrine already states the rule for documents — *"Similarity cannot
identify a form document. Duplicate detection must be field equality, not similarity."*

**2026-09-29 — keep the capability thin, on the expectation that Immich is the destination.** The
risk is stated rather than hidden: build albums, faces, thumbnails or search and this becomes a
competitor to migrate away from. Own three facts and nothing else.

**2026-09-29 — the recovered set is integrated, not quarantined.** `_recovered/` no longer exists;
its 4,943 rows (4,918 distinct digests) were integrated into `by-date/2006-01 … 2012-08` with
EXIF-derived names. Keeping recovered material in a parallel tree was considered and rejected as
partially defeating "unified on both".

**2026-09-29 — pre-HTTP, no service manifest and no schedule.** Verified against
`tools/doctor.ts` (enabled capabilities are checked as directories),
`tools/check-manifest-integrity.sh` (checks only existing manifests' `requires`),
`tools/check-service-tomls.sh` (process needs a scheduled command or port; data needs a backup
source and target), and `tools/capability.sh` (a real directory may have no manifest; the
registry emits only runnable services). This multi-verb, operator-invoked CLI is neither an
unattended one-command job nor the owner of the SQLite file; `capabilities/store/service.toml`
already owns `backup_sqlite_online`. When F3 adds an HTTP process, that process earns a manifest.
`media` has no registry collision: the registry derives names from capability directories and
`capabilities/comms/src/media.rs` is an internal module, not a capability.

**2026-09-29 — no periodic schedule yet.** Re-index cadence is not measured and a scheduled
multi-verb CLI without a chosen subcommand cannot run. Manual `media index` and `media audit`
are the contract until an interval and a job boundary are justified.

## Schema sketch

Not binding — `libs/sjel-store` migrations own the shape — but the model the rules above require:

```
media_volumes      (uuid PK, label, first_seen, last_seen)
media_files        (digest PK, size, first_seen)
media_locations    (uuid, relpath, digest, size, mtime_ns, first_seen, last_seen,
                    PRIMARY KEY (uuid, relpath))
media_ingests      (id PK, source, staging, manifest_json, started, finished, counts)
media_ingest_items (ingest_id, relpath, digest, size, disposition, reason, imported_relpath,
                    verified, PRIMARY KEY (ingest_id, relpath))
-- F4, not created: media_phashes, media_reviews
```

`media_files` / `media_locations` is the split Principle 5 forces. `media_ingest_items` makes
refusal a row rather than an absence (Principle 6); `media_phashes` and `media_reviews` do not
exist yet (F4).
