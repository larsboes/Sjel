# documents

Sjel's local document library. Original bytes are stored once by SHA-256 under the active overlay; each source filename has a separate metadata record in the shared store, so byte-identical documents keep their names and filing history while sharing one blob. Intake is idempotent for the same bytes and filename. New documents start in `inbox`; a human must choose a category before a document becomes `filed`. Extraction is advisory and never moves or renames an original.

The `documents` capability owns document records and originals. `projects` remains the owner of project/task records and project attachments. A project label on a document is a reference only; this capability does not write into the projects store. The local CLI and authenticated operator UI use the same library. The HTTP server deliberately does not admit agent credentials, so raw document contents are not exposed through agent tools.

## Documents in the UI

Open **Documents** in Sjel's navigation, or use `sjel://documents` from the desktop app.
The operator must first have provisioned Sjel's shared inbound authentication with
`tools/setup-inbound-auth.sh`. After adding this service, restart `documents` and `sjel-status`
so the server loads the credential and the shell discovers its proxy route.
The collection searches filenames and extracted titles and filters by status or category.
Open a row to read the original PDF, or the extracted text of a PDF, Markdown, or text file.
Other formats offer the original download. Office previews and Office text extraction are
not implemented. The library keeps its last successful listing when a refresh fails.

**Add document** uploads up to 128 MiB into the inbox. Repeating an upload preserves its
record and filing decision. The reader's **Filing** controls set a category and optional
project label. Suggestions fill the category only when selected; **File document** commits
the decision. **Archive** retains the original; filing the record again restores it.

The dashboard owns presentation at `/documents`; the capability owns `/api/documents`.
`proxy_api_only` keeps the page and API separate. The desktop destination currently opens
this dashboard page; it is not a native GPUI reader. The older Tauri shell can read through
its existing binary bridge; raw file uploads require the browser UI.

| Operator API | Result |
| --- | --- |
| `GET /api/documents` | Metadata only; `q` searches local extracted text, `status` filters, and `limit`/`offset` paginate. |
| `GET /api/documents/{id}` | Metadata and locally extracted text for the reader. |
| `GET /api/documents/{id}/original` | Hash-verified bytes; PDF may be inline, all other formats download. `download=true` forces download. |
| `POST /api/documents?filename=...` | Raw-byte upload, idempotent by bytes and filename. |
| `PATCH /api/documents/{id}` | Operator-chosen `category` and optional `project`; marks the record filed. |
| `DELETE /api/documents/{id}` | Archive the record without deleting bytes. |

The server uses Sjel's credential-required loopback gate and foreign-origin refusal. Agent
tokens are not admitted. Document responses prohibit caching; originals cannot be served
without passing their digest check. Only a PDF with a matching file signature is served
inline. Other bytes are attachments with MIME sniffing disabled. Names suggesting confidential
material are refused before the upload body is read. Responses conservatively carry `c2` for
local-only handling; there is no cloud or model dispatch on this surface.

## Use

```sh
documents intake <file>
documents inbox <folder> [--limit N] [--dry-run]
documents watch <folder>
documents list [--status inbox|filed|archived]
documents search <words...>
documents review <id> <category> [--project NAME]
documents archive <id>
documents export <id> <destination>
documents restore-index
documents verify
```

The CLI stores the original locally and extracts PDF/plain-text content through Sjel's registered extractor when available. It records extractor provenance; an extraction failure does not lose the original or prevent review. Extracted text is retained locally for later search and filing suggestions, but is never emitted by the CLI. Filename/text keyword matches can suggest `Finance`, `Events`, or `Reference`; `?` marks a suggestion, not a filing decision. Ambiguous and unmatched documents remain in `inbox`. `search` matches filenames, extracted titles and local extracted text; it prints only matching filenames and metadata. `review` records a human-chosen category and optional project label; it does not rewrite original bytes. `archive` marks a record without deleting it. `export` refuses to overwrite an existing destination.

## Folder inbox

`inbox` scans one directory without recursion. It selects the latest 100 supported document
files by modification time, or the count given by `--limit`. Equal timestamps are ordered by
filename. Selection happens before exclusions; skipped files are never replaced by older ones.
`--dry-run` reads directory metadata only and does not open the document store.

Before reading bytes, folder intake excludes hidden names, control characters, and names
containing sensitive terms such as `secret`, `password`, `credential`, `token`, `private`,
`confidential`, `vertraulich`, `geheim`, or identity, banking, tax, payroll, and medical terms.
Excluded names are reported as a count only. This is a conservative filename filter, not a
content classification or a guarantee that the remaining files contain no private material.
Symlinks and subdirectories are ignored. Empty files, files over 128 MiB, and files modified
within 30 seconds are deferred. Intake checks file identity and metadata again before and
after a bounded read and refuses changed files.

`watch` polls every 60 seconds while the command runs; Ctrl-C stops it. It scans all supported
files, so a batch larger than the one-shot default does not starve older uploads. Repeated
imports reuse existing records and do not undo filing decisions. Individual import failures
are reported and retried on the next scan; a directory scan failure exits the watcher.
The command installs no background service. Put only intended uploads in the watched folder.
Both commands retain source files and keep extraction local. Office and other supported
document containers can be stored without extracted text; the current extractor registration
reads PDF and plain text only. Neither command generates summaries or files suggestions
automatically.

## Why this shape: document intelligence belongs with its originals

Sjel owns every document and all material about the operator's personal life, including facts,
summaries, and insights derived from those documents. Obsidian is solely a knowledge base for
learning about subjects such as mathematics, science, and business. Document summaries,
personal records, and life administration never become Obsidian notes as a filing operation.
General subject learning can be written there deliberately; its source documents remain in Sjel.
Folder intake and UI uploads use one library. Mail attachments should use that same contract.

The next stages need evidence before unattended use:

1. **Source receipts.** Retain source kind, stable source identity, original filename, and
   import time. A mail receipt needs account, message, and attachment identity so a retry
   cannot duplicate an attachment or erase its origin. Content hashes continue to own blobs.
2. **Page extraction and segmentation.** Extract text and layout per page, recording the
   engine and failures. Start with explicit page ranges or separator pages. A model may
   propose boundaries for mixed scans, but a boundary is distinct from writing a PDF.
   Child documents must retain the parent hash and page ranges; every input page must be
   accounted for, and the unsplit original must remain recoverable.
3. **Evidence-backed enrichment.** Store document type, issuer, date, summary, key facts, and
   proposed actions separately from originals. Each fact needs a page or source span,
   extractor/model identity, and evaluation status. Extraction failures and insufficient
   evidence must remain visible rather than becoming plausible generated facts.
4. **Model policy.** Reuse `libs/inference` roles and Sjel's data classes. Unknown documents
   start local-only. `c2` stays local; `c3` never enters a prompt. Cloud enrichment requires
   an allowed `c0`/`c1` classification and reviewed provider policy. Evaluate German/English
   extraction, classification, and boundary detection separately before automatic filing.
5. **Reviewed domain links.** Invoice candidates can be offered to Finance and event candidates
   to Calendar through each owner's reviewed contract. Personal insights remain in Sjel.
   A document category does not itself authorize a journal entry, calendar event, or task.

Reuse `libs/extraction` for the existing xberg PDF and plain-text paths and its evaluated OCR
ladder. Upstream xberg documents Office extraction, but Sjel's current registered classes do
not expose it; support needs local fixtures and explicit failure reporting before expansion.
[lopdf's page extraction](https://github.com/J-F-Liu/lopdf/blob/main/pdfutil/README.md) is a
candidate for materializing reviewed page ranges in Rust, not a boundary detector. It is
already present transitively in the lockfile; no direct dependency or splitter is adopted here.
[Paperless-ngx workflows](https://docs.paperless-ngx.com/usage/#workflows) remain useful prior
art for metadata automation under the existing `upstreams.toml` inspiration verdict.

## Data and backup

Canonical files are in `<overlay>/data/documents/blobs/`, mode `0600`; metadata and extracted text use the `documents_` tables in the shared SQLite store. A private JSON sidecar for each document is written under `records/` as a recoverable copy of its metadata. The self-contained `documents` backup includes originals and sidecars on the deployment's private `documents-ssd` target. `restore-index` reconstructs missing shared-store rows from verified sidecars and refuses conflicting rows. Backup readiness remains open until an archive and full restore are verified.

Document names, extracted text, and project labels are personal data. Keep them in the overlay and its backup; do not add them to repository fixtures, logs, or public indexes. The `documents-ssd` target's path is private overlay configuration and must be writable by the Sjel operator.
