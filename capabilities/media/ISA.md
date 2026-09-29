# ISA · media

Draft 2026-09-29, written outside the repo because the capability does not exist yet.
**Intended home: `capabilities/media/ISA.md`** — `CONTRIBUTING.md#the-backlog-is-isas` says open
work lives in an `ISA.md` and nowhere else, and this is not yet in that shape.

A capability this size carries its own ISA at its root (`capabilities/places/ISA.md` is the
precedent), so the root `ISA.md` is not the place for it.

---

## Problem

Photos and video live on two external volumes — `Extreme` (canonical) and `INTENSO` (mirror) — as
a temporary NAS until a real one exists. Two facts are now measured rather than assumed:

**Duplicate detection has no memory.** The 2026-09-29 merge hashed 390 GB at ~85 MB/s to prove
17,583 files were duplicates — **75 minutes** — and that number is paid again in full on the next
question, because nothing recorded the answer. Nothing in the repo can answer "is this file
already here" as a lookup.

**The cheap heuristic is wrong at a measurable rate.** Grouping by (size, basename) was wrong 4
times in 17,583: same name, same byte count, different bytes (truncated DJI MP4s with no `moov`
atom). And the `recover/` set — 4,943 files all named `[NNNNNN].jpg` — *looked* like a duplicate
pile and was **0% duplicates**: six years of unique photographs, 2006–2012, that no name-based view
could distinguish from redundancy. Both classes of error were caught only by reading bytes.

**The next ingest has no safe path.** iCloud Photos is 86 GB / ~1,824 assets and is the next piece
of work. Exporting straight into the library risks re-importing 2006–2020 content already present,
and would produce no record of what was skipped or why.

## Vision

An **ingest gate and integrity ledger**. Every byte entering the library is hashed once, at the
moment it is cheapest — during the write, where it costs no extra read — and every later question
is a lookup: *is this already here*, *which volumes hold it*, *is the mirror faithful*.

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
2. **Hash once, at ingest.** Re-deriving a digest already known is the cost this exists to remove.
3. **An absent volume is not a volume whose files are gone.** Availability is not liveness — the
   same distinction `capabilities/vault` draws between `/health` and `/ready`.
4. **Identity comes from the volume UUID, never the device node.** Measured 2026-09-29: erasing
   INTENSO renumbered it `/dev/disk10` → `/dev/disk11` in the same boot. A device path is not an
   identity.
5. **A digest is not a location.** A mirror means one digest legitimately has two paths. Keying
   files by digest alone would collapse the mirror into a single row and make verification
   impossible.
6. **A refusal is recorded, not silent.** A file not imported gets a row saying so and why.

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
It writes to the library only through an explicit import whose result has already been verified.

## Features

Each is a claim, and names the probe that would falsify it.

### F0 · The index

**Claim.** Every file under a registered volume's library path has exactly one `media_files` row
and at least one `media_locations` row, digest = SHA-256, and the row count equals what `find`
reports for that path.

> *Probe:* `media audit --library --sample N` recomputes digests for N files chosen by stride and
> reports disagreements; a separate count check compares rows to `find`. Falsified by any
> disagreement, or by a row-count difference.

**Claim.** Indexing is idempotent — indexing the same tree twice adds no rows and changes no
digests.

> *Probe:* index a scratch tree, snapshot row counts and digests, index again, compare. Falsified
> by any difference.

**Claim.** A digest present on two volumes produces two `media_locations` rows and one
`media_files` row.

> *Probe:* index a fixture tree present on both volumes; assert 1 file row and 2 location rows.
> Falsified by 2 file rows (Principle 5 broken) or 1 location row (the mirror collapsed).

### F1 · The ingest gate

**Claim.** Ingesting a directory whose contents are already in the library imports zero files and
reports them as duplicates.

> *Probe:* copy a sample of already-present library files to a staging directory and ingest it;
> assert `imported == 0` and that every file has a duplicate disposition row. **This is falsifiable
> today with the 2026-09-29 labelled set** — the 4,943 recovered photographs are all in the
> library, so re-ingesting a copy must import 0.

**Claim.** Ingest imports exactly those files whose digests are absent, and no others.

> *Probe:* a fixture staging directory with `n` new and `m` duplicate files yields exactly `n`
> imports, and the `n` digests are the ones that were absent.

**Claim.** Ingest never overwrites or deletes: a name collision between a new file and a different
existing digest leaves both present and the existing digest unchanged.

> *Probe:* ingest a file whose name is taken by a different digest; assert both paths exist and the
> pre-existing file's digest is byte-identical to before.

**Claim.** Staging is not pruned until verification has passed.

> *Probe:* inject a failure after import but before verify; assert the staging directory is intact
> and no row says `verified`.

**Claim.** Every file not imported has a recorded disposition and a reason.

> *Probe:* assert `imported + duplicates + refused + failed == files considered`, on a run
> constructed to include each disposition.

### F2 · Mirror verification

**Claim.** For every digest with a location on both volumes, the files are byte-identical.

> *Probe:* `media verify-mirror` reports 0 discrepancies across the full mirror (317 GB,
> 2026-09-29). Falsified by any digest mismatch, size mismatch, or a location recorded on a volume
> where no file exists.

**Claim.** A volume that is not mounted is reported as absent, not as N missing files.

> *Probe:* unmount the mirror volume, run `verify-mirror`; assert the output names the volume as
> not mounted and reports 0 "missing files". Falsified by a missing-file count, or by an exit
> status that reads as a failed verification (Principle 3).

### F3 · HTTP surface and dashboard panel *(not started)*

**Claim.** A panel exists whose numbers are read from the capability's own HTTP surface.

> *Probe:* `tools/capability.sh` registry entry plus `panel_port`/`panel_path`; the dashboard mounts
> it and the displayed totals equal `media status --json`.

### F4 · Perceptual layer — advisory *(not started)*

**Claim.** No perceptual result can import, delete or modify anything without an explicit
`media_reviews` verdict row.

> *Probe:* run the perceptual pass; assert zero rows in any import or delete ledger attributable to
> it, and that every candidate pair has a review row.

## Not yet specified

- **Periodic re-indexing.** Whether a `schedule` is wanted, and whether it belongs on this
  capability or a separate scheduled one in the `punctuality-ingest` / `finance-prices` shape.
- **Whether a `service.toml` is required at all before F3.** `kind = "data"` exists for state with
  no process and requires a `backup_*` target, which this has none of (the store owns the file).
  The `kind = "process"` CLI-only precedent is `punctuality-ingest` and `finance-prices` — both
  single scheduled subcommands, which is not this shape. Resolve against `tools/doctor` and
  `tools/check-manifest-integrity.sh` before writing the manifest.
- **Where a partial ingest resumes from.** The 2026-09-29 work made every stage resumable via its
  ledger; whether that is a property here or a phase-2 concern is open.
- **Name collision.** `capabilities/comms/src/media.rs` is an unrelated module (attachments on feed
  items). Confirm the registered name `media` does not confuse the `tools/capability.sh` registry.

## Test Strategy

**The 2026-09-29 run left labelled ground truth, and it should be the acceptance fixture.** This is
unusual and worth exploiting: these answers are already known to be right.

| Artefact | Ground truth |
|---|---|
| `verify-redundant.tsv` | 17,583 pairs, **4 known non-duplicates** |
| `recover-dedup.tsv` | 4,943 known-**unique** files (0 duplicates) |
| `guard-exceptions.tsv` | 4 truncated MP4s, ffprobe evidence |
| `guard3-exceptions.tsv` | 29 game assets, not personal media |

Two acceptance assertions follow directly, and both are falsifiable without touching the library:

1. **A tool that reports any of the 4,943 recovered photographs as a duplicate of a library file is
   wrong.** They were hashed against the whole library and matched nothing.
2. **A tool that treats any of the 4 pairs in `verify-redundant.tsv` as duplicates is wrong.** Same
   name, same size, different bytes.

Acceptance scripts belong in `capabilities/media/acceptance/`, following
`capabilities/vault/acceptance/`. Per the vault precedent, tests check counts by running a second
implementation at the same moment rather than against a stored number.

## Anti-claims

- **Not a photo manager.** No album, no face, no thumbnail, no search, no browsing UI.
- **Not a backup.** It verifies that a mirror is faithful. It does not create redundancy and must
  not be described as protecting anything.
- **Not a deduplicator that deletes.** It reports duplicates. Deleting is a separate, declared
  action with its own guard, as the 2026-09-29 `guard-exceptions.tsv` pattern established.
- **Not Immich, and not a stepping stone to be thrown away.** It is the ingest gate that stays
  useful after Immich arrives, because it is what makes the Immich import idempotent.

## Decisions

**2026-09-29 — exact hashing is authoritative, perceptual is advisory.** Evidence: a name-and-size
heuristic was wrong 4 in 17,583; the recovered set looked like duplicates and was 0% duplicates.
The vault's `Documents.md` doctrine already states the rule for documents — *"Similarity cannot
identify a form document. Duplicate detection must be field equality, not similarity."*

**2026-09-29 — keep the capability thin, on the expectation that Immich is the destination.** The
risk is stated rather than hidden: build albums, faces, thumbnails or search and this becomes a
competitor to migrate away from. Own three facts and nothing else.

**2026-09-29 — the recovered set is integrated, not quarantined.** `_recovered/` no longer exists;
its 4,943 files are in `by-date/2006-01 … 2012-08` with EXIF-derived names. Keeping recovered
material in a parallel tree was considered and rejected as partially defeating "unified on both".

## Schema sketch

Not binding — `libs/sjel-store` migrations own the shape — but the model the rules above require:

```
media_volumes   (volume_id PK, label, uuid, first_seen, last_seen)
media_files     (digest PK, size, first_seen)
media_locations (digest, volume_id, relpath, mtime, first_seen, last_seen,
                 PRIMARY KEY (digest, volume_id))
media_ingests   (ingest_id PK, source, manifest_json, started, finished,
                 considered, imported, duplicates, refused, failed)
media_phashes   (digest, algo, phash)                    -- advisory (F4)
media_reviews   (digest_a, digest_b, verdict, decided_at, note)   -- human layer (F4)
```

`media_files` / `media_locations` is the split Principle 5 forces. `media_ingests` exists so a
refusal is a row rather than an absence (Principle 6).

## Log

- 2026-09-29 — drafted after the eight-source merge, the 4-in-17,583 verification result, and the
  discovery that `recover/` was six years of unique photographs rather than a duplicate pile.
