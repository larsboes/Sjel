# documents

Sjel's local document library. Original bytes are stored once by SHA-256 under the active overlay;
each source filename has a separate metadata record in the shared store, so byte-identical documents
keep their names and filing history while sharing one blob. Intake is idempotent for the same bytes
and filename. New documents start in `inbox`; a human must choose a category before a document becomes
`filed`. Extraction is advisory and never moves or renames an original.

The `documents` capability owns document records and originals. `projects` remains the owner of
project/task records and attachments; a project label on a document is a reference only. The local
CLI and authenticated desktop interface use the same library. Agent credentials reach one route,
`GET /api/documents/duplicates`: member ids, kind, score, digest and pseudonymized source paths, with
no extracted text and no originals. Every other route is operator-only.

A confirmed duplicate group keeps the survivor and archives every other member in the same
transaction. Archived records keep their originals, source paths and sidecars, and leave the
duplicate review queue.

## Surface

### CLI

```bash
documents intake <file>                                        # store file in inbox
documents inbox <folder> [--limit N] [--dry-run] [--recursive] # scan directory
documents next [--limit N]                                     # list inbox items by stage
documents search <words...> [--status inbox|filed|archived]    # search text and metadata
documents analyze <id>                                         # extract pages and text
documents suggest-splits <id>                                  # inspect suggested cuts
documents suggest-model <id>                                   # request local category proposal
documents split <id> <ranges>                                  # partition PDF into children (e.g. 1-2,3-4)
documents review <id> <category> [--project NAME]              # file document
documents archive <id>                                         # archive document record
documents duplicates                                           # inspect duplicate groups
documents merge-copies [--apply]                               # keep one record per exact-copy group
documents collection list|show <name>                          # named groups of document references
documents collection new <name> [--category C]                 # create a collection
documents collection add|remove <name> <id>...                 # change membership; records stay put
documents verify                                               # verify blob digests
```

### HTTP API

The `documents` service binds loopback on `SJEL_DOCUMENTS_PORT` (default 8104) behind Sjel's inbound authentication:

- `GET /api/documents` — paginated metadata listing with derived `workflow_stage`, the original's `sha256`, token-prefix query search (`q`), and status filtering.
- `GET /api/documents/{id}` — record metadata, page evidence, split suggestions, child documents, and proposal review history.
- `POST /api/documents?filename=...` — idempotent raw-byte upload.
- `GET /api/documents/{id}/original` — hash-verified raw bytes (inline for PDF, download for other formats).
- `POST /api/documents/{id}/analyze` — re-run local text extraction and page segmentation.
- `GET /api/documents/{id}/children` — list stored child records with parent page ranges.
- `POST /api/documents/{id}/split` — partition PDF into standalone child documents for specified page ranges.
- `GET|POST /api/documents/collections`, `POST /api/documents/collections/{id}/members` — named
  collections of document references. A sidecar per collection lets `restore-index` rebuild them.
- `POST /api/documents/{id}/proposals/model` — request local classification proposal from loopback inference role.
- `POST /api/documents/{id}/proposals/{proposal_id}/review` — record operator feedback (`accepted`, `rejected`, `overridden`).
- `PATCH /api/documents/{id}` — commit operator filing category and optional project reference.
- `DELETE /api/documents/{id}` — archive record without deleting stored blob bytes.

## Why this shape: Document intelligence belongs with originals

Document storage and metadata analysis are colocated in Sjel:

1. **Separation from academic knowledge.** Sjel stores personal records, receipts, contracts, and life
   administration. Obsidian serves strictly as an academic and conceptual knowledge base (such as science,
   engineering, or philosophy). Personal documents and summaries never file into Obsidian.
2. **Hash-addressed deduplication.** Source files are content-addressed by SHA-256 in `<overlay>/data/documents/blobs/`.
   An explicit upload of known bytes under a new filename gets its own record that shares the payload.
   A folder import (`inbox`, `watch`) adds the source path to the existing record instead, and its
   `--dry-run` reports new and already-stored files by hash. `merge-copies` keeps the filed record,
   else the inbox record, else the earliest import, and archives the other members.
3. **Immutable originals.** Intake, segmentation, and enrichment never overwrite, move, or rename source
   bytes. Reanalysis is idempotent and safe to rerun.

## Why this shape: Bounded and evidence-grounded proposals

Automated assistance proposes metadata but never commits actions unattended:

1. **Human review required.** Classification rules and inference roles append suggestions to the document's
   proposal ledger. A document remains in `inbox` until the operator explicitly confirms a filing category.
2. **Exact evidence spans.** Category proposals must provide verbatim evidence text from the document.
   Proposals failing length or exact-match verification are withheld.
3. **Strict data boundaries.** Personal documents are classified `c2` (local-only) or `c3` (secret).
   Cloud-hosted OCR or inference APIs are refused outright. Classification delegates exclusively to
   loopback inference roles (`document_classification`) via `crates/inference`. `c3` documents are blocked
   from entering any model prompt.

## Why this shape: Page-level segmentation and lineage

Multi-page document handling preserves complete page lineage:

1. **Per-page text and OCR provenance.** The running server analyzes every active original PDF that has
   no page evidence, a failed page or no text, one at a time, and repeats the pass every six hours.
   A document that fails is tried again only after a restart. PDF pages are analyzed independently. Pages with sufficient
   vector text use native extraction; scanned or image-heavy pages fall back to local OCR. Each page retains
   its engine version and error status.
2. **Explicit partition splits.** When a mixed document contains multiple receipts or forms, `split`
   materializes standalone child PDFs for non-overlapping, complete page ranges using `lopdf`.
   `suggest-splits` proposes cuts from local page signals: an explicit page-one marker, a last-page
   marker on the previous page, a salutation, subject or invoice heading at the top, or a vocabulary
   break with a date at the top. Each cut names its signal and quote; nothing splits without review.
3. **Child independence.** Child documents receive distinct SHA-256 content IDs, retain extracted page text,
   and can be searched, reviewed, and exported independently while maintaining links to the parent original.

## Why this shape: Native reader and in-process rendering

Desktop integration in GPUI provides direct document interaction:

1. **Client-side PDF rasterization.** The GPUI client rasterizes PDF pages directly through platform APIs
   (`CGPDF`/`PDFKit` on macOS) into textures, ensuring fast scrolling and crisp display without round-tripping
   rendered images through an HTTP API.
2. **Unified contract.** CLI, GPUI, and HTTP endpoints share identical core library functions (`src/library.rs`),
   guaranteeing identical duplicate detection, stage derivation, and search ranking across all interfaces.

## Considered and declined

- **Automatic filing without operator review.** Declined: category proposals remain advisory; automatic
  filing risks misplacing sensitive documents and obscuring low-confidence classifications.
- **Filing personal documents into Obsidian.** Declined: Obsidian is reserved for subject knowledge notes;
  personal records and tax documents belong in Sjel's hash-addressed store and private overlay.
- **Closing the owner socket to agent sessions.** Declined: `documents` commands reach the running owner
  through a local socket that accepts any session holding the loopback credential, agent sessions
  included, so an agent can run imports, merges and splits for the operator. The HTTP agent gate does
  not apply there.
- **Admitting agent credentials to the whole Documents HTTP API.** Declined: autonomous coding agents and
  external tools must not have direct read access to personal financial, legal, or medical originals,
  extracted text or metadata routes. Only the duplicate candidate read is admitted.
- **Cloud-hosted OCR or classification endpoints.** Declined: documents are personal `c2` data by default;
  external transmission is prohibited.
- **Standalone CLM model runtime.** Declined: evaluated on synthetic test corpora and rejected due to high
  resource footprint and absence of grounded evidence spans; local classification uses Apple Foundation Models
  or keyword rules.
