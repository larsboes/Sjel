---
id: "gpui-documents-decisions"
title: "Complete reviewable document decisions"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 2
depends_on: ["gpui-documents-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
issue: "https://github.com/larsboes/Sjel/issues/246"
---

# Complete reviewable document decisions

## Outcome

Complete reviewable document decisions through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Reconcile GitHub reports 245/246/247 with current implementation before coding; retain source-linked filing/compare, reviewable proposals and durable duplicate/split decisions.
- Use Documents as authority; merge retains survivor/absorbed IDs, filenames/source paths and hash-keyed dismissals, while split retains parent SHA-256, accounts for every page and keeps the original openable.
- A merge, dismissal and split survive restart and restore-index through versioned sidecars; dismissed unchanged pairs stay dismissed and every source path remains.
- Keep operator title/date/issuer/notes/page-count separate from extracted/proposed metadata; the owner chose Documents collections over project-label grouping. Do not preempt concurrent owner implementation or reimplement duplicate detection.

## Evidence

- The owner merges a confirmed group: `save_duplicate_decision` archives every member except the survivor in the decision's transaction and writes their record sidecars (`crates/documents/src/library.rs`). Archived members keep originals and source paths and leave the review queue (`open_candidates` in `crates/documents/src/server.rs`).
- Duplicate decision rows have no versioned sidecar. A dismissal and the absorbed-record link are lost on `restore-index`, while the archived status survives.
- Folder imports reuse the record for known bytes (`Library::intake_known`), and `inbox --dry-run` reports new and stored files by hash. `documents merge-copies --apply` merged 174 exact-copy groups and archived 199 records on the live library; a second run reports 0 groups. A catalog backup preceded it.
- Every eligible file in the Extreme archive folders was already stored: 0 new of 1,405 across eight folders.
- The Documents owner stores collections in tables and rebuilds them from sidecars. The Mac split picker can assign every piece to an existing collection in the split request.
- `suggest-splits` uses local page signals. After re-analysis of the 43 open part parents (562 pages), 9 received cut proposals: 6 vocabulary breaks with a date, 4 openings, 3 explicit page-one markers, 1 last-page marker. The page-one rule alone produced at most 3. Recall stays low; most known contained documents still get no cut.
- The owner category list has 12 entries in the overlay's `documents-classification.json`, drafted from filename keyword counts. 994 active records have no page extraction yet, so rule proposals for them rest on filenames only.
- A rescan needs equal known page counts. Contained candidates offer **Archive shorter**, **Pick pages in longer PDF** for an unsplit original, and **Keep both**.
- One PDF page view (`apps/gpui/ui/src/pdf_kit/view.rs`) serves the reader, filing compare and duplicate panes with fit page, fit width and 50–400 % zoom. Native acceptance of scrolling and zoom has not run.
- The list reads cached derived values instead of extracted text: 500 rows in about 27 ms instead of 1.0–1.65 s. Duplicate candidates over active records are cached per library revision: about 0.1 s warm instead of about 17 s.
- **Big documents** orders analyzed, unsplit originals by descending page count. The reader preselects suggested cuts and shows editable cuts, names, and live page counts. The owner stores one PDF per range and refreshes duplicate candidates in the background after a split.
- Documents tests pass on Turso WAL (64) and SQLite (65). The split test verifies that the source bytes stay intact, the named pieces enter duplicate review, and an index rebuild restores the pieces and their collection. The 17 headless Documents UI tests cover the queue order and the picker’s editable suggested cuts.
- The installed Mac app still needs visual acceptance of the picker, PDF scrolling and zoom, and a real multi-page split.
