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
| `GET /api/documents/{id}` | Metadata, locally extracted text, and source-separated proposal records with review history. |
| `GET /api/documents/{id}/original` | Hash-verified bytes; PDF may be inline, all other formats download. `download=true` forces download. |
| `POST /api/documents?filename=...` | Raw-byte upload, idempotent by bytes and filename; returns rule proposals separately from filing metadata. |
| `POST /api/documents/{id}/proposals/regenerate` | Generate and append a fresh rule-proposal run, even when its output matches an earlier run. |
| `POST /api/documents/{id}/proposals/{proposal_id}/review` | Append `accepted`, `rejected`, or `overridden` to proposal history; does not file. |
| `PATCH /api/documents/{id}` | Operator-chosen `category` and optional `project`; marks the record filed. |
| `DELETE /api/documents/{id}` | Archive the record without deleting bytes. |

The server uses Sjel's credential-required loopback gate and foreign-origin refusal. Agent
tokens are not admitted. Document responses prohibit caching; originals cannot be served
without passing their digest check. Only a PDF with a matching file signature is served
inline. Other bytes are attachments with MIME sniffing disabled. Names suggesting confidential
material are refused before the upload body is read. Responses conservatively carry `c2` for
local-only handling; the Documents API does not dispatch to a model.

### Experimental CLM loopback adapter

`clm_mlx_adapter.py` is a separate, manually started evaluation service. It binds only to
`127.0.0.1:8700`, serves the System One `/v1/systemone` shape used by the opt-in Pi classifier,
and loads the local MLX model into memory once. Set `SJEL_DOCUMENTS_CLM_MODEL_DIR` to the
installed model directory before launching it with the model's isolated Python environment. Set
`CLM_API_KEY` to require a bearer token; without it, the process still
accepts only requests without a browser `Origin` and on the IPv4 loopback host. It enforces a
128 KiB request-body limit and rejects any state or candidate over CLM's 2,048-token context.
Run it in the foreground for a trial; Ctrl-C stops it:

```sh
MODEL_DIR="$HOME/.cache/sjel/models/clm-v0.1-8b-mlx-4bit"
SJEL_DOCUMENTS_CLM_MODEL_DIR="$MODEL_DIR" \
  "$HOME/.cache/sjel/clm-mlx-4bit-venv/bin/python" capabilities/documents/clm_mlx_adapter.py
```

In another terminal, evaluate the synthetic corpus:

```sh
SJEL_SYSTEMONE_URL=http://127.0.0.1:8700/v1/ \
  "$HOME/.cache/sjel/clm-mlx-4bit-venv/bin/python" capabilities/documents/eval/clm_eval.py
```

The adapter does not write requests or responses, contact hosted inference, file documents, or
run as part of the Documents service. It has no data-class gate: send only the synthetic corpus
to it. Do not send real documents or any `c3` content. Its output contains category scores but no
source evidence spans, so it is an evaluation surface only; the Documents API and GPUI do not
call it. Hugging Face runs in offline mode. Stop the process manually when the trial ends.

`eval/clm-corpus.json` is the original frozen 12-item synthetic English/German set. On
2026-10-22, model revision `489b421bafc007e3ae409528d77006d26d362689` classified 9/12 examples
correctly (English 4/6, German 5/6). All three errors were Events examples; two were labeled
Finance and one Reference. The German conference sample was labeled Events, but its confidence
margin was only 0.0774. Its complete results remain in `eval/clm-eval-2026-10-22.json`.

`eval/clm-corpus-v2.json` broadens the test to 48 balanced synthetic examples: eight per language
and category, with clear and overlapping cases. Run `eval/clm_eval.py --corpus clm-corpus-v2.json`
only while the local adapter is running. The same model revision classified 32/48 correctly
(English 15/24, German 17/24). Category accuracy was Events 1/16, Finance 16/16, and Reference
15/16. Of the 16 Events cases, nine were labeled Finance and six Reference. Six wrong answers had
a confidence margin of at least 0.5; that margin is not calibrated, and 0.5 is only a descriptive
bucket, not a filing threshold. These synthetic results argue against integrating this CLM trial
for the current category set. Automatic filing remains disabled. Per-item results and the corpus
SHA-256 are recorded in `eval/clm-eval-v2-2026-10-22.json`.

The v2 set was used only for prompt development. Three Events descriptions were compared there;
`gathering-material` won at 39/48, with Events 10/16. A separate balanced 48-item holdout was
frozen before that comparison; CLM was evaluated once on it with the selected description. It
returned 36/48
(English 18/24, German 18/24): Events 9/16, Finance 12/16, Reference 15/16. The candidate scores
and selection rule are in `eval/clm-prompt-tuning-v2-2026-10-22.json`; the holdout's per-item
results and SHA-256 are in `eval/clm-eval-holdout-v1-2026-10-22.json`. The prompt improved the
development score, but the holdout still has 7/16 Events errors and 4/16 Finance errors. It is
not ready for Documents integration. The holdout has also been used for the rules baseline; it
must not be used to tune another prompt or rule set. A new holdout is needed for another tuning
round.

The current keyword rules were measured separately by `current_keyword_rules_have_frozen_synthetic_baselines`
in `src/library.rs`, using each synthetic text with a neutral `synthetic.txt` filename. On the
development set they proposed 16/48 categories: 14 correct, 2 wrong, 32 unresolved. On the holdout
they proposed 10/48: 9 correct, 1 wrong, 38 unresolved. They found 2/3 Events cases on development
and 0/1 on holdout. This is a high-precision, low-coverage baseline, not an evidence-bearing rule
system; the current reason text does not preserve the matched source span. Metrics and corpus hashes
are in `eval/rule-baseline-v2-holdout-v1-2026-10-22.json`.

### Cross-source synthetic evaluation (2026-10-22)

After fixing the CLM and Apple FM prompts, all three sources were evaluated on the fresh,
balanced 48-item `eval/clm-corpus-combined-holdout-v1.json` set. It contains eight examples per
language and category. The prompts and rules were fixed before this set was evaluated.

| Source | Category output | Evidence-valid proposals |
|---|---:|---:|
| Keyword rules | 13/48; 11 correct, 2 wrong | 0; the current rule reason has no source span |
| CLM, 4-bit MLX | 40/48 correct | 0; it returns category scores, not evidence |
| Apple Foundation Models, local `apfel` | 47/48 category outputs correct | 45/48; all 45 correct |

Apple FM returned no confidence. One synthetic tax-summary request was blocked by Apple's safety
guardrails and recorded as no proposal; it was not rephrased or retried. Two other Apple outputs
were unusable under the operator-selected evidence limit of 20 words and 160 characters: one quote
was too long and one was not an exact source span.

The separate category outputs matched on 9/13 rule–CLM cases, 11/13 rule–Apple cases, and 39/47
CLM–Apple cases. At least one source returned the correct category on all 48 examples, but only 45 had a
correct evidence-valid proposal. This is not an ensemble accuracy score: the sources were not
merged, and CLM/rules do not yet satisfy the evidence contract. Prompt selection is recorded in `eval/clm-prompt-tuning-v2-2026-10-22.json` and
`eval/apple-fm-prompt-tuning-hybrid-v1-2026-10-22.json`. The comparison report is
`eval/cross-source-eval-combined-holdout-v1-2026-10-22.json`; individual results are in the
corresponding CLM, Apple FM, and rules reports. This set is now spent for comparison and must not be
used to tune prompts or rules. No real documents or `c3` content were sent, and none of these
sources is wired to the Documents API or GPUI. Automatic filing remains disabled.

## Use

```sh
documents intake <file>
documents inbox <folder> [--limit N] [--dry-run] [--recursive] [--include-sensitive]
documents duplicates
documents watch <folder>
documents list [--status inbox|filed|archived]
documents search <words...>
documents review <id> <category> [--project NAME]
documents archive <id>
documents export <id> <destination>
documents restore-index
documents verify
```

The CLI stores the original locally and extracts PDF/plain-text content through Sjel's registered extractor when available. It records extractor provenance; an extraction failure does not lose the original or prevent review. Extracted text is retained locally for later search and filing suggestions, but is never emitted by the CLI. The active keyword-rule config can suggest any configured category; the replaceable defaults are `Finance`, `Events`, and `Reference`. `?` marks a suggestion, not a filing decision. Ambiguous and unmatched documents remain in `inbox`. `search` matches filenames, extracted titles and local extracted text; it prints only matching filenames and metadata. `review` records a human-chosen category and optional project label; it does not rewrite original bytes. `archive` marks a record without deleting it. `export` refuses to overwrite an existing destination.

## Folder inbox

`inbox` scans one directory by default; `--recursive` includes its subdirectories. It selects
the latest 100 supported document files by modification time, or the count given by `--limit`.
Plain text, Markdown and CSV are not document files here: `capabilities/import` routes them
to their owners (decision 2026-10-23, `capabilities/import/README.md`).
Equal timestamps are ordered by filename. Selection happens before exclusions; skipped files
are never replaced by older ones. `--dry-run` reads directory metadata only and does not open
the document store.

Before reading bytes, folder intake excludes hidden names, control characters, and names
containing sensitive terms such as `secret`, `password`, `credential`, `token`, `private`,
`confidential`, `vertraulich`, `geheim`, or identity, banking, tax, payroll, and medical terms.
Excluded names are reported as a count only. `--include-sensitive` opts in to those filenames;
the screen stays the default. This is a conservative filename filter, not a content
classification or a guarantee that the remaining files contain no private material.
Symlinks and subdirectories are ignored. Empty files, files over 128 MiB, and files modified
within 30 seconds are deferred. Intake checks file identity and metadata again before and
after a bounded read and refuses changed files.

`watch` polls every 60 seconds while the command runs; Ctrl-C stops it. It scans all supported
files, so a batch larger than the one-shot default does not starve older uploads. Repeated
imports reuse existing records and do not undo filing decisions. Individual import failures
are reported and retried on the next scan; a directory scan failure exits the watcher.
The command installs no background service. Put only intended uploads in the watched folder.
Folder intake retains each original source and records its path and last-seen time in the
`documents_sources` table. Repeated sightings update that receipt without creating another
receipt row. Office and other supported document containers can be stored without extracted
text; the current extractor registration reads PDF and plain text only. Neither command
generates summaries or files suggestions automatically.

For PDFs, xberg reads an existing text layer first. If fewer than 64 non-space characters are
available, the extraction ladder sends the whole PDF to local Apple Vision OCR through `visocr`;
recognized pages are joined with form feeds. The 64-character cutoff is an estimate, and mixed
text-and-scan PDFs currently keep only the text-layer result rather than OCRing missing pages.
On hosts without Apple Vision or `visocr`, the original is still retained and extraction may be
unavailable. `documents duplicates` reports exact SHA-256 copy groups as authoritative, plus
probable rescans (word-triple Jaccard ≥ 0.6) and split parts (text containment ≥ 0.8) for human
review. These thresholds are estimates, not measured classifiers; the command changes nothing.

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

### Classification contract — decisions (2026-10-22)

The first classifier release proposes categories only. It does not write filing metadata or
change a document's state. Every intake remains `inbox` until an operator explicitly files it.
Rules and model results remain separate proposals so the UI and future evaluation can show
agreement and conflict instead of hiding one behind the other.

Categories come from an explicit operator-configured list in the private overlay. A proposal must
match that list; an unknown or ambiguous result stays unresolved. The optional
`<overlay>/config/documents-classification.json` file replaces the complete built-in category and
keyword-rule set; it does not merge with defaults. It contains a rule-set `version`, an explicit
`categories` list, and `rules` with unique `id`, `category`, and `keywords` fields. If the file is
absent, replaceable built-in rules preserve the current `Finance`, `Events`, and `Reference`
suggestions. The active configuration is loaded when Documents opens. If the file exists but is
unreadable or invalid, the capability fails closed instead of silently using defaults. Filed
categories remain free-form and are not migrated or constrained by this proposal config. Do not
put personal category names or rules in repository fixtures, logs, or public config. The
experimental adapter accepts caller-supplied System One questions; it is not the Documents
category configuration or proposal API.

Document contents and derived proposals are `c2`. Classification may use a configured inference
role only when its endpoint is loopback on the document-owning host. Do not send document text,
filenames, or derived personal facts to cloud endpoints or remotely hosted inference APIs. Model
weights may come from NVIDIA, Hugging Face, or another source and still be used if the model runs
on the document-owning host. The operator selected
[`mlx-community/CLM-v0.1-8B-MLX-4bit`](https://huggingface.co/mlx-community/CLM-v0.1-8B-MLX-4bit)
for a local trial. The 12-item synthetic DE/EN evaluation on 2026-10-22 scored 9/12 overall
(English 4/6, German 5/6); all three misclassifications were Events items. This is not enough to
rely on the model, and the sample is too small to set a threshold. Model families and serving
stacks remain replaceable through `libs/inference`; Documents must not hardcode a provider or
URL. The current adapter is isolated from the Documents API pending that shared inference
contract and evidence-bearing proposal records. `c3` must never enter a prompt. A future remote
path requires an explicit data-policy change, not a backend configuration toggle.

Each proposal must retain its rule/model identity, version where available, evidence span from the
filename or extracted text, confidence (or an explicit no-confidence value), and review outcome.
Confidence is not proof: low confidence, conflicting proposals, missing evidence, unavailable
models, malformed output, and extraction failure all remain visible and reviewable. Model
classification is optional; rules continue to work offline. Automatic filing is out of scope until
a frozen local evaluation set, measured threshold, and auditable reversal path are accepted.

Implementation order: (1) define the overlay category/rule configuration and typed proposal
record without changing filing; (2) add evidence-producing deterministic rules and tests; (3) use
the existing development corpus for iteration, then freeze a new balanced German/English holdout
before measuring changed rules or prompts; existing holdouts are spent; (4) add a local-only
classification role and structured-output validation through the shared inference contract; (5)
expose proposals and review outcomes through the operator API and UI. Steps 1–2 exist in
`src/classification.rs`. Rule proposals are stored separately from filing metadata. Each proposal
has an id scoped to its generation. Opening the library backfills the active rule version only
when that version has no stored run; an explicit regenerate request always creates a new run,
even if its output matches an earlier one. Review outcomes are append-only events, and the latest
outcome is projected onto the proposal for display. A dedicated private classification sidecar v2
preserves proposals and review events for `restore-index`. The authenticated API exposes stored
records, regeneration, and review; the UI does not display them yet. The legacy `suggested_category`
field uses the first matching configured rule for compatibility. Filing remains an explicit,
separate operator action. The current inference library resolves roles and enforces runtime/cloud
policy, but does not yet expose a general classification call; add that through its owning contract
rather than dispatching ad hoc from Documents.

Second local model decision (2026-10-22): evaluate Apple Foundation Models (`foundation-models`,
`apple-foundationmodel`) as a separate local proposal source, selected over Qwen3.5 to diversify
model families. `tools/model-check --local` found the model available through the existing
`summarization_light` role, but Documents has no classification role or call yet. Synthetic
JSON-schema chat-completion probes returned valid JSON on 2026-10-22. The apfel schema subset
rejects `minLength` with HTTP 400, so the evaluation script uses supported constraints and checks
non-empty, exact evidence in code. This verifies one local request path, not full schema enforcement,
availability behavior, or classification accuracy. Any Documents role must remain replaceable
through `libs/inference`, keep c2 text on the owner host, and exclude c3 from prompts.
[lopdf's page extraction](https://github.com/J-F-Liu/lopdf/blob/main/pdfutil/README.md) is a
candidate for materializing reviewed page ranges in Rust, not a boundary detector. It is
already present transitively in the lockfile; no direct dependency or splitter is adopted here.
[Paperless-ngx workflows](https://docs.paperless-ngx.com/usage/#workflows) remain useful prior
art for metadata automation under the existing `upstreams.toml` inspiration verdict.

## Data and backup

Canonical files are in `<overlay>/data/documents/blobs/`, mode `0600`; metadata and extracted text use the `documents_` tables in the shared SQLite store. A private JSON sidecar for each document is written under `records/` as a recoverable copy of its metadata. Classification proposal and review history use a separate v2 sidecar under `records/classification/`; both sidecar trees are included in the self-contained `documents` backup on the deployment's private `documents-ssd` target. `restore-index` reconstructs missing shared-store rows from verified sidecars and refuses conflicting rows. Backup readiness remains open until an archive and full restore are verified.

Document names, extracted text, and project labels are personal data. Keep them in the overlay and its backup; do not add them to repository fixtures, logs, or public indexes. The `documents-ssd` target's path is private overlay configuration and must be writable by the Sjel operator.

## Native reader and review decisions (2026-10-22)

The GPUI client owns this surface. The dashboard page retires when the native reader covers it, and
no rule that both Apple clients share may live only in the dashboard. Four decisions settle the
shape, and the phases are ordered so the first one cannot collide with concurrent work in
`src/library.rs` and `src/duplicates.rs`.

1. **PDF pages render in-process on the client.** A dedicated module carries
   `#![allow(unsafe_code)]` with its reason in the header, as `apps/gpui/src/macos_glass.rs` does,
   and draws each page through CGPDF/PDFKit into a GPUI `RenderImage`. Clients fetch the
   hash-verified original from `GET /api/documents/{id}/original` and render it themselves, so the
   iPhone reads documents offline. A rendering failure shows the extracted text and an explicit
   open-in-system-viewer action; it never shows an empty pane. Linux and Windows need a different
   rasterizer and are out of scope.
2. **Grouping uses the metadata a record already has** — category, project, year, status — before
   any new filing concept. Collections and folders are a later decision, taken only if grouping by
   metadata proves insufficient.
3. **A split writes metadata, not bytes.** A child carries the parent's SHA-256 and a page range;
   the unsplit original stays openable and every input page is accounted for. A child PDF is
   materialized only when one is exported or sent, so no orphaned blobs are created by a wrong
   boundary.
4. **The duplicate report is consumed, never reimplemented.** `src/duplicates.rs` owns the report.
   This capability records what a human decided about it: a survivor with its absorbed records and
   source paths, a dismissal that does not come back, or a part that becomes a split.

Phase order:

1. Reader parity in `apps/gpui/**` only: original bytes through the shell, the page renderer, page
   strip and zoom, the text pane, and filing with an operator-typed category and project. Adds no
   route and touches nothing under `src/`.
2. Decisions in new modules — `collections`, `merge`, `split`, `summary` — with an `IF NOT EXISTS`
   migration, one route each, and sidecar v2 so `restore-index` keeps them. UI: the duplicates
   queue with side-by-side compare, the split editor over the page strip, and the metadata panel.
3. Summaries and proposals are stored and reviewable as their own records: engine identity,
   revision, evidence page, confidence, review outcome. Rule and model proposals stay separate
   rows, an operator value is never overwritten by a proposal, and nothing files automatically.
   Generation stays blocked until a loopback inference role exists; the pane says no summary was
   generated rather than showing a substituted one.

Out of the first release: PDF text selection and in-page search from the page image (the extracted
text pane carries search), printing, and materialized child PDFs.
