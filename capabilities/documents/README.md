# documents

Sjel's local document library. Original bytes are stored once by SHA-256 under the active overlay; each source filename has a separate metadata record in the shared store, so byte-identical documents keep their names and filing history while sharing one blob. Intake is idempotent for the same bytes and filename. New documents start in `inbox`; a human must choose a category before a document becomes `filed`. Extraction is advisory and never moves or renames an original.

The `documents` capability owns document records and originals. `projects` remains the owner of project/task records and project attachments. A project label on a document is a reference only; this capability does not write into the projects store. The local CLI and authenticated operator UI use the same library. The HTTP server deliberately does not admit agent credentials, so raw document contents are not exposed through agent tools.

## Documents in the UI

Open **Documents** in Sjel's navigation, or use `sjel://documents` from the desktop app.
The operator must first have provisioned Sjel's shared inbound authentication with
`tools/setup-inbound-auth.sh`. After adding this service, restart `documents` and `sjel-status`
so the server loads the credential and the shell discovers its proxy route.
The collection searches filenames and extracted text, filters by status/category/workflow stage, and groups by category, project, year, status, or workflow stage. Stages are **Needs analysis**, **Review split**, **Split into children**, **Review enrichment**, **Ready to file**, **Filed**, or **Archived**. They are derived from page evidence, child records, the newest category-proposal run, and filing state, so they cannot drift after reanalysis. The CLI exposes the same queue with `documents next`.
Open a row to read the original PDF, or the extracted text of a PDF, Markdown, or text file.
Other formats offer the original download. Office previews and Office text extraction are
not implemented. The library keeps its last successful listing when a refresh fails.

**Add document** uploads up to 128 MiB into the inbox. Repeating an upload preserves its
record and filing decision. The reader's **Filing** controls set a category and optional
project label. Suggestions fill the category only when selected; **File document** commits
the decision. **Archive** retains the original; filing the record again restores it.

GPUI owns the native Documents view and talks to the Documents capability API. CLI and GPUI
use the same backend contracts for local analysis, page-range splits, search, and filing.

Classification offers deterministic rules and an explicit **Ask local model** action. The model
action sends a bounded excerpt only to the loopback `document_classification` inference role,
validates its category and exact evidence quote, and stores the result as a proposal for operator
review. It does not file the document. CLI equivalent: `documents suggest-model <id>`. A missing
or unavailable local role leaves the document unchanged; hosted OCR and hosted classification
are not used for this path.

| Operator API | Result |
| --- | --- |
| `GET /api/documents` | Metadata only, including a derived `workflow_stage`; `q` ranks local token-prefix matches across query words and returns matching page numbers; `status` filters and `limit`/`offset` paginate. |
| `GET /api/documents/{id}` | Metadata, locally extracted text, page evidence, review-only split suggestions, child records, and source-separated proposal records with review history. |
| `POST /api/documents/{id}/analyze` | Re-run local extraction and persist page text plus engine/failure provenance. Filing metadata and original bytes are preserved. |
| `GET /api/documents/{id}/children` | List stored child PDFs with exact source page ranges. |
| `POST /api/documents/{id}/split` | Store a standalone child PDF for each range in a complete, ordered partition. Repeating the same request is idempotent; a different partition is refused while children exist. |
| `GET /api/documents/{id}/original` | Hash-verified bytes for this record, including a stored child PDF; PDF may be inline, all other formats download. `download=true` forces download. |
| `POST /api/documents?filename=...` | Raw-byte upload, idempotent by bytes and filename; returns rule proposals separately from filing metadata. |
| `POST /api/documents/{id}/proposals/regenerate` | Generate and append a fresh rule-proposal run, even when its output matches an earlier run. |
| `POST /api/documents/{id}/proposals/model` | Run the configured loopback model on a bounded text excerpt; validate and append one proposal without filing. |
| `POST /api/documents/{id}/proposals/{proposal_id}/review` | Append `accepted`, `rejected`, or `overridden` to proposal history; does not file. |
| `PATCH /api/documents/{id}` | Operator-chosen `category` and optional `project`; marks the record filed. |
| `DELETE /api/documents/{id}` | Archive the record without deleting bytes. |

The server uses Sjel's credential-required loopback gate and foreign-origin refusal. Agent
tokens are not admitted. Document responses prohibit caching; originals cannot be served
without passing their digest check. Only a PDF with a matching file signature is served
inline. Other bytes are attachments with MIME sniffing disabled. Names suggesting confidential
material are refused before the upload body is read. Responses conservatively carry `c2` for
local-only handling. The explicit model-proposal route requires a loopback inference role and
uses Sjel's runtime admission checks; cloud endpoints are refused.

### Experimental CLM loopback adapter — trial closed, weights removed

The CLM trial was closed on 2026-10-25 and its local checkpoint deleted (4.5 GB) with the isolated
venv (380 MB). Two measurements closed it. On the 48-item comparison set the 4-bit MLX port scored
40/48 category suggestions with no evidence spans, while Apple Foundation Models scored 45/48 with
bounded evidence proposals, and `document_classification` already runs on that framework for zero
model bytes. No smaller CLM was ever published to move to: the authors released one checkpoint,
`CLM-v0.1-8B`, and the heads are trained on Qwen3-8B's 4096-wide embeddings, so another encoder
means retrained heads rather than a smaller download. At 4.4 GB it was also the largest set of
weights on a machine moving to smaller local models, for a surface that lost its own comparison.

The code below is kept as the re-open recipe, not as a live surface. Nothing here runs on its own
and nothing loads without a foreground `python`. Re-opening costs one model download and one venv
build:

```sh
dir="$HOME/.cache/sjel/models/clm-v0.1-8b-mlx-4bit"
venv="$HOME/.cache/sjel/clm-mlx-4bit-venv"
uv venv "$venv"
uv pip install --python "$venv/bin/python" mlx safetensors numpy huggingface_hub
"$venv/bin/python" - <<'PY'
from huggingface_hub import snapshot_download
snapshot_download("mlx-community/CLM-v0.1-8B-MLX-4bit",
                  revision="489b421bafc007e3ae409528d77006d26d362689",
                  local_dir="$HOME/.cache/sjel/models/clm-v0.1-8b-mlx-4bit")
PY
```

Until then the adapter refuses to start: it checks for `encoder/model.safetensors` and
`heads/CLM_v0.1-8B.safetensors` under `SJEL_DOCUMENTS_CLM_MODEL_DIR` before importing anything from
the model directory, so the missing weights are a named error rather than an import traceback, and
no code path re-downloads them. The eval corpora, the item-level results and the parity record stay
in `eval/` and in [`research/local-decision-models.md`](../../research/local-decision-models.md).

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
source evidence spans, so it remains an evaluation surface only and the Documents API and GPUI do
not call it. Hugging Face runs in offline mode. Stop the process manually when the trial ends.

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
used to tune prompts or rules. No real documents or `c3` content were sent during this evaluation.
Apple Foundation Models is now available through the explicit, review-only local model action;
the measured synthetic score is not a production accuracy guarantee. CLM remains evaluation-only.
Automatic filing remains disabled.

## Use

```sh
documents intake <file>
documents inbox <folder> [--limit N] [--dry-run] [--recursive] [--include-sensitive]
documents duplicates
documents next [--limit N]
documents watch <folder>
documents list [--status inbox|filed|archived]
documents search <words...> [--status inbox|filed|archived] [--limit N]
documents analyze <id>
documents suggest-splits <id>
documents suggest-model <id>
documents pages <id>
documents split <id> <ranges>
documents review <id> <category> [--project NAME]
documents archive <id>
documents export <id> <destination>
documents restore-index
documents verify
```

`documents next [--limit N] [--offset N]` lists up to 100 inbox records by newest import and labels the derived action. The limit accepts 1–500; offset pages through the queue. `Needs analysis` means local PDF extraction is missing or has a recorded page failure; `Review split` requires explicit page-one evidence; `Split into children` marks an original that already has stored children; `Review enrichment` means the newest proposal run contains an unreviewed category proposal; `Ready to file` means no split or category proposal is awaiting review. The command is read-only. Accepting a category proposal records the decision and does not file the document.

The CLI stores source files locally and extracts PDF/plain-text content through Sjel's registered extractor. Search ranks documents by matched query words and prints matching page numbers; it shows 50 results by default. `analyze` refreshes PDF page evidence locally. Repeating an analysis with unchanged output does not append another proposal run; changed extraction creates a new run. `pages` shows per-page engine, character count, and extraction status without printing document text. `suggest-splits` proposes ranges only when page text contains explicit page-one markers; the proposal includes marker evidence and never creates records. GPUI shows the same proposal with page evidence. Review or edit the ranges, then `split` writes one standalone child PDF into Sjel for each inclusive range, for example `documents split <id> 1-2,3-4`. Child records have their own hash-addressed bytes, retain page-specific text and provenance, and can be searched, filed, previewed, and exported independently. The imported source file remains unchanged. Extraction failures remain visible per page and do not lose the original. The active keyword-rule config can suggest any configured category; `?` marks a suggestion, not a filing decision. Ambiguous and unmatched documents remain in `inbox`. `review` records a human-chosen category and optional project label; it does not rewrite original bytes. `archive` marks a record without deleting it. `export` refuses to overwrite an existing destination.

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

For PDFs, xberg extracts page text and the local extraction layer records each page separately.
Pages below the existing 64 non-space-character estimate are OCR'd through Apple Vision `visocr`
in one pass; successful page text replaces only that page, while other native page text remains.
Page provenance and OCR failures are retained. `analyze` applies this to already imported PDFs.
`documents duplicates` reports exact SHA-256 copy groups as authoritative, plus
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
   Child documents must retain parent lineage and page ranges; every input page must be
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

### Intake to reviewed intelligence

The target workflow treats import, page analysis, splitting, enrichment, and filing as separate
resumable stages. Today, intake stores the original and extracts locally in the same operation;
reanalysis of existing records is operator-triggered with `analyze`. The `next` queue is a
read-only list of operator stages. A bounded background worker resumes local rule-proposal
backfill by finding records without a proposal for the active version; it stores per-version row
progress and retries safely. Intake creates proposals for new records idempotently. Extraction of
existing records and model enrichment still require an operator action. Any later analysis queue
should resume from the last completed stage and never create another source or child record on
retry.

1. **Inspect each page.** Record page count, text-layer availability, extracted text, layout
references, engine/version, and failures per page. Run OCR only for pages whose text layer is
missing or inadequate. Mixed PDFs must retain both their native text and OCR results, with page
provenance, instead of choosing one document-wide extraction path.
2. **Propose boundaries.** Use page signals such as separator pages, abrupt issuer/date changes,
and page numbering to propose complete page-range partitions. A model may rank or explain a cut,
but every proposed cut carries the pages and text that support it. OCR accuracy and boundary
accuracy have separate evaluation sets and acceptance criteria.
3. **Review the split.** Show the source page strip, extracted text, and proposed ranges together.
An operator can adjust the ranges and approve the partition. Children remain metadata records
pointing to the original hash and exact page ranges; the original remains recoverable. Enforce
ordered, non-overlapping ranges covering every page exactly once.
4. **Enrich each child independently.** Propose type, issuer, date, reference numbers, amounts,
summary, category, project, and follow-up candidates. Keep each proposal separate from operator
metadata and attach its page or source span, engine/model revision, confidence, and review state.
Low evidence and extraction errors stay visible; filing and cross-capability actions require
operator review.
5. **Surface useful work.** Group and filter by approved category, project, year, and status, then
show unresolved splits and evidence-backed follow-ups in the Documents review queue and relevant
Today/Ask surfaces. Suggestions can link to Finance, Calendar, or Tasks through their owner APIs
after approval; analysis never silently creates or changes those records.

Keep private documents local by default: `c2` cannot be sent to hosted OCR or inference, and `c3`
never enters a prompt. xberg is the current extraction baseline; first extend its adapter to retain
page-level results and measure it against mixed German/English scans. Evaluate Docling only as a
focused local experiment if that baseline cannot supply the layout or boundary evidence needed;
its prior rejection measured formula OCR yield and runtime, not segmentation quality. Mistral OCR
is a hosted option for explicitly eligible `c0`/`c1` material, subject to a separate provider
policy and measured comparison. Do not send private scans to it. Any automatic stage must pass
the frozen DE/EN evaluation and idempotent retry/restore checks before it can affect records.

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
URL. The experimental CLM adapter remains isolated from the Documents API. The Documents model
action uses the shared inference contract and persists evidence-bearing proposals. `c3` must never
enter a prompt. A future remote path requires an explicit data-policy change, not a backend
configuration toggle.

Each proposal must retain its rule/model identity, version where available, evidence span from the
filename or extracted text, confidence (or an explicit no-confidence value), and review outcome.
Confidence is not proof: low confidence, conflicting proposals, missing evidence, unavailable
models, malformed output, and extraction failure all remain visible and reviewable. Model
classification is optional; rules continue to work offline. Automatic filing is out of scope until
a frozen local evaluation set, measured threshold, and auditable reversal path are accepted.

The overlay category/rule configuration, typed proposal records, deterministic evidence-producing
rules, persistence, regeneration, append-only review events, restore sidecars, and explicit local
model proposals through the shared inference contract are implemented. The authenticated API, GPUI,
and CLI expose those actions. The legacy `suggested_category` field still uses the first matching
configured rule for compatibility. Filing remains a separate operator action. The existing synthetic
holdouts are spent; any prompt or rule changes need a fresh balanced German/English evaluation set.

Second local model decision (2026-10-22): evaluate Apple Foundation Models (`foundation-models`,
`apple-foundationmodel`) as a separate local proposal source, selected over Qwen3.5 to diversify
model families. `tools/model-check --local` found the model available through the existing
`summarization_light` role at evaluation time. Documents now has a dedicated
`document_classification` role and explicit local proposal call through `libs/inference`. The
production prompt uses the evaluated category guidance and evidence limits; returned categories and
exact source spans are validated again in code. The apfel schema subset rejects `minLength` with
HTTP 400, so non-empty and exact evidence remain application checks. This synthetic evaluation does
not guarantee accuracy on personal documents. The role remains replaceable through `libs/inference`,
keeps c2 text on the owner host, and excludes c3 from prompts.
[lopdf](https://github.com/J-F-Liu/lopdf) materializes operator-reviewed page ranges as
standalone PDFs. It does not infer boundaries; the direct dependency and its narrow role are
recorded in `upstreams.toml`.
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
   hash-verified bytes for the selected record from `GET /api/documents/{id}/original` and render
   them themselves, so a child displays its own stored pages. A rendering failure shows the extracted text and an explicit
   open-in-system-viewer action; it never shows an empty pane. Linux and Windows need a different
   rasterizer and are out of scope.
2. **Grouping uses the metadata a record already has** — category, project, year, status — before
   any new filing concept. Collections and folders are a later decision, taken only if grouping by
   metadata proves insufficient.
3. **A split writes child PDFs inside Sjel.** Each reviewed page range becomes a separate
   content-addressed blob with parent and page-range lineage. The source PDF remains unchanged,
   every source page belongs to exactly one child, and exports copy the already stored child.
4. **The duplicate report is consumed, never reimplemented.** `src/duplicates.rs` owns the report.
   This capability records what a human decided about it: a survivor with its absorbed records and
   source paths, a dismissal that does not come back, or a part that becomes a split.

Document workflow:

1. Import source bytes into Sjel's hash-addressed library. Intake and later operations never rename,
   move, split, or sort the import source.
2. Extract page text and provenance locally. Keep failures visible per page; do not treat a blank
   extraction as evidence that pages are blank.
3. Group records with category, project, year, and inbox status. Keep inferred categories and
   document facts as proposals until an operator accepts them.
4. Review a complete, ordered page partition in the GPUI reader. Each cut must be traceable to page
   evidence or an explicit visual review.
5. Store one standalone PDF per approved range inside Sjel. Keep the source PDF unchanged and link
   every child to its source id and exact page range.
6. Enrich and file each child using evidence from its own pages. Preserve source text and proposals
   so later revisions do not overwrite the operator's choices.

The OCR inference rung remains gated by the frozen DE/EN evaluation corpus. Until an engine clears
that gate, extraction failures stay visible and split ranges require direct operator review.
