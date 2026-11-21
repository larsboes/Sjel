---
id: "gpui-documents-decisions"
title: "Complete reviewable document decisions"
state: "done"
area: "gpui"
kind: "feature"
priority: 2
depends_on: ["gpui-documents-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
issue: "https://github.com/larsboes/Sjel/issues/246"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
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
- Duplicate history uses versioned sidecars. The owner restores decisions, source receipts, and reopen state during `restore-index`. New merges save the absorbed records’ prior filing state. Older merges can appear in history with no saved filing state to restore.
- Folder imports reuse the record for known bytes (`Library::intake_known`), and `inbox --dry-run` reports new and stored files by hash. `documents merge-copies --apply` merged 174 exact-copy groups and archived 199 records on the live library; a second run reports 0 groups. A catalog backup preceded it.
- Every eligible file in the Extreme archive folders was already stored: 0 new of 1,405 across eight folders.
- The Documents owner stores collections in tables and rebuilds them from sidecars. The Mac split picker can assign every piece to an existing collection in the split request.
- `suggest-splits` uses local page signals. After re-analysis of the 43 open part parents (562 pages), 9 received cut proposals: 6 vocabulary breaks with a date, 4 openings, 3 explicit page-one markers, 1 last-page marker. The page-one rule alone produced at most 3. Recall stays low; most known contained documents still get no cut.
- The owner category list has 12 entries in the overlay's `documents-classification.json`, drafted from filename keyword counts. 994 active records have no page extraction yet, so rule proposals for them rest on filenames only.
- A rescan needs equal known page counts. Contained candidates offer **Archive shorter**, **Pick pages in longer PDF** for an unsplit original, and **Keep both**.
- One PDF page view (`apps/gpui/ui/src/pdf_kit/view.rs`) serves the reader, filing compare and duplicate panes with fit page, fit width and 50–400 % zoom. Native acceptance of scrolling and zoom has not run.
- The list reads cached derived values instead of extracted text: 500 rows in about 27 ms instead of 1.0–1.65 s. Duplicate candidates over active records are cached per library revision: about 0.1 s warm instead of about 17 s.
- **Big documents** orders analyzed, unsplit originals by descending page count. The reader preselects suggested cuts and shows editable cuts, names, and live page counts. The owner stores one PDF per range and refreshes duplicate candidates in the background after a split.
- The queue saves the active bundle, draft cuts, and deferred entries locally. **Save and open next** continues with the next eligible bundle. The queue shows the count of original PDFs awaiting page analysis.
- The picker compares the PDF pages on both sides of a cut with the shared zoom and scroll controls. A contained candidate supplies cuts at the edges of one uniquely matching contiguous span. Ambiguous matches leave the draft available for manual review.
- **Collections** opens from the library and the active document. It lists members and supports adding the active document or removing a member.
- **Decision history** shows the kept record, archived members, and source receipts. Reopening a recorded merge restores its saved filing states. A later manual change clears reopening eligibility.
- Documents tests pass on Turso WAL (66) and SQLite (67). The tests exercise contiguous matching, collection membership writes, split and merge reconstruction, source receipts, and restoring a filed record when a merge is reopened.
- The full GPUI UI suite passed 248 headless tests, including rendering both pages at a cut and changing the gap and zoom without losing cuts.
- Installed Mac acceptance still needs three real bundles, including boundary preview, PDF scrolling and zoom, and a multi-page split. The background desktop bridge did not return a window for the installed app.
